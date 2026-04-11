use codex_app_server_protocol::Thread as AppServerThread;
use codex_specialist::CheckpointPaths;
use codex_specialist::SpecialistSession;
use codex_specialist::load_specialist_session;
use codex_specialist::write_checkpoint_for_turn;
use std::path::Path;

use crate::cli::SpecialistArgs;

pub(crate) use codex_specialist::apply_specialist_config;
pub(crate) use codex_specialist::inject_specialist_prompt;

pub(crate) fn resolve_specialist_session(
    start_dir: &Path,
    args: &SpecialistArgs,
) -> anyhow::Result<Option<SpecialistSession>> {
    if !args.specialist {
        return Ok(None);
    }

    load_specialist_session(
        start_dir,
        args.workspace_manifest.as_deref(),
        args.machine_profile.as_deref(),
        args.context_set.as_deref(),
    )
    .map(Some)
}

pub(crate) fn write_specialist_checkpoint(
    codex_home: &Path,
    specialist: &mut SpecialistSession,
    thread: &AppServerThread,
    turn_id: &str,
    objective: &str,
) -> anyhow::Result<Option<CheckpointPaths>> {
    write_checkpoint_for_turn(codex_home, specialist, thread, turn_id, objective)
        .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::SpecialistSession;
    use super::apply_specialist_config;
    use super::inject_specialist_prompt;
    use super::resolve_specialist_session;
    use super::write_specialist_checkpoint;
    use codex_app_server_protocol::PatchApplyStatus;
    use codex_app_server_protocol::PatchChangeKind;
    use codex_app_server_protocol::Thread as AppServerThread;
    use codex_app_server_protocol::ThreadItem as AppServerThreadItem;
    use codex_app_server_protocol::ThreadStatus;
    use codex_app_server_protocol::Turn;
    use codex_app_server_protocol::TurnStatus;
    use codex_core::config::ConfigBuilder;
    use codex_protocol::protocol::SandboxPolicy;
    use codex_protocol::user_input::UserInput;
    use codex_utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    #[test]
    fn resolve_specialist_session_uses_default_context_when_present() {
        let fixture = SpecialistFixture::new().expect("fixture");

        let specialist = resolve_specialist_session(
            &fixture.workspace_root,
            &crate::cli::SpecialistArgs {
                specialist: true,
                workspace_manifest: None,
                machine_profile: None,
                context_set: None,
            },
        )
        .expect("resolve")
        .expect("specialist session");

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
    fn write_specialist_checkpoint_tracks_changed_files() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let mut specialist = fixture.specialist_session();
        let codex_home = TempDir::new().expect("codex home");
        let deliverable = specialist.workspace.roles["deliverables"]
            .path
            .join("report.md");
        let thread = AppServerThread {
            id: "thread-1".to_string(),
            forked_from_id: None,
            preview: String::new(),
            ephemeral: false,
            model_provider: "ollama".to_string(),
            created_at: 0,
            updated_at: 0,
            status: ThreadStatus::Idle,
            path: None,
            cwd: specialist.workspace_root.clone(),
            cli_version: "0.0.1".to_string(),
            source: codex_app_server_protocol::SessionSource::Exec,
            agent_nickname: None,
            agent_role: None,
            git_info: None,
            name: None,
            turns: vec![Turn {
                id: "turn-1".to_string(),
                items: vec![
                    AppServerThreadItem::FileChange {
                        id: "change-1".to_string(),
                        changes: vec![codex_app_server_protocol::FileUpdateChange {
                            path: deliverable.display().to_string(),
                            kind: PatchChangeKind::Update { move_path: None },
                            diff: String::new(),
                        }],
                        status: PatchApplyStatus::Completed,
                    },
                    AppServerThreadItem::AgentMessage {
                        id: "msg-1".to_string(),
                        text: "Prepared the specialist summary.".to_string(),
                        phase: None,
                        memory_citation: None,
                    },
                ],
                status: TurnStatus::Completed,
                error: None,
                started_at: Some(1_744_309_600),
                completed_at: Some(1_744_309_700),
                duration_ms: Some(100),
            }],
        };

        let paths = write_specialist_checkpoint(
            codex_home.path(),
            &mut specialist,
            &thread,
            "turn-1",
            "Prepare the report.",
        )
        .expect("write checkpoint")
        .expect("checkpoint paths");
        let checkpoint_json = fs::read_to_string(paths.latest_json).expect("read checkpoint");

        assert!(checkpoint_json.contains("Prepared the specialist summary."));
        assert!(checkpoint_json.contains(&deliverable.display().to_string()));
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
            resolve_specialist_session(
                &self.workspace_root,
                &crate::cli::SpecialistArgs {
                    specialist: true,
                    workspace_manifest: None,
                    machine_profile: None,
                    context_set: None,
                },
            )
            .expect("resolve specialist")
            .expect("session")
        }
    }
}
