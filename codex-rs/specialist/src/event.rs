use crate::SpecialistCheckpoint;
use crate::SpecialistCheckpointStatus;
use crate::manifest::SpecialistError;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistEvent {
    pub event_id: String,
    pub agent_id: String,
    pub workspace: PathBuf,
    pub event_type: SpecialistEventType,
    pub severity: SpecialistEventSeverity,
    pub summary: String,
    pub created_at: i64,
    pub completed_outputs: Vec<SpecialistCompletedOutput>,
    pub recommended_next: Option<String>,
    pub context_level: Option<String>,
    pub approval_gate: SpecialistApprovalGate,
    pub needs_user: bool,
    pub source_gaps: Vec<String>,
    #[serde(default)]
    pub blocked_alternative: Option<SpecialistBlockedAlternative>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpecialistEventType {
    Started,
    CheckpointWritten,
    ArtifactCreated,
    Done,
    WaitingContinuation,
    NeedsUser,
    Blocked,
    Error,
    LowContext,
    RecommendationReady,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpecialistEventSeverity {
    Info,
    Warning,
    Error,
    Critical,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpecialistApprovalGate {
    PlainContinuation,
    NeedsUser,
    ExternalFacing,
    SourceEdit,
    Strategy,
    BroadLowContext,
    Blocked,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistCompletedOutput {
    pub artifact_path: PathBuf,
    pub artifact_type: String,
    pub version: Option<String>,
    pub source_basis: Vec<PathBuf>,
    pub verification_status: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistBlockedAlternative {
    pub recommended_next: String,
    pub approval_gate: SpecialistApprovalGate,
    pub context_level: Option<String>,
}

impl SpecialistEvent {
    pub fn from_checkpoint(
        agent_id: String,
        workspace: PathBuf,
        checkpoint: &SpecialistCheckpoint,
    ) -> Self {
        let (event_type, severity, approval_gate) = classify_checkpoint(checkpoint);
        let created_at = current_unix_timestamp();
        let event_id = format!(
            "{}-{}-{}",
            sanitize_event_id_part(&agent_id),
            checkpoint.timestamp,
            event_type_name(event_type)
        );
        Self {
            event_id,
            agent_id,
            workspace,
            event_type,
            severity,
            summary: checkpoint_summary(checkpoint),
            created_at,
            completed_outputs: checkpoint
                .deliverables_touched
                .iter()
                .chain(checkpoint.files_changed.iter())
                .cloned()
                .map(|artifact_path| SpecialistCompletedOutput {
                    artifact_path,
                    artifact_type: "file".to_string(),
                    version: None,
                    source_basis: checkpoint.files_relied_on.clone(),
                    verification_status: None,
                })
                .collect(),
            recommended_next: checkpoint.recommended_next_prompt.clone(),
            context_level: None,
            approval_gate,
            needs_user: !checkpoint.needs_from_user.is_empty(),
            source_gaps: checkpoint.unresolved_proof_gaps.clone(),
            blocked_alternative: None,
        }
    }
}

pub fn write_specialist_event(
    event_dir: &Path,
    event: &SpecialistEvent,
) -> Result<PathBuf, SpecialistError> {
    fs::create_dir_all(event_dir).map_err(|source| SpecialistError::CreateEventDirectory {
        path: event_dir.to_path_buf(),
        source,
    })?;
    let path = event_dir.join(format!("{}.json", sanitize_event_id_part(&event.event_id)));
    let contents = serde_json::to_string_pretty(event)
        .map_err(|source| SpecialistError::SerializeEventJson { source })?;
    fs::write(&path, contents).map_err(|source| SpecialistError::WriteEvent {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

fn classify_checkpoint(
    checkpoint: &SpecialistCheckpoint,
) -> (
    SpecialistEventType,
    SpecialistEventSeverity,
    SpecialistApprovalGate,
) {
    if !checkpoint.needs_from_user.is_empty() {
        return (
            SpecialistEventType::NeedsUser,
            SpecialistEventSeverity::Warning,
            SpecialistApprovalGate::NeedsUser,
        );
    }

    match checkpoint.status {
        SpecialistCheckpointStatus::Working => (
            SpecialistEventType::CheckpointWritten,
            SpecialistEventSeverity::Info,
            SpecialistApprovalGate::PlainContinuation,
        ),
        SpecialistCheckpointStatus::Blocked => (
            SpecialistEventType::Blocked,
            SpecialistEventSeverity::Warning,
            SpecialistApprovalGate::Blocked,
        ),
        SpecialistCheckpointStatus::Waiting | SpecialistCheckpointStatus::NeedsReview => (
            SpecialistEventType::WaitingContinuation,
            SpecialistEventSeverity::Info,
            SpecialistApprovalGate::PlainContinuation,
        ),
        SpecialistCheckpointStatus::Completed => (
            SpecialistEventType::Done,
            SpecialistEventSeverity::Info,
            SpecialistApprovalGate::PlainContinuation,
        ),
        SpecialistCheckpointStatus::Failed => (
            SpecialistEventType::Error,
            SpecialistEventSeverity::Error,
            SpecialistApprovalGate::Blocked,
        ),
    }
}

fn checkpoint_summary(checkpoint: &SpecialistCheckpoint) -> String {
    checkpoint
        .last_completed_step
        .clone()
        .or_else(|| checkpoint.next_concrete_step.clone())
        .unwrap_or_else(|| checkpoint.objective.clone())
}

fn current_unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

fn event_type_name(event_type: SpecialistEventType) -> &'static str {
    match event_type {
        SpecialistEventType::Started => "started",
        SpecialistEventType::CheckpointWritten => "checkpoint-written",
        SpecialistEventType::ArtifactCreated => "artifact-created",
        SpecialistEventType::Done => "done",
        SpecialistEventType::WaitingContinuation => "waiting-continuation",
        SpecialistEventType::NeedsUser => "needs-user",
        SpecialistEventType::Blocked => "blocked",
        SpecialistEventType::Error => "error",
        SpecialistEventType::LowContext => "low-context",
        SpecialistEventType::RecommendationReady => "recommendation-ready",
    }
}

fn sanitize_event_id_part(value: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::SpecialistApprovalGate;
    use super::SpecialistEvent;
    use super::SpecialistEventSeverity;
    use super::SpecialistEventType;
    use super::write_specialist_event;
    use crate::SpecialistCheckpoint;
    use crate::SpecialistCheckpointStatus;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[test]
    fn checkpoint_waiting_maps_to_plain_continuation_event() {
        let checkpoint = checkpoint(SpecialistCheckpointStatus::NeedsReview);
        let event = SpecialistEvent::from_checkpoint(
            "agent-003".to_string(),
            Path::new("/workspace").to_path_buf(),
            &checkpoint,
        );

        assert_eq!(event.event_type, SpecialistEventType::WaitingContinuation);
        assert_eq!(event.severity, SpecialistEventSeverity::Info);
        assert_eq!(
            event.approval_gate,
            SpecialistApprovalGate::PlainContinuation
        );
        assert_eq!(event.needs_user, false);
        assert_eq!(event.completed_outputs.len(), 1);
    }

    #[test]
    fn checkpoint_needs_user_overrides_status() {
        let mut checkpoint = checkpoint(SpecialistCheckpointStatus::NeedsReview);
        checkpoint.needs_from_user = vec!["Choose source package.".to_string()];
        let event = SpecialistEvent::from_checkpoint(
            "agent-003".to_string(),
            Path::new("/workspace").to_path_buf(),
            &checkpoint,
        );

        assert_eq!(event.event_type, SpecialistEventType::NeedsUser);
        assert_eq!(event.approval_gate, SpecialistApprovalGate::NeedsUser);
        assert_eq!(event.needs_user, true);
    }

    #[test]
    fn write_event_creates_json_file() {
        let tempdir = tempfile::TempDir::new().expect("tempdir");
        let checkpoint = checkpoint(SpecialistCheckpointStatus::NeedsReview);
        let event = SpecialistEvent::from_checkpoint(
            "agent-003".to_string(),
            Path::new("/workspace").to_path_buf(),
            &checkpoint,
        );

        let path = write_specialist_event(tempdir.path(), &event).expect("write event");
        let written = std::fs::read_to_string(path).expect("read event");
        let decoded: SpecialistEvent = serde_json::from_str(&written).expect("decode event");

        assert_eq!(decoded, event);
    }

    #[test]
    fn phase_three_required_event_fixtures_round_trip() {
        let tempdir = tempfile::TempDir::new().expect("tempdir");
        let mut needs_user_checkpoint = checkpoint(SpecialistCheckpointStatus::NeedsReview);
        needs_user_checkpoint.needs_from_user = vec!["Choose source package.".to_string()];
        let artifact_event = SpecialistEvent {
            event_id: "agent-003-1744309600-artifact-created".to_string(),
            agent_id: "agent-003".to_string(),
            workspace: Path::new("/workspace").to_path_buf(),
            event_type: SpecialistEventType::ArtifactCreated,
            severity: SpecialistEventSeverity::Info,
            summary: "Created issue memo artifact".to_string(),
            created_at: 1_744_309_600,
            completed_outputs: vec![crate::event::SpecialistCompletedOutput {
                artifact_path: Path::new("/workspace/memo.md").to_path_buf(),
                artifact_type: "file".to_string(),
                version: Some("V1.0".to_string()),
                source_basis: vec![Path::new("/source/case-file.md").to_path_buf()],
                verification_status: Some("fixture-verified".to_string()),
            }],
            recommended_next: Some("Review the created artifact.".to_string()),
            context_level: Some("medium".to_string()),
            approval_gate: SpecialistApprovalGate::PlainContinuation,
            needs_user: false,
            source_gaps: Vec::new(),
            blocked_alternative: None,
        };
        let fixtures = vec![
            SpecialistEvent::from_checkpoint(
                "agent-003".to_string(),
                Path::new("/workspace").to_path_buf(),
                &checkpoint(SpecialistCheckpointStatus::NeedsReview),
            ),
            SpecialistEvent::from_checkpoint(
                "agent-003".to_string(),
                Path::new("/workspace").to_path_buf(),
                &needs_user_checkpoint,
            ),
            SpecialistEvent::from_checkpoint(
                "agent-003".to_string(),
                Path::new("/workspace").to_path_buf(),
                &checkpoint(SpecialistCheckpointStatus::Blocked),
            ),
            SpecialistEvent::from_checkpoint(
                "agent-003".to_string(),
                Path::new("/workspace").to_path_buf(),
                &checkpoint(SpecialistCheckpointStatus::Failed),
            ),
            artifact_event,
        ];

        let decoded = fixtures
            .iter()
            .map(|event| {
                let path = write_specialist_event(tempdir.path(), event).expect("write event");
                let written = std::fs::read_to_string(path).expect("read event");
                serde_json::from_str::<SpecialistEvent>(&written).expect("decode event")
            })
            .collect::<Vec<_>>();

        assert_eq!(decoded, fixtures);
        assert_eq!(
            decoded
                .iter()
                .map(|event| (
                    event.event_type,
                    event.approval_gate,
                    event.needs_user,
                    event.completed_outputs.is_empty(),
                ))
                .collect::<Vec<_>>(),
            vec![
                (
                    SpecialistEventType::WaitingContinuation,
                    SpecialistApprovalGate::PlainContinuation,
                    false,
                    false,
                ),
                (
                    SpecialistEventType::NeedsUser,
                    SpecialistApprovalGate::NeedsUser,
                    true,
                    false,
                ),
                (
                    SpecialistEventType::Blocked,
                    SpecialistApprovalGate::Blocked,
                    false,
                    false,
                ),
                (
                    SpecialistEventType::Error,
                    SpecialistApprovalGate::Blocked,
                    false,
                    false,
                ),
                (
                    SpecialistEventType::ArtifactCreated,
                    SpecialistApprovalGate::PlainContinuation,
                    false,
                    false,
                ),
            ]
        );
        assert_eq!(
            decoded
                .iter()
                .map(
                    |event| serde_json::to_value(event).expect("event value")["eventType"]
                        .as_str()
                        .expect("eventType")
                        .to_string()
                )
                .collect::<Vec<_>>(),
            vec![
                "waiting-continuation",
                "needs-user",
                "blocked",
                "error",
                "artifact-created",
            ]
        );
    }

    fn checkpoint(status: SpecialistCheckpointStatus) -> SpecialistCheckpoint {
        SpecialistCheckpoint {
            issue_id: "issue-123".to_string(),
            session_id: "session-abc".to_string(),
            timestamp: 1_744_309_600,
            objective: "Finish the workplan".to_string(),
            current_phase: Some("review".to_string()),
            current_deliverable: Some("memo.md".to_string()),
            status,
            last_completed_step: Some("Drafted the facts section".to_string()),
            next_concrete_step: Some("Review the open proof gaps".to_string()),
            stop_conditions: Vec::new(),
            blocked_reasons: Vec::new(),
            open_questions: Vec::new(),
            needs_from_user: Vec::new(),
            risk_level: None,
            files_relied_on: vec![Path::new("/source/case-file.md").to_path_buf()],
            citations_used: Vec::new(),
            files_changed: Vec::new(),
            deliverables_touched: vec![Path::new("/workspace/memo.md").to_path_buf()],
            unresolved_proof_gaps: Vec::new(),
            recommended_next_prompt: Some("Review the proof gaps.".to_string()),
        }
    }
}
