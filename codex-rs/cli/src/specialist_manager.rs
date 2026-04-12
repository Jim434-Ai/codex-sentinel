use anyhow::Context;
use anyhow::bail;
use codex_specialist::SpecialistWorkerCommand;
use codex_specialist::SpecialistWorkerEvent;
use codex_specialist::decode_worker_event;
use codex_specialist::encode_worker_command;
use std::io::BufRead;
use std::io::BufReader;
use std::io::IsTerminal;
use std::io::Write;
use std::path::PathBuf;
use std::process::Child;
use std::process::ChildStdin;
use std::process::ChildStdout;
use std::process::Command;
use std::process::Stdio;
use std::sync::mpsc;
use std::time::Duration;

use crate::specialist_cmd::SpecialistManagerArgs;

const READY_TIMEOUT: Duration = Duration::from_secs(10);

pub fn run_specialist_manager(args: SpecialistManagerArgs) -> anyhow::Result<()> {
    let autostart = args.autostart;
    let mut manager = SpecialistManager::new(args)?;
    let stdin_is_terminal = std::io::stdin().is_terminal();
    let stdin = std::io::stdin();
    let mut stdout = std::io::BufWriter::new(std::io::stdout());

    writeln!(stdout, "Codex specialist manager daemon started.")?;
    print_help(&mut stdout)?;
    if autostart {
        manager.start_worker(&mut stdout)?;
    }

    print_prompt(&mut stdout, stdin_is_terminal)?;
    for line in stdin.lock().lines() {
        let line = line.context("read manager command")?;
        if !manager.handle_line(&line, &mut stdout)? {
            break;
        }
        print_prompt(&mut stdout, stdin_is_terminal)?;
    }

    manager.shutdown_if_running(&mut stdout)?;
    Ok(())
}

struct SpecialistManager {
    args: SpecialistManagerArgs,
    worker: Option<WorkerHandle>,
    next_command: u64,
    manager_cwd: PathBuf,
    worker_bin: PathBuf,
}

impl SpecialistManager {
    fn new(args: SpecialistManagerArgs) -> anyhow::Result<Self> {
        Ok(Self {
            args,
            worker: None,
            next_command: 0,
            manager_cwd: std::env::current_dir().context("resolve manager cwd")?,
            worker_bin: std::env::current_exe().context("resolve current Codex executable")?,
        })
    }

    fn handle_line<W: Write>(&mut self, line: &str, writer: &mut W) -> anyhow::Result<bool> {
        if line.trim().is_empty() {
            return Ok(true);
        }

        let command = match parse_manager_command(line) {
            Ok(command) => command,
            Err(err) => {
                writeln!(writer, "manager error: {err:#}")?;
                return Ok(true);
            }
        };

        let result = match command {
            ManagerCommand::Help => print_help(writer),
            ManagerCommand::Start => self.start_worker(writer),
            ManagerCommand::Status => self.request_status(writer),
            ManagerCommand::Prompt(prompt) => self.send_prompt(prompt, writer),
            ManagerCommand::Shutdown => self.shutdown_worker(writer),
            ManagerCommand::Quit => return Ok(false),
        };

        if let Err(err) = result {
            writeln!(writer, "manager error: {err:#}")?;
        }

        Ok(true)
    }

    fn start_worker<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        if let Some(worker) = self.worker.as_mut() {
            if let Some(status) = worker.child.try_wait().context("poll specialist worker")? {
                writeln!(writer, "previous specialist worker exited: {status}")?;
                self.worker = None;
            } else {
                writeln!(writer, "specialist worker already running")?;
                return Ok(());
            }
        }

        let mut command = Command::new(&self.worker_bin);
        command
            .arg("specialist")
            .arg("worker")
            .current_dir(&self.manager_cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        if let Some(workspace_manifest) = self.args.locator.workspace_manifest.as_deref() {
            command.arg("--workspace-manifest").arg(workspace_manifest);
        }
        if let Some(machine_profile) = self.args.locator.machine_profile.as_deref() {
            command.arg("--machine-profile").arg(machine_profile);
        }
        if let Some(context_set) = self.args.context_set.as_deref() {
            command.arg("--context-set").arg(context_set);
        }
        if let Some(exec_bin) = self.args.exec_bin.as_deref() {
            command.arg("--exec-bin").arg(exec_bin);
        }
        if self.args.dry_run {
            command.arg("--dry-run");
        }

        let mut child = command.spawn().with_context(|| {
            format!(
                "start specialist worker with executable {}",
                self.worker_bin.display()
            )
        })?;
        let stdin = child.stdin.take().context("open specialist worker stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("open specialist worker stdout")?;

        self.worker = Some(WorkerHandle {
            child,
            stdin,
            events: spawn_event_reader(stdout),
        });

        if let Err(err) = self.wait_for_ready(writer) {
            self.stop_failed_worker();
            return Err(err);
        }

        Ok(())
    }

    fn request_status<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        let id = self.next_command_id("status");
        self.send_worker_command(&SpecialistWorkerCommand::Status { id: id.clone() })?;
        self.wait_for_command(&id, CommandCompletion::Status, writer)
    }

