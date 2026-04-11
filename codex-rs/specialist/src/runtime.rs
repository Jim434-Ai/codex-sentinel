use anyhow::Context;
use codex_core::config::Config;
use codex_protocol::permissions::FileSystemAccessMode;
use codex_protocol::permissions::FileSystemPath;
use codex_protocol::permissions::FileSystemSandboxEntry;
use codex_protocol::permissions::FileSystemSandboxKind;
use codex_protocol::permissions::FileSystemSandboxPolicy;
use codex_protocol::protocol::ReadOnlyAccess;
use codex_protocol::protocol::SandboxPolicy;
use codex_protocol::user_input::UserInput;
use codex_utils_absolute_path::AbsolutePathBuf;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use crate::ResolvedContextSet;
use crate::ResolvedTaskContract;
use crate::ResolvedWorkspace;
use crate::RoleAccess;
use crate::SpecialistCheckpoint;
use crate::SpecialistCheckpointStatus;
use crate::hydrate_latest_checkpoint;
use crate::load_task_contract;
use crate::load_workspace;

const MAX_CONTEXT_FILE_BYTES: usize = 8 * 1024;
const MAX_CONTEXT_TOTAL_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone)]
pub struct SpecialistSession {
    pub workspace: ResolvedWorkspace,
    pub workspace_root: PathBuf,
    pub context_set: Option<ResolvedContextSet>,
    pub task_contract: Option<ResolvedTaskContract>,
    pub latest_checkpoint: Option<SpecialistCheckpoint>,
}

pub fn load_specialist_session(
    start_dir: &Path,
    workspace_manifest: Option<&Path>,
    machine_profile: Option<&Path>,
    context_set: Option<&str>,
) -> anyhow::Result<SpecialistSession> {
    let workspace = load_workspace(start_dir, workspace_manifest, machine_profile)?;
    let context_set = if let Some(name) = context_set {
        Some(workspace.context_set(Some(name))?.clone())
    } else if workspace.default_context_set.is_some() {
        Some(workspace.context_set(None)?.clone())
    } else {
        None
    };
    let workspace_root = workspace.workspace_root();
    let task_contract = load_task_contract(&workspace)?;

    Ok(SpecialistSession {
        workspace,
        workspace_root,
        context_set,
        task_contract,
        latest_checkpoint: None,
    })
}

pub fn refresh_specialist_runtime_state(
    specialist: &mut SpecialistSession,
    codex_home: &Path,
) -> anyhow::Result<()> {
    specialist.latest_checkpoint = hydrate_latest_checkpoint(
        codex_home,
        &specialist.workspace.workspace_id,
        &specialist.workspace.issue_id,
    )?;
    Ok(())
}

