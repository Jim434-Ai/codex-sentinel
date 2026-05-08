use anyhow::Context;
use codex_core::config::find_codex_home;
use codex_specialist::SpecialistWorkerCommand;
use codex_specialist::SpecialistWorkerEvent;
use codex_specialist::decode_worker_command;
use codex_specialist::encode_worker_event;
use codex_specialist::load_specialist_session;
use codex_specialist::read_latest_checkpoint;
use codex_specialist::read_latest_context_snapshot;
use codex_specialist::write_latest_checkpoint;
use codex_specialist::write_specialist_event;
use std::io::BufRead;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

use crate::specialist_cmd::SpecialistWorkerArgs;

const MAX_ERROR_OUTPUT_CHARS: usize = 4096;

pub fn run_specialist_worker(args: SpecialistWorkerArgs) -> anyhow::Result<()> {
    let start_dir = std::env::current_dir().context("resolve current directory")?;
    let specialist = load_specialist_session(
        &start_dir,
        args.locator.workspace_manifest.as_deref(),
        args.locator.machine_profile.as_deref(),
        args.context_set.as_deref(),
    )
    .context("load specialist worker session")?;
    let codex_home = find_codex_home().context("find CODEX_HOME")?;
    let exec_bin = match args.exec_bin.clone() {
        Some(path) => path,
        None => std::env::current_exe().context("resolve current Codex executable")?,
    };
    let model = args
        .model
        .clone()
        .or_else(|| std::env::var("SPECIALIST_CODEX_MODEL").ok());
    let event_dir = args
        .event_dir
        .or_else(|| std::env::var_os("SPECIALIST_EVENT_DIR").map(PathBuf::from));

    let stdin = std::io::stdin();
    let mut stdout = std::io::BufWriter::new(std::io::stdout());
    emit_event(
        &mut stdout,
        &SpecialistWorkerEvent::Ready {
            workspace_id: specialist.workspace.workspace_id.clone(),
            issue_id: specialist.workspace.issue_id.clone(),
            worker_pid: std::process::id(),
        },
    )?;

    for line in stdin.lock().lines() {
        let line = line.context("read worker command")?;
        if line.trim().is_empty() {
            continue;
        }

        let command = match decode_worker_command(&line) {
            Ok(command) => command,
            Err(err) => {
                emit_event(
                    &mut stdout,
                    &SpecialistWorkerEvent::Error {
                        command_id: None,
                        message: format!("invalid worker command: {err}"),
                    },
                )?;
                continue;
            }
        };

        match command {
            SpecialistWorkerCommand::Status { id } => {
                let checkpoint = match read_latest_checkpoint(
                    &codex_home,
                    &specialist.workspace.workspace_id,
                    &specialist.workspace.issue_id,
                ) {
                    Ok(checkpoint) => checkpoint,
                    Err(err) => {
                        emit_event(
                            &mut stdout,
                            &SpecialistWorkerEvent::Error {
                                command_id: Some(id),
                                message: format!("read latest specialist checkpoint: {err:#}"),
                            },
                        )?;
                        continue;
                    }
                };
                emit_event(
                    &mut stdout,
                    &SpecialistWorkerEvent::Status {
                        command_id: id,
                        workspace_id: specialist.workspace.workspace_id.clone(),
                        issue_id: specialist.workspace.issue_id.clone(),
                        checkpoint: checkpoint.map(Box::new),
                    },
                )?;
            }
            SpecialistWorkerCommand::Prompt { id, prompt } => {
                emit_event(
                    &mut stdout,
                    &SpecialistWorkerEvent::TurnStarted {
                        command_id: id.clone(),
                    },
                )?;
                let outcome = if args.dry_run {
                    PromptOutcome {
                        exit_code: Some(0),
                        final_message: Some("dry-run prompt accepted".to_string()),
                        stderr: String::new(),
                        stdout: String::new(),
                    }
                } else {
                    match run_prompt_turn(
                        &exec_bin,
                        &specialist,
                        args.context_set.as_deref(),
                        model.as_deref(),
                        &prompt,
                    ) {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            emit_event(
                                &mut stdout,
                                &SpecialistWorkerEvent::Error {
                                    command_id: Some(id),
                                    message: format!("run prompt turn: {err:#}"),
                                },
                            )?;
                            continue;
                        }
                    }
                };

                emit_event(
                    &mut stdout,
                    &SpecialistWorkerEvent::TurnCompleted {
                        command_id: id.clone(),
                        exit_code: outcome.exit_code,
                        final_message: outcome.final_message.clone(),
                    },
                )?;

                if outcome.exit_code == Some(0) {
                    if let Some(event_dir) = event_dir.as_deref() {
                        match checkpoint_for_event(&codex_home, &specialist, &outcome, args.dry_run)
                            .and_then(|checkpoint| {
                                let mut event = codex_specialist::SpecialistEvent::from_checkpoint(
                                    specialist.workspace.workspace_id.clone(),
                                    specialist.workspace_root.clone(),
                                    &checkpoint,
                                );
                                apply_event_overrides(
                                    &codex_home,
                                    &specialist,
                                    args.event_recommended_next.as_deref(),
                                    &mut event,
                                )?;
                                write_specialist_event(event_dir, &event)
                                    .map(|_| ())
                                    .map_err(anyhow::Error::from)
                            }) {
                            Ok(()) => {}
                            Err(err) => {
                                emit_event(
                                    &mut stdout,
                                    &SpecialistWorkerEvent::Error {
                                        command_id: Some(id.clone()),
                                        message: format!("write durable specialist event: {err:#}"),
                                    },
                                )?;
                            }
                        }
                    }
                    emit_event(
                        &mut stdout,
                        &SpecialistWorkerEvent::NeedsDirection {
                            command_id: id,
                            reason: "prompt turn completed; manager should inspect the latest checkpoint and decide the next action".to_string(),
                        },
                    )?;
                } else {
                    emit_event(
                        &mut stdout,
                        &SpecialistWorkerEvent::Error {
                            command_id: Some(id),
                            message: format_failed_prompt_message(&outcome),
                        },
                    )?;
                }
            }
            SpecialistWorkerCommand::Shutdown { id } => {
                emit_event(
                    &mut stdout,
                    &SpecialistWorkerEvent::Exited {
                        command_id: id,
                        reason: "shutdown command received".to_string(),
                    },
                )?;
                break;
            }
        }
    }

    Ok(())
}

