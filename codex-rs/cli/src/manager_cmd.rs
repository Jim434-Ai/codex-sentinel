mod runtime;
mod workspace;

use clap::Args;
use clap::Subcommand;
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
        }

        Ok(())
    }
}
