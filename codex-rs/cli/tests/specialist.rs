use assert_cmd::Command;
use codex_specialist::SpecialistCheckpoint;
use codex_specialist::SpecialistCheckpointStatus;
use codex_specialist::write_latest_checkpoint;
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
            files_changed: Vec::new(),
            deliverables_touched: Vec::new(),
            unresolved_proof_gaps: vec!["No support for damages schedule".to_string()],
            recommended_next_prompt: Some("Review the proof gaps.".to_string()),
        },
    )?;

    let output = codex_command(&codex_home, &workspace_root)?
        .args(["specialist", "status", "--json"])
        .output()?;

    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(json["workspaceId"], "matter");
    assert_eq!(json["checkpoint"]["status"], "needs_review");
    assert_eq!(
        json["checkpoint"]["next_concrete_step"],
        "Review the open proof gaps"
    );

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
