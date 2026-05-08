use assert_cmd::Command;
use codex_specialist::SpecialistCheckpoint;
use codex_specialist::SpecialistCheckpointStatus;
use codex_specialist::SpecialistContextLevel;
use codex_specialist::SpecialistContextSnapshot;
use codex_specialist::write_latest_checkpoint;
use codex_specialist::write_latest_context_snapshot;
use pretty_assertions::assert_eq;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn codex_command(
    codex_home: &TempDir,
    cwd: &std::path::Path,
) -> Result<Command, Box<dyn std::error::Error>> {
    let mut command = Command::new(codex_utils_cargo_bin::cargo_bin("codex")?);
    command
        .env("CODEX_HOME", codex_home.path())
        .current_dir(cwd);
    Ok(command)
}

fn write_specialist_workspace(
    workspace_root: &std::path::Path,
    source_root: &std::path::Path,
    analysis_root: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
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
fn specialist_validate_workspace_prints_resolved_workspace_json()
-> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    let output = codex_command(&codex_home, &workspace_root)?
        .args(["specialist", "validate-workspace", "--json"])
        .output()?;

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["workspace"]["workspaceId"], "matter");
    assert_eq!(json["workspace"]["issueId"], "issue-123");
    assert_eq!(json["workspace"]["roles"]["source"]["access"], "read-only");

    Ok(())
}

#[test]
fn specialist_show_context_returns_ordered_context_files() -> Result<(), Box<dyn std::error::Error>>
{
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    let output = codex_command(&codex_home, &workspace_root)?
        .args(["specialist", "show-context", "--json"])
        .output()?;

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["contextSet"], "core");
    assert_eq!(
        json["files"]
            .as_array()
            .expect("files array")
            .iter()
            .map(|file| file["path"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            analysis_root
                .join("notes/master.md")
                .canonicalize()?
                .to_string_lossy()
                .to_string(),
            source_root
                .join("corpus/case-file.md")
                .canonicalize()?
                .to_string_lossy()
                .to_string(),
        ]
    );

    Ok(())
}

#[test]
fn specialist_status_reads_latest_checkpoint() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    write_latest_checkpoint(
        codex_home.path(),
        "matter",
        &SpecialistCheckpoint {
            issue_id: "issue-123".to_string(),
            session_id: "session-abc".to_string(),
            timestamp: 1_744_309_600,
            objective: "Finish the workplan".to_string(),
            current_phase: Some("review".to_string()),
            current_deliverable: Some("memo.md".to_string()),
            status: SpecialistCheckpointStatus::NeedsReview,
            last_completed_step: Some("Drafted the facts section".to_string()),
            next_concrete_step: Some("Review the open proof gaps".to_string()),
            stop_conditions: vec!["Review the open proof gaps".to_string()],
            blocked_reasons: vec!["Missing damages support".to_string()],
            open_questions: vec!["Need a final cite".to_string()],
            needs_from_user: Vec::new(),
            risk_level: Some("medium".to_string()),
            files_relied_on: Vec::new(),
            citations_used: Vec::new(),
            files_changed: vec![workspace_root.join("notes.md")],
            deliverables_touched: vec![workspace_root.join("memo.md")],
            unresolved_proof_gaps: vec!["No support for damages schedule".to_string()],
            recommended_next_prompt: Some("Review the proof gaps.".to_string()),
        },
    )?;

    let output = codex_command(&codex_home, &workspace_root)?
        .args(["specialist", "status", "--json"])
        .output()?;

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["agentId"], "matter");
    assert_eq!(json["workspaceId"], "matter");
    assert_eq!(json["sessionId"], "session-abc");
    assert_eq!(json["state"], "waiting-continuation");
    assert_eq!(json["model"], Value::Null);
    assert_eq!(json["contextLevel"], Value::Null);
    assert_eq!(json["contextReport"]["source"], "unavailable");
    assert_eq!(json["contextReport"]["level"], Value::Null);
    assert_eq!(json["contextReport"]["percentRemaining"], Value::Null);
    assert_eq!(json["contextReport"]["tokensRemaining"], Value::Null);
    assert_eq!(json["contextReport"]["modelContextWindow"], Value::Null);
    assert_eq!(
        json["contextReport"]["compressionExpectedSoon"],
        Value::Null
    );
    assert_eq!(json["contextReport"]["safeToContinue"], Value::Null);
    assert_eq!(
        json["contextReport"]["unavailableReason"],
        "runtime token usage is not persisted in specialist checkpoints"
    );
    assert_eq!(json["currentTask"], "memo.md");
    assert_eq!(json["lastActivityAt"], 1_744_309_600);
    assert_eq!(json["needsUser"], false);
    assert_eq!(json["recommendedNext"], "Review the proof gaps.");
    assert_eq!(
        json["newestArtifacts"]
            .as_array()
            .expect("newestArtifacts array")
            .len(),
        2
    );
    assert_eq!(
        json["blockers"],
        serde_json::json!(["Missing damages support"])
    );
    assert_eq!(
        json["artifactReport"]["updatedDerivedFiles"],
        serde_json::json!([workspace_root.join("memo.md")])
    );
    assert_eq!(json["checkpoint"]["status"], "needs_review");
    assert_eq!(
        json["checkpoint"]["next_concrete_step"],
        "Review the open proof gaps"
    );

    Ok(())
}