    fn send_prompt<W: Write>(&mut self, prompt: String, writer: &mut W) -> anyhow::Result<()> {
        let id = self.next_command_id("prompt");
        self.send_worker_command(&SpecialistWorkerCommand::Prompt {
            id: id.clone(),
            prompt,
        })?;
        self.wait_for_command(&id, CommandCompletion::Prompt, writer)
    }

    fn shutdown_worker<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        if self.worker.is_none() {
            writeln!(writer, "no specialist worker running")?;
            return Ok(());
        }

        let id = self.next_command_id("shutdown");
        self.send_worker_command(&SpecialistWorkerCommand::Shutdown { id: id.clone() })?;
        self.wait_for_command(&id, CommandCompletion::Shutdown, writer)?;

        if let Some(mut worker) = self.worker.take() {
            let status = worker.child.wait().context("wait for specialist worker")?;
            writeln!(writer, "worker process exited: {status}")?;
        }

        Ok(())
    }

    fn shutdown_if_running<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        if self.worker.is_some() {
            self.shutdown_worker(writer)?;
        }

        Ok(())
    }

    fn send_worker_command(&mut self, command: &SpecialistWorkerCommand) -> anyhow::Result<()> {
        let encoded = encode_worker_command(command).context("encode specialist worker command")?;
        let worker = self.active_worker()?;
        writeln!(worker.stdin, "{encoded}").context("write specialist worker command")?;
        worker
            .stdin
            .flush()
            .context("flush specialist worker command")?;
        Ok(())
    }

    fn wait_for_ready<W: Write>(&mut self, writer: &mut W) -> anyhow::Result<()> {
        loop {
            let event = self.recv_worker_event(Some(READY_TIMEOUT))?;
            print_worker_event(writer, &event)?;
            match event {
                SpecialistWorkerEvent::Ready { .. } => return Ok(()),
                SpecialistWorkerEvent::Error {
                    command_id: None, ..
                } => bail!("specialist worker failed before ready"),
                _ => {}
            }
        }
    }

    fn wait_for_command<W: Write>(
        &mut self,
        command_id: &str,
        completion: CommandCompletion,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        loop {
            let event = self.recv_worker_event(None)?;
            print_worker_event(writer, &event)?;
            if completion.matches(command_id, &event) {
                return Ok(());
            }
        }
    }

    fn recv_worker_event(
        &mut self,
        timeout: Option<Duration>,
    ) -> anyhow::Result<SpecialistWorkerEvent> {
        let worker = self.active_worker()?;
        let event = match timeout {
            Some(timeout) => worker
                .events
                .recv_timeout(timeout)
                .map_err(|err| match err {
                    mpsc::RecvTimeoutError::Timeout => {
                        anyhow::anyhow!("timed out waiting for specialist worker event")
                    }
                    mpsc::RecvTimeoutError::Disconnected => {
                        anyhow::anyhow!("specialist worker event stream closed")
                    }
                })?,
            None => worker
                .events
                .recv()
                .context("specialist worker event stream closed")?,
        };

        match event {
            WorkerReaderEvent::Event(event) => Ok(event),
            WorkerReaderEvent::ReaderError(message) => bail!("{message}"),
            WorkerReaderEvent::Eof => bail!("specialist worker exited"),
        }
    }

    fn active_worker(&mut self) -> anyhow::Result<&mut WorkerHandle> {
        let exited = if let Some(worker) = self.worker.as_mut() {
            worker.child.try_wait().context("poll specialist worker")?
        } else {
            None
        };

        if let Some(status) = exited {
            self.worker = None;
            bail!("specialist worker exited before command: {status}");
        }

        self.worker
            .as_mut()
            .context("no specialist worker running; run `start` first")
    }

    fn stop_failed_worker(&mut self) {
        if let Some(mut worker) = self.worker.take() {
            let _ = worker.child.kill();
            let _ = worker.child.wait();
        }
    }

    fn next_command_id(&mut self, prefix: &str) -> String {
        self.next_command += 1;
        format!("{prefix}-{}", self.next_command)
    }
}

struct WorkerHandle {
    child: Child,
    stdin: ChildStdin,
    events: mpsc::Receiver<WorkerReaderEvent>,
}

enum WorkerReaderEvent {
    Event(SpecialistWorkerEvent),
    ReaderError(String),
    Eof,
}

#[derive(Debug, Clone, Copy)]
enum CommandCompletion {
    Status,
    Prompt,
    Shutdown,
}

