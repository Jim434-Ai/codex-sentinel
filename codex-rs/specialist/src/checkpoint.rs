use crate::manifest::SpecialistError;
use crate::runtime::SpecialistSession;
use codex_app_server_protocol::PatchApplyStatus;
use codex_app_server_protocol::Thread as AppServerThread;
use codex_app_server_protocol::ThreadItem as AppServerThreadItem;
use codex_app_server_protocol::TurnStatus;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpecialistCheckpointStatus {
    Working,
    Blocked,
    Waiting,
    NeedsReview,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SpecialistCheckpoint {
    pub issue_id: String,
    pub session_id: String,
    pub timestamp: i64,
    pub objective: String,
    pub current_phase: Option<String>,
    pub current_deliverable: Option<String>,
    pub status: SpecialistCheckpointStatus,
    pub last_completed_step: Option<String>,
    pub next_concrete_step: Option<String>,
    #[serde(default)]
    pub stop_conditions: Vec<String>,
    #[serde(default)]
    pub blocked_reasons: Vec<String>,
    #[serde(default)]
    pub open_questions: Vec<String>,
    #[serde(default)]
    pub needs_from_user: Vec<String>,
    pub risk_level: Option<String>,
    #[serde(default)]
    pub files_relied_on: Vec<PathBuf>,
    #[serde(default)]
    pub citations_used: Vec<String>,
    #[serde(default)]
    pub files_changed: Vec<PathBuf>,
    #[serde(default)]
    pub deliverables_touched: Vec<PathBuf>,
    #[serde(default)]
    pub unresolved_proof_gaps: Vec<String>,
    pub recommended_next_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointPaths {
    pub directory: PathBuf,
    pub latest_json: PathBuf,
    pub latest_markdown: PathBuf,
    pub history_directory: PathBuf,
}

impl CheckpointPaths {
    pub fn new(codex_home: &Path, workspace_id: &str, issue_id: &str) -> Self {
        let directory = codex_home
            .join("specialist")
            .join("checkpoints")
            .join(sanitize_path_component(workspace_id))
            .join(sanitize_path_component(issue_id));
        let history_directory = directory.join("history");
        Self {
            latest_json: directory.join("latest.json"),
            latest_markdown: directory.join("latest.md"),
            directory,
            history_directory,
        }
    }
}

pub fn read_latest_checkpoint(
    codex_home: &Path,
    workspace_id: &str,
    issue_id: &str,
) -> Result<Option<SpecialistCheckpoint>, SpecialistError> {
    let paths = CheckpointPaths::new(codex_home, workspace_id, issue_id);
    if !paths.latest_json.is_file() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&paths.latest_json).map_err(|source| {
        SpecialistError::ReadCheckpoint {
            path: paths.latest_json.clone(),
            source,
        }
    })?;
    let checkpoint =
        serde_json::from_str(&contents).map_err(|source| SpecialistError::ParseCheckpoint {
            path: paths.latest_json.clone(),
            source,
        })?;
    Ok(Some(checkpoint))
}

pub fn hydrate_latest_checkpoint(
    codex_home: &Path,
    workspace_id: &str,
    issue_id: &str,
) -> Result<Option<SpecialistCheckpoint>, SpecialistError> {
    read_latest_checkpoint(codex_home, workspace_id, issue_id)
}

pub fn write_latest_checkpoint(
    codex_home: &Path,
    workspace_id: &str,
    checkpoint: &SpecialistCheckpoint,
) -> Result<CheckpointPaths, SpecialistError> {
    let paths = CheckpointPaths::new(codex_home, workspace_id, &checkpoint.issue_id);
    fs::create_dir_all(&paths.history_directory).map_err(|source| {
        SpecialistError::CreateCheckpointDirectory {
            path: paths.history_directory.clone(),
            source,
        }
    })?;

    let checkpoint_json = serde_json::to_string_pretty(checkpoint)
        .map_err(|source| SpecialistError::SerializeCheckpointJson { source })?;
    let checkpoint_markdown = checkpoint_to_markdown(checkpoint);

    write_file(&paths.latest_json, &checkpoint_json)?;
    write_file(&paths.latest_markdown, &checkpoint_markdown)?;
    write_file(
        &paths
            .history_directory
            .join(format!("{}.json", checkpoint.timestamp)),
        &checkpoint_json,
    )?;
    write_file(
        &paths
            .history_directory
            .join(format!("{}.md", checkpoint.timestamp)),
        &checkpoint_markdown,
    )?;

    Ok(paths)
}

fn write_file(path: &Path, contents: &str) -> Result<(), SpecialistError> {
    fs::write(path, contents).map_err(|source| SpecialistError::WriteCheckpoint {
        path: path.to_path_buf(),
        source,
    })
}

fn checkpoint_to_markdown(checkpoint: &SpecialistCheckpoint) -> String {
    let mut lines = vec![
        format!("# Specialist Checkpoint: {}", checkpoint.issue_id),
        String::new(),
        format!("- Session: {}", checkpoint.session_id),
        format!("- Timestamp: {}", checkpoint.timestamp),
        format!("- Status: {:?}", checkpoint.status),
        format!("- Objective: {}", checkpoint.objective),
    ];

    if let Some(current_deliverable) = checkpoint.current_deliverable.as_deref() {
        lines.push(format!("- Current deliverable: {current_deliverable}"));
    }
    if let Some(current_phase) = checkpoint.current_phase.as_deref() {
        lines.push(format!("- Current phase: {current_phase}"));
    }
    if let Some(last_completed_step) = checkpoint.last_completed_step.as_deref() {
        lines.push(format!("- Last completed step: {last_completed_step}"));
    }
    if let Some(next_concrete_step) = checkpoint.next_concrete_step.as_deref() {
        lines.push(format!("- Next concrete step: {next_concrete_step}"));
    }
    if let Some(risk_level) = checkpoint.risk_level.as_deref() {
        lines.push(format!("- Risk level: {risk_level}"));
    }
    if let Some(recommended_next_prompt) = checkpoint.recommended_next_prompt.as_deref() {
        lines.push(format!(
            "- Recommended next prompt: {recommended_next_prompt}"
        ));
    }

    append_list_section(&mut lines, "Open Questions", &checkpoint.open_questions);
    append_list_section(&mut lines, "Needs From User", &checkpoint.needs_from_user);
    append_list_section(&mut lines, "Stop Conditions", &checkpoint.stop_conditions);
    append_list_section(&mut lines, "Blocked Reasons", &checkpoint.blocked_reasons);
    append_list_section(
        &mut lines,
        "Unresolved Proof Gaps",
        &checkpoint.unresolved_proof_gaps,
    );

    lines.join("\n")
}

fn append_list_section(lines: &mut Vec<String>, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }

    lines.push(String::new());
    lines.push(format!("## {title}"));
    for item in items {
        lines.push(format!("- {item}"));
    }
}