fn apply_event_overrides(
    codex_home: &Path,
    specialist: &codex_specialist::SpecialistSession,
    recommended_next: Option<&str>,
    event: &mut codex_specialist::SpecialistEvent,
) -> anyhow::Result<()> {
    if let Some(recommended_next) = recommended_next.map(str::trim)
        && !recommended_next.is_empty()
    {
        event.recommended_next = Some(recommended_next.to_string());
    }
    if let Some(snapshot) = read_latest_context_snapshot(
        codex_home,
        &specialist.workspace.workspace_id,
        &specialist.workspace.issue_id,
    )
    .context("read latest context snapshot for durable event")?
    {
        event.context_level = Some(context_level_name(snapshot.level).to_string());
    }
    Ok(())
}

fn context_level_name(level: codex_specialist::SpecialistContextLevel) -> &'static str {
    match level {
        codex_specialist::SpecialistContextLevel::High => "high",
        codex_specialist::SpecialistContextLevel::Medium => "medium",
        codex_specialist::SpecialistContextLevel::Low => "low",
        codex_specialist::SpecialistContextLevel::Critical => "critical",
    }
}

fn checkpoint_for_event(
    codex_home: &Path,
    specialist: &codex_specialist::SpecialistSession,
    outcome: &PromptOutcome,
    dry_run: bool,
) -> anyhow::Result<codex_specialist::SpecialistCheckpoint> {
    if let Some(checkpoint) = read_latest_checkpoint(
        codex_home,
        &specialist.workspace.workspace_id,
        &specialist.workspace.issue_id,
    )
    .context("read latest checkpoint for durable event")?
    {
        return Ok(checkpoint);
    }

    if !dry_run {
        anyhow::bail!("no checkpoint available for durable event");
    }

    let checkpoint = dry_run_checkpoint(specialist, outcome);
    write_latest_checkpoint(codex_home, &specialist.workspace.workspace_id, &checkpoint)
        .context("write dry-run checkpoint for durable event")?;
    Ok(checkpoint)
}

