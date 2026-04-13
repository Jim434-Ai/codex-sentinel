use super::workspace::ManagerAgent;
use anyhow::Context;
use anyhow::bail;
use codex_specialist::SpecialistWorkerCommand;
use codex_specialist::SpecialistWorkerEvent;
use codex_specialist::decode_worker_event;
use codex_specialist::encode_worker_command;
use std::io::BufRead;
use std::io::BufReader;
use std::io::Write;
use std::path::Path;
use std::process::Child;
use std::process::ChildStderr;
use std::process::ChildStdin;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const WORKER_EXIT_WAIT_MILLIS: u64 = 1000;

pub(super) struct WorkerProcess {
    pub(super) agent: ManagerAgent,
    pub(super) child: Child,
    stdin: ChildStdin,
    pub(super) busy: bool,
    sequence: u64,
}

impl WorkerProcess {
    pub(super) fn spawn(
        specialist_codex_bin: &Path,
        agent: &ManagerAgent,
        context_set: Option<&str>,
        dry_run: bool,
        sender: Sender<WorkerPoolMessage>,
    ) -> anyhow::Result<Self> {
        if !agent.workspace.is_dir() {
            bail!(
                "workspace does not exist for {}: {}",
                agent.agent_id,
                agent.workspace.display()
            );
        }
        if !specialist_codex_bin.is_file() {
            bail!(
                "specialist Codex binary does not exist: {}",
                specialist_codex_bin.display()
            );
        }

        let mut command = Command::new(specialist_codex_bin);
        command
            .arg("specialist")
            .arg("worker")
            .arg("--exec-bin")
            .arg(specialist_codex_bin)
            .current_dir(&agent.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(context_set) = context_set {
            command.arg("--context-set").arg(context_set);
        }
        if dry_run {
            command.arg("--dry-run");
        }

        let mut child = command.spawn().with_context(|| {
            format!(
                "start specialist worker for {} in {}",
                agent.agent_id,
                agent.workspace.display()
            )
        })?;
        let stdin = child.stdin.take().context("open specialist worker stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("open specialist worker stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("open specialist worker stderr")?;
        spawn_stdout_reader(agent.agent_id.clone(), stdout, sender.clone());
        spawn_stderr_reader(agent.agent_id.clone(), stderr, sender);

        Ok(Self {
            agent: agent.clone(),
            child,
            stdin,
            busy: false,
            sequence: 0,
        })
    }

    pub(super) fn next_command_id(&mut self, kind: &str) -> String {
        self.sequence += 1;
        format!(
            "manager-worker-{}-{}-{}",
            self.agent.agent_id, kind, self.sequence
        )
    }

    pub(super) fn send(&mut self, command: SpecialistWorkerCommand) -> anyhow::Result<()> {
        writeln!(self.stdin, "{}", encode_worker_command(&command)?)
            .context("write specialist worker command")?;
        self.stdin
            .flush()
            .context("flush specialist worker command")
    }
}

pub(super) enum WorkerPoolMessage {
    Event {
        agent_id: String,
        event: SpecialistWorkerEvent,
    },
    StdoutDecodeError {
        agent_id: String,
        line: String,
        error: String,
    },
    Stderr {
        agent_id: String,
        line: String,
    },
}

pub(super) fn wait_or_kill_worker<W: Write>(
    worker: &mut WorkerProcess,
    writer: &mut W,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(WORKER_EXIT_WAIT_MILLIS);
    loop {
        if let Some(status) = worker
            .child
            .try_wait()
            .context("query specialist worker during shutdown")?
        {
            writeln!(
                writer,
                "worker process exited agent_id={} status={status}",
                worker.agent.agent_id
            )?;
            return Ok(());
        }
        if Instant::now() >= deadline {
            worker
                .child
                .kill()
                .context("kill unresponsive specialist worker")?;
            let status = worker
                .child
                .wait()
                .context("wait for killed specialist worker")?;
            writeln!(
                writer,
                "worker process killed agent_id={} status={status}",
                worker.agent.agent_id
            )?;
            return Ok(());
        }
        thread::sleep(Duration::from_millis(50));
    }
}

pub(super) fn write_worker_pool_event<W: Write>(
    writer: &mut W,
    agent_id: &str,
    event: &SpecialistWorkerEvent,
) -> anyhow::Result<()> {
    match event {
        SpecialistWorkerEvent::Ready {
            workspace_id,
            issue_id,
            worker_pid,
        } => writeln!(
            writer,
            "worker ready agent_id={agent_id} workspace_id={workspace_id} issue_id={issue_id} pid={worker_pid}"
        )?,
        SpecialistWorkerEvent::Status {
            command_id,
            checkpoint,
            ..
        } => {
            write!(
                writer,
                "worker status agent_id={agent_id} command_id={command_id}"
            )?;
            if let Some(checkpoint) = checkpoint {
                write!(writer, " checkpoint_status={:?}", checkpoint.status)?;
                if let Some(next_step) = checkpoint.next_concrete_step.as_deref() {
                    write!(writer, " next_step={next_step}")?;
                }
            } else {
                write!(writer, " checkpoint=none")?;
            }
            writeln!(writer)?;
        }
        SpecialistWorkerEvent::TurnStarted { command_id } => {
            writeln!(
                writer,
                "worker turn started agent_id={agent_id} command_id={command_id}"
            )?;
        }
        SpecialistWorkerEvent::TurnCompleted {
            command_id,
            exit_code,
            final_message,
        } => {
            writeln!(
                writer,
                "worker turn completed agent_id={agent_id} command_id={command_id} exit_code={exit_code:?}"
            )?;
            if let Some(final_message) = final_message.as_deref() {
                writeln!(
                    writer,
                    "worker final message agent_id={agent_id}: {final_message}"
                )?;
            }
        }
        SpecialistWorkerEvent::NeedsDirection { command_id, reason } => {
            writeln!(
                writer,
                "worker needs direction agent_id={agent_id} command_id={command_id} reason={reason}"
            )?;
        }
        SpecialistWorkerEvent::Error {
            command_id,
            message,
        } => {
            writeln!(
                writer,
                "worker error agent_id={agent_id} command_id={command_id:?} message={message}"
            )?;
        }
        SpecialistWorkerEvent::Exited { command_id, reason } => {
            writeln!(
                writer,
                "worker exited agent_id={agent_id} command_id={command_id} reason={reason}"
            )?;
        }
    }
    Ok(())
}

fn spawn_stdout_reader(agent_id: String, stdout: ChildStdout, sender: Sender<WorkerPoolMessage>) {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            match decode_worker_event(&line) {
                Ok(event) => {
                    if sender
                        .send(WorkerPoolMessage::Event {
                            agent_id: agent_id.clone(),
                            event,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
                Err(error) => {
                    if sender
                        .send(WorkerPoolMessage::StdoutDecodeError {
                            agent_id: agent_id.clone(),
                            line,
                            error: error.to_string(),
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
    });
}

fn spawn_stderr_reader(agent_id: String, stderr: ChildStderr, sender: Sender<WorkerPoolMessage>) {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            let Ok(line) = line else {
                break;
            };
            if sender
                .send(WorkerPoolMessage::Stderr {
                    agent_id: agent_id.clone(),
                    line,
                })
                .is_err()
            {
                break;
            }
        }
    });
}
