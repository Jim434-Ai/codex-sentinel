mod runtime;
mod supervisor;
mod worker_backend;
mod worker_pool;
mod worker_pool_process;
mod worker_pool_queue;
mod workspace;

use clap::Args;
use clap::Subcommand;
use clap::ValueEnum;
use runtime::ManagerRuntime;
use std::path::PathBuf;
use workspace::ManagerWorkspace;

#[derive(Debug, clap::Parser)]
pub struct ManagerCli {
    /// Manager workspace root. Defaults to walking up from the current directory.
    #[arg(long = "manager-workspace", value_name = "DIR")]
    manager_workspace: Option<PathBuf>,

    /// tmux socket used for managed specialist sessions.
    #[arg(long = "tmux-socket", value_name = "NAME")]
    tmux_socket: Option<String>,

    /// Codex binary used when starting specialist sessions.
    #[arg(long = "specialist-codex-bin", value_name = "FILE")]
    specialist_codex_bin: Option<PathBuf>,

    #[command(subcommand)]
    subcommand: ManagerSubcommand,
}

#[derive(Debug, Subcommand)]
enum ManagerSubcommand {
    /// Validate the manager workspace and registry.
    Validate,

    /// List registered agents.
    List,

    /// List live tmux sessions on the manager socket.
    Sessions,

    /// Start one registered agent.
    Start(ManagerAgentArgs),

    /// Start every non-paused, non-closed registered agent.
    StartActive,

    /// Stop one registered agent.
    Stop(ManagerAgentArgs),

    /// Restart one registered agent.
    Restart(ManagerAgentArgs),

    /// Capture one registered agent's tmux pane.
    Capture(ManagerCaptureArgs),

    /// Capture one registered agent and print a status heuristic.
    Check(ManagerCaptureArgs),

    /// List manager-readable pings.
    Pings,

    /// Check pings and every active registered agent.
    Cycle(ManagerCycleArgs),

    /// Watch active specialists and report status changes.
    Watch(ManagerWatchArgs),

    /// Paste a prompt into a managed specialist tmux session.
    Prompt(ManagerPromptArgs),

    /// Run a foreground manager daemon loop.
    Daemon(ManagerDaemonArgs),

    /// Ask one specialist via the structured worker protocol for status.
    WorkerStatus(ManagerWorkerStatusArgs),

    /// Send one prompt turn through the structured specialist worker protocol.
    WorkerPrompt(ManagerWorkerPromptArgs),

    /// Run active specialists as a persistent structured worker pool.
    WorkerDaemon(ManagerWorkerDaemonArgs),

    /// Queue a prompt file for a running worker-daemon.
    WorkerEnqueue(ManagerWorkerEnqueueArgs),
}

#[derive(Debug, Args)]
struct ManagerAgentArgs {
    #[arg(value_name = "AGENT_ID")]
    agent_id: String,
}

#[derive(Debug, Args)]
struct ManagerCaptureArgs {
    #[arg(value_name = "AGENT_ID")]
    agent_id: String,

    #[arg(long = "lines", default_value_t = 120)]
    lines: usize,
}

#[derive(Debug, Args)]
struct ManagerCycleArgs {
    #[arg(long = "lines", default_value_t = 80)]
    lines: usize,
}

#[derive(Debug, Args)]
pub(crate) struct ManagerWatchArgs {
    #[arg(long = "lines", default_value_t = 80)]
    lines: usize,

    #[arg(long = "interval-seconds", default_value_t = 30)]
    interval_seconds: u64,

    #[arg(long = "iterations", value_name = "COUNT")]
    iterations: Option<u32>,

    #[arg(long = "start-active", default_value_t = false)]
    start_active: bool,
}

#[derive(Debug, Args)]
struct ManagerPromptArgs {
    #[arg(value_name = "AGENT_ID")]
    agent_id: String,

    #[arg(
        value_name = "PROMPT",
        required = true,
        num_args = 1..,
        trailing_var_arg = true
    )]
    prompt: Vec<String>,

    #[arg(long = "delivery", value_enum, default_value_t = ManagerPromptDelivery::Auto)]
    delivery: ManagerPromptDelivery,

    #[arg(long = "lines", default_value_t = 120)]
    lines: usize,
}

#[derive(Debug, Args)]
pub(crate) struct ManagerDaemonArgs {
    #[arg(long = "lines", default_value_t = 80)]
    lines: usize,

    #[arg(long = "interval-seconds", default_value_t = 30)]
    interval_seconds: u64,

    #[arg(long = "iterations", value_name = "COUNT")]
    iterations: Option<u32>,

    #[arg(long = "no-start-active", default_value_t = false)]
    no_start_active: bool,

    #[arg(long = "restart-missing", default_value_t = false)]
    restart_missing: bool,
}

#[derive(Debug, Args)]
struct ManagerWorkerStatusArgs {
    #[arg(value_name = "AGENT_ID")]
    agent_id: String,

