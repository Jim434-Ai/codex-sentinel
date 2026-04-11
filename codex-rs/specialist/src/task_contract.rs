use crate::ResolvedWorkspace;
use crate::SpecialistError;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

pub const DEFAULT_TASK_CONTRACT_RELATIVE_PATH: &str = ".codex/task-contract.toml";

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct TaskContract {
    pub objective: String,
    #[serde(default)]
    pub current_phase: Option<String>,
    #[serde(default)]
    pub current_deliverable: Option<String>,
    #[serde(default)]
    pub stop_conditions: Vec<String>,
    #[serde(default)]
    pub blocked_reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTaskContract {
    pub path: PathBuf,
    pub contract: TaskContract,
}

pub fn load_task_contract(
    workspace: &ResolvedWorkspace,
) -> Result<Option<ResolvedTaskContract>, SpecialistError> {
    let path = workspace
        .workspace_root()
        .join(DEFAULT_TASK_CONTRACT_RELATIVE_PATH);
    load_task_contract_from_path(&path)
}

pub fn load_task_contract_from_path(
    path: &Path,
) -> Result<Option<ResolvedTaskContract>, SpecialistError> {
    if !path.is_file() {
        return Ok(None);
    }

    let contents = fs::read_to_string(path).map_err(|source| SpecialistError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let contract = toml::from_str(&contents).map_err(|source| SpecialistError::ParseToml {
        path: path.to_path_buf(),
        source,
    })?;

    Ok(Some(ResolvedTaskContract {
        path: path.to_path_buf(),
        contract,
    }))
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_TASK_CONTRACT_RELATIVE_PATH;
    use super::TaskContract;
    use super::load_task_contract_from_path;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn load_task_contract_reads_contract_from_default_path() {
        let tempdir = TempDir::new().expect("tempdir");
        let contract_path = tempdir.path().join(DEFAULT_TASK_CONTRACT_RELATIVE_PATH);
        fs::create_dir_all(
            contract_path
                .parent()
                .expect("task contract file should have parent directory"),
        )
        .expect("create .codex dir");
        fs::write(
            &contract_path,
            r#"
objective = "Draft the first memo"
current_phase = "fact review"
current_deliverable = "deliverables/first-pass.md"
stop_conditions = ["Write the deliverable", "List every open question"]
blocked_reasons = ["Missing a signed contract"]
"#,
        )
        .expect("write task contract");

        let contract = load_task_contract_from_path(&contract_path)
            .expect("load task contract")
            .expect("task contract should exist");

        assert_eq!(
            contract.contract,
            TaskContract {
                objective: "Draft the first memo".to_string(),
                current_phase: Some("fact review".to_string()),
                current_deliverable: Some("deliverables/first-pass.md".to_string()),
                stop_conditions: vec![
                    "Write the deliverable".to_string(),
                    "List every open question".to_string(),
                ],
                blocked_reasons: vec!["Missing a signed contract".to_string()],
            }
        );
    }
}
