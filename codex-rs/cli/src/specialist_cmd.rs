use anyhow::Context;
use anyhow::bail;
use clap::Args;
use clap::Subcommand;
use codex_core::config::find_codex_home;
use codex_specialist::ResolvedContextFile;
use codex_specialist::ResolvedRole;
use codex_specialist::ResolvedWorkspace;
use codex_specialist::SpecialistCheckpoint;
use codex_specialist::SpecialistContextLevel;
use codex_specialist::SpecialistContextSnapshot;
use codex_specialist::load_specialist_session;
use codex_specialist::load_workspace;
use codex_specialist::read_latest_checkpoint;
use codex_specialist::read_latest_context_snapshot;
use codex_specialist::write_latest_context_snapshot;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use crate::specialist_init::SpecialistInitArgs;
use crate::specialist_init::run_specialist_init;
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

    /// Run a JSONL specialist worker loop for manager-owned lifecycle control.
    Worker(SpecialistWorkerArgs),

    /// Send one manager message to a specialist session and return an acknowledgement.
    Send(SpecialistSendArgs),
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

    /// Model used for prompt turns. Defaults to SPECIALIST_CODEX_MODEL when set.
    #[arg(short = 'm', long = "model", value_name = "MODEL")]
    pub model: Option<String>,

    /// Codex binary used for prompt turns. Defaults to the current executable.
    #[arg(long = "exec-bin", value_name = "FILE")]
    pub exec_bin: Option<PathBuf>,

    /// Accept commands and emit events without running model turns.
    #[arg(long = "dry-run", default_value_t = false)]
    pub dry_run: bool,

    /// Directory for durable specialist events consumed by a manager.
    #[arg(long = "event-dir", value_name = "DIR")]
    pub event_dir: Option<PathBuf>,

    /// Override the recommended continuation prompt written to durable events.
    #[arg(long = "event-recommended-next", value_name = "PROMPT")]
    pub event_recommended_next: Option<String>,
}

#[derive(Debug, Args)]
pub struct SpecialistSendArgs {
    #[command(flatten)]
    locator: SpecialistWorkspaceLocatorArgs,

    /// Session id that should receive the message.
    #[arg(long = "session", value_name = "ID")]
    session: String,

    /// File containing the message to send.
    #[arg(long = "message-file", value_name = "FILE")]
    message_file: PathBuf,

    /// Idempotency key used to avoid duplicate prompt delivery.
    #[arg(long = "idempotency-key", value_name = "KEY")]
    idempotency_key: Option<String>,

    /// Named context set to preload for the prompt turn.
    #[arg(long = "context-set", value_name = "NAME")]
    context_set: Option<String>,

    /// Model used for the prompt turn. Defaults to SPECIALIST_CODEX_MODEL when set.
    #[arg(short = 'm', long = "model", value_name = "MODEL")]
    model: Option<String>,

    /// Codex binary used for prompt turns. Defaults to the current executable.
    #[arg(long = "exec-bin", value_name = "FILE")]
    exec_bin: Option<PathBuf>,

    /// Accept the message without running a model turn.
    #[arg(long = "dry-run", default_value_t = false)]
    dry_run: bool,

