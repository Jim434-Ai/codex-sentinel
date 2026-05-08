use assert_cmd::Command;
use pretty_assertions::assert_eq;
use serde_json::Value;
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
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

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
fn manager_agents_json_and_pause_update_registry() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

    let output = codex_command(&manager_root)?
        .args(["manager", "agents", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output)?;
    assert_eq!(json["agents"][0]["agentId"], "agent-003");
    assert_eq!(json["agents"][0]["matterId"], "matter");
    assert_eq!(json["agents"][0]["sessionId"], "agent-003");
    assert_eq!(
        json["agents"][0]["sourceRoots"],
        serde_json::json!([source_root.join("corpus").canonicalize()?])
    );
    assert_eq!(json["agents"][0]["lifecycleState"], "working");
    assert_eq!(json["agents"][0]["automationMode"], "actively-managing");
    assert_eq!(json["agents"][0]["observerWindowStatus"], "unknown");

    let status_output = codex_command(&manager_root)?
        .args(["manager", "agents", "status", "agent-003", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status: Value = serde_json::from_slice(&status_output)?;
    assert_eq!(status, json["agents"][0]);

    let pause_output = codex_command(&manager_root)?
        .args(["manager", "agents", "pause", "agent-003"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let pause_stdout = String::from_utf8(pause_output)?;
    assert!(pause_stdout.contains("Paused agent-003"));
    assert!(specialist_root.is_dir());
    assert!(
        fs::read_to_string(manager_root.join("agents.tsv"))?
            .contains("agent-003\tmatter\tVeterans\t")
    );
    assert!(
        fs::read_to_string(manager_root.join("agents.tsv"))?
            .contains("\tagent-003\tspecialist\tpaused\tContinue NOI work")
    );

    let paused_output = codex_command(&manager_root)?
        .args(["manager", "agents", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let paused: Value = serde_json::from_slice(&paused_output)?;
    assert_eq!(paused["agents"][0]["lifecycleState"], "paused");
    assert_eq!(paused["agents"][0]["automationMode"], "observing");

    let export_output = codex_command(&manager_root)?
        .args(["manager", "agents", "export", "--output", "agents.json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let export_stdout = String::from_utf8(export_output)?;
    assert!(export_stdout.contains("Wrote agent registry"));
    let exported: Value = serde_json::from_slice(&fs::read(manager_root.join("agents.json"))?)?;
    assert_eq!(exported, paused);

    let validate_output = codex_command(&manager_root)?
        .args(["manager", "agents", "validate-schema"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let validate_stdout = String::from_utf8(validate_output)?;
    assert!(validate_stdout.contains("agent registry validates against"));

    Ok(())
}

#[test]
fn manager_agents_json_registry_is_native() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = manager_root.join("specialists/veterans");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    fs::create_dir_all(manager_root.join(".codex-manager"))?;
    fs::create_dir_all(&specialist_root)?;
    fs::write(manager_root.join("AGENTS.md"), "# Manager Workspace\n")?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;
    fs::write(
        manager_root.join("agents.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "agents": [
                {
                    "agentId": "agent-003",
                    "matterId": "matter",
                    "name": "Veterans",
                    "workspace": "specialists/veterans",
                    "sourceRoots": [],
                    "sessionId": "agent-003",
                    "role": "specialist",
                    "lifecycleState": "working",
                    "automationMode": "actively-managing",
                    "observerWindowStatus": "unknown",
                    "currentObjective": "Continue NOI work"
                }
            ]
        }))?,
    )?;

    let output = codex_command(&manager_root)?
        .args(["manager", "agents", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output)?;
    assert_eq!(json["agents"][0]["agentId"], "agent-003");
    assert_eq!(
        json["agents"][0]["workspace"],
        specialist_root
            .canonicalize()?
            .to_string_lossy()
            .to_string()
    );
    assert_eq!(
        json["agents"][0]["sourceRoots"],
        serde_json::json!([source_root.join("corpus").canonicalize()?])
    );

    codex_command(&manager_root)?
        .args(["manager", "agents", "pause", "agent-003"])
        .assert()
        .success();
    let updated: Value = serde_json::from_slice(&fs::read(manager_root.join("agents.json"))?)?;
    assert_eq!(updated["agents"][0]["lifecycleState"], "paused");
    assert!(!manager_root.join("agents.tsv").exists());

    Ok(())
}

#[test]
fn manager_agents_source_audit_snapshots_and_compares() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;

    let baseline_output = codex_command(&manager_root)?
        .args([
            "manager",
            "agents",
            "source-audit",
            "--output",
            "source-baseline.json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let baseline_stdout = String::from_utf8(baseline_output)?;
    assert!(baseline_stdout.contains("Wrote source audit"));
    let baseline: Value =
        serde_json::from_slice(&fs::read(manager_root.join("source-baseline.json"))?)?;
    assert_eq!(baseline["roots"][0]["agentId"], "agent-003");
    assert_eq!(baseline["roots"][0]["kind"], "directory");
    assert_eq!(baseline["roots"][0]["fileCount"], 1);
    assert_eq!(baseline["comparison"], Value::Null);

    let clean_compare_output = codex_command(&manager_root)?
        .args([
            "manager",
            "agents",
            "source-audit",
            "--compare",
            "source-baseline.json",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let clean_compare: Value = serde_json::from_slice(&clean_compare_output)?;
    assert_eq!(clean_compare["comparison"]["changed"], false);
    assert_eq!(clean_compare["comparison"]["unchangedRoots"], 1);

    fs::write(source_root.join("corpus/case-file.md"), "source changed")?;
    let changed_compare_output = codex_command(&manager_root)?
        .args([
            "manager",
            "agents",
            "source-audit",
            "--compare",
            "source-baseline.json",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let changed_compare: Value = serde_json::from_slice(&changed_compare_output)?;
    assert_eq!(changed_compare["comparison"]["changed"], true);
    assert_eq!(
        changed_compare["comparison"]["changedRoots"]
            .as_array()
            .expect("changedRoots array")
            .len(),
        1
    );

    Ok(())
}

#[test]
fn manager_dashboard_generates_from_registry() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    write_manager_workspace(&manager_root, &specialist_root)?;
    fs::write(
        manager_root.join("pings/event-1.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "eventId": "event-1",
            "agentId": "agent-003",
            "workspace": specialist_root,
            "eventType": "waiting-continuation",
            "severity": "info",
            "summary": "Ready for continuation",
            "createdAt": 1,
            "completedOutputs": [],
            "recommendedNext": "Continue.",
            "contextLevel": null,
            "approvalGate": "plain-continuation",
            "needsUser": false,
            "sourceGaps": []
        }))?,
    )?;
    fs::create_dir_all(manager_root.join("pings/acks"))?;
    fs::write(
        manager_root.join("pings/acks/event-1.ack"),
        "event_id=event-1\nagent_id=agent-003\ndecision=would-auto-continue\n",
    )?;

    let output = codex_command(&manager_root)?
        .args(["manager", "dashboard", "--output", "manager_dashboard.md"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert!(stdout.contains("Wrote dashboard"));

    let dashboard = fs::read_to_string(manager_root.join("manager_dashboard.md"))?;
    assert!(dashboard.contains("# Manager Dashboard"));
    assert!(dashboard.contains(
        "| agent-003 | working | actively-managing | unknown | agent-003 | Continue NOI work |"
    ));
    assert!(dashboard.contains("## Event Rollup"));
    assert!(dashboard.contains("- Structured events: 1"));
    assert!(dashboard.contains("  - waiting-continuation: 1"));
    assert!(dashboard.contains("  - would-auto-continue: 1"));

    Ok(())
}

#[test]
fn manager_process_events_dry_run_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    write_manager_workspace(&manager_root, &specialist_root)?;
    fs::write(
        manager_root.join("pings/plain.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "eventId": "plain",
            "agentId": "agent-003",
            "workspace": specialist_root,
            "eventType": "waiting-continuation",
            "severity": "info",
            "summary": "Ready for continuation",
            "createdAt": 1,
            "completedOutputs": [],
            "recommendedNext": "Continue the bounded fixture.",
            "contextLevel": "medium",
            "approvalGate": "plain-continuation",
            "needsUser": false,
            "sourceGaps": []
        }))?,
    )?;
    fs::write(
        manager_root.join("pings/matter-id.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "eventId": "matter-id",
            "agentId": "matter",
            "workspace": specialist_root,
            "eventType": "waiting-continuation",
            "severity": "info",
            "summary": "Ready for continuation from workspace id",
            "createdAt": 1,
            "completedOutputs": [],
            "recommendedNext": "Continue the bounded fixture.",
            "contextLevel": "medium",
            "approvalGate": "plain-continuation",
            "needsUser": false,
            "sourceGaps": []
        }))?,
    )?;
    fs::write(
        manager_root.join("pings/needs-user.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "eventId": "needs-user",
            "agentId": "agent-003",
            "workspace": specialist_root,
            "eventType": "needs-user",
            "severity": "warning",
            "summary": "Needs direction",
            "createdAt": 2,
            "completedOutputs": [],
            "recommendedNext": "Choose the next source set.",
            "contextLevel": null,
            "approvalGate": "needs-user",
            "needsUser": true,
            "sourceGaps": []
        }))?,
    )?;
    fs::write(
        manager_root.join("pings/blocked-alternative.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "eventId": "blocked-alternative",
            "agentId": "agent-003",
            "workspace": specialist_root,
            "eventType": "blocked",
            "severity": "warning",
            "summary": "Blocked on one source bundle",
            "createdAt": 3,
            "completedOutputs": [],
            "recommendedNext": null,
            "contextLevel": "medium",
            "approvalGate": "blocked",
            "needsUser": false,
            "sourceGaps": ["Missing source bundle"],
            "blockedAlternative": {
                "recommendedNext": "Continue the independent citation audit.",
                "approvalGate": "plain-continuation",
                "contextLevel": "medium"
            }
        }))?,
    )?;

    let first_output = codex_command(&manager_root)?
        .args(["manager", "process-events", "--dry-run"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first_stdout = String::from_utf8(first_output)?;
    assert!(
        first_stdout.contains(
            "event_id=blocked-alternative agent_id=agent-003 decision=would-auto-continue"
        )
    );
    assert!(
        first_stdout.contains("event_id=plain agent_id=agent-003 decision=would-auto-continue")
    );
    assert!(
        first_stdout.contains("event_id=matter-id agent_id=agent-003 decision=would-auto-continue")
    );
    assert!(first_stdout.contains("event_id=needs-user agent_id=agent-003 decision=escalated"));
    assert!(
        manager_root
            .join("pings/acks/blocked-alternative.ack")
            .is_file()
    );
    assert!(manager_root.join("pings/acks/plain.ack").is_file());
    assert!(manager_root.join("pings/acks/matter-id.ack").is_file());
    assert!(manager_root.join("pings/acks/needs-user.ack").is_file());

    let second_output = codex_command(&manager_root)?
        .args(["manager", "process-events", "--dry-run"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second_stdout = String::from_utf8(second_output)?;
    assert!(
        second_stdout.contains(
            "event_id=blocked-alternative agent_id=agent-003 decision=already-acknowledged"
        )
    );
    assert!(
        second_stdout.contains("event_id=plain agent_id=agent-003 decision=already-acknowledged")
    );
    assert!(
        second_stdout
            .contains("event_id=matter-id agent_id=agent-003 decision=already-acknowledged")
    );
    assert!(
        second_stdout
            .contains("event_id=needs-user agent_id=agent-003 decision=already-acknowledged")
    );

    Ok(())
}

#[test]
fn manager_observers_list_and_open_dry_run() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    write_manager_workspace(&manager_root, &specialist_root)?;

    let output = codex_command(&manager_root)?
        .args(["manager", "observers", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output)?;
    assert_eq!(json["observers"][0]["agentId"], "agent-003");
    assert_eq!(json["observers"][0]["sessionId"], "agent-003");
    assert_eq!(json["observers"][0]["observerWindowStatus"], "missing");
    assert_eq!(
        json["observers"][0]["attachCommand"],
        "tmux -L test-socket attach-session -t agent-003"
    );

    let output = codex_command(&manager_root)?
        .args(["manager", "observers", "open", "agent-003", "--dry-run"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert_eq!(
        stdout.trim(),
        "tmux -L test-socket attach-session -t agent-003"
    );

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
fn manager_worker_prompt_ack_json_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let manager_root = tempdir.path().join("manager");
    let specialist_root = tempdir.path().join("specialist");
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let message_file = tempdir.path().join("message.txt");
    write_manager_workspace(&manager_root, &specialist_root)?;
    write_specialist_workspace(&specialist_root, &source_root, &analysis_root)?;
    fs::write(&message_file, "Continue from the safe message file.\n")?;

    let codex_bin = codex_utils_cargo_bin::cargo_bin("codex")?;
    let args = [
        "manager",
        "--specialist-codex-bin",
        codex_bin.to_str().expect("utf8 temp path"),
        "worker-prompt",
        "--dry-run",
        "--ack-json",
        "--idempotency-key",
        "fixture-key",
        "--message-file",
        message_file.to_str().expect("utf8 temp path"),
        "agent-003",
    ];
    let first_output = codex_command_with_home(&codex_home, &manager_root)?
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second_output = codex_command_with_home(&codex_home, &manager_root)?
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let first: Value = serde_json::from_slice(&first_output)?;
    let second: Value = serde_json::from_slice(&second_output)?;

    assert_eq!(first["accepted"], true);
    assert_eq!(first["started"], true);
    assert_eq!(first["duplicate"], false);
    assert_eq!(first["messageId"], "fixture-key");
    assert_eq!(first["sessionId"], "agent-003");
    assert_eq!(first["error"], Value::Null);
    assert_eq!(second["accepted"], true);
    assert_eq!(second["started"], true);
    assert_eq!(second["duplicate"], true);
    assert_eq!(second["messageId"], "fixture-key");
    assert_eq!(second["sessionId"], "agent-003");
    assert_eq!(second["error"], Value::Null);
    assert!(
        manager_root
            .join(".codex-manager/message-acks/agent-003/fixture-key.json")
            .is_file()
    );

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
fn manager_worker_daemon_accepts_interactive_console_commands()
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
            "--interactive",
            "--interval-seconds",
            "0",
            "--heartbeat-seconds",
            "900",
        ])
        .write_stdin(
            "agents\nstatus agent-003\nchat agent-003\nContinue through chat mode.\n@agent-003 Continue through shorthand.\nquit\n",
        )
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    assert!(stdout.contains("interactive console enabled"));
    assert!(stdout.contains("agent-003\tidle"));
    assert!(stdout.contains("worker status requested agent_id=agent-003 reason=console"));
    assert!(stdout.contains("chat target set agent_id=agent-003"));
    assert!(stdout.contains("worker prompt sent agent_id=agent-003"));
    assert!(stdout.contains("worker busy agent_id=agent-003; prompt queued"));
    assert!(stdout.contains("Queued prompt for agent-003"));
    assert!(stdout.contains("worker-daemon shutdown requested from console"));

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
