use anyhow::Context;
use anyhow::bail;
use clap::Args;
use std::fs;
use std::io;
use std::io::IsTerminal;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;

#[derive(Debug, Args)]
pub(crate) struct SpecialistInitArgs {
    /// Workspace identifier stored in .codex/workspace.toml.
    #[arg(long = "workspace-id", value_name = "ID")]
    pub(crate) workspace_id: Option<String>,

    /// Issue or matter identifier stored in .codex/workspace.toml.
    #[arg(long = "issue-id", value_name = "ID")]
    pub(crate) issue_id: Option<String>,

    /// Primary objective for the task contract.
    #[arg(long = "objective", value_name = "TEXT")]
    pub(crate) objective: Option<String>,

    /// Current phase for the task contract.
    #[arg(long = "current-phase", value_name = "TEXT")]
    pub(crate) current_phase: Option<String>,

    /// Current deliverable path for the task contract.
    #[arg(long = "deliverable", value_name = "PATH")]
    pub(crate) deliverable: Option<String>,

    /// Additional read-only source folders outside the workspace.
    #[arg(long = "source-root", value_name = "DIR")]
    pub(crate) source_roots: Vec<PathBuf>,

    /// Stop condition for the task contract. May be provided multiple times.
    #[arg(long = "stop-condition", value_name = "TEXT")]
    pub(crate) stop_conditions: Vec<String>,

    /// Known blocked reason for the task contract. May be provided multiple times.
    #[arg(long = "blocked-reason", value_name = "TEXT")]
    pub(crate) blocked_reasons: Vec<String>,

    /// Overwrite existing scaffold files.
    #[arg(long = "force", default_value_t = false)]
    pub(crate) force: bool,

    /// Accept defaults for unspecified values instead of prompting.
    #[arg(long = "yes", default_value_t = false)]
    pub(crate) yes: bool,
}

pub(crate) fn run_specialist_init(args: SpecialistInitArgs) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("failed to resolve current directory")?;
    let interactive = !args.yes && io::stdin().is_terminal() && io::stdout().is_terminal();
    let defaults = InitDefaults::for_workspace(&cwd);
    let workspace_id = required_value(
        args.workspace_id,
        "Workspace id",
        &defaults.workspace_id,
        interactive,
    )?;
    let issue_id = required_value(args.issue_id, "Issue id", &workspace_id, interactive)?;
    let objective = required_value(
        args.objective,
        "Objective",
        &defaults.objective,
        interactive,
    )?;
    let current_phase = optional_value(
        args.current_phase,
        "Current phase",
        Some(defaults.current_phase.as_str()),
        interactive,
    )?;
    let deliverable = optional_value(
        args.deliverable,
        "Current deliverable",
        Some(defaults.current_deliverable.as_str()),
        interactive,
    )?;
    let stop_conditions = list_value(
        args.stop_conditions,
        "Stop conditions",
        &defaults.stop_conditions,
        interactive,
    )?;
    let blocked_reasons = list_value(
        args.blocked_reasons,
        "Known blocked reasons",
        &[],
        interactive,
    )?;
    let source_roots = source_roots(args.source_roots, interactive)?;

    let scaffold = SpecialistScaffold {
        workspace_id,
        issue_id,
        objective,
        current_phase,
        current_deliverable: deliverable,
        stop_conditions,
        blocked_reasons,
        workspace_root: cwd.clone(),
        source_roots,
    };

    scaffold.write(args.force)?;
    println!("Initialized specialist workspace in {}", cwd.display());
    println!("Created:");
    println!("  - .codex/workspace.toml");
    println!("  - .codex/machine.local.toml");
    println!("  - .codex/task-contract.toml");
    println!("  - .codex/issue.md");
    println!("  - .codex/soul.md");
    println!("  - AGENTS.md");
    println!("  - analysis/");
    println!("  - deliverables/");
    println!();
    println!("Next steps:");
    println!("  1. Review the generated files.");
    println!("  2. Add any pinned source files to a context set if needed.");
    println!("  3. Start the workspace with `codex --specialist`.");
    Ok(())
}

struct SpecialistScaffold {
    workspace_id: String,
    issue_id: String,
    objective: String,
    current_phase: Option<String>,
    current_deliverable: Option<String>,
    stop_conditions: Vec<String>,
    blocked_reasons: Vec<String>,
    workspace_root: PathBuf,
    source_roots: Vec<PathBuf>,
}

