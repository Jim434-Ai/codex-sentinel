use crate::manifest::SpecialistError;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SpecialistContextLevel {
    High,
    Medium,
    Low,
    Critical,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SpecialistContextSnapshot {
    #[serde(default)]
    pub model: Option<String>,
    pub level: SpecialistContextLevel,
    pub percent_remaining: u8,
    pub tokens_remaining: i64,
    pub model_context_window: i64,
    pub compression_expected_soon: bool,
    pub safe_to_continue: bool,
    pub updated_at: i64,
}

impl SpecialistContextSnapshot {
    pub fn from_usage(
        total_tokens: i64,
        model_context_window: i64,
        updated_at: i64,
    ) -> Option<Self> {
        if model_context_window <= 0 {
            return None;
        }

        let tokens_remaining = (model_context_window - total_tokens).max(0);
        let percent_remaining =
            ((tokens_remaining * 100) / model_context_window).clamp(0, 100) as u8;
        let level = match percent_remaining {
            51..=100 => SpecialistContextLevel::High,
            26..=50 => SpecialistContextLevel::Medium,
            11..=25 => SpecialistContextLevel::Low,
            0..=10 => SpecialistContextLevel::Critical,
            101..=u8::MAX => SpecialistContextLevel::High,
        };

        Some(Self {
            model: None,
            level,
            percent_remaining,
            tokens_remaining,
            model_context_window,
            compression_expected_soon: percent_remaining <= 15,
            safe_to_continue: percent_remaining > 10,
            updated_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSnapshotPaths {
    pub directory: PathBuf,
    pub latest_json: PathBuf,
}

impl ContextSnapshotPaths {
    pub fn new(codex_home: &Path, workspace_id: &str, issue_id: &str) -> Self {
        let directory = codex_home
            .join("specialist")
            .join("context")
            .join(sanitize_path_component(workspace_id))
            .join(sanitize_path_component(issue_id));
        Self {
            latest_json: directory.join("latest.json"),
            directory,
        }
    }
}

pub fn read_latest_context_snapshot(
    codex_home: &Path,
    workspace_id: &str,
    issue_id: &str,
) -> Result<Option<SpecialistContextSnapshot>, SpecialistError> {
    let paths = ContextSnapshotPaths::new(codex_home, workspace_id, issue_id);
    if !paths.latest_json.is_file() {
        return Ok(None);
    }

    let contents = fs::read_to_string(&paths.latest_json).map_err(|source| {
        SpecialistError::ReadContextSnapshot {
            path: paths.latest_json.clone(),
            source,
        }
    })?;
    let snapshot = serde_json::from_str(&contents).map_err(|source| {
        SpecialistError::ParseContextSnapshot {
            path: paths.latest_json.clone(),
            source,
        }
    })?;
    Ok(Some(snapshot))
}

pub fn write_latest_context_snapshot(
    codex_home: &Path,
    workspace_id: &str,
    issue_id: &str,
    snapshot: &SpecialistContextSnapshot,
) -> Result<ContextSnapshotPaths, SpecialistError> {
    let paths = ContextSnapshotPaths::new(codex_home, workspace_id, issue_id);
    fs::create_dir_all(&paths.directory).map_err(|source| {
        SpecialistError::CreateContextSnapshotDirectory {
            path: paths.directory.clone(),
            source,
        }
    })?;

    let snapshot_json = serde_json::to_string_pretty(snapshot)
        .map_err(|source| SpecialistError::SerializeContextSnapshotJson { source })?;
    fs::write(&paths.latest_json, snapshot_json).map_err(|source| {
        SpecialistError::WriteContextSnapshot {
            path: paths.latest_json.clone(),
            source,
        }
    })?;
    Ok(paths)
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