impl CommandCompletion {
    fn matches(self, command_id: &str, event: &SpecialistWorkerEvent) -> bool {
        match event {
            SpecialistWorkerEvent::Status {
                command_id: event_command_id,
                ..
            } => matches!(self, CommandCompletion::Status) && event_command_id == command_id,
            SpecialistWorkerEvent::NeedsDirection {
                command_id: event_command_id,
                ..
            } => matches!(self, CommandCompletion::Prompt) && event_command_id == command_id,
            SpecialistWorkerEvent::Exited {
                command_id: event_command_id,
                ..
            } => matches!(self, CommandCompletion::Shutdown) && event_command_id == command_id,
            SpecialistWorkerEvent::Error {
                command_id: Some(event_command_id),
                ..
            } => event_command_id == command_id,
            SpecialistWorkerEvent::Ready { .. }
            | SpecialistWorkerEvent::TurnStarted { .. }
            | SpecialistWorkerEvent::TurnCompleted { .. }
            | SpecialistWorkerEvent::Error {
                command_id: None, ..
            } => false,
        }
    }
}

enum ManagerCommand {
    Help,
    Start,
    Status,
    Prompt(String),
    Shutdown,
    Quit,
}

fn parse_manager_command(line: &str) -> anyhow::Result<ManagerCommand> {
    let line = line.trim();
    let mut parts = line.splitn(2, |ch: char| ch.is_whitespace());
    let name = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default().trim();

    match name {
        "help" | "?" => Ok(ManagerCommand::Help),
        "start" => Ok(ManagerCommand::Start),
        "status" => Ok(ManagerCommand::Status),
        "prompt" | "p" => {
            if rest.is_empty() {
                bail!("prompt requires text");
            }
            Ok(ManagerCommand::Prompt(rest.to_string()))
        }
        "shutdown" | "stop" => Ok(ManagerCommand::Shutdown),
        "quit" | "exit" => Ok(ManagerCommand::Quit),
        _ => bail!(
            "unknown manager command `{name}`; expected start, status, prompt <text>, shutdown, help, or quit"
        ),
    }
}

fn spawn_event_reader(stdout: ChildStdout) -> mpsc::Receiver<WorkerReaderEvent> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(err) => {
                    let _ = sender.send(WorkerReaderEvent::ReaderError(format!(
                        "read specialist worker event: {err}"
                    )));
                    break;
                }
            };
            if line.trim().is_empty() {
                continue;
            }

            match decode_worker_event(&line) {
                Ok(event) => {
                    if sender.send(WorkerReaderEvent::Event(event)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = sender.send(WorkerReaderEvent::ReaderError(format!(
                        "decode specialist worker event `{line}`: {err}"
                    )));
                }
            }
        }

        let _ = sender.send(WorkerReaderEvent::Eof);
    });
    receiver
}

fn print_help<W: Write>(writer: &mut W) -> anyhow::Result<()> {
    writeln!(
        writer,
        "Commands: start, status, prompt <text>, shutdown, help, quit"
    )?;
    Ok(())
}

fn print_prompt<W: Write>(writer: &mut W, enabled: bool) -> anyhow::Result<()> {
    if enabled {
        write!(writer, "manager> ")?;
        writer.flush()?;
    }
    Ok(())
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
            "worker ready: workspace={workspace_id} issue={issue_id} pid={worker_pid}"
        )?,
        SpecialistWorkerEvent::Status {
            command_id,
            checkpoint,
            ..
        } => {
            if let Some(checkpoint) = checkpoint {
                writeln!(
                    writer,
                    "status {command_id}: checkpoint status={:?}",
                    checkpoint.status
                )?;
                if let Some(next_concrete_step) = checkpoint.next_concrete_step.as_deref() {
                    writeln!(writer, "  next: {next_concrete_step}")?;
                }
            } else {
                writeln!(writer, "status {command_id}: no checkpoint found")?;
            }
        }
        SpecialistWorkerEvent::TurnStarted { command_id } => {
            writeln!(writer, "turn {command_id} started")?;
        }
        SpecialistWorkerEvent::TurnCompleted {
            command_id,
            exit_code,
            final_message,
        } => {
            let exit_code = exit_code
                .map(|code| code.to_string())
                .unwrap_or_else(|| "signal".to_string());
            writeln!(writer, "turn {command_id} completed: exit_code={exit_code}")?;
            if let Some(final_message) = final_message.as_deref() {
                writeln!(writer, "  final_message: {final_message}")?;
            }
        }
        SpecialistWorkerEvent::NeedsDirection { command_id, reason } => {
            writeln!(writer, "worker needs direction for {command_id}: {reason}")?;
        }
        SpecialistWorkerEvent::Error {
            command_id,
            message,
        } => match command_id {
            Some(command_id) => writeln!(writer, "worker error for {command_id}: {message}")?,
            None => writeln!(writer, "worker error: {message}")?,
        },
        SpecialistWorkerEvent::Exited { reason, .. } => {
            writeln!(writer, "worker exited: {reason}")?;
        }
    }

    Ok(())
}