impl SpecialistScaffold {
    fn write(&self, force: bool) -> anyhow::Result<()> {
        let codex_dir = self.workspace_root.join(".codex");
        fs::create_dir_all(&codex_dir)
            .with_context(|| format!("failed to create {}", codex_dir.display()))?;
        fs::create_dir_all(self.workspace_root.join("analysis")).with_context(|| {
            format!(
                "failed to create {}",
                self.workspace_root.join("analysis").display()
            )
        })?;
        fs::create_dir_all(self.workspace_root.join("deliverables")).with_context(|| {
            format!(
                "failed to create {}",
                self.workspace_root.join("deliverables").display()
            )
        })?;

        write_file_if_allowed(
            &self.workspace_root.join(".codex/workspace.toml"),
            &self.render_workspace_manifest(),
            force,
        )?;
        write_file_if_allowed(
            &self.workspace_root.join(".codex/machine.local.toml"),
            &self.render_machine_profile(),
            force,
        )?;
        write_file_if_allowed(
            &self.workspace_root.join(".codex/task-contract.toml"),
            &self.render_task_contract(),
            force,
        )?;
        write_file_if_allowed(
            &self.workspace_root.join(".codex/issue.md"),
            &self.render_issue_brief(),
            force,
        )?;
        write_file_if_allowed(
            &self.workspace_root.join(".codex/soul.md"),
            &self.render_soul_file(),
            force,
        )?;
        write_file_if_allowed(
            &self.workspace_root.join("AGENTS.md"),
            &self.render_agents_file(),
            force,
        )?;

        Ok(())
    }

    fn render_workspace_manifest(&self) -> String {
        let mut contents = String::new();
        push_line(
            &mut contents,
            format!("workspace_id = {:?}", self.workspace_id),
        );
        push_line(&mut contents, format!("issue_id = {:?}", self.issue_id));
        push_line(&mut contents, "default_context_set = \"core\"");
        push_blank_line(&mut contents);
        push_line(&mut contents, "[roles.workspace]");
        push_line(&mut contents, "root = \"workspace_root\"");
        push_line(&mut contents, "access = \"read-write\"");
        push_line(&mut contents, "default_context = true");
        push_blank_line(&mut contents);
        push_line(&mut contents, "[roles.analysis]");
        push_line(&mut contents, "root = \"workspace_root\"");
        push_line(&mut contents, "path = \"analysis\"");
        push_line(&mut contents, "access = \"read-write\"");
        push_blank_line(&mut contents);
        push_line(&mut contents, "[roles.deliverables]");
        push_line(&mut contents, "root = \"workspace_root\"");
        push_line(&mut contents, "path = \"deliverables\"");
        push_line(&mut contents, "access = \"read-write\"");
        for (index, _) in self.source_roots.iter().enumerate() {
            let role_name = format!("source_{}", index + 1);
            let root_name = format!("source_root_{}", index + 1);
            push_blank_line(&mut contents);
            push_line(&mut contents, format!("[roles.{role_name}]"));
            push_line(&mut contents, format!("root = \"{root_name}\""));
            push_line(&mut contents, "access = \"read-only\"");
        }
        push_blank_line(&mut contents);
        push_line(&mut contents, "[context_sets.core]");
        push_line(&mut contents, "files = [");
        for path in [
            "AGENTS.md",
            ".codex/issue.md",
            ".codex/soul.md",
            ".codex/task-contract.toml",
        ] {
            push_line(
                &mut contents,
                format!("  {{ role = \"workspace\", path = {path:?} }},"),
            );
        }
        push_line(&mut contents, "]");
        contents
    }

    fn render_machine_profile(&self) -> String {
        let mut contents = String::new();
        push_line(&mut contents, "[roots]");
        push_line(
            &mut contents,
            format!(
                "workspace_root = {:?}",
                self.workspace_root.display().to_string()
            ),
        );
        for (index, source_root) in self.source_roots.iter().enumerate() {
            push_line(
                &mut contents,
                format!(
                    "source_root_{} = {:?}",
                    index + 1,
                    source_root.display().to_string()
                ),
            );
        }
        contents
    }