pub fn apply_specialist_config(
    config: &mut Config,
    specialist: &SpecialistSession,
) -> anyhow::Result<()> {
    let previous_sandbox_policy = config.permissions.sandbox_policy.get().clone();
    config.cwd = AbsolutePathBuf::from_absolute_path(&specialist.workspace_root)
        .context("specialist workspace root must be an absolute path")?;

    if matches!(
        previous_sandbox_policy,
        SandboxPolicy::DangerFullAccess | SandboxPolicy::ExternalSandbox { .. }
    ) {
        return Ok(());
    }

    let read_only_roots = specialist
        .workspace
        .roles
        .values()
        .filter(|role| role.access == RoleAccess::ReadOnly)
        .map(|role| {
            AbsolutePathBuf::from_absolute_path(&role.path).with_context(|| {
                format!(
                    "specialist role `{}` resolved to a non-absolute path",
                    role.name
                )
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let writable_roots = specialist
        .workspace
        .roles
        .values()
        .filter(|role| {
            role.access == RoleAccess::ReadWrite
                && !role.path.starts_with(&specialist.workspace_root)
        })
        .map(|role| {
            AbsolutePathBuf::from_absolute_path(&role.path).with_context(|| {
                format!(
                    "specialist role `{}` resolved to a non-absolute path",
                    role.name
                )
            })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    let specialist_sandbox_policy = SandboxPolicy::WorkspaceWrite {
        writable_roots,
        read_only_access: ReadOnlyAccess::Restricted {
            include_platform_defaults: true,
            readable_roots: read_only_roots,
        },
        network_access: previous_sandbox_policy.has_full_network_access(),
        exclude_tmpdir_env_var: false,
        exclude_slash_tmp: false,
    };
    config
        .permissions
        .sandbox_policy
        .set(specialist_sandbox_policy)
        .context("specialist mode could not update the effective sandbox policy")?;

    if matches!(
        config.permissions.file_system_sandbox_policy.kind,
        FileSystemSandboxKind::Restricted
    ) {
        let mut entries = config
            .permissions
            .file_system_sandbox_policy
            .entries
            .clone();
        for role in specialist.workspace.roles.values() {
            let access = match role.access {
                RoleAccess::ReadOnly => FileSystemAccessMode::Read,
                RoleAccess::ReadWrite => FileSystemAccessMode::Write,
            };
            let path = AbsolutePathBuf::from_absolute_path(&role.path).with_context(|| {
                format!(
                    "specialist role `{}` resolved to a non-absolute path",
                    role.name
                )
            })?;
            push_path_entry_if_missing(&mut entries, path, access);
        }
        config.permissions.file_system_sandbox_policy =
            FileSystemSandboxPolicy::restricted(entries);
    }

    Ok(())
}

pub fn inject_specialist_prompt(
    mut items: Vec<UserInput>,
    specialist: &SpecialistSession,
) -> anyhow::Result<Vec<UserInput>> {
    for item in &mut items {
        if let UserInput::Text { text, .. } = item {
            *text = render_specialist_prompt(text, specialist)?;
            return Ok(items);
        }
    }

    items.push(UserInput::Text {
        text: render_specialist_prompt("", specialist)?,
        text_elements: Vec::new(),
    });
    Ok(items)
}

pub fn render_specialist_prompt(
    user_prompt: &str,
    specialist: &SpecialistSession,
) -> anyhow::Result<String> {
    let mut lines = vec![
        "<specialist_workspace>".to_string(),
        format!("workspace_id: {}", specialist.workspace.workspace_id),
        format!("issue_id: {}", specialist.workspace.issue_id),
        format!("workspace_root: {}", specialist.workspace_root.display()),
        "roles:".to_string(),
    ];

    for role in specialist.workspace.roles.values() {
        let access = match role.access {
            RoleAccess::ReadOnly => "read-only",
            RoleAccess::ReadWrite => "read-write",
        };
        lines.push(format!(
            "- {} | access={} | path={}",
            role.name,
            access,
            role.path.display()
        ));
    }

    lines.push("</specialist_workspace>".to_string());
    lines.push(String::new());
    lines.push("<specialist_instructions>".to_string());
    lines.push("- Read-only roles are authoritative source material.".to_string());
    lines.push("- Write new analysis and deliverables only inside read-write roles.".to_string());
    lines.push("- Cite source file paths for important factual claims when possible.".to_string());
    lines.push(
        "- Treat the task contract stop conditions as binding; do not declare the work complete until they are satisfied or a real blocked reason applies."
            .to_string(),
    );
    lines.push("</specialist_instructions>".to_string());

    if let Some(task_contract) = specialist.task_contract.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "<specialist_task_contract path=\"{}\">",
            task_contract.path.display()
        ));
        lines.push(format!("objective: {}", task_contract.contract.objective));
        if let Some(current_phase) = task_contract.contract.current_phase.as_deref() {
            lines.push(format!("current_phase: {current_phase}"));
        }
        if let Some(current_deliverable) = task_contract.contract.current_deliverable.as_deref() {
            lines.push(format!("current_deliverable: {current_deliverable}"));
        }
        if !task_contract.contract.stop_conditions.is_empty() {
            lines.push("stop_conditions:".to_string());
            for stop_condition in &task_contract.contract.stop_conditions {
                lines.push(format!("- {stop_condition}"));
            }
        }
        if !task_contract.contract.blocked_reasons.is_empty() {
            lines.push("known_blocked_reasons:".to_string());
            for blocked_reason in &task_contract.contract.blocked_reasons {
                lines.push(format!("- {blocked_reason}"));
            }
        }
        lines.push("</specialist_task_contract>".to_string());
    }

    if let Some(checkpoint) = specialist.latest_checkpoint.as_ref() {
        lines.push(String::new());
        lines.push("<specialist_checkpoint>".to_string());
        lines.push(format!(
            "status: {}",
            checkpoint_status_name(checkpoint.status)
        ));
        lines.push(format!("objective: {}", checkpoint.objective));
        if let Some(current_phase) = checkpoint.current_phase.as_deref() {
            lines.push(format!("current_phase: {current_phase}"));
        }
        if let Some(current_deliverable) = checkpoint.current_deliverable.as_deref() {
            lines.push(format!("current_deliverable: {current_deliverable}"));
        }
        if let Some(last_completed_step) = checkpoint.last_completed_step.as_deref() {
            lines.push(format!("last_completed_step: {last_completed_step}"));
        }
        if let Some(next_concrete_step) = checkpoint.next_concrete_step.as_deref() {
            lines.push(format!("next_concrete_step: {next_concrete_step}"));
        }
        if !checkpoint.stop_conditions.is_empty() {
            lines.push("stop_conditions:".to_string());
            for stop_condition in &checkpoint.stop_conditions {
                lines.push(format!("- {stop_condition}"));
            }
        }
        if !checkpoint.blocked_reasons.is_empty() {
            lines.push("blocked_reasons:".to_string());
            for blocked_reason in &checkpoint.blocked_reasons {
                lines.push(format!("- {blocked_reason}"));
            }
        }
        lines.push("</specialist_checkpoint>".to_string());
    }

    if let Some(context_set) = specialist.context_set.as_ref() {
        lines.push(String::new());
        lines.push(format!(
            "<specialist_context_set name=\"{}\">",
            context_set.name
        ));
        let mut remaining_bytes = MAX_CONTEXT_TOTAL_BYTES;
        for file in &context_set.files {
            lines.extend(render_context_file(
                file.path.as_path(),
                &file.role,
                &mut remaining_bytes,
            )?);
        }
        lines.push("</specialist_context_set>".to_string());
    }

    lines.push(String::new());
    lines.push("<user_task>".to_string());
    lines.push(user_prompt.to_string());
    lines.push("</user_task>".to_string());

    Ok(lines.join("\n"))
}

fn checkpoint_status_name(status: SpecialistCheckpointStatus) -> &'static str {
    match status {
        SpecialistCheckpointStatus::Working => "working",
        SpecialistCheckpointStatus::Blocked => "blocked",
        SpecialistCheckpointStatus::Waiting => "waiting",
        SpecialistCheckpointStatus::NeedsReview => "needs_review",
        SpecialistCheckpointStatus::Completed => "completed",
        SpecialistCheckpointStatus::Failed => "failed",
    }
}

fn render_context_file(
    path: &Path,
    role: &str,
    remaining_bytes: &mut usize,
) -> anyhow::Result<Vec<String>> {
    let mut lines = vec![format!(
        "<context_file role=\"{}\" path=\"{}\">",
        role,
        path.display()
    )];

    if *remaining_bytes == 0 {
        lines.push("[omitted: context budget exhausted]".to_string());
        lines.push("</context_file>".to_string());
        return Ok(lines);
    }

    let bytes = fs::read(path)
        .with_context(|| format!("failed to read specialist context file {}", path.display()))?;
    let snippet_len = bytes
        .len()
        .min(MAX_CONTEXT_FILE_BYTES)
        .min(*remaining_bytes);
    *remaining_bytes = remaining_bytes.saturating_sub(snippet_len);

    lines.push("```text".to_string());
    lines.push(String::from_utf8_lossy(&bytes[..snippet_len]).into_owned());
    lines.push("```".to_string());
    if snippet_len < bytes.len() {
        lines.push(format!(
            "[truncated: showing {} of {} bytes]",
            snippet_len,
            bytes.len()
        ));
    }
    lines.push("</context_file>".to_string());

    Ok(lines)
}

fn push_path_entry_if_missing(
    entries: &mut Vec<FileSystemSandboxEntry>,
    path: AbsolutePathBuf,
    access: FileSystemAccessMode,
) {
    if entries.iter().any(|entry| {
        entry.access == access
            && matches!(&entry.path, FileSystemPath::Path { path: entry_path } if entry_path == &path)
    }) {
        return;
    }

    entries.push(FileSystemSandboxEntry {
        path: FileSystemPath::Path { path },
        access,
    });
}

#[cfg(test)]
mod tests {
    use super::SpecialistSession;
    use super::apply_specialist_config;
    use super::inject_specialist_prompt;
    use super::load_specialist_session;
    use super::refresh_specialist_runtime_state;
    use crate::SpecialistCheckpoint;
    use crate::SpecialistCheckpointStatus;
    use crate::write_latest_checkpoint;
    use codex_core::config::ConfigBuilder;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::user_input::UserInput;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn load_specialist_session_uses_default_context_when_present() {
        let fixture = SpecialistFixture::new().expect("fixture");

        let specialist =
            load_specialist_session(&fixture.workspace_root, None, None, None).expect("resolve");

        assert_eq!(specialist.workspace.workspace_id, "matter");
        assert_eq!(
            specialist
                .context_set
                .as_ref()
                .map(|context_set| context_set.name.as_str()),
            Some("core")
        );
    }

    #[tokio::test]
    async fn apply_specialist_config_updates_workspace_write_policy() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let codex_home = TempDir::new().expect("codex home");
        let cwd = TempDir::new().expect("cwd");
        let mut config = ConfigBuilder::default()
            .codex_home(codex_home.path().to_path_buf())
            .fallback_cwd(Some(cwd.path().to_path_buf()))
            .build()
            .await
            .expect("config");
        let specialist = fixture.specialist_session();

        apply_specialist_config(&mut config, &specialist).expect("apply specialist config");

        assert_eq!(
            config.cwd,
            AbsolutePathBuf::from_absolute_path(&specialist.workspace_root).expect("absolute path")
        );
        match config.permissions.sandbox_policy.get() {
            SandboxPolicy::WorkspaceWrite {
                writable_roots,
                read_only_access,
                ..
            } => {
                assert_eq!(writable_roots.len(), 1);
                assert!(writable_roots[0].as_path().ends_with("deliverables"));
                match read_only_access {
                    codex_protocol::protocol::ReadOnlyAccess::Restricted {
                        readable_roots, ..
                    } => {
                        assert_eq!(readable_roots.len(), 1);
                        assert!(readable_roots[0].as_path().ends_with("source"));
                    }
                    codex_protocol::protocol::ReadOnlyAccess::FullAccess => {
                        panic!("expected restricted specialist read roots");
                    }
                }
            }
            _ => panic!("expected workspace-write sandbox"),
        }
    }

    #[test]
    fn inject_specialist_prompt_wraps_workspace_and_context() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let specialist = fixture.specialist_session();
        let items = vec![UserInput::Text {
            text: "Summarize the evidence.".to_string(),
            text_elements: Vec::new(),
        }];

        let injected = inject_specialist_prompt(items, &specialist).expect("inject prompt");

        assert_eq!(injected.len(), 1);
        let UserInput::Text { text, .. } = &injected[0] else {
            panic!("expected text input");
        };
        assert!(text.contains("<specialist_workspace>"));
        assert!(text.contains("case-file.md"));
        assert!(text.contains("<user_task>"));
    }

    #[test]
    fn inject_specialist_prompt_adds_text_block_for_image_only_turns() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let specialist = fixture.specialist_session();
        let image = fixture.workspace_root.join("image.png");
        fs::write(&image, "fake-image").expect("write image");

        let injected = inject_specialist_prompt(
            vec![UserInput::LocalImage {
                path: image.clone(),
            }],
            &specialist,
        )
        .expect("inject prompt");

        assert_eq!(injected.len(), 2);
        assert_eq!(injected[0], UserInput::LocalImage { path: image });
        let UserInput::Text { text, .. } = &injected[1] else {
            panic!("expected injected text input");
        };
        assert!(text.contains("<specialist_workspace>"));
        assert!(text.contains("<user_task>"));
    }

    #[test]
    fn inject_specialist_prompt_includes_task_contract_and_checkpoint() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let codex_home = TempDir::new().expect("codex home");
        let mut specialist = fixture.specialist_session();
        write_latest_checkpoint(
            codex_home.path(),
            &specialist.workspace.workspace_id,
            &SpecialistCheckpoint {
                issue_id: specialist.workspace.issue_id.clone(),
                session_id: "thread-123".to_string(),
                timestamp: 1_744_309_700,
                objective: "Prepare the first pass".to_string(),
                current_phase: Some("evidence review".to_string()),
                current_deliverable: Some("deliverables/first-pass.md".to_string()),
                status: SpecialistCheckpointStatus::NeedsReview,
                last_completed_step: Some("Indexed the source set".to_string()),
                next_concrete_step: Some("Draft the first deliverable".to_string()),
                stop_conditions: vec!["Write the first deliverable".to_string()],
                blocked_reasons: vec!["Missing signed exhibit".to_string()],
                open_questions: Vec::new(),
                needs_from_user: Vec::new(),
                risk_level: None,
                files_relied_on: Vec::new(),
                citations_used: Vec::new(),
                files_changed: Vec::new(),
                deliverables_touched: Vec::new(),
                unresolved_proof_gaps: Vec::new(),
                recommended_next_prompt: None,
            },
        )
        .expect("write checkpoint");
        refresh_specialist_runtime_state(&mut specialist, codex_home.path())
            .expect("refresh specialist state");

        let injected = inject_specialist_prompt(
            vec![UserInput::Text {
                text: "Continue the work.".to_string(),
                text_elements: Vec::new(),
            }],
            &specialist,
        )
        .expect("inject prompt");

        let UserInput::Text { text, .. } = &injected[0] else {
            panic!("expected text input");
        };
        assert!(text.contains("<specialist_task_contract"));
        assert!(text.contains("current_phase: fact review"));
        assert!(text.contains("<specialist_checkpoint>"));
        assert!(text.contains("blocked_reasons:"));
    }

    struct SpecialistFixture {
        _tempdir: TempDir,
        workspace_root: PathBuf,
    }

    impl SpecialistFixture {
        fn new() -> anyhow::Result<Self> {
            let tempdir = TempDir::new()?;
            let source_root = tempdir.path().join("source");
            let deliverables_root = tempdir.path().join("deliverables");
            let workspace_root = tempdir.path().join("workspace");
            fs::create_dir_all(&source_root)?;
            fs::create_dir_all(&deliverables_root)?;
            fs::create_dir_all(workspace_root.join(".codex"))?;
            fs::write(source_root.join("case-file.md"), "source evidence")?;
            fs::write(deliverables_root.join("report.md"), "draft")?;
            fs::write(
                workspace_root.join(".codex/task-contract.toml"),
                r#"
objective = "Prepare the first pass"
current_phase = "fact review"
current_deliverable = "deliverables/first-pass.md"
stop_conditions = [
  "Write the current deliverable",
  "List every open question",
]
blocked_reasons = ["Missing a signed exhibit"]
"#,
            )?;
            fs::write(
                workspace_root.join(".codex/workspace.toml"),
                r#"
workspace_id = "matter"
issue_id = "issue-123"
default_context_set = "core"

[roles.source]
root = "source_root"
access = "read-only"

[roles.analysis]
root = "workspace_root"
access = "read-write"

[roles.deliverables]
root = "deliverables_root"
access = "read-write"

[context_sets.core]
files = [
  { role = "source", path = "case-file.md" },
]
"#,
            )?;
            fs::write(
                workspace_root.join(".codex/machine.local.toml"),
                format!(
                    "[roots]\nsource_root = \"{}\"\nworkspace_root = \"{}\"\ndeliverables_root = \"{}\"\n",
                    source_root.display(),
                    workspace_root.display(),
                    deliverables_root.display(),
                ),
            )?;

            Ok(Self {
                _tempdir: tempdir,
                workspace_root,
            })
        }

        fn specialist_session(&self) -> SpecialistSession {
            load_specialist_session(&self.workspace_root, None, None, None)
                .expect("resolve specialist")
        }
    }
}
