use clap::Args;
use clap::Subcommand;
use codex_core::config::find_codex_home;
use codex_specialist::ResolvedContextFile;
use codex_specialist::ResolvedRole;
use codex_specialist::ResolvedWorkspace;
use codex_specialist::SpecialistCheckpoint;
use codex_specialist::load_workspace;
use codex_specialist::read_latest_checkpoint;
use serde::Serialize;
use std::path::PathBuf;

use crate::specialist_init::SpecialistInitArgs;
use crate::specialist_init::run_specialist_init;
use crate::specialist_manager::run_specialist_manager;
use crate::specialist_worker::run_specialist_worker;

#[derive(Debug, clap::Parser)]
pub struct SpecialistCli {
    #[command(subcommand)]
    pub subcommand: SpecialistSubcommand,
}

#[derive(Debug, Subcommand)]
pub enum SpecialistSubcommand {
    /// Initialize a specialist workspace scaffold in the current directory.
    Init(SpecialistInitArgs),

    /// Validate the specialist workspace manifest and machine profile.
    ValidateWorkspace(SpecialistWorkspaceArgs),

    /// Show the resolved files for a context set.
    ShowContext(SpecialistContextArgs),

    /// Read the latest specialist checkpoint for the workspace.
    Status(SpecialistStatusArgs),

    /// Run a local manager daemon that supervises a specialist worker.
    Manager(SpecialistManagerArgs),

    /// Run a JSONL specialist worker loop for manager-owned lifecycle control.
    Worker(SpecialistWorkerArgs),
}

#[derive(Debug, Args, Clone)]
pub struct SpecialistWorkspaceLocatorArgs {
    /// Path to the committed specialist workspace manifest.
    #[arg(long = "workspace-manifest", value_name = "FILE")]
    pub workspace_manifest: Option<PathBuf>,

    /// Path to the local-only machine profile that resolves logical roots.
    #[arg(long = "machine-profile", value_name = "FILE")]
    pub machine_profile: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct SpecialistWorkspaceArgs {
    #[command(flatten)]
    locator: SpecialistWorkspaceLocatorArgs,

    /// Print the resolved workspace as JSON.
    #[arg(long = "json", default_value_t = false)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct SpecialistContextArgs {
    #[command(flatten)]
    locator: SpecialistWorkspaceLocatorArgs,

    /// Named context set to resolve. Defaults to the workspace default_context_set.
    #[arg(long = "context-set", value_name = "NAME")]
    context_set: Option<String>,

    /// Print the resolved context set as JSON.
    #[arg(long = "json", default_value_t = false)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct SpecialistStatusArgs {
    #[command(flatten)]
    locator: SpecialistWorkspaceLocatorArgs,

    /// Print the latest checkpoint state as JSON.
    #[arg(long = "json", default_value_t = false)]
    json: bool,
}

#[derive(Debug, Args)]
pub struct SpecialistWorkerArgs {
    #[command(flatten)]
    pub locator: SpecialistWorkspaceLocatorArgs,

    /// Named context set to preload for prompt turns.
    #[arg(long = "context-set", value_name = "NAME")]
    pub context_set: Option<String>,

    /// Codex binary used for prompt turns. Defaults to the current executable.
    #[arg(long = "exec-bin", value_name = "FILE")]
    pub exec_bin: Option<PathBuf>,

    /// Accept commands and emit events without running model turns.
    #[arg(long = "dry-run", default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Debug, Args, Clone)]
pub struct SpecialistManagerArgs {
    #[command(flatten)]
    pub locator: SpecialistWorkspaceLocatorArgs,

    /// Named context set to preload for specialist worker prompt turns.
    #[arg(long = "context-set", value_name = "NAME")]
    pub context_set: Option<String>,

    /// Codex binary used by the specialist worker for prompt turns.
    #[arg(long = "exec-bin", value_name = "FILE")]
    pub exec_bin: Option<PathBuf>,

    /// Start the specialist worker immediately instead of waiting for `start`.
    #[arg(long = "autostart", default_value_t = false)]
    pub autostart: bool,