fn dry_run_checkpoint(
    specialist: &codex_specialist::SpecialistSession,
    outcome: &PromptOutcome,
) -> codex_specialist::SpecialistCheckpoint {
    codex_specialist::SpecialistCheckpoint {
        issue_id: specialist.workspace.issue_id.clone(),
        session_id: format!("dry-run-worker-{}", std::process::id()),
        timestamp: current_unix_timestamp(),
        objective: "Dry-run specialist worker prompt.".to_string(),
        current_phase: None,
        current_deliverable: None,
        status: codex_specialist::SpecialistCheckpointStatus::NeedsReview,
        last_completed_step: outcome.final_message.clone(),
        next_concrete_step: Some("Manager should inspect the dry-run event.".to_string()),
        stop_conditions: Vec::new(),
        blocked_reasons: Vec::new(),
        open_questions: Vec::new(),
        needs_from_user: Vec::new(),
        risk_level: None,
        files_relied_on: Vec::new(),
        citations_used: Vec::new(),
        files_changed: Vec::new(),
        deliverables_touched: Vec::new(),
        unresolved_proof_gaps: Vec::new(),
        recommended_next_prompt: Some("Continue the dry-run specialist fixture.".to_string()),
    }
}

fn current_unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

struct PromptOutcome {
    exit_code: Option<i32>,
    final_message: Option<String>,
    stdout: String,
    stderr: String,
}

fn run_prompt_turn(
    exec_bin: &Path,
    specialist: &codex_specialist::SpecialistSession,
    context_set: Option<&str>,
    model: Option<&str>,
    prompt: &str,
) -> anyhow::Result<PromptOutcome> {
    let last_message_file = tempfile::NamedTempFile::new().context("create last-message file")?;
    let last_message_path = last_message_file.path().to_path_buf();
    let mut command = Command::new(exec_bin);
    command
        .arg("exec")
        .arg("--specialist")
        .arg("--skip-git-repo-check")
        .arg("--json")
        .arg("--workspace-manifest")
        .arg(&specialist.workspace.manifest_path)
        .arg("--machine-profile")
        .arg(&specialist.workspace.machine_profile_path)
        .arg("--output-last-message")
        .arg(&last_message_path)
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
    let final_message = std::fs::read_to_string(&last_message_path)
        .ok()
        .and_then(|message| {
            let message = message.trim().to_string();
            if message.is_empty() {
                None
            } else {
                Some(message)
            }
        });

    Ok(PromptOutcome {
        exit_code: output.status.code(),
        final_message,
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn emit_event<W: Write>(writer: &mut W, event: &SpecialistWorkerEvent) -> anyhow::Result<()> {
    writeln!(writer, "{}", encode_worker_event(event)?).context("write worker event")?;
    writer.flush().context("flush worker event")?;
    Ok(())
}

fn format_failed_prompt_message(outcome: &PromptOutcome) -> String {
    let mut message = match outcome.exit_code {
        Some(code) => format!("codex exec prompt turn exited with status {code}"),
        None => "codex exec prompt turn was terminated by signal".to_string(),
    };
    let stderr = truncate_output(&outcome.stderr);
    if !stderr.is_empty() {
        message.push_str(": ");
        message.push_str(&stderr);
    } else {
        let stdout = truncate_output(&outcome.stdout);
        if !stdout.is_empty() {
            message.push_str(": ");
            message.push_str(&stdout);
        }
    }
    message
}

fn truncate_output(output: &str) -> String {
    let output = output.trim();
    if output.chars().count() <= MAX_ERROR_OUTPUT_CHARS {
        return output.to_string();
    }

    let truncated: String = output.chars().take(MAX_ERROR_OUTPUT_CHARS).collect();
    format!("{truncated}...")
}