fn sanitize_path_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

pub fn build_checkpoint_for_turn(
    specialist: &SpecialistSession,
    thread: &AppServerThread,
    turn_id: &str,
    objective_fallback: &str,
) -> Option<SpecialistCheckpoint> {
    let turn = thread.turns.iter().find(|turn| turn.id == turn_id)?;
    let task_contract = specialist
        .task_contract
        .as_ref()
        .map(|contract| &contract.contract);
    let files_changed = collect_changed_files(&thread.cwd, &turn.items);
    let deliverables_touched = collect_deliverables_touched(specialist, &files_changed);
    let mut blocked_reasons = task_contract
        .map(|contract| contract.blocked_reasons.clone())
        .unwrap_or_default();
    if let Some(error) = turn.error.as_ref() {
        blocked_reasons.push(error.message.clone());
    }
    let current_phase = task_contract.and_then(|contract| contract.current_phase.clone());
    let current_deliverable = task_contract
        .and_then(|contract| contract.current_deliverable.clone())
        .or_else(|| {
            deliverables_touched
                .first()
                .map(|path| path.display().to_string())
        });
    let objective = task_contract
        .map(|contract| contract.objective.clone())
        .unwrap_or_else(|| objective_fallback.trim().to_string());
    let next_concrete_step =
        next_specialist_step(current_phase.as_deref(), current_deliverable.as_deref());

    Some(SpecialistCheckpoint {
        issue_id: specialist.workspace.issue_id.clone(),
        session_id: thread.id.clone(),
        timestamp: turn.completed_at.or(turn.started_at).unwrap_or_else(|| {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs() as i64)
                .unwrap_or_default()
        }),
        objective,
        current_phase,
        current_deliverable,
        status: match turn.status {
            TurnStatus::Completed => {
                if blocked_reasons.is_empty() {
                    SpecialistCheckpointStatus::NeedsReview
                } else {
                    SpecialistCheckpointStatus::Blocked
                }
            }
            TurnStatus::Interrupted => SpecialistCheckpointStatus::Waiting,
            TurnStatus::Failed => SpecialistCheckpointStatus::Failed,
            TurnStatus::InProgress => SpecialistCheckpointStatus::Working,
        },
        last_completed_step: last_agent_message_summary(&turn.items),
        next_concrete_step,
        stop_conditions: task_contract
            .map(|contract| contract.stop_conditions.clone())
            .unwrap_or_default(),
        blocked_reasons,
        open_questions: Vec::new(),
        needs_from_user: Vec::new(),
        risk_level: None,
        files_relied_on: specialist
            .context_set
            .as_ref()
            .map(|context_set| {
                context_set
                    .files
                    .iter()
                    .map(|file| file.path.clone())
                    .collect()
            })
            .unwrap_or_default(),
        citations_used: Vec::new(),
        files_changed,
        deliverables_touched,
        unresolved_proof_gaps: Vec::new(),
        recommended_next_prompt: Some(
            "Continue the specialist workspace from the latest checkpoint, reload the pinned context, and execute the next unfinished step.".to_string(),
        ),
    })
}