    fn render_task_contract(&self) -> String {
        let mut contents = String::new();
        push_line(&mut contents, format!("objective = {:?}", self.objective));
        if let Some(current_phase) = self.current_phase.as_deref() {
            push_line(&mut contents, format!("current_phase = {current_phase:?}"));
        }
        if let Some(current_deliverable) = self.current_deliverable.as_deref() {
            push_line(
                &mut contents,
                format!("current_deliverable = {current_deliverable:?}"),
            );
        }
        write_list(&mut contents, "stop_conditions", &self.stop_conditions);
        write_list(&mut contents, "blocked_reasons", &self.blocked_reasons);
        contents
    }

    fn render_issue_brief(&self) -> String {
        let mut contents = vec![
            format!("# Issue Brief: {}", self.issue_id),
            String::new(),
            "## Objective".to_string(),
            self.objective.clone(),
        ];
        if let Some(current_phase) = self.current_phase.as_deref() {
            contents.push(String::new());
            contents.push("## Current Phase".to_string());
            contents.push(current_phase.to_string());
        }
        if let Some(current_deliverable) = self.current_deliverable.as_deref() {
            contents.push(String::new());
            contents.push("## Current Deliverable".to_string());
            contents.push(current_deliverable.to_string());
        }
        if !self.source_roots.is_empty() {
            contents.push(String::new());
            contents.push("## Source Roots".to_string());
            for (index, source_root) in self.source_roots.iter().enumerate() {
                contents.push(format!("- source_{}: {}", index + 1, source_root.display()));
            }
        }
        if !self.stop_conditions.is_empty() {
            contents.push(String::new());
            contents.push("## Stop Conditions".to_string());
            for stop_condition in &self.stop_conditions {
                contents.push(format!("- {stop_condition}"));
            }
        }
        if !self.blocked_reasons.is_empty() {
            contents.push(String::new());
            contents.push("## Known Blocked Reasons".to_string());
            for blocked_reason in &self.blocked_reasons {
                contents.push(format!("- {blocked_reason}"));
            }
        }
        contents.join("\n")
    }

    fn render_soul_file(&self) -> String {
        let mut lines = vec![
            format!("# Specialist Soul: {}", self.issue_id),
            String::new(),
            "You are a durable specialist agent for this workspace.".to_string(),
            "Operate from source-backed evidence, preserve provenance, and prefer clear deliverables over narration."
                .to_string(),
            "Do not modify original source documents stored outside the writable specialist workspace."
                .to_string(),
            "Separate sourced facts, inferences, open questions, and recommendations.".to_string(),
            format!("Primary objective: {}", self.objective),
        ];
        if let Some(current_deliverable) = self.current_deliverable.as_deref() {
            lines.push(format!("Primary deliverable: {current_deliverable}"));
        }
        lines.join("\n")
    }

    fn render_agents_file(&self) -> String {
        let mut lines = vec![
            format!("# Specialist Workspace: {}", self.workspace_id),
            String::new(),
            "## Operating Rules".to_string(),
            "- Treat external source roles as read-only evidence.".to_string(),
            "- Write working notes only under `analysis/`.".to_string(),
            "- Write final or user-facing outputs only under `deliverables/` unless the task contract says otherwise."
                .to_string(),
            "- Keep important factual claims tied to source file paths when possible.".to_string(),
            "- Do not declare the work complete until the task contract stop conditions are satisfied or a real blocked reason applies."
                .to_string(),
        ];
        if !self.source_roots.is_empty() {
            lines.push(String::new());
            lines.push("## External Source Roots".to_string());
            for (index, source_root) in self.source_roots.iter().enumerate() {
                lines.push(format!("- source_{}: {}", index + 1, source_root.display()));
            }
        }
        lines.join("\n")
    }
}

struct InitDefaults {
    workspace_id: String,
    objective: String,
    current_phase: String,
    current_deliverable: String,
    stop_conditions: Vec<String>,
}

