use crate::AppServerTarget;
use crate::cli::Cli;
use codex_specialist::RoleAccess;
use codex_specialist::SpecialistSession;
use codex_specialist::load_specialist_session;
use color_eyre::eyre::Context;

pub(crate) fn prepare_specialist_cli(
    cli: &mut Cli,
    app_server_target: &AppServerTarget,
) -> color_eyre::Result<Option<SpecialistSession>> {
    if !cli.specialist.specialist {
        return Ok(None);
    }

    if matches!(app_server_target, AppServerTarget::Remote { .. }) {
        color_eyre::eyre::bail!("`--specialist` is only supported for local interactive sessions");
    }

    let start_dir = cli
        .cwd
        .clone()
        .unwrap_or(std::env::current_dir().wrap_err("failed to resolve current directory")?);
    let specialist = load_specialist_session(
        start_dir.as_path(),
        cli.specialist.workspace_manifest.as_deref(),
        cli.specialist.machine_profile.as_deref(),
        cli.specialist.context_set.as_deref(),
    )
    .map_err(|err| color_eyre::eyre::eyre!("{err}"))?;

    cli.cwd = Some(specialist.workspace_root.clone());
    extend_additional_writable_roots(cli, &specialist);

    Ok(Some(specialist))
}

fn extend_additional_writable_roots(cli: &mut Cli, specialist: &SpecialistSession) {
    for role in specialist.workspace.roles.values() {
        if role.access != RoleAccess::ReadWrite || role.path.starts_with(&specialist.workspace_root)
        {
            continue;
        }
        if cli.add_dir.iter().any(|path| path == &role.path) {
            continue;
        }
        cli.add_dir.push(role.path.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::prepare_specialist_cli;
    use crate::AppServerTarget;
    use crate::cli::Cli;
    use clap::Parser;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn prepare_specialist_cli_updates_cwd_and_external_writable_roots() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let mut cli = Cli::parse_from([
            "codex",
            "--specialist",
            "-C",
            fixture
                .workspace_root
                .to_str()
                .expect("workspace root path"),
        ]);

        let specialist = prepare_specialist_cli(&mut cli, &AppServerTarget::Embedded)
            .expect("specialist session")
            .expect("enabled");

        assert_eq!(cli.cwd, Some(specialist.workspace_root.clone()));
        assert!(
            cli.add_dir
                .contains(&specialist.workspace.roles["deliverables"].path)
        );
        assert_eq!(specialist.workspace.workspace_id, "matter");
    }

    #[test]
    fn prepare_specialist_cli_rejects_remote_sessions() {
        let fixture = SpecialistFixture::new().expect("fixture");
        let mut cli = Cli::parse_from([
            "codex",
            "--specialist",
            "-C",
            fixture
                .workspace_root
                .to_str()
                .expect("workspace root path"),
        ]);

        let err = prepare_specialist_cli(
            &mut cli,
            &AppServerTarget::Remote {
                websocket_url: "ws://127.0.0.1:4321/".to_string(),
                auth_token: None,
            },
        )
        .expect_err("specialist should reject remote mode");

        assert!(
            err.to_string()
                .contains("only supported for local interactive sessions")
        );
    }

    struct SpecialistFixture {
        _tempdir: TempDir,
        workspace_root: std::path::PathBuf,
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
    }
}