pub fn write_checkpoint_for_turn(
    codex_home: &Path,
    specialist: &mut SpecialistSession,
    thread: &AppServerThread,
    turn_id: &str,
    objective_fallback: &str,
) -> Result<Option<CheckpointPaths>, SpecialistError> {
    let Some(checkpoint) =
        build_checkpoint_for_turn(specialist, thread, turn_id, objective_fallback)
    else {
        return Ok(None);
    };

    let paths =
        write_latest_checkpoint(codex_home, &specialist.workspace.workspace_id, &checkpoint)?;
    specialist.latest_checkpoint = Some(checkpoint);
    Ok(Some(paths))
}

fn next_specialist_step(
    current_phase: Option<&str>,
    current_deliverable: Option<&str>,
) -> Option<String> {
    match (current_phase, current_deliverable) {
        (Some(current_phase), Some(current_deliverable)) => Some(format!(
            "Continue phase `{current_phase}` and advance `{current_deliverable}`."
        )),
        (Some(current_phase), None) => Some(format!("Continue phase `{current_phase}`.")),
        (None, Some(current_deliverable)) => {
            Some(format!("Continue work on `{current_deliverable}`."))
        }
        (None, None) => Some(
            "Resume from this checkpoint, verify unfinished work, and continue the next concrete specialist task."
                .to_string(),
        ),
    }
}

fn last_agent_message_summary(items: &[AppServerThreadItem]) -> Option<String> {
    items.iter().rev().find_map(|item| match item {
        AppServerThreadItem::AgentMessage { text, .. } => {
            let summary = text.lines().find(|line| !line.trim().is_empty())?.trim();
            let mut truncated = summary.chars().take(240).collect::<String>();
            if summary.chars().count() > 240 {
                truncated.push_str("...");
            }
            Some(truncated)
        }
        _ => None,
    })
}

fn collect_changed_files(cwd: &Path, items: &[AppServerThreadItem]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for item in items {
        if let AppServerThreadItem::FileChange {
            changes,
            status: PatchApplyStatus::Completed,
            ..
        } = item
        {
            for change in changes {
                let path = PathBuf::from(&change.path);
                files.push(if path.is_absolute() {
                    path
                } else {
                    cwd.join(path)
                });
            }
        }
    }
    files
}

fn collect_deliverables_touched(
    specialist: &SpecialistSession,
    files_changed: &[PathBuf],
) -> Vec<PathBuf> {
    let writable_roots: Vec<&Path> = specialist
        .workspace
        .roles
        .values()
        .filter(|role| role.access == crate::RoleAccess::ReadWrite)
        .map(|role| role.path.as_path())
        .collect();

    files_changed
        .iter()
        .filter(|path| writable_roots.iter().any(|root| path.starts_with(root)))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::SpecialistCheckpoint;
    use super::SpecialistCheckpointStatus;
    use super::read_latest_checkpoint;
    use super::write_latest_checkpoint;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    #[test]
    fn checkpoint_roundtrip_updates_latest_and_history() {
        let codex_home = TempDir::new().expect("codex home");
        let checkpoint = SpecialistCheckpoint {
            issue_id: "issue-123".to_string(),
            session_id: "session-abc".to_string(),
            timestamp: 1_744_309_600,
            objective: "Finalize the master memo".to_string(),
            current_phase: Some("final drafting".to_string()),
            current_deliverable: Some("deliverable.md".to_string()),
            status: SpecialistCheckpointStatus::Working,
            last_completed_step: Some("Reviewed the source corpus".to_string()),
            next_concrete_step: Some("Draft the conclusion section".to_string()),
            stop_conditions: vec!["Draft the conclusion section".to_string()],
            blocked_reasons: vec!["Missing exhibit 5".to_string()],
            open_questions: vec!["Need one more cite".to_string()],
            needs_from_user: vec!["Confirm the target filing date".to_string()],
            risk_level: Some("medium".to_string()),
            files_relied_on: Vec::new(),
            citations_used: Vec::new(),
            files_changed: Vec::new(),
            deliverables_touched: Vec::new(),
            unresolved_proof_gaps: vec!["Missing corroborating invoice".to_string()],
            recommended_next_prompt: Some("Continue drafting the conclusion.".to_string()),
        };

        let paths = write_latest_checkpoint(codex_home.path(), "workspace-main", &checkpoint)
            .expect("write checkpoint");
        let roundtrip =
            read_latest_checkpoint(codex_home.path(), "workspace-main", &checkpoint.issue_id)
                .expect("read checkpoint")
                .expect("checkpoint present");

        assert_eq!(roundtrip, checkpoint);
        assert!(paths.latest_json.is_file());
        assert!(paths.latest_markdown.is_file());
        assert!(
            paths
                .history_directory
                .join(format!("{}.json", checkpoint.timestamp))
                .is_file()
        );
    }
}
