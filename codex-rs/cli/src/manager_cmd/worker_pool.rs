use super::ManagerWorkerDaemonArgs;
use super::runtime::ManagerRuntime;
use super::worker_pool_process::WorkerPoolMessage;
use super::worker_pool_process::WorkerProcess;
use super::worker_pool_process::wait_or_kill_worker;
use super::worker_pool_process::write_worker_pool_event;
use super::worker_pool_queue::DEFAULT_PROMPT_QUEUE_DIR;
use super::worker_pool_queue::move_processed_prompt;
use super::worker_pool_queue::next_prompt_file_path;
use super::worker_pool_queue::next_queued_prompt;
use super::worker_pool_queue::sanitize_queue_component;
use super::workspace::ManagerAgent;
use super::workspace::resolve_user_path;
use anyhow::Context;
use anyhow::bail;
use codex_specialist::SpecialistWorkerCommand;
use codex_specialist::SpecialistWorkerEvent;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::Duration;
use std::time::Instant;

const EVENT_DRAIN_MILLIS: u64 = 250;
const SHUTDOWN_DRAIN_MILLIS: u64 = 750;

impl ManagerRuntime {
    pub(crate) fn worker_daemon<W: Write>(
        &self,
        args: &ManagerWorkerDaemonArgs,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let active_agents = self.workspace.active_agents().cloned().collect::<Vec<_>>();
        if active_agents.is_empty() {
            writeln!(writer, "No active agents are registered.")?;
            return Ok(());
        }

        let prompt_queue_dir = self.prepare_prompt_queue(args)?;
        let (sender, receiver) = mpsc::channel();
        let mut workers = self.start_worker_pool(
            active_agents,
            args.context_set.as_deref(),
            args.dry_run,
            &sender,
            writer,
        )?;

        writeln!(
            writer,
            "worker-pool started agents={} queue={}",
            workers.len(),
            prompt_queue_dir
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|| "disabled".to_string())
        )?;

        let mut iteration = 0_u64;
        loop {
            iteration += 1;
            writeln!(writer, "== worker-daemon iteration {iteration} ==")?;
            self.restart_exited_workers(
                &mut workers,
                args.context_set.as_deref(),
                args.dry_run,
                !args.no_restart_exited,
                &sender,
                writer,
            )?;
            self.send_status_requests(&mut workers, writer)?;
            if let Some(prompt_queue_dir) = prompt_queue_dir.as_ref() {
                self.dispatch_prompt_queue(prompt_queue_dir, &mut workers, writer)?;
            }
            self.drain_worker_messages(
                &receiver,
                &mut workers,
                Duration::from_millis(EVENT_DRAIN_MILLIS),
                writer,
            )?;
            writer.flush().context("flush worker daemon output")?;

            if let Some(iterations) = args.iterations
                && iteration >= u64::from(iterations)
            {
                break;
            }
            thread::sleep(Duration::from_secs(args.interval_seconds));
        }

