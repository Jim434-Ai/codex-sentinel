use crate::checkpoint::CheckpointPaths;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use thiserror::Error;

pub const DEFAULT_WORKSPACE_MANIFEST_RELATIVE_PATH: &str = ".codex/workspace.toml";
pub const DEFAULT_MACHINE_PROFILE_FILE_NAME: &str = "machine.local.toml";

#[derive(Debug, Error)]
pub enum SpecialistError {
    #[error("could not find {DEFAULT_WORKSPACE_MANIFEST_RELATIVE_PATH} from {start_dir}")]
    WorkspaceManifestNotFound { start_dir: PathBuf },

    #[error("workspace manifest not found at {path}")]
    WorkspaceManifestMissing { path: PathBuf },

    #[error("machine profile not found at {path}")]
    MachineProfileMissing { path: PathBuf },

    #[error("failed to read {path}: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse TOML in {path}: {source}")]
    ParseToml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },

    #[error("workspace manifest must define at least one role")]
    EmptyRoles,

    #[error("role `{role}` references unmapped root `{root_id}` in {machine_profile_path}")]
    MissingRootMapping {
        role: String,
        root_id: String,
        machine_profile_path: PathBuf,
    },

    #[error("resolved path for role `{role}` does not exist: {path}")]
    RolePathMissing { role: String, path: PathBuf },

    #[error("workspace does not define context set `{name}`")]
    ContextSetNotFound { name: String },

    #[error("workspace does not define a default context set")]
    MissingDefaultContextSet,

    #[error("context set `{context_set}` references unknown role `{role}`")]
    UnknownContextRole { context_set: String, role: String },

    #[error("resolved context file does not exist for `{context_set}` via role `{role}`: {path}")]
    ContextPathMissing {
        context_set: String,
        role: String,
        path: PathBuf,
    },

    #[error("failed to read checkpoint at {path}: {source}")]
    ReadCheckpoint {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse checkpoint JSON at {path}: {source}")]
    ParseCheckpoint {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("failed to create checkpoint directory {path}: {source}")]
    CreateCheckpointDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to write checkpoint at {path}: {source}")]
    WriteCheckpoint {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to read context snapshot at {path}: {source}")]
    ReadContextSnapshot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse context snapshot JSON at {path}: {source}")]
    ParseContextSnapshot {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to create context snapshot directory {path}: {source}")]
    CreateContextSnapshotDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write context snapshot at {path}: {source}")]
    WriteContextSnapshot {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to create specialist event directory {path}")]
    CreateEventDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize specialist event JSON")]
    SerializeEventJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to write specialist event file {path}")]
    WriteEvent {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to serialize checkpoint JSON: {source}")]
    SerializeCheckpointJson {
        #[source]
        source: serde_json::Error,
    },
    #[error("failed to serialize context snapshot JSON")]
    SerializeContextSnapshotJson {
        #[source]
        source: serde_json::Error,
    },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum RoleAccess {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceManifest {
    pub workspace_id: String,
    #[serde(default)]
    pub issue_id: Option<String>,
    #[serde(default)]
    pub default_context_set: Option<String>,
    #[serde(default)]
    pub roles: BTreeMap<String, WorkspaceRole>,
    #[serde(default)]
    pub context_sets: BTreeMap<String, ContextSet>,
}

impl WorkspaceManifest {
    pub fn issue_id(&self) -> &str {
        self.issue_id.as_deref().unwrap_or(&self.workspace_id)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct WorkspaceRole {
    pub root: String,
    #[serde(default)]
    pub path: PathBuf,
    pub access: RoleAccess,
    #[serde(default)]
    pub default_context: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextSet {
    #[serde(default)]
    pub files: Vec<ContextFileRef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextFileRef {
    pub role: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct MachineProfile {
    #[serde(default)]
    pub roots: BTreeMap<String, PathBuf>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedWorkspace {
    pub manifest_path: PathBuf,
    pub machine_profile_path: PathBuf,
    pub workspace_id: String,
    pub issue_id: String,
    pub default_context_set: Option<String>,
    pub roles: BTreeMap<String, ResolvedRole>,
    pub context_sets: BTreeMap<String, ResolvedContextSet>,
}

impl ResolvedWorkspace {
    pub fn checkpoint_paths(&self, codex_home: &Path) -> CheckpointPaths {
        CheckpointPaths::new(codex_home, &self.workspace_id, &self.issue_id)
    }

    pub fn workspace_root(&self) -> PathBuf {
        let Some(manifest_dir) = self.manifest_path.parent() else {
            return PathBuf::from(".");
        };

        if manifest_dir.file_name() == Some(OsStr::new(".codex"))
            && let Some(workspace_root) = manifest_dir.parent()
        {
            return canonicalize_existing_path(workspace_root)
                .unwrap_or_else(|| workspace_root.to_path_buf());
        }

        canonicalize_existing_path(manifest_dir).unwrap_or_else(|| manifest_dir.to_path_buf())
    }

    pub fn context_set(
        &self,
        requested_name: Option<&str>,
    ) -> Result<&ResolvedContextSet, SpecialistError> {
        let context_set_name = if let Some(name) = requested_name {
            name
        } else if let Some(default_context_set) = self.default_context_set.as_deref() {
            default_context_set
        } else {
            return Err(SpecialistError::MissingDefaultContextSet);
        };

        self.context_sets
            .get(context_set_name)
            .ok_or_else(|| SpecialistError::ContextSetNotFound {
                name: context_set_name.to_string(),
            })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedRole {
    pub name: String,
    pub root_id: String,
    pub access: RoleAccess,
    pub default_context: bool,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedContextSet {
    pub name: String,
    pub files: Vec<ResolvedContextFile>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedContextFile {
    pub role: String,
    pub path: PathBuf,
}

pub fn discover_workspace_manifest(start_dir: &Path) -> Result<PathBuf, SpecialistError> {
    for candidate_dir in start_dir.ancestors() {
        let candidate = candidate_dir.join(DEFAULT_WORKSPACE_MANIFEST_RELATIVE_PATH);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(SpecialistError::WorkspaceManifestNotFound {
        start_dir: start_dir.to_path_buf(),
    })
}

pub fn load_workspace(
    start_dir: &Path,
    manifest_path: Option<&Path>,
    machine_profile_path: Option<&Path>,
) -> Result<ResolvedWorkspace, SpecialistError> {
    let manifest_path = match manifest_path {
        Some(path) => resolve_user_path(start_dir, path),
        None => discover_workspace_manifest(start_dir)?,
    };

    if !manifest_path.is_file() {
        return Err(SpecialistError::WorkspaceManifestMissing {
            path: manifest_path,
        });
    }

    let manifest_dir = manifest_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| start_dir.to_path_buf());
    let machine_profile_path = match machine_profile_path {
        Some(path) => resolve_user_path(start_dir, path),
        None => manifest_dir.join(DEFAULT_MACHINE_PROFILE_FILE_NAME),
    };

    if !machine_profile_path.is_file() {
        return Err(SpecialistError::MachineProfileMissing {
            path: machine_profile_path,
        });
    }

    let manifest = read_toml_file::<WorkspaceManifest>(&manifest_path)?;
    let machine_profile = read_toml_file::<MachineProfile>(&machine_profile_path)?;

    resolve_workspace(
        manifest,
        manifest_path,
        machine_profile,
        machine_profile_path,
    )
}

fn read_toml_file<T>(path: &Path) -> Result<T, SpecialistError>
where
    T: serde::de::DeserializeOwned,
{
    let contents = fs::read_to_string(path).map_err(|source| SpecialistError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    toml::from_str(&contents).map_err(|source| SpecialistError::ParseToml {
        path: path.to_path_buf(),
        source,
    })
}

fn resolve_workspace(
    manifest: WorkspaceManifest,
    manifest_path: PathBuf,
    machine_profile: MachineProfile,
    machine_profile_path: PathBuf,
) -> Result<ResolvedWorkspace, SpecialistError> {
    if manifest.roles.is_empty() {
        return Err(SpecialistError::EmptyRoles);
    }

    let machine_profile_dir = machine_profile_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));

    let mut roles = BTreeMap::new();
    for (role_name, role) in &manifest.roles {
        let mapped_root = machine_profile.roots.get(&role.root).ok_or_else(|| {
            SpecialistError::MissingRootMapping {
                role: role_name.clone(),
                root_id: role.root.clone(),
                machine_profile_path: machine_profile_path.clone(),
            }
        })?;
        let root_path = resolve_user_path(&machine_profile_dir, mapped_root);
        let role_path = join_if_non_empty(root_path, &role.path);
        let role_path = canonicalize_existing_path(&role_path).ok_or_else(|| {
            SpecialistError::RolePathMissing {
                role: role_name.clone(),
                path: role_path.clone(),
            }
        })?;

        roles.insert(
            role_name.clone(),
            ResolvedRole {
                name: role_name.clone(),
                root_id: role.root.clone(),
                access: role.access,
                default_context: role.default_context,
                path: role_path,
            },
        );
    }

    let mut context_sets = BTreeMap::new();
    for (context_set_name, context_set) in &manifest.context_sets {
        let mut files = Vec::with_capacity(context_set.files.len());
        for context_file in &context_set.files {
            let role = roles.get(&context_file.role).ok_or_else(|| {
                SpecialistError::UnknownContextRole {
                    context_set: context_set_name.clone(),
                    role: context_file.role.clone(),
                }
            })?;
            let file_path = join_if_non_empty(role.path.clone(), &context_file.path);
            let file_path = canonicalize_existing_path(&file_path).ok_or_else(|| {
                SpecialistError::ContextPathMissing {
                    context_set: context_set_name.clone(),
                    role: context_file.role.clone(),
                    path: file_path.clone(),
                }
            })?;

            files.push(ResolvedContextFile {
                role: context_file.role.clone(),
                path: file_path,
            });
        }

        context_sets.insert(
            context_set_name.clone(),
            ResolvedContextSet {
                name: context_set_name.clone(),
                files,
            },
        );
    }

    Ok(ResolvedWorkspace {
        manifest_path,
        machine_profile_path,
        workspace_id: manifest.workspace_id.clone(),
        issue_id: manifest.issue_id().to_string(),
        default_context_set: manifest.default_context_set,
        roles,
        context_sets,
    })
}

fn resolve_user_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn join_if_non_empty(base: PathBuf, suffix: &Path) -> PathBuf {
    if suffix.as_os_str().is_empty() {
        base
    } else {
        base.join(suffix)
    }
}

fn canonicalize_existing_path(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }

    path.canonicalize().ok()
}

#[cfg(test)]
mod tests {
    use super::RoleAccess;
    use super::SpecialistError;
    use super::discover_workspace_manifest;
    use super::load_workspace;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn discover_workspace_manifest_walks_up_ancestors() {
        let tempdir = TempDir::new().expect("tempdir");
        let workspace_root = tempdir.path().join("matter");
        let nested_dir = workspace_root.join("notes/2026");
        fs::create_dir_all(workspace_root.join(".codex")).expect("workspace manifest dir");
        fs::create_dir_all(&nested_dir).expect("nested dir");
        let manifest_path = workspace_root.join(".codex/workspace.toml");
        fs::write(&manifest_path, "workspace_id = \"matter\"\n").expect("manifest");

        let discovered = discover_workspace_manifest(&nested_dir).expect("discover manifest");

        assert_eq!(discovered, manifest_path);
    }

    #[test]
    fn load_workspace_resolves_roles_and_contexts() {
        let tempdir = TempDir::new().expect("tempdir");
        let source_root = tempdir.path().join("source");
        let analysis_root = tempdir.path().join("analysis");
        fs::create_dir_all(source_root.join("corpus")).expect("source corpus");
        fs::create_dir_all(analysis_root.join("notes")).expect("analysis notes");
        fs::write(source_root.join("corpus/case-file.md"), "source").expect("source file");
        fs::write(analysis_root.join("notes/master.md"), "analysis").expect("analysis file");

        let workspace_root = tempdir.path().join("workspace");
        fs::create_dir_all(workspace_root.join(".codex")).expect("workspace manifest dir");
        fs::write(
            workspace_root.join(".codex/workspace.toml"),
            r#"
workspace_id = "matter"
issue_id = "issue-123"
default_context_set = "core"

[roles.source]
root = "source_root"
path = "corpus"
access = "read-only"

[roles.analysis]
root = "analysis_root"
path = "notes"
access = "read-write"

[context_sets.core]
files = [
  { role = "analysis", path = "master.md" },
  { role = "source", path = "case-file.md" },
]
"#,
        )
        .expect("manifest");
        fs::write(
            workspace_root.join(".codex/machine.local.toml"),
            format!(
                "[roots]\nsource_root = \"{}\"\nanalysis_root = \"{}\"\n",
                source_root.display(),
                analysis_root.display()
            ),
        )
        .expect("machine profile");

        let resolved = load_workspace(&workspace_root, None, None).expect("load workspace");
        let context_set = resolved.context_set(None).expect("default context set");

        assert_eq!(resolved.workspace_id, "matter");
        assert_eq!(resolved.issue_id, "issue-123");
        assert_eq!(
            resolved.workspace_root(),
            workspace_root.canonicalize().unwrap()
        );
        assert_eq!(resolved.roles["source"].access, RoleAccess::ReadOnly);
        assert_eq!(
            context_set
                .files
                .iter()
                .map(|file| file
                    .path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["master.md", "case-file.md"]
        );
    }

    #[test]
    fn load_workspace_errors_when_context_file_is_missing() {
        let tempdir = TempDir::new().expect("tempdir");
        let analysis_root = tempdir.path().join("analysis");
        fs::create_dir_all(&analysis_root).expect("analysis root");
        let workspace_root = tempdir.path().join("workspace");
        fs::create_dir_all(workspace_root.join(".codex")).expect("workspace manifest dir");
        fs::write(
            workspace_root.join(".codex/workspace.toml"),
            r#"
workspace_id = "matter"

[roles.analysis]
root = "analysis_root"
access = "read-write"

[context_sets.core]
files = [{ role = "analysis", path = "missing.md" }]
"#,
        )
        .expect("manifest");
        fs::write(
            workspace_root.join(".codex/machine.local.toml"),
            format!("[roots]\nanalysis_root = \"{}\"\n", analysis_root.display()),
        )
        .expect("machine profile");

        let error = load_workspace(&workspace_root, None, None).expect_err("missing context file");

        assert!(matches!(error, SpecialistError::ContextPathMissing { .. }));
    }
}
