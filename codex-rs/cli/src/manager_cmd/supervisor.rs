use super::ManagerDaemonArgs;
use super::ManagerPromptDelivery;
use super::ManagerWatchArgs;
use super::runtime::ManagerRuntime;
use super::runtime::current_timestamp;
use super::runtime::status_hint;
use anyhow::Context;
use anyhow::bail;
use std::collections::BTreeMap;
use std::io::Write;
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
            self.run_supervisor_iteration(&options, &mut previous_statuses, writer)?;
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
        options: &SupervisorOptions<'_>,
        previous_statuses: &mut BTreeMap<String, String>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let pings = self.pings()?;
        if pings.is_empty() {
            writeln!(writer, "pings=none")?;
        } else {
            for ping in pings {
                writeln!(writer, "ping={}", ping.display())?;
            }
        }

        for agent in self.workspace.active_agents() {
            let status = match self.capture(&agent.agent_id, options.lines) {
                Ok(output) => status_hint(&output).to_string(),
                Err(err) => {
                    writeln!(writer, "{}\terror\t{err:#}", agent.agent_id)?;
                    if options.restart_missing {
                        self.start_agent_by_ref(agent, writer)?;
                        "restarted".to_string()
                    } else {
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
            writeln!(writer, "{}\t{}", agent.agent_id, status)?;
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
}