    /// Print the acknowledgement as JSON.
    #[arg(long = "json", default_value_t = false)]
    json: bool,
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
    agent_id: String,
    workspace_id: String,
    issue_id: String,
    workspace: PathBuf,
    session_id: Option<String>,
    state: SpecialistStatusState,
    model: Option<String>,
    context_level: Option<String>,
    context_report: SpecialistContextReport,
    current_task: Option<String>,
    last_activity_at: Option<i64>,
    newest_artifacts: Vec<PathBuf>,
    blockers: Vec<String>,
    needs_user: bool,
    recommended_next: Option<String>,
    artifact_report: SpecialistArtifactReport,
    checkpoint_path: PathBuf,
    checkpoint: Option<SpecialistCheckpoint>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
enum SpecialistStatusState {
    Working,
    WaitingContinuation,
    NeedsUser,
    Blocked,
    Error,
    Stale,
    Drifting,
    Paused,
    UserControl,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistArtifactReport {
    created_files: Vec<PathBuf>,
    updated_derived_files: Vec<PathBuf>,
    original_source_files_consulted: Vec<PathBuf>,
    verification_commands: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistContextReport {
    source: &'static str,
    level: Option<String>,
    percent_remaining: Option<u8>,
    tokens_remaining: Option<i64>,
    model_context_window: Option<i64>,
    compression_expected_soon: Option<bool>,
    safe_to_continue: Option<bool>,
    unavailable_reason: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecialistSendAck {
    accepted: bool,
    started: bool,
    message_id: String,
    session_id: String,
    duplicate: bool,
    error: Option<String>,
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
            let context_snapshot = read_latest_context_snapshot(
                &codex_home,
                &workspace.workspace_id,
                &workspace.issue_id,
            )?;
            let checkpoint_path = workspace.checkpoint_paths(&codex_home).latest_json;
            if args.json {
                print_json(&status_output(
                    workspace,
                    checkpoint_path,
                    checkpoint,
                    context_snapshot,
                ))?;
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
        SpecialistSubcommand::Worker(args) => {
            run_specialist_worker(args)?;
        }
        SpecialistSubcommand::Send(args) => {
            run_specialist_send(args)?;
        }
    }

    Ok(())
}

fn run_specialist_send(args: SpecialistSendArgs) -> anyhow::Result<()> {
    let message = fs::read_to_string(&args.message_file)
        .with_context(|| format!("read message file {}", args.message_file.display()))?;
    let message = message.trim();
    if message.is_empty() {
        bail!("message file must not be empty");
    }

    let codex_home = find_codex_home()?;
    let command_id = next_message_id("specialist-send");
    let message_id = args
        .idempotency_key
        .clone()
        .unwrap_or_else(|| command_id.clone());
    if let Some(idempotency_key) = args.idempotency_key.as_deref() {
        let ack_path = specialist_send_ack_path(&codex_home, &args.session, idempotency_key);
        if ack_path.is_file() {
            let ack = fs::read_to_string(&ack_path)
                .with_context(|| format!("read specialist send ack {}", ack_path.display()))?;
            print_ack_text(&ack, args.json)?;
            return Ok(());
        }
    }

    let specialist = load_specialist_session(
        &std::env::current_dir().context("resolve current directory")?,
        args.locator.workspace_manifest.as_deref(),
        args.locator.machine_profile.as_deref(),
        args.context_set.as_deref(),
    )
    .context("load specialist session")?;

    let ack = if args.dry_run {
        SpecialistSendAck {
            accepted: true,
            started: true,
            message_id,
            session_id: args.session.clone(),
            duplicate: false,
            error: None,
        }
    } else {
        let exec_bin = match args.exec_bin {
            Some(path) => path,
            None => std::env::current_exe().context("resolve current Codex executable")?,
        };
        let model = args
            .model
            .as_deref()
            .map(str::to_string)
            .or_else(|| std::env::var("SPECIALIST_CODEX_MODEL").ok());
        match run_send_prompt_turn(
            &exec_bin,
            &specialist,
            args.context_set.as_deref(),
            model.as_deref(),
            message,
        ) {
            Ok(result) if result.exit_code == Some(0) => {
                if let Some(mut context_snapshot) = result.context_snapshot {
                    context_snapshot.model = model;
                    write_latest_context_snapshot(
                        &codex_home,
                        &specialist.workspace.workspace_id,
                        &specialist.workspace.issue_id,
                        &context_snapshot,
                    )
                    .context("write specialist context snapshot")?;
                }
                SpecialistSendAck {
                    accepted: true,
                    started: true,
                    message_id,
                    session_id: args.session.clone(),
                    duplicate: false,
                    error: None,
                }
            }
            Ok(result) => SpecialistSendAck {
                accepted: false,
                started: true,
                message_id,
                session_id: args.session.clone(),
                duplicate: false,
                error: Some(match result.exit_code {
                    Some(code) => format!("codex exec prompt turn exited with status {code}"),
                    None => "codex exec prompt turn was terminated by signal".to_string(),
                }),
            },
            Err(err) => SpecialistSendAck {
                accepted: false,
                started: false,
                message_id,
                session_id: args.session.clone(),
                duplicate: false,
                error: Some(format!("{err:#}")),
            },
        }
    };

    if let Some(idempotency_key) = args.idempotency_key.as_deref() {
        let ack_path = specialist_send_ack_path(&codex_home, &args.session, idempotency_key);
        if let Some(parent) = ack_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create specialist send ack dir {}", parent.display()))?;
        }
        fs::write(&ack_path, serde_json::to_string_pretty(&ack)?)
            .with_context(|| format!("write specialist send ack {}", ack_path.display()))?;
    }

    if args.json {
        print_json(&ack)?;
    } else {
        println!(
            "specialist send acknowledged session_id={} message_id={} accepted={} started={}",
            ack.session_id, ack.message_id, ack.accepted, ack.started
        );
    }
    Ok(())
}

fn status_output(
    workspace: ResolvedWorkspace,
    checkpoint_path: PathBuf,
    checkpoint: Option<SpecialistCheckpoint>,
    context_snapshot: Option<SpecialistContextSnapshot>,
) -> SpecialistStatusOutput {
    debug_assert_eq!(canonical_status_states().len(), 9);
    let state = checkpoint
        .as_ref()
        .map(checkpoint_state)
        .unwrap_or(SpecialistStatusState::Paused);
    let session_id = checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.session_id.clone());
    let current_task = checkpoint.as_ref().map(checkpoint_current_task);
    let last_activity_at = checkpoint.as_ref().map(|checkpoint| checkpoint.timestamp);
    let newest_artifacts = checkpoint
        .as_ref()
        .map(checkpoint_newest_artifacts)
        .unwrap_or_default();
    let blockers = checkpoint
        .as_ref()
        .map(|checkpoint| checkpoint.blocked_reasons.clone())
        .unwrap_or_default();
    let needs_user = checkpoint
        .as_ref()
        .is_some_and(|checkpoint| !checkpoint.needs_from_user.is_empty());
    let recommended_next = checkpoint
        .as_ref()
        .and_then(|checkpoint| checkpoint.recommended_next_prompt.clone());
    let artifact_report = checkpoint
        .as_ref()
        .map(checkpoint_artifact_report)
        .unwrap_or_else(empty_artifact_report);
    let context_report = context_report(context_snapshot.as_ref(), checkpoint.as_ref());
    let model = context_snapshot
        .as_ref()
        .and_then(|context_snapshot| context_snapshot.model.clone());
    let workspace_root = workspace.workspace_root();
    SpecialistStatusOutput {
        agent_id: workspace.workspace_id.clone(),
        workspace_id: workspace.workspace_id,
        issue_id: workspace.issue_id,
        workspace: workspace_root,
        session_id,
        state,
        model,
        context_level: context_report.level.clone(),
        context_report,
        current_task,
        last_activity_at,
        newest_artifacts,
        blockers,
        needs_user,
        recommended_next,
        artifact_report,
        checkpoint_path,
        checkpoint,
    }
}

fn context_report(
    context_snapshot: Option<&SpecialistContextSnapshot>,
    _checkpoint: Option<&SpecialistCheckpoint>,
) -> SpecialistContextReport {
    if let Some(context_snapshot) = context_snapshot {
        return SpecialistContextReport {
            source: "runtime",
            level: Some(context_level_name(context_snapshot.level).to_string()),
            percent_remaining: Some(context_snapshot.percent_remaining),
            tokens_remaining: Some(context_snapshot.tokens_remaining),
            model_context_window: Some(context_snapshot.model_context_window),
            compression_expected_soon: Some(context_snapshot.compression_expected_soon),
            safe_to_continue: Some(context_snapshot.safe_to_continue),
            unavailable_reason: None,
        };
    }

    SpecialistContextReport {
        source: "unavailable",
        level: None,
        percent_remaining: None,
        tokens_remaining: None,
        model_context_window: None,
        compression_expected_soon: None,
        safe_to_continue: None,
        unavailable_reason: Some(
            "runtime token usage is not persisted in specialist checkpoints".to_string(),
        ),
    }
}

fn context_level_name(level: SpecialistContextLevel) -> &'static str {
    match level {
        SpecialistContextLevel::High => "high",
        SpecialistContextLevel::Medium => "medium",
        SpecialistContextLevel::Low => "low",
        SpecialistContextLevel::Critical => "critical",
    }
}

fn canonical_status_states() -> [SpecialistStatusState; 9] {
    [
        SpecialistStatusState::Working,
        SpecialistStatusState::WaitingContinuation,
        SpecialistStatusState::NeedsUser,
        SpecialistStatusState::Blocked,
        SpecialistStatusState::Error,
        SpecialistStatusState::Stale,
        SpecialistStatusState::Drifting,
        SpecialistStatusState::Paused,
        SpecialistStatusState::UserControl,
    ]
}

fn checkpoint_state(checkpoint: &SpecialistCheckpoint) -> SpecialistStatusState {
    if !checkpoint.needs_from_user.is_empty() {
        return SpecialistStatusState::NeedsUser;
    }

    match checkpoint.status {
        codex_specialist::SpecialistCheckpointStatus::Working => SpecialistStatusState::Working,
        codex_specialist::SpecialistCheckpointStatus::Blocked => SpecialistStatusState::Blocked,
        codex_specialist::SpecialistCheckpointStatus::Waiting
        | codex_specialist::SpecialistCheckpointStatus::NeedsReview
        | codex_specialist::SpecialistCheckpointStatus::Completed => {
            SpecialistStatusState::WaitingContinuation
        }
        codex_specialist::SpecialistCheckpointStatus::Failed => SpecialistStatusState::Error,
    }
}

fn checkpoint_current_task(checkpoint: &SpecialistCheckpoint) -> String {
    checkpoint
        .current_deliverable
        .clone()
        .or_else(|| checkpoint.current_phase.clone())
        .unwrap_or_else(|| checkpoint.objective.clone())
}

fn checkpoint_newest_artifacts(checkpoint: &SpecialistCheckpoint) -> Vec<PathBuf> {
    checkpoint
        .deliverables_touched
        .iter()
        .chain(checkpoint.files_changed.iter())
        .cloned()
        .collect()
}

fn checkpoint_artifact_report(checkpoint: &SpecialistCheckpoint) -> SpecialistArtifactReport {
    SpecialistArtifactReport {
        created_files: checkpoint.files_changed.clone(),
        updated_derived_files: checkpoint.deliverables_touched.clone(),
        original_source_files_consulted: checkpoint.files_relied_on.clone(),
        verification_commands: Vec::new(),
    }
}

fn empty_artifact_report() -> SpecialistArtifactReport {
    SpecialistArtifactReport {
        created_files: Vec::new(),
        updated_derived_files: Vec::new(),
        original_source_files_consulted: Vec::new(),
        verification_commands: Vec::new(),
    }
}

#[derive(Debug)]
struct PromptTurnResult {
    exit_code: Option<i32>,
    context_snapshot: Option<SpecialistContextSnapshot>,
}

fn run_send_prompt_turn(
    exec_bin: &Path,
    specialist: &codex_specialist::SpecialistSession,
    context_set: Option<&str>,
    model: Option<&str>,
    prompt: &str,
) -> anyhow::Result<PromptTurnResult> {
    let mut command = Command::new(exec_bin);
    command
        .arg("exec")
        .arg("--json")
        .arg("--specialist")
        .arg("--skip-git-repo-check")
        .arg("--workspace-manifest")
        .arg(&specialist.workspace.manifest_path)
        .arg("--machine-profile")
        .arg(&specialist.workspace.machine_profile_path)
        .current_dir(&specialist.workspace_root);
    if let Some(context_set) = context_set {
        command.arg("--context-set").arg(context_set);
    }
    if let Some(model) = model {
        command.arg("-m").arg(model);
    }
    command.arg("--").arg(prompt);

    let output = command.output().with_context(|| {
        format!(
            "run Codex specialist prompt turn with executable {}",
            exec_bin.display()
        )
    })?;
    Ok(PromptTurnResult {
        exit_code: output.status.code(),
        context_snapshot: context_snapshot_from_exec_jsonl(&output.stdout),
    })
}

fn context_snapshot_from_exec_jsonl(stdout: &[u8]) -> Option<SpecialistContextSnapshot> {
    let stdout = std::str::from_utf8(stdout).ok()?;
    let mut latest = None;
    for line in stdout.lines() {
        let event: serde_json::Value = serde_json::from_str(line).ok()?;
        if event.get("type").and_then(serde_json::Value::as_str) != Some("turn.completed") {
            continue;
        }
        let usage = event.get("usage")?;
        let total_tokens = usage.get("total_tokens")?.as_i64()?;
        let model_context_window = usage.get("model_context_window")?.as_i64()?;
        latest = context_snapshot_from_usage(total_tokens, model_context_window);
    }
    latest
}

fn context_snapshot_from_usage(
    total_tokens: i64,
    model_context_window: i64,
) -> Option<SpecialistContextSnapshot> {
    SpecialistContextSnapshot::from_usage(
        total_tokens,
        model_context_window,
        current_unix_timestamp(),
    )
}

fn specialist_send_ack_path(codex_home: &Path, session_id: &str, idempotency_key: &str) -> PathBuf {
    codex_home
        .join("specialist-send-acks")
        .join(sanitize_ack_component(session_id))
        .join(format!("{}.json", sanitize_ack_component(idempotency_key)))
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

fn next_message_id(prefix: &str) -> String {
    let millis = current_unix_millis();
    format!("{prefix}-{}-{millis}", std::process::id())
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

fn current_unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn print_ack_text(ack: &str, json: bool) -> anyhow::Result<()> {
    let mut parsed: SpecialistSendAck = serde_json::from_str(ack)?;
    parsed.duplicate = true;
    if json {
        print_json(&parsed)?;
    } else {
        println!(
            "specialist send already handled session_id={} message_id={} accepted={} started={} duplicate={}",
            parsed.session_id, parsed.message_id, parsed.accepted, parsed.started, parsed.duplicate
        );
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