    /// Run the managed specialist worker in dry-run mode.
    #[arg(long = "dry-run", default_value_t = false)]
    pub dry_run: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistWorkspaceOutput<'a> {
    workspace: &'a ResolvedWorkspace,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistContextOutput<'a> {
    workspace_id: &'a str,
    issue_id: &'a str,
    context_set: &'a str,
    files: &'a [ResolvedContextFile],
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistStatusOutput {
    workspace_id: String,
    issue_id: String,
    checkpoint_path: PathBuf,
    checkpoint: Option<SpecialistCheckpoint>,
}

pub fn run_specialist_command(command: SpecialistCli) -> anyhow::Result<()> {
    match command.subcommand {
        SpecialistSubcommand::Init(args) => {
            run_specialist_init(args)?;
        }
        SpecialistSubcommand::ValidateWorkspace(args) => {
            let workspace = load_resolved_workspace(&args.locator)?;
            if args.json {
                print_json(&SpecialistWorkspaceOutput {
                    workspace: &workspace,
                })?;
            } else {
                print_workspace_summary(&workspace);
            }
        }
        SpecialistSubcommand::ShowContext(args) => {
            let workspace = load_resolved_workspace(&args.locator)?;
            let context_set = workspace.context_set(args.context_set.as_deref())?;
            if args.json {
                print_json(&SpecialistContextOutput {
                    workspace_id: &workspace.workspace_id,
                    issue_id: &workspace.issue_id,
                    context_set: &context_set.name,
                    files: &context_set.files,
                })?;
            } else {
                println!(
                    "Context set `{}` for workspace `{}`:",
                    context_set.name, workspace.workspace_id
                );
                for (index, file) in context_set.files.iter().enumerate() {
                    println!("{}. [{}] {}", index + 1, file.role, file.path.display());
                }
            }
        }
        SpecialistSubcommand::Status(args) => {
            let workspace = load_resolved_workspace(&args.locator)?;
            let codex_home = find_codex_home()?;
            let checkpoint =
                read_latest_checkpoint(&codex_home, &workspace.workspace_id, &workspace.issue_id)?;
            let checkpoint_path = workspace.checkpoint_paths(&codex_home).latest_json;
            if args.json {
                print_json(&SpecialistStatusOutput {
                    workspace_id: workspace.workspace_id,
                    issue_id: workspace.issue_id,
                    checkpoint_path,
                    checkpoint,
                })?;
            } else {
                println!("Workspace: {}", workspace.workspace_id);
                println!("Issue: {}", workspace.issue_id);
                println!("Checkpoint path: {}", checkpoint_path.display());
                if let Some(checkpoint) = checkpoint {
                    println!("Status: {:?}", checkpoint.status);
                    if let Some(next_concrete_step) = checkpoint.next_concrete_step.as_deref() {
                        println!("Next step: {next_concrete_step}");
                    }
                } else {
                    println!("No checkpoint found.");
                }
            }
        }
        SpecialistSubcommand::Manager(args) => {
            run_specialist_manager(args)?;
        }
        SpecialistSubcommand::Worker(args) => {
            run_specialist_worker(args)?;
        }
    }

    Ok(())
}

fn load_resolved_workspace(
    locator: &SpecialistWorkspaceLocatorArgs,
) -> anyhow::Result<ResolvedWorkspace> {
    let cwd = std::env::current_dir()?;
    load_workspace(
        &cwd,
        locator.workspace_manifest.as_deref(),
        locator.machine_profile.as_deref(),
    )
    .map_err(anyhow::Error::from)
}

fn print_workspace_summary(workspace: &ResolvedWorkspace) {
    println!("Workspace `{}` is valid.", workspace.workspace_id);
    println!("Manifest: {}", workspace.manifest_path.display());
    println!(
        "Machine profile: {}",
        workspace.machine_profile_path.display()
    );
    if let Some(default_context_set) = workspace.default_context_set.as_deref() {
        println!("Default context set: {default_context_set}");
    }
    println!("Roles:");
    for role in workspace.roles.values() {
        print_role(role);
    }
}

fn print_role(role: &ResolvedRole) {
    let access = match role.access {
        codex_specialist::RoleAccess::ReadOnly => "read-only",
        codex_specialist::RoleAccess::ReadWrite => "read-write",
    };
    println!("  - {} ({access}) {}", role.name, role.path.display());
}

fn print_json<T>(value: &T) -> anyhow::Result<()>
where
    T: Serialize,
{
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