impl InitDefaults {
    fn for_workspace(workspace_root: &Path) -> Self {
        let workspace_id = sanitize_identifier(
            workspace_root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("specialist-workspace"),
        );
        Self {
            objective: format!(
                "Review the specialist source set for `{workspace_id}` and prepare the next deliverable."
            ),
            current_phase: "initial review".to_string(),
            current_deliverable: "deliverables/first-pass.md".to_string(),
            stop_conditions: vec![
                "Write or update the current deliverable".to_string(),
                "List the remaining open questions".to_string(),
                "Capture any blocked reason explicitly before stopping".to_string(),
            ],
            workspace_id,
        }
    }
}

fn required_value(
    provided: Option<String>,
    label: &str,
    default: &str,
    interactive: bool,
) -> anyhow::Result<String> {
    if let Some(provided) = provided {
        return Ok(provided);
    }
    if interactive {
        return prompt_string(label, Some(default));
    }
    Ok(default.to_string())
}

fn optional_value(
    provided: Option<String>,
    label: &str,
    default: Option<&str>,
    interactive: bool,
) -> anyhow::Result<Option<String>> {
    if let Some(provided) = provided {
        return Ok(Some(provided));
    }
    if interactive {
        let value = prompt_string(label, default)?;
        if value.trim().is_empty() {
            return Ok(None);
        }
        return Ok(Some(value));
    }
    Ok(default.map(ToOwned::to_owned))
}

fn list_value(
    provided: Vec<String>,
    label: &str,
    defaults: &[String],
    interactive: bool,
) -> anyhow::Result<Vec<String>> {
    if !provided.is_empty() {
        return Ok(provided);
    }
    if interactive {
        let default_text = if defaults.is_empty() {
            None
        } else {
            Some(defaults.join(" | "))
        };
        let value = prompt_string(
            &format!("{label} (separate multiple entries with `|`)"),
            default_text.as_deref(),
        )?;
        if value.trim().is_empty() {
            return Ok(defaults.to_vec());
        }
        return Ok(split_entries(&value));
    }
    Ok(defaults.to_vec())
}

fn source_roots(provided: Vec<PathBuf>, interactive: bool) -> anyhow::Result<Vec<PathBuf>> {
    if !provided.is_empty() {
        return validate_source_roots(provided);
    }
    if !interactive {
        return Ok(Vec::new());
    }

    println!("Add external source folders. Press return on a blank line when done.");
    let mut roots = Vec::new();
    loop {
        let input = prompt_string("Source folder", None)?;
        if input.trim().is_empty() {
            break;
        }
        roots.push(PathBuf::from(input));
    }
    validate_source_roots(roots)
}

fn validate_source_roots(source_roots: Vec<PathBuf>) -> anyhow::Result<Vec<PathBuf>> {
    let mut validated = Vec::new();
    for source_root in source_roots {
        let absolute = if source_root.is_absolute() {
            source_root
        } else {
            std::env::current_dir()
                .context("failed to resolve current directory for source root")?
                .join(source_root)
        };
        if !absolute.is_dir() {
            bail!(
                "source root does not exist or is not a directory: {}",
                absolute.display()
            );
        }
        validated.push(
            absolute
                .canonicalize()
                .with_context(|| format!("failed to canonicalize {}", absolute.display()))?,
        );
    }
    Ok(validated)
}

fn prompt_string(label: &str, default: Option<&str>) -> anyhow::Result<String> {
    let mut stdout = io::stdout();
    if let Some(default) = default {
        write!(stdout, "{label} [{default}]: ")?;
    } else {
        write!(stdout, "{label}: ")?;
    }
    stdout.flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        return Ok(default.unwrap_or_default().to_string());
    }
    Ok(trimmed)
}

fn write_list(contents: &mut String, key: &str, values: &[String]) {
    push_line(contents, format!("{key} = ["));
    for value in values {
        push_line(contents, format!("  {value:?},"));
    }
    push_line(contents, "]");
}

fn split_entries(value: &str) -> Vec<String> {
    value
        .split('|')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn sanitize_identifier(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn write_file_if_allowed(path: &Path, contents: &str, force: bool) -> anyhow::Result<()> {
    if path.exists() && !force {
        bail!(
            "{} already exists; rerun with --force to overwrite it",
            path.display()
        );
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, contents).with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

fn push_blank_line(contents: &mut String) {
    contents.push('\n');
}

fn push_line(contents: &mut String, line: impl AsRef<str>) {
    contents.push_str(line.as_ref());
    contents.push('\n');
}
