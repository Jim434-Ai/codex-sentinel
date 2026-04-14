use super::ManagerDaemonArgs;
use super::ManagerPromptDelivery;
use super::ManagerWatchArgs;
use super::runtime::ManagerRuntime;
use super::runtime::current_timestamp;
use super::runtime::status_hint;
use super::workspace::resolve_user_path;
use anyhow::Context;
use anyhow::bail;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

impl ManagerRuntime {
    pub(crate) fn watch<W: Write>(
        &self,
        args: &ManagerWatchArgs,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let options = SupervisorOptions {
            mode: "watch",
            lines: args.lines,
            interval_seconds: args.interval_seconds,
            iterations: args.iterations,
            start_active: args.start_active,
            restart_missing: false,
            restart_errors: false,
            auto_nudge: false,
            nudge_prompt: "",
            status_file: None,
        };
        self.run_supervisor_loop(options, writer)
    }

    pub(crate) fn daemon<W: Write>(
        &self,
        args: &ManagerDaemonArgs,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let options = SupervisorOptions {
            mode: "daemon",
            lines: args.lines,
            interval_seconds: args.interval_seconds,
            iterations: args.iterations,
            start_active: !args.no_start_active,
            restart_missing: args.restart_missing,
            restart_errors: args.restart_errors,
            auto_nudge: args.auto_nudge,
            nudge_prompt: &args.nudge_prompt,
            status_file: if args.no_status_file {
                None
            } else {
                Some(args.status_file.clone())
            },
        };
        self.run_supervisor_loop(options, writer)
    }

    pub(crate) fn prompt_agent<W: Write>(
        &self,
        agent_id: &str,
        prompt: &str,
        delivery: ManagerPromptDelivery,
        lines: usize,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            bail!("prompt must not be empty");
        }

        let agent = self.workspace.agent(agent_id)?;
        let target = self.tmux_target_for(&agent.session)?;
        let resolved_delivery = match delivery {
            ManagerPromptDelivery::Auto => {
                let output = self.capture(agent_id, lines).unwrap_or_default();
                if status_hint(&output) == "working" {
                    ManagerPromptDelivery::Stage
                } else {
                    ManagerPromptDelivery::Submit
                }
            }
            ManagerPromptDelivery::Stage => ManagerPromptDelivery::Stage,
            ManagerPromptDelivery::Submit => ManagerPromptDelivery::Submit,
        };

        let buffer_name = format!("codex-manager-prompt-{}", agent.agent_id);
        let set_output = self
            .tmux_command()
            .arg("set-buffer")
            .arg("-b")
            .arg(&buffer_name)
            .arg(prompt)
            .output()
            .context("stage manager prompt in tmux buffer")?;
        if !set_output.status.success() {
            bail!(
                "tmux set-buffer failed: {}",
                super::runtime::stderr_text(&set_output)
            );
        }

        let paste_output = self
            .tmux_command()
            .arg("paste-buffer")
            .arg("-b")
            .arg(&buffer_name)
            .arg("-t")
            .arg(&target)
            .output()
            .context("paste manager prompt into specialist tmux pane")?;
        if !paste_output.status.success() {
            bail!(
                "tmux paste-buffer failed: {}",
                super::runtime::stderr_text(&paste_output)
            );
        }

        if matches!(resolved_delivery, ManagerPromptDelivery::Submit) {
            let send_output = self
                .tmux_command()
                .arg("send-keys")
                .arg("-t")
                .arg(&target)
                .arg("Enter")
                .output()
                .context("submit manager prompt in specialist tmux pane")?;
            if !send_output.status.success() {
                bail!(
                    "tmux send-keys failed: {}",
                    super::runtime::stderr_text(&send_output)
                );
            }
        }

