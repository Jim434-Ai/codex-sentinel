use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn codex_command(cwd: &std::path::Path) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    command.current_dir(cwd);
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