#[test]
fn specialist_status_reports_checkpoint_state_fixtures() -> Result<(), Box<dyn std::error::Error>> {
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    let cases = [
        (
            "working",
            Some(SpecialistCheckpointStatus::Working),
            Vec::new(),
            Vec::new(),
            "working",
            false,
        ),
        (
            "waiting-continuation",
            Some(SpecialistCheckpointStatus::NeedsReview),
            Vec::new(),
            Vec::new(),
            "waiting-continuation",
            false,
        ),
        (
            "needs-user",
            Some(SpecialistCheckpointStatus::NeedsReview),
            vec!["Choose the next source set"],
            Vec::new(),
            "needs-user",
            true,
        ),
        (
            "blocked",
            Some(SpecialistCheckpointStatus::Blocked),
            Vec::new(),
            vec!["Missing source bundle"],
            "blocked",
            false,
        ),
        (
            "error",
            Some(SpecialistCheckpointStatus::Failed),
            Vec::new(),
            Vec::new(),
            "error",
            false,
        ),
        ("paused", None, Vec::new(), Vec::new(), "paused", false),
    ];

    for (name, checkpoint_status, needs_from_user, blocked_reasons, expected_state, needs_user) in
        cases
    {
        let codex_home = TempDir::new()?;
        if let Some(status) = checkpoint_status {
            write_latest_checkpoint(
                codex_home.path(),
                "matter",
                &SpecialistCheckpoint {
                    issue_id: "issue-123".to_string(),
                    session_id: format!("session-{name}"),
                    timestamp: 1_744_309_600,
                    objective: "Finish the workplan".to_string(),
                    current_phase: Some("review".to_string()),
                    current_deliverable: Some(format!("{name}.md")),
                    status,
                    last_completed_step: Some("Drafted the facts section".to_string()),
                    next_concrete_step: Some("Review the open proof gaps".to_string()),
                    stop_conditions: vec!["Review the open proof gaps".to_string()],
                    blocked_reasons: blocked_reasons.into_iter().map(str::to_string).collect(),
                    open_questions: Vec::new(),
                    needs_from_user: needs_from_user.into_iter().map(str::to_string).collect(),
                    risk_level: Some("medium".to_string()),
                    files_relied_on: Vec::new(),
                    citations_used: Vec::new(),
                    files_changed: Vec::new(),
                    deliverables_touched: Vec::new(),
                    unresolved_proof_gaps: Vec::new(),
                    recommended_next_prompt: Some("Continue from the checkpoint.".to_string()),
                },
            )?;
        }

        let output = codex_command(&codex_home, &workspace_root)?
            .args(["specialist", "status", "--json"])
            .output()?;

        assert!(
            output.status.success(),
            "{name} fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let json: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(json["state"], expected_state, "{name} state");
        assert_eq!(json["needsUser"], needs_user, "{name} needsUser");
        assert_eq!(json["contextReport"]["source"], "unavailable", "{name}");
        if let Some(status) = checkpoint_status {
            let expected_checkpoint_status = serde_json::to_value(status)?;
            assert_eq!(
                json["checkpoint"]["status"], expected_checkpoint_status,
                "{name} checkpoint status"
            );
            assert_eq!(json["sessionId"], format!("session-{name}"), "{name}");
        } else {
            assert_eq!(json["checkpoint"], Value::Null, "{name}");
            assert_eq!(json["sessionId"], Value::Null, "{name}");
        }
    }

    Ok(())
}

#[test]
fn specialist_status_reads_latest_context_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    write_latest_context_snapshot(
        codex_home.path(),
        "matter",
        "issue-123",
        &SpecialistContextSnapshot {
            model: Some("gpt-test".to_string()),
            level: SpecialistContextLevel::Low,
            percent_remaining: 17,
            tokens_remaining: 170_000,
            model_context_window: 1_000_000,
            compression_expected_soon: true,
            safe_to_continue: false,
            updated_at: 1_744_309_700,
        },
    )?;

    let output = codex_command(&codex_home, &workspace_root)?
        .args(["specialist", "status", "--json"])
        .output()?;

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["contextLevel"], "low");
    assert_eq!(json["model"], "gpt-test");
    assert_eq!(json["contextReport"]["source"], "runtime");
    assert_eq!(json["contextReport"]["level"], "low");
    assert_eq!(json["contextReport"]["percentRemaining"], 17);
    assert_eq!(json["contextReport"]["tokensRemaining"], 170_000);
    assert_eq!(json["contextReport"]["modelContextWindow"], 1_000_000);
    assert_eq!(json["contextReport"]["compressionExpectedSoon"], true);
    assert_eq!(json["contextReport"]["safeToContinue"], false);
    assert_eq!(json["contextReport"]["unavailableReason"], Value::Null);

    Ok(())
}