        self.shutdown_worker_pool(workers, &receiver, writer)
    }

    pub(crate) fn worker_enqueue<W: Write>(
        &self,
        agent_id: &str,
        prompt: &str,
        prompt_queue_dir: Option<&Path>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let agent = self.workspace.agent(agent_id)?;
        let prompt = prompt.trim();
        if prompt.is_empty() {
            bail!("prompt must not be empty");
        }

        let queue_dir = self
            .prompt_queue_dir(prompt_queue_dir)
            .join(sanitize_queue_component(&agent.agent_id));
        fs::create_dir_all(&queue_dir)
            .with_context(|| format!("create prompt queue dir {}", queue_dir.display()))?;
        let prompt_path = next_prompt_file_path(&queue_dir)?;
        fs::write(&prompt_path, format!("{prompt}\n"))
            .with_context(|| format!("write prompt queue file {}", prompt_path.display()))?;

        self.log_event(format!(
            "queued worker prompt for {} at {}",
            agent.agent_id,
            prompt_path.display()
        ))?;
        writeln!(
            writer,
            "Queued prompt for {}: {}",
            agent.agent_id,
            prompt_path.display()
        )?;
        Ok(())
    }

    fn prepare_prompt_queue(
        &self,
        args: &ManagerWorkerDaemonArgs,
    ) -> anyhow::Result<Option<PathBuf>> {
        if args.no_prompt_queue {
            return Ok(None);
        }

        let prompt_queue_dir = self.prompt_queue_dir(args.prompt_queue_dir.as_deref());
        fs::create_dir_all(&prompt_queue_dir)
            .with_context(|| format!("create prompt queue dir {}", prompt_queue_dir.display()))?;
        Ok(Some(prompt_queue_dir))
    }

    fn start_worker_pool<W: Write>(
        &self,
        active_agents: Vec<ManagerAgent>,
        context_set: Option<&str>,
        dry_run: bool,
        sender: &Sender<WorkerPoolMessage>,
        writer: &mut W,
    ) -> anyhow::Result<BTreeMap<String, WorkerProcess>> {
        let mut workers = BTreeMap::new();
        for agent in active_agents {
            match WorkerProcess::spawn(
                &self.specialist_codex_bin,
                &agent,
                context_set,
                dry_run,
                sender.clone(),
            ) {
                Ok(worker) => {
                    writeln!(
                        writer,
                        "started worker agent_id={} pid={}",
                        worker.agent.agent_id,
                        worker.child.id()
                    )?;
                    workers.insert(worker.agent.agent_id.clone(), worker);
                }
                Err(err) => {
                    writeln!(writer, "failed to start {}: {err:#}", agent.agent_id)?;
                    self.log_event(format!(
                        "failed to start worker {}: {err:#}",
                        agent.agent_id
                    ))?;
                }
            }
        }

        if workers.is_empty() {
            bail!("no specialist workers started");
        }
        Ok(workers)
    }

    fn send_status_requests<W: Write>(
        &self,
        workers: &mut BTreeMap<String, WorkerProcess>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        for worker in workers.values_mut() {
            if worker.busy {
                writeln!(
                    writer,
                    "worker busy agent_id={} status_request=skipped",
                    worker.agent.agent_id
                )?;
                continue;
            }
            let id = worker.next_command_id("status");
            match worker.send(SpecialistWorkerCommand::Status { id }) {
                Ok(()) => writeln!(
                    writer,
                    "worker status requested agent_id={}",
                    worker.agent.agent_id
                )?,
                Err(err) => writeln!(
                    writer,
                    "worker status request failed agent_id={}: {err:#}",
                    worker.agent.agent_id
                )?,
            }
        }
        Ok(())
    }

    fn dispatch_prompt_queue<W: Write>(
        &self,
        prompt_queue_dir: &Path,
        workers: &mut BTreeMap<String, WorkerProcess>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        for worker in workers.values_mut() {
            if worker.busy {
                continue;
            }
            let agent_queue_dir =
                prompt_queue_dir.join(sanitize_queue_component(&worker.agent.agent_id));
            let Some(prompt_path) = next_queued_prompt(&agent_queue_dir)? else {
                continue;
            };
            let prompt = fs::read_to_string(&prompt_path)
                .with_context(|| format!("read queued prompt {}", prompt_path.display()))?;
            let prompt = prompt.trim().to_string();
            if prompt.is_empty() {
                move_processed_prompt(prompt_queue_dir, &worker.agent.agent_id, &prompt_path)?;
                continue;
            }

            let id = worker.next_command_id("prompt");
            worker.send(SpecialistWorkerCommand::Prompt { id, prompt })?;
            worker.busy = true;
            move_processed_prompt(prompt_queue_dir, &worker.agent.agent_id, &prompt_path)?;
            self.log_event(format!(
                "sent queued worker prompt to {} from {}",
                worker.agent.agent_id,
                prompt_path.display()
            ))?;
            writeln!(
                writer,
                "worker prompt dispatched agent_id={} file={}",
                worker.agent.agent_id,
                prompt_path.display()
            )?;
        }
        Ok(())
    }

    fn restart_exited_workers<W: Write>(
        &self,
        workers: &mut BTreeMap<String, WorkerProcess>,
        context_set: Option<&str>,
        dry_run: bool,
        restart_exited: bool,
        sender: &Sender<WorkerPoolMessage>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let mut exited_agents = Vec::new();
        for (agent_id, worker) in workers.iter_mut() {
            if let Some(status) = worker
                .child
                .try_wait()
                .context("query specialist worker status")?
            {
                writeln!(writer, "worker exited agent_id={agent_id} status={status}")?;
                exited_agents.push(worker.agent.clone());
            }
        }

        for agent in &exited_agents {
            workers.remove(&agent.agent_id);
        }
        if !restart_exited {
            return Ok(());
        }
        for agent in exited_agents {
            let worker = WorkerProcess::spawn(
                &self.specialist_codex_bin,
                &agent,
                context_set,
                dry_run,
                sender.clone(),
            )?;
            writeln!(
                writer,
                "worker restarted agent_id={} pid={}",
                worker.agent.agent_id,
                worker.child.id()
            )?;
            self.log_event(format!("restarted worker {}", worker.agent.agent_id))?;
            workers.insert(worker.agent.agent_id.clone(), worker);
        }
        Ok(())
    }

    fn drain_worker_messages<W: Write>(
        &self,
        receiver: &Receiver<WorkerPoolMessage>,
        workers: &mut BTreeMap<String, WorkerProcess>,
        wait: Duration,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let deadline = Instant::now() + wait;
        loop {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            match receiver.recv_timeout(deadline.saturating_duration_since(now)) {
                Ok(message) => self.handle_worker_message(message, workers, writer)?,
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        Ok(())
    }

    fn handle_worker_message<W: Write>(
        &self,
        message: WorkerPoolMessage,
        workers: &mut BTreeMap<String, WorkerProcess>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        match message {
            WorkerPoolMessage::Event { agent_id, event } => {
                if let Some(worker) = workers.get_mut(&agent_id) {
                    match &event {
                        SpecialistWorkerEvent::TurnStarted { .. } => worker.busy = true,
                        SpecialistWorkerEvent::TurnCompleted { .. }
                        | SpecialistWorkerEvent::Error { .. }
                        | SpecialistWorkerEvent::Exited { .. } => worker.busy = false,
                        SpecialistWorkerEvent::Ready { .. }
                        | SpecialistWorkerEvent::Status { .. }
                        | SpecialistWorkerEvent::NeedsDirection { .. } => {}
                    }
                }
                write_worker_pool_event(writer, &agent_id, &event)?;
            }
            WorkerPoolMessage::StdoutDecodeError {
                agent_id,
                line,
                error,
            } => {
                writeln!(
                    writer,
                    "worker stdout decode error agent_id={agent_id} error={error} line={line}"
                )?;
            }
            WorkerPoolMessage::Stderr { agent_id, line } => {
                writeln!(writer, "worker stderr agent_id={agent_id}: {line}")?;
            }
        }
        Ok(())
    }

    fn shutdown_worker_pool<W: Write>(
        &self,
        mut workers: BTreeMap<String, WorkerProcess>,
        receiver: &Receiver<WorkerPoolMessage>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        for worker in workers.values_mut() {
            let id = worker.next_command_id("shutdown");
            if let Err(err) = worker.send(SpecialistWorkerCommand::Shutdown { id }) {
                writeln!(
                    writer,
                    "worker shutdown send failed agent_id={}: {err:#}",
                    worker.agent.agent_id
                )?;
            }
        }
        self.drain_worker_messages(
            receiver,
            &mut workers,
            Duration::from_millis(SHUTDOWN_DRAIN_MILLIS),
            writer,
        )?;
        for worker in workers.values_mut() {
            wait_or_kill_worker(worker, writer)?;
        }
        Ok(())
    }

    fn prompt_queue_dir(&self, prompt_queue_dir: Option<&Path>) -> PathBuf {
        prompt_queue_dir
            .map(|path| resolve_user_path(&self.workspace.root, path))
            .unwrap_or_else(|| self.workspace.root.join(DEFAULT_PROMPT_QUEUE_DIR))
    }
}
