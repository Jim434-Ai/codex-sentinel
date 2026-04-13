use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn codex_command(cwd: &std::path::Path) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    command.current_dir(cwd);
    Ok(command)
}

fn codex_command_with_home(
    codex_home: &TempDir,
    cwd: &std::path::Path,
) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = codex_command(cwd)?;
    command.env("CODEX_HOME", codex_home.path());
    Ok(command)
}

fn write_manager_workspace(
    manager_root: &std::path::Path,
    specialist_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(manager_root.join(".codex-manager"))?;
    fs::create_dir_all(manager_root.join("pings"))?;
    fs::create_dir_all(specialist_root)?;
    fs::write(manager_root.join("AGENTS.md"), "# Manager Workspace\n")?;
    fs::write(
        manager_root.join(".codex-manager/manager.toml"),
        "[tmux]\nsocket = \"test-socket\"\n",
    )?;
    fs::write(
        manager_root.join("agents.tsv"),
        format!(
            "agent_id\tmatter_id\tname\tworkspace\tsession\trole\tstatus\tcurrent_objective\n\
             agent-003\tmatter\tVeterans\t{}\tagent-003\tspecialist\tworking\tContinue NOI work\n",
            specialist_root.display()
        ),
    )?;
    fs::write(manager_root.join("pings/README.md"), "ignored")?;
    fs::write(
        manager_root.join("pings/agent-003.md"),
        "agent_id: agent-003\n",
    )?;
    Ok(())
}

fn write_specialist_workspace(
    workspace_root: &std::path::Path,
    source_root: &std::path::Path,
    analysis_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    fs::create_dir_all(workspace_root.join(".codex"))?;
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
    )?;
    fs::write(
        workspace_root.join(".codex/machine.local.toml"),
        format!(
            "[roots]\nsource_root = \"{}\"\nanalysis_root = \"{}\"\n",
            source_root.display(),
            analysis_root.display()
        ),
    )?;
    Ok(())
}

#[test]
fn manager_validate_loads_portable_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    write_manager_workspace(&manager_root, &specialist_root)?;

    let output = codex_command(&manager_root)?
        .args(["manager", "validate"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;

    assert!(stdout.contains("Manager workspace:"));
    assert!(stdout.contains("Agents: 1"));
    assert!(stdout.contains("Active agents: 1"));
    assert!(stdout.contains("tmux socket: test-socket"));

    Ok(())
}

#[test]
fn manager_worker_status_uses_specialist_worker_protocol() -> Result<(), Box<dyn std::error::Error>>
{
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let output = codex_command_with_home(&codex_home, &manager_root)?
        .args([
            "manager",
            "--specialist-codex-bin",
            codex_bin.to_str().expect("utf8 temp path"),
            "worker-status",
            "agent-003",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert!(stdout.contains("worker ready workspace_id=matter issue_id=issue-123"));
    assert!(stdout.contains("checkpoint=none"));
    assert!(stdout.contains("worker exited command_id="));

    Ok(())
}

#[test]
fn manager_worker_prompt_dry_run_uses_specialist_worker_protocol()
-> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let output = codex_command_with_home(&codex_home, &manager_root)?
        .args([
            "manager",
            "--specialist-codex-bin",
            codex_bin.to_str().expect("utf8 temp path"),
            "worker-prompt",
            "--dry-run",
            "agent-003",
            "Continue",
            "the",
            "review.",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert!(stdout.contains("worker turn started command_id="));
    assert!(stdout.contains("worker turn completed command_id="));
    assert!(stdout.contains("final_message=dry-run prompt accepted"));
    assert!(stdout.contains("worker needs direction command_id="));

    Ok(())
}

#[test]
fn manager_worker_daemon_dispatches_queued_prompt() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let enqueue_output = codex_command_with_home(&codex_home, &manager_root)?
        .args([
            "manager",
            "--specialist-codex-bin",
            codex_bin.to_str().expect("utf8 temp path"),
            "worker-enqueue",
            "agent-003",
            "Continue",
            "with",
            "the",
            "persistent",
            "pool.",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let enqueue_stdout = String::from_utf8(enqueue_output)?;
    assert!(enqueue_stdout.contains("Queued prompt for agent-003"));
    assert_eq!(
        fs::read_dir(manager_root.join("worker-prompts/agent-003"))?.count(),
        1
    );

    let daemon_output = codex_command_with_home(&codex_home, &manager_root)?
        .args([
            "manager",
            "--specialist-codex-bin",
            codex_bin.to_str().expect("utf8 temp path"),
            "worker-daemon",
            "--dry-run",
            "--iterations",
            "1",
            "--interval-seconds",
            "0",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let daemon_stdout = String::from_utf8(daemon_output)?;
    assert!(daemon_stdout.contains("worker-pool started agents=1"));
    assert!(daemon_stdout.contains("worker prompt dispatched agent_id=agent-003"));
    assert!(daemon_stdout.contains("worker turn completed agent_id=agent-003"));
    assert!(daemon_stdout.contains("manager attention needed agent_id=agent-003"));
    assert_eq!(
        fs::read_dir(manager_root.join("worker-prompts/agent-003"))?.count(),
        0
    );
    assert_eq!(
        fs::read_dir(manager_root.join("worker-prompts/.processed/agent-003"))?.count(),
        1
    );

    Ok(())
}

#[test]
fn manager_worker_daemon_uses_heartbeat_for_status_checkins()
-> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let output = codex_command_with_home(&codex_home, &manager_root)?
        .args([
            "manager",
            "--specialist-codex-bin",
            codex_bin.to_str().expect("utf8 temp path"),
            "worker-daemon",
            "--dry-run",
            "--iterations",
            "2",
            "--interval-seconds",
            "0",
            "--heartbeat-seconds",
            "900",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert_eq!(
        stdout
            .matches("worker status requested agent_id=agent-003")
            .count(),
        1
    );

    Ok(())
}

#[test]
fn manager_list_and_pings_use_manager_workspace() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    write_manager_workspace(&manager_root, &specialist_root)?;

    let list_output = codex_command(tempdir.path())?
        .args([
            "manager",
            "--manager-workspace",
            manager_root.to_str().expect("utf8 temp path"),
            "list",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let list_stdout = String::from_utf8(list_output)?;
    assert!(list_stdout.contains("agent-003"));
    assert!(list_stdout.contains("Continue NOI work"));

    let pings_output = codex_command(&manager_root)?
        .args(["manager", "pings"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pings = String::from_utf8(pings_output)?;
    assert!(pings.trim().ends_with("manager/pings/agent-003.md"));

    Ok(())
}

#[test]
fn manager_absolute_paths_do_not_require_current_dir() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let removed_cwd = tempdir.path().join("removed-cwd");
    fs::create_dir(&removed_cwd)?;
    write_manager_workspace(&manager_root, &specialist_root)?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let output = std::process::Command::new("/bin/sh")
        .current_dir(&removed_cwd)
        .env("CODEX_BIN", &codex_bin)
        .env("MANAGER_ROOT", &manager_root)
        .arg("-c")
        .arg(
            "rmdir \"$PWD\" && exec \"$CODEX_BIN\" manager --manager-workspace \"$MANAGER_ROOT\" --specialist-codex-bin \"$CODEX_BIN\" validate",
        )
        .output()?;

    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    assert!(stdout.contains("Manager workspace:"));
    assert!(stdout.contains("Agents: 1"));

    Ok(())
}
