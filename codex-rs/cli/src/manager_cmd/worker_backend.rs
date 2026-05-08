use super::runtime::ManagerRuntime;
use super::runtime::stderr_text;
use anyhow::Context;
use anyhow::bail;
use codex_specialist::SpecialistWorkerCommand;
use codex_specialist::SpecialistWorkerEvent;
use codex_specialist::decode_worker_event;
use codex_specialist::encode_worker_command;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub(crate) struct WorkerPromptRequest<'a> {
    pub(crate) agent_id: &'a str,
    pub(crate) prompt: &'a str,
    pub(crate) context_set: Option<&'a str>,
    pub(crate) idempotency_key: Option<&'a str>,
    pub(crate) dry_run: bool,
    pub(crate) json: bool,
    pub(crate) ack_json: bool,
}

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
        )?;
        Ok(())
    }

    pub(crate) fn worker_prompt<W: Write>(
        &self,
        request: WorkerPromptRequest<'_>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let WorkerPromptRequest {
            agent_id,
            prompt,
            context_set,
            idempotency_key,
            dry_run,
            json,
            ack_json,
        } = request;
        let prompt = prompt.trim();
        if prompt.is_empty() {
            bail!("prompt must not be empty");
        }

        let command_id = next_command_id("prompt");
        let agent = self.workspace.agent(agent_id)?;
        let message_id = idempotency_key
            .map(ToString::to_string)
            .unwrap_or_else(|| command_id.clone());
        if let Some(idempotency_key) = idempotency_key {
            let ack_path = self.worker_prompt_ack_path(&agent.agent_id, idempotency_key);
            if ack_path.is_file() {
                let ack = fs::read_to_string(&ack_path)
                    .with_context(|| format!("read worker prompt ack {}", ack_path.display()))?;
                let mut parsed: WorkerPromptAck = serde_json::from_str(&ack)
                    .with_context(|| format!("parse worker prompt ack {}", ack_path.display()))?;
                parsed.duplicate = true;
                if ack_json {
                    writeln!(writer, "{}", serde_json::to_string_pretty(&parsed)?)?;
                } else {
                    writeln!(
                        writer,
                        "worker prompt already handled agent_id={} message_id={} duplicate={}",
                        agent.agent_id, idempotency_key, parsed.duplicate
                    )?;
                }
                return Ok(());
            }
        }

        let commands = vec![
            SpecialistWorkerCommand::Prompt {
                id: command_id.clone(),
                prompt: prompt.to_string(),
            },
            SpecialistWorkerCommand::Shutdown {
                id: format!("{command_id}-shutdown"),
            },
        ];
        let events = if ack_json {
            let mut sink = std::io::sink();
            self.run_worker_commands(
                agent_id,
                context_set,
                dry_run,
                /*json*/ false,
                commands,
                &mut sink,
            )?
        } else {
            self.run_worker_commands(agent_id, context_set, dry_run, json, commands, writer)?
        };
        let ack = worker_prompt_ack(&agent.agent_id, &message_id, &command_id, &events);
        if let Some(idempotency_key) = idempotency_key {
            let ack_path = self.worker_prompt_ack_path(&agent.agent_id, idempotency_key);
            if let Some(parent) = ack_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!("create worker prompt ack dir {}", parent.display())
                })?;
            }
            fs::write(&ack_path, serde_json::to_string_pretty(&ack)?)
                .with_context(|| format!("write worker prompt ack {}", ack_path.display()))?;
        }
        if ack_json {
            writeln!(writer, "{}", serde_json::to_string_pretty(&ack)?)?;
        } else if ack.accepted {
            writeln!(
                writer,
                "worker prompt acknowledged agent_id={} message_id={} started={}",
                agent.agent_id, message_id, ack.started
            )?;
        }
        Ok(())
    }

    fn run_worker_commands<W: Write>(
        &self,
        agent_id: &str,
        context_set: Option<&str>,
        dry_run: bool,
        json: bool,
        commands: Vec<SpecialistWorkerCommand>,
        writer: &mut W,
    ) -> anyhow::Result<Vec<SpecialistWorkerEvent>> {
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
        let mut events = Vec::new();
        for line in stdout.lines() {
            if json {
                writeln!(writer, "{line}")?;
            } else {
                let event = decode_worker_event(line).with_context(|| {
                    format!("decode specialist worker event from line `{line}`")
                })?;
                print_worker_event(writer, &event)?;
                events.push(event);
            }
            if json {
                let event = decode_worker_event(line).with_context(|| {
                    format!("decode specialist worker event from line `{line}`")
                })?;
                events.push(event);
            }
        }

        if !output.status.success() {
            bail!("specialist worker failed: {}", stderr_text(&output));
        }
        self.log_event(format!(
            "ran worker protocol command for {}",
            agent.agent_id
        ))?;
        Ok(events)
    }

    fn worker_prompt_ack_path(&self, agent_id: &str, idempotency_key: &str) -> PathBuf {
        self.workspace
            .root
            .join(".codex-manager")
            .join("message-acks")
            .join(sanitize_ack_component(agent_id))
            .join(format!("{}.json", sanitize_ack_component(idempotency_key)))
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerPromptAck {
    accepted: bool,
    started: bool,
    message_id: String,
    session_id: String,
    duplicate: bool,
    error: Option<String>,
}

fn worker_prompt_ack(
    agent_id: &str,
    message_id: &str,
    command_id: &str,
    events: &[SpecialistWorkerEvent],
) -> WorkerPromptAck {
    let started = events.iter().any(|event| {
        matches!(event, SpecialistWorkerEvent::TurnStarted { command_id: id } if id == command_id)
    });
    let error = events.iter().find_map(|event| {
        if let SpecialistWorkerEvent::Error { message, .. } = event {
            Some(message.clone())
        } else {
            None
        }
    });
    WorkerPromptAck {
        accepted: error.is_none(),
        started,
        message_id: message_id.to_string(),
        session_id: agent_id.to_string(),
        duplicate: false,
        error,
    }
}

fn sanitize_ack_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
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