        self.log_event(format!(
            "prompted {} via tmux session {} delivery {:?}",
            agent.agent_id, agent.session, resolved_delivery
        ))?;
        writeln!(
            writer,
            "Prompt delivered to {} on tmux socket {} session {} with delivery {:?}.",
            agent.agent_id, self.tmux_socket, agent.session, resolved_delivery
        )?;
        if matches!(resolved_delivery, ManagerPromptDelivery::Stage) {
            writeln!(
                writer,
                "Prompt was staged only; the specialist session has not been interrupted."
            )?;
        }
        Ok(())
    }

    fn run_supervisor_loop<W: Write>(
        &self,
        options: SupervisorOptions<'_>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        if options.start_active {
            self.start_active(writer)?;
        }

        let mut previous_statuses = BTreeMap::new();
        let mut iteration = 0_u64;
        loop {
            iteration += 1;
            writeln!(
                writer,
                "== manager {} iteration {} ({}) ==",
                options.mode,
                iteration,
                current_timestamp()
            )?;
            self.run_supervisor_iteration(iteration, &options, &mut previous_statuses, writer)?;
            writer.flush().context("flush manager supervisor output")?;

            if let Some(iterations) = options.iterations
                && iteration >= u64::from(iterations)
            {
                break;
            }
            thread::sleep(Duration::from_secs(options.interval_seconds));
        }
        Ok(())
    }

    fn run_supervisor_iteration<W: Write>(
        &self,
        iteration: u64,
        options: &SupervisorOptions<'_>,
        previous_statuses: &mut BTreeMap<String, String>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let updated_at = current_timestamp();
        let pings = self.pings()?;
        if pings.is_empty() {
            writeln!(writer, "pings=none")?;
        } else {
            for ping in pings {
                writeln!(writer, "ping={}", ping.display())?;
            }
        }

        let mut current_statuses = Vec::new();
        for agent in self.workspace.active_agents() {
            let mut action = "none".to_string();
            let status = match self.capture(&agent.agent_id, options.lines) {
                Ok(output) => {
                    let status = status_hint(&output).to_string();
                    if status == "error" && options.restart_errors {
                        self.restart_agent(&agent.agent_id, writer)?;
                        action = "restarted-error".to_string();
                    } else if status == "waiting-at-prompt" && options.auto_nudge {
                        let prompt = format!(
                            "{}\n\nApproved current objective: {}",
                            options.nudge_prompt, agent.current_objective
                        );
                        self.prompt_agent(
                            &agent.agent_id,
                            &prompt,
                            ManagerPromptDelivery::Submit,
                            options.lines,
                            writer,
                        )?;
                        action = "auto-nudged".to_string();
                    }
                    status
                }
                Err(err) => {
                    writeln!(writer, "{}\terror\t{err:#}", agent.agent_id)?;
                    if options.restart_missing {
                        self.start_agent_by_ref(agent, writer)?;
                        action = "restarted-missing".to_string();
                        "missing".to_string()
                    } else {
                        action = "missing".to_string();
                        "missing".to_string()
                    }
                }
            };

            let previous = previous_statuses.insert(agent.agent_id.clone(), status.clone());
            if previous.as_deref() != Some(status.as_str()) {
                let previous_label = previous.as_deref().unwrap_or("new");
                self.log_event(format!(
                    "status change {}: {} -> {}",
                    agent.agent_id, previous_label, status
                ))?;
            }
            writeln!(writer, "{}\t{}\t{}", agent.agent_id, status, action)?;
            current_statuses.push(SupervisorAgentStatus {
                agent_id: agent.agent_id.clone(),
                status,
                action,
                current_objective: agent.current_objective.clone(),
            });
        }

        if let Some(status_file) = options.status_file.as_ref() {
            let status_path = resolve_user_path(&self.workspace.root, status_file);
            if let Some(parent) = status_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("create supervisor status dir {}", parent.display())
                })?;
            }
            fs::write(
                &status_path,
                supervisor_status_tsv(&updated_at, options.mode, iteration, &current_statuses),
            )
            .with_context(|| format!("write supervisor status file {}", status_path.display()))?;
            writeln!(writer, "status_file={}", status_path.display())?;
        }

        Ok(())
    }
}

struct SupervisorOptions<'a> {
    mode: &'a str,
    lines: usize,
    interval_seconds: u64,
    iterations: Option<u32>,
    start_active: bool,
    restart_missing: bool,
    restart_errors: bool,
    auto_nudge: bool,
    nudge_prompt: &'a str,
    status_file: Option<PathBuf>,
}

struct SupervisorAgentStatus {
    agent_id: String,
    status: String,
    action: String,
    current_objective: String,
}

fn supervisor_status_tsv(
    updated_at: &str,
    mode: &str,
    iteration: u64,
    statuses: &[SupervisorAgentStatus],
) -> String {
    let mut output =
        "updated_at\tmode\titeration\tagent_id\tstatus\taction\tcurrent_objective\n".to_string();
    for status in statuses {
        output.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            tsv_field(updated_at),
            tsv_field(mode),
            iteration,
            tsv_field(&status.agent_id),
            tsv_field(&status.status),
            tsv_field(&status.action),
            tsv_field(&status.current_objective)
        ));
    }
    output
}

fn tsv_field(value: &str) -> String {
    value.replace(['\t', '\n', '\r'], " ")
}

#[cfg(test)]
mod tests {
    use super::SupervisorAgentStatus;
    use super::supervisor_status_tsv;
    use pretty_assertions::assert_eq;

    #[test]
    fn supervisor_status_tsv_sanitizes_fields() {
        let rows = vec![SupervisorAgentStatus {
            agent_id: "agent\t003".to_string(),
            status: "waiting-at-prompt".to_string(),
            action: "auto-nudged".to_string(),
            current_objective: "line one\nline two".to_string(),
        }];

        assert_eq!(
            supervisor_status_tsv("2026-04-14 10:00 EDT", "daemon", 7, &rows),
            "updated_at\tmode\titeration\tagent_id\tstatus\taction\tcurrent_objective\n\
             2026-04-14 10:00 EDT\tdaemon\t7\tagent 003\twaiting-at-prompt\tauto-nudged\tline one line two\n"
        );
    }
}