#[test]
fn specialist_worker_dry_run_speaks_jsonl_protocol() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;
    write_latest_context_snapshot(
        codex_home.path(),
        "matter",
        "issue-123",
        &SpecialistContextSnapshot {
            model: Some("gpt-test".to_string()),
            level: SpecialistContextLevel::High,
            percent_remaining: 88,
            tokens_remaining: 880_000,
            model_context_window: 1_000_000,
            compression_expected_soon: false,
            safe_to_continue: true,
            updated_at: 1_744_309_700,
        },
    )?;
    let event_dir = tempdir.path().join("events");

    let input = concat!(
        r#"{"type":"status","id":"status-1"}"#,
        "\n",
        r#"{"type":"prompt","id":"turn-1","prompt":"Continue the review."}"#,
        "\n",
        r#"{"type":"shutdown","id":"shutdown-1"}"#,
        "\n",
    );
    let output = codex_command(&codex_home, &workspace_root)?
        .args([
            "specialist",
            "worker",
            "--dry-run",
            "--event-dir",
            event_dir.to_str().expect("event dir path"),
            "--event-recommended-next",
            "Reply acknowledged for the bounded fixture and do not modify files.",
        ])
        .write_stdin(input)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8(output)?;
    let events = stdout
        .lines()
        .map(serde_json::from_str::<Value>)
        .collect::<Result<Vec<_>, _>>()?;

    assert_eq!(
        events
            .iter()
            .map(|event| event["type"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        vec![
            "ready",
            "status",
            "turn_started",
            "turn_completed",
            "needs_direction",
            "exited",
        ]
    );
    assert_eq!(events[0]["workspace_id"], "matter");
    assert_eq!(events[1]["command_id"], "status-1");
    assert_eq!(events[3]["exit_code"], 0);
    assert_eq!(events[5]["command_id"], "shutdown-1");
    let event_paths = fs::read_dir(&event_dir)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(event_paths.len(), 1);
    let durable_event: Value = serde_json::from_slice(&fs::read(&event_paths[0])?)?;
    assert_eq!(durable_event["agentId"], "matter");
    assert_eq!(durable_event["eventType"], "waiting-continuation");
    assert_eq!(durable_event["approvalGate"], "plain-continuation");
    assert_eq!(durable_event["needsUser"], false);
    assert_eq!(durable_event["contextLevel"], "high");
    assert_eq!(
        durable_event["recommendedNext"],
        "Reply acknowledged for the bounded fixture and do not modify files."
    );

    Ok(())
}

#[test]
fn specialist_send_dry_run_ack_json_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    let message_file = tempdir.path().join("message.md");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    fs::write(&message_file, "Continue the source review.\n")?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    let args = [
        "specialist",
        "send",
        "--session",
        "session-abc",
        "--message-file",
        message_file.to_str().expect("message path"),
        "--idempotency-key",
        "send-fixture",
        "--dry-run",
        "--json",
    ];
    let first_output = codex_command(&codex_home, &workspace_root)?
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let second_output = codex_command(&codex_home, &workspace_root)?
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
    assert_eq!(first["messageId"], "send-fixture");
    assert_eq!(first["sessionId"], "session-abc");
    assert_eq!(first["error"], Value::Null);
    assert_eq!(second["accepted"], true);
    assert_eq!(second["started"], true);
    assert_eq!(second["duplicate"], true);
    assert_eq!(second["messageId"], "send-fixture");
    assert_eq!(second["sessionId"], "session-abc");
    assert_eq!(second["error"], Value::Null);
    assert!(
        codex_home
            .path()
            .join("specialist-send-acks/session-abc/send-fixture.json")
            .is_file()
    );

    Ok(())
}

