use super::workspace::ManagerAgent;
use super::workspace::ManagerWorkspace;
use anyhow::Context;
use codex_specialist::RoleAccess;
use codex_specialist::load_workspace;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceAuditOutput {
    pub(crate) generated_at: i64,
    pub(crate) roots: Vec<SourceRootAudit>,
    pub(crate) comparison: Option<SourceAuditComparison>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceRootAudit {
    pub(crate) agent_id: String,
    pub(crate) root: PathBuf,
    pub(crate) exists: bool,
    pub(crate) kind: SourceRootKind,
    pub(crate) file_count: u64,
    pub(crate) dir_count: u64,
    pub(crate) total_bytes: u64,
    pub(crate) latest_modified_at: Option<i64>,
    pub(crate) fingerprint: String,
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SourceRootKind {
    File,
    Directory,
    Missing,
    Other,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceAuditComparison {
    pub(crate) baseline: PathBuf,
    pub(crate) changed: bool,
    pub(crate) unchanged_roots: usize,
    pub(crate) changed_roots: Vec<SourceAuditChange>,
    pub(crate) new_roots: Vec<SourceAuditChange>,
    pub(crate) missing_roots: Vec<SourceAuditChange>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SourceAuditChange {
    pub(crate) agent_id: String,
    pub(crate) root: PathBuf,
    pub(crate) before: Option<SourceRootAudit>,
    pub(crate) after: Option<SourceRootAudit>,
}

pub(crate) fn audit_source_roots(
    workspace: &ManagerWorkspace,
    baseline_path: Option<&Path>,
) -> anyhow::Result<SourceAuditOutput> {
    let roots = workspace
        .agents()
        .iter()
        .flat_map(source_roots_for_agent)
        .map(|(agent_id, root)| audit_source_root(agent_id, root))
        .collect::<Vec<_>>();
    let comparison = baseline_path
        .map(|path| compare_source_audit(path, &roots))
        .transpose()?;

    Ok(SourceAuditOutput {
        generated_at: unix_timestamp(),
        roots,
        comparison,
    })
}

fn source_roots_for_agent(agent: &ManagerAgent) -> Vec<(String, PathBuf)> {
    let Ok(workspace) = load_workspace(&agent.workspace, None, None) else {
        return Vec::new();
    };
    let mut roots = workspace
        .roles
        .values()
        .filter(|role| role.access == RoleAccess::ReadOnly)
        .map(|role| (agent.agent_id.clone(), role.path.clone()))
        .collect::<Vec<_>>();
    roots.sort_by(|(_, left), (_, right)| left.cmp(right));
    roots
}

fn audit_source_root(agent_id: String, root: PathBuf) -> SourceRootAudit {
    let mut stats = SourceRootStats::new(agent_id, root);
    stats.audit();
    stats.finish()
}

fn compare_source_audit(
    baseline_path: &Path,
    roots: &[SourceRootAudit],
) -> anyhow::Result<SourceAuditComparison> {
    let contents = fs::read_to_string(baseline_path)
        .with_context(|| format!("read source audit baseline {}", baseline_path.display()))?;
    let baseline: SourceAuditOutput = serde_json::from_str(&contents)
        .with_context(|| format!("parse source audit baseline {}", baseline_path.display()))?;

    let before = baseline
        .roots
        .into_iter()
        .map(|root| (source_root_key(&root), root))
        .collect::<BTreeMap<_, _>>();
    let after = roots
        .iter()
        .cloned()
        .map(|root| (source_root_key(&root), root))
        .collect::<BTreeMap<_, _>>();

    let mut unchanged_roots = 0;
    let mut changed_roots = Vec::new();
    let mut new_roots = Vec::new();
    let mut missing_roots = Vec::new();

    for (key, before_root) in &before {
        match after.get(key) {
            Some(after_root) if source_root_matches(before_root, after_root) => {
                unchanged_roots += 1;
            }
            Some(after_root) => changed_roots.push(SourceAuditChange {
                agent_id: before_root.agent_id.clone(),
                root: before_root.root.clone(),
                before: Some(before_root.clone()),
                after: Some(after_root.clone()),
            }),
            None => missing_roots.push(SourceAuditChange {
                agent_id: before_root.agent_id.clone(),
                root: before_root.root.clone(),
                before: Some(before_root.clone()),
                after: None,
            }),
        }
    }

    for (key, after_root) in &after {
        if !before.contains_key(key) {
            new_roots.push(SourceAuditChange {
                agent_id: after_root.agent_id.clone(),
                root: after_root.root.clone(),
                before: None,
                after: Some(after_root.clone()),
            });
        }
    }

    let changed = !changed_roots.is_empty() || !new_roots.is_empty() || !missing_roots.is_empty();
    Ok(SourceAuditComparison {
        baseline: baseline_path.to_path_buf(),
        changed,
        unchanged_roots,
        changed_roots,
        new_roots,
        missing_roots,
    })
}

fn source_root_key(root: &SourceRootAudit) -> String {
    format!("{}\n{}", root.agent_id, root.root.display())
}

fn source_root_matches(before: &SourceRootAudit, after: &SourceRootAudit) -> bool {
    before.exists == after.exists
        && before.kind == after.kind
        && before.file_count == after.file_count
        && before.dir_count == after.dir_count
        && before.total_bytes == after.total_bytes
        && before.latest_modified_at == after.latest_modified_at
        && before.fingerprint == after.fingerprint
        && before.errors == after.errors
}

struct SourceRootStats {
    audit: SourceRootAudit,
    hash: u64,
}

impl SourceRootStats {
    fn new(agent_id: String, root: PathBuf) -> Self {
        Self {
            audit: SourceRootAudit {
                agent_id,
                root,
                exists: false,
                kind: SourceRootKind::Missing,
                file_count: 0,
                dir_count: 0,
                total_bytes: 0,
                latest_modified_at: None,
                fingerprint: String::new(),
                errors: Vec::new(),
            },
            hash: FNV_OFFSET_BASIS,
        }
    }

    fn audit(&mut self) {
        let root = self.audit.root.clone();
        let Ok(metadata) = fs::symlink_metadata(&root) else {
            self.update_hash(b"missing");
            return;
        };

        self.audit.exists = true;
        if metadata.is_file() {
            self.audit.kind = SourceRootKind::File;
            self.record_entry(Path::new("."), &metadata);
        } else if metadata.is_dir() {
            self.audit.kind = SourceRootKind::Directory;
            self.walk_directory(&root);
        } else {
            self.audit.kind = SourceRootKind::Other;
            self.record_entry(Path::new("."), &metadata);
        }
    }

    fn walk_directory(&mut self, root: &Path) {
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                self.audit
                    .errors
                    .push(format!("could not read directory {}", dir.display()));
                continue;
            };
            let mut paths = entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect::<Vec<_>>();
            paths.sort();

            for path in paths {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
                match fs::symlink_metadata(&path) {
                    Ok(metadata) => {
                        self.record_entry(&relative, &metadata);
                        if metadata.is_dir() {
                            stack.push(path);
                        }
                    }
                    Err(err) => self
                        .audit
                        .errors
                        .push(format!("could not stat {}: {err}", path.display())),
                }
            }
        }
    }

    fn record_entry(&mut self, relative: &Path, metadata: &fs::Metadata) {
        if metadata.is_file() {
            self.audit.file_count += 1;
            self.audit.total_bytes = self.audit.total_bytes.saturating_add(metadata.len());
        } else if metadata.is_dir() {
            self.audit.dir_count += 1;
        }

        self.update_hash(relative.to_string_lossy().as_bytes());
        self.update_hash(if metadata.is_file() {
            b"file"
        } else if metadata.is_dir() {
            b"dir"
        } else {
            b"other"
        });
        self.update_hash(&metadata.len().to_le_bytes());
        if let Some((seconds, nanos)) = modified_time(metadata) {
            self.audit.latest_modified_at = Some(
                self.audit
                    .latest_modified_at
                    .unwrap_or(seconds)
                    .max(seconds),
            );
            self.update_hash(&seconds.to_le_bytes());
            self.update_hash(&nanos.to_le_bytes());
        }
    }

    fn update_hash(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.hash ^= u64::from(*byte);
            self.hash = self.hash.wrapping_mul(FNV_PRIME);
        }
        self.hash ^= 0xff;
        self.hash = self.hash.wrapping_mul(FNV_PRIME);
    }

    fn finish(mut self) -> SourceRootAudit {
        self.audit.fingerprint = format!("{:016x}", self.hash);
        self.audit
    }
}

fn modified_time(metadata: &fs::Metadata) -> Option<(i64, u32)> {
    let duration = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((duration.as_secs() as i64, duration.subsec_nanos()))
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}