    #[arg(long = "context-set", value_name = "NAME")]
    context_set: Option<String>,

    #[arg(long = "json", default_value_t = false)]
    json: bool,
}

#[derive(Debug, Args)]
struct ManagerWorkerPromptArgs {
    #[arg(value_name = "AGENT_ID")]
    agent_id: String,

    #[arg(
        value_name = "PROMPT",
        required = true,
        num_args = 1..,
        trailing_var_arg = true
    )]
    prompt: Vec<String>,

    #[arg(long = "context-set", value_name = "NAME")]
    context_set: Option<String>,

    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,

    #[arg(long = "json", default_value_t = false)]
    json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct ManagerWorkerDaemonArgs {
    #[arg(long = "interval-seconds", default_value_t = 30)]
    interval_seconds: u64,

    #[arg(long = "iterations", value_name = "COUNT")]
    iterations: Option<u32>,

    #[arg(long = "context-set", value_name = "NAME")]
    context_set: Option<String>,

    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,

    #[arg(long = "prompt-queue-dir", value_name = "DIR")]
    prompt_queue_dir: Option<PathBuf>,

    #[arg(long = "no-prompt-queue", default_value_t = false)]
    no_prompt_queue: bool,

    #[arg(long = "no-restart-exited", default_value_t = false)]
    no_restart_exited: bool,
}

#[derive(Debug, Args)]
struct ManagerWorkerEnqueueArgs {
    #[arg(value_name = "AGENT_ID")]
    agent_id: String,

    #[arg(
        value_name = "PROMPT",
        required = true,
        num_args = 1..,
        trailing_var_arg = true
    )]
    prompt: Vec<String>,

    #[arg(long = "prompt-queue-dir", value_name = "DIR")]
    prompt_queue_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ManagerPromptDelivery {
    /// Submit if the specialist is waiting; otherwise stage the prompt text.
    Auto,

    /// Paste text without pressing Enter.
    Stage,

    /// Paste text and press Enter.
    Submit,
}

impl ManagerCli {
    pub fn run(self) -> anyhow::Result<()> {
        let workspace = ManagerWorkspace::load(self.manager_workspace.as_deref())?;
        let runtime = ManagerRuntime::new(workspace, self.tmux_socket, self.specialist_codex_bin)?;
        let mut stdout = std::io::BufWriter::new(std::io::stdout());

        match self.subcommand {
            ManagerSubcommand::Validate => runtime.validate(&mut stdout)?,
            ManagerSubcommand::List => runtime.list_agents(&mut stdout)?,
            ManagerSubcommand::Sessions => runtime.list_sessions(&mut stdout)?,
            ManagerSubcommand::Start(args) => runtime.start_agent(&args.agent_id, &mut stdout)?,
            ManagerSubcommand::StartActive => runtime.start_active(&mut stdout)?,
            ManagerSubcommand::Stop(args) => runtime.stop_agent(&args.agent_id, &mut stdout)?,
            ManagerSubcommand::Restart(args) => {
                runtime.restart_agent(&args.agent_id, &mut stdout)?;
            }
            ManagerSubcommand::Capture(args) => {
                runtime.capture_agent(&args.agent_id, args.lines, &mut stdout)?;
            }
            ManagerSubcommand::Check(args) => {
                runtime.check_agent(&args.agent_id, args.lines, &mut stdout)?;
            }
            ManagerSubcommand::Pings => runtime.list_pings(&mut stdout)?,
            ManagerSubcommand::Cycle(args) => runtime.cycle(args.lines, &mut stdout)?,
            ManagerSubcommand::Watch(args) => runtime.watch(&args, &mut stdout)?,
            ManagerSubcommand::Prompt(args) => {
                runtime.prompt_agent(
                    &args.agent_id,
                    &args.prompt.join(" "),
                    args.delivery,
                    args.lines,
                    &mut stdout,
                )?;
            }
            ManagerSubcommand::Daemon(args) => runtime.daemon(&args, &mut stdout)?,
            ManagerSubcommand::WorkerStatus(args) => {
                runtime.worker_status(
                    &args.agent_id,
                    args.context_set.as_deref(),
                    args.json,
                    &mut stdout,
                )?;
            }
            ManagerSubcommand::WorkerPrompt(args) => {
                runtime.worker_prompt(
                    &args.agent_id,
                    &args.prompt.join(" "),
                    args.context_set.as_deref(),
                    args.dry_run,
                    args.json,
                    &mut stdout,
                )?;
            }
            ManagerSubcommand::WorkerDaemon(args) => runtime.worker_daemon(&args, &mut stdout)?,
            ManagerSubcommand::WorkerEnqueue(args) => {
                runtime.worker_enqueue(
                    &args.agent_id,
                    &args.prompt.join(" "),
                    args.prompt_queue_dir.as_deref(),
                    &mut stdout,
                )?;
            }
        }

        Ok(())
    }
}