#[cfg(unix)]
#[test]
fn specialist_send_writes_runtime_context_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::PermissionsExt;

    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let analysis_root = tempdir.path().join("analysis");
    let workspace_root = tempdir.path().join("workspace");
    let message_file = tempdir.path().join("message.md");
    let fake_exec = tempdir.path().join("fake-codex-exec");
    fs::create_dir_all(source_root.join("corpus"))?;
    fs::create_dir_all(analysis_root.join("notes"))?;
    fs::write(source_root.join("corpus/case-file.md"), "source")?;
    fs::write(analysis_root.join("notes/master.md"), "analysis")?;
    fs::write(&message_file, "Continue the source review.\n")?;
    fs::write(
        &fake_exec,
        concat!(
            "#!/bin/sh\n",
            "printf '%s\\n' ",
            "'{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":10,",
            "\"cached_input_tokens\":0,\"output_tokens\":20,",
            "\"total_tokens\":850000,\"model_context_window\":1000000}}'\n",
            "exit 0\n",
        ),
    )?;
    let mut permissions = fs::metadata(&fake_exec)?.permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fake_exec, permissions)?;
    write_specialist_workspace(&workspace_root, &source_root, &analysis_root)?;

    let send_output = codex_command(&codex_home, &workspace_root)?
        .args([
            "specialist",
            "send",
            "--session",
            "session-abc",
            "--message-file",
            message_file.to_str().expect("message path"),
            "--model",
            "gpt-5.5",
            "--exec-bin",
            fake_exec.to_str().expect("fake exec path"),
            "--json",
        ])
        .output()?;

    assert!(send_output.status.success());
    let ack: Value = serde_json::from_slice(&send_output.stdout)?;
    assert_eq!(ack["accepted"], true);
    assert_eq!(ack["started"], true);

    let status_output = codex_command(&codex_home, &workspace_root)?
        .args(["specialist", "status", "--json"])
        .output()?;

    assert!(status_output.status.success());
    let status: Value = serde_json::from_slice(&status_output.stdout)?;
    assert_eq!(status["model"], "gpt-5.5");
    assert_eq!(status["contextLevel"], "low");
    assert_eq!(status["contextReport"]["source"], "runtime");
    assert_eq!(status["contextReport"]["percentRemaining"], 15);
    assert_eq!(status["contextReport"]["tokensRemaining"], 150_000);
    assert_eq!(status["contextReport"]["modelContextWindow"], 1_000_000);
    assert_eq!(status["contextReport"]["compressionExpectedSoon"], true);
    assert_eq!(status["contextReport"]["safeToContinue"], true);
    assert_eq!(status["contextReport"]["unavailableReason"], Value::Null);

    Ok(())
}

#[test]
fn specialist_init_creates_workspace_scaffold() -> Result<(), Box<dyn std::error::Error>> {
    let codex_home = TempDir::new()?;
    let tempdir = TempDir::new()?;
    let source_root = tempdir.path().join("source");
    let workspace_root = tempdir.path().join("workspace");
    fs::create_dir_all(&source_root)?;
    fs::create_dir_all(&workspace_root)?;

    let output = codex_command(&codex_home, &workspace_root)?
        .args([
            "specialist",
            "init",
            "--yes",
            "--workspace-id",
            "matter",
            "--issue-id",
            "issue-123",
            "--objective",
            "Draft the first memo",
            "--current-phase",
            "fact review",
            "--deliverable",
            "deliverables/first-pass.md",
            "--source-root",
            source_root.to_str().expect("source path"),
            "--stop-condition",
            "Write the first deliverable",
        ])
        .output()?;

    assert!(output.status.success());
    assert!(workspace_root.join(".codex/workspace.toml").is_file());
    assert!(workspace_root.join(".codex/machine.local.toml").is_file());
    assert!(workspace_root.join(".codex/task-contract.toml").is_file());
    assert!(workspace_root.join(".codex/issue.md").is_file());
    assert!(workspace_root.join(".codex/soul.md").is_file());
    assert!(workspace_root.join("AGENTS.md").is_file());
    assert!(workspace_root.join("analysis").is_dir());
    assert!(workspace_root.join("deliverables").is_dir());

    let task_contract = fs::read_to_string(workspace_root.join(".codex/task-contract.toml"))?;
    assert!(task_contract.contains("objective = \"Draft the first memo\""));
    assert!(task_contract.contains("current_phase = \"fact review\""));

    let machine_profile = fs::read_to_string(workspace_root.join(".codex/machine.local.toml"))?;
    assert!(machine_profile.contains("workspace_root"));
    assert!(machine_profile.contains("source_root_1"));

    Ok(())
}
