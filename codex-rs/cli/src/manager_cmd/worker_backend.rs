use super::runtime::ManagerRuntime;
use super::runtime::stderr_text;
use anyhow::Context;
use anyhow::bail;
use codex_specialist::SpecialistWorkerCommand;
use codex_specialist::SpecialistWorkerEvent;
use codex_specialist::decode_worker_event;
use codex_specialist::encode_worker_command;
use std::io::Write;
use std::process::Command;
use std::process::Stdio;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

impl ManagerRuntime {
    pub(crate) fn worker_status<W: Write>(
        &self,
        agent_id: &str,
        context_set: Option<&str>,
        json: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let command_id = next_command_id("status");
        self.run_worker_commands(
            agent_id,
            context_set,
            /*dry_run*/ false,
            json,
            vec![
                SpecialistWorkerCommand::Status {
                    id: command_id.clone(),
                },
                SpecialistWorkerCommand::Shutdown {
                    id: format!("{command_id}-shutdown"),
                },
            ],
            writer,
        )
    }

    pub(crate) fn worker_prompt<W: Write>(
        &self,
        agent_id: &str,
        prompt: &str,
        context_set: Option<&str>,
        dry_run: bool,
        json: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let prompt = prompt.trim();
        if prompt.is_empty() {
            bail!("prompt must not be empty");
        }

        let command_id = next_command_id("prompt");
        self.run_worker_commands(
            agent_id,
            context_set,
            dry_run,
            json,
            vec![
                SpecialistWorkerCommand::Prompt {
                    id: command_id.clone(),
                    prompt: prompt.to_string(),
                },
                SpecialistWorkerCommand::Shutdown {
                    id: format!("{command_id}-shutdown"),
                },
            ],
            writer,
        )
    }

    fn run_worker_commands<W: Write>(
        &self,
        agent_id: &str,
        context_set: Option<&str>,
        dry_run: bool,
        json: bool,
        commands: Vec<SpecialistWorkerCommand>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let agent = self.workspace.agent(agent_id)?;
        if !agent.workspace.is_dir() {
            bail!(
                "workspace does not exist for {}: {}",
                agent.agent_id,
                agent.workspace.display()
            );
        }
        if !self.specialist_codex_bin.is_file() {
            bail!(
                "specialist Codex binary does not exist: {}",
                self.specialist_codex_bin.display()
            );
        }

        let mut child_command = Command::new(&self.specialist_codex_bin);
        child_command
            .arg("specialist")
            .arg("worker")
            .arg("--exec-bin")
            .arg(&self.specialist_codex_bin)
            .current_dir(&agent.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(context_set) = context_set {
            child_command.arg("--context-set").arg(context_set);
        }
        if dry_run {
            child_command.arg("--dry-run");
        }

        let mut child = child_command.spawn().with_context(|| {
            format!(
                "start specialist worker for {} in {}",
                agent.agent_id,
                agent.workspace.display()
            )
        })?;
        let mut stdin = child.stdin.take().context("open specialist worker stdin")?;
        for command in &commands {
            writeln!(stdin, "{}", encode_worker_command(command)?)
                .context("write specialist worker command")?;
        }
        drop(stdin);

        let output = child
            .wait_with_output()
            .context("wait for specialist worker")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            if json {
                writeln!(writer, "{line}")?;
            } else {
                let event = decode_worker_event(line).with_context(|| {
                    format!("decode specialist worker event from line `{line}`")
                })?;
                print_worker_event(writer, &event)?;
            }
        }

        if !output.status.success() {
            bail!("specialist worker failed: {}", stderr_text(&output));
        }
        self.log_event(format!(
            "ran worker protocol command for {}",
            agent.agent_id
        ))?;
        Ok(())
    }
}

fn print_worker_event<W: Write>(
    writer: &mut W,
    event: &SpecialistWorkerEvent,
) -> anyhow::Result<()> {
    match event {
        SpecialistWorkerEvent::Ready {
            workspace_id,
            issue_id,
            worker_pid,
        } => writeln!(
            writer,
            "worker ready workspace_id={workspace_id} issue_id={issue_id} pid={worker_pid}"
        )?,
        SpecialistWorkerEvent::Status {
            command_id,
            workspace_id,
            issue_id,
            checkpoint,
        } => {
            writeln!(
                writer,
                "worker status command_id={command_id} workspace_id={workspace_id} issue_id={issue_id}"
            )?;
            if let Some(checkpoint) = checkpoint {
                writeln!(writer, "checkpoint_status={:?}", checkpoint.status)?;
                writeln!(writer, "objective={}", checkpoint.objective)?;
                if let Some(next_concrete_step) = checkpoint.next_concrete_step.as_deref() {
                    writeln!(writer, "next_step={next_concrete_step}")?;
                }
                if let Some(recommended_next_prompt) = checkpoint.recommended_next_prompt.as_deref()
                {
                    writeln!(writer, "recommended_next_prompt={recommended_next_prompt}")?;
                }
            } else {
                writeln!(writer, "checkpoint=none")?;
            }
        }
        SpecialistWorkerEvent::TurnStarted { command_id } => {
            writeln!(writer, "worker turn started command_id={command_id}")?;
        }
        SpecialistWorkerEvent::TurnCompleted {
            command_id,
            exit_code,
            final_message,
        } => {
            writeln!(
                writer,
                "worker turn completed command_id={command_id} exit_code={exit_code:?}"
            )?;
            if let Some(final_message) = final_message.as_deref() {
                writeln!(writer, "final_message={final_message}")?;
            }
        }
        SpecialistWorkerEvent::NeedsDirection { command_id, reason } => {
            writeln!(
                writer,
                "worker needs direction command_id={command_id} reason={reason}"
            )?;
        }
        SpecialistWorkerEvent::Error {
            command_id,
            message,
        } => {
            writeln!(
                writer,
                "worker error command_id={command_id:?} message={message}"
            )?;
        }
        SpecialistWorkerEvent::Exited { command_id, reason } => {
            writeln!(
                writer,
                "worker exited command_id={command_id} reason={reason}"
            )?;
        }
    }
    Ok(())
}

fn next_command_id(kind: &str) -> String {
    let epoch_millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("manager-{kind}-{}-{epoch_millis}", std::process::id())
}
