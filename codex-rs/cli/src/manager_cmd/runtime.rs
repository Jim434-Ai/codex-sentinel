use super::source_audit::audit_source_roots;
use super::worker_backend::WorkerPromptRequest;
use super::workspace::ManagerAgent;
use super::workspace::ManagerWorkspace;
use super::workspace::resolve_user_path;
use anyhow::Context;
use anyhow::bail;
use codex_specialist::RoleAccess;
use codex_specialist::load_workspace;
use serde::Deserialize;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;

const DEFAULT_MANAGER_TMUX_SOCKET: &str = "codex-clean";

pub(crate) struct ManagerRuntime {
    pub(crate) workspace: ManagerWorkspace,
    pub(crate) tmux_socket: String,
    pub(crate) specialist_codex_bin: PathBuf,
}

impl ManagerRuntime {
    pub(crate) fn new(
        workspace: ManagerWorkspace,
        tmux_socket: Option<String>,
        specialist_codex_bin: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let tmux_socket = tmux_socket
            .or_else(|| std::env::var("MANAGER_TMUX_SOCKET").ok())
            .or_else(|| {
                workspace
                    .config
                    .tmux
                    .as_ref()
                    .and_then(|tmux| tmux.socket.clone())
            })
            .unwrap_or_else(|| DEFAULT_MANAGER_TMUX_SOCKET.to_string());
        let specialist_codex_bin = if let Some(path) = specialist_codex_bin {
            resolve_current_dir_path(path)?
        } else if let Some(path) = std::env::var_os("SPECIALIST_CODEX_BIN").map(PathBuf::from) {
            resolve_current_dir_path(path)?
        } else if let Some(path) = workspace
            .config
            .paths
            .as_ref()
            .and_then(|paths| paths.specialist_codex_bin.clone())
        {
            resolve_user_path(&workspace.root, &path)
        } else {
            std::env::current_exe().context("resolve current Codex executable")?
        };

        Ok(Self {
            workspace,
            tmux_socket,
            specialist_codex_bin,
        })
    }

    pub(crate) fn validate<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        writeln!(
            writer,
            "Manager workspace: {}",
            self.workspace.root.display()
        )?;
        writeln!(
            writer,
            "Registry: {}",
            self.workspace.registry_path.display()
        )?;
        writeln!(writer, "Agents: {}", self.workspace.agents().len())?;
        writeln!(
            writer,
            "Active agents: {}",
            self.workspace.active_agents().count()
        )?;
        writeln!(writer, "tmux socket: {}", self.tmux_socket)?;
        writeln!(
            writer,
            "Specialist Codex binary: {}",
            self.specialist_codex_bin.display()
        )?;
        Ok(())
    }

    pub(crate) fn list_agents<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        writeln!(
            writer,
            "agent_id\tstatus\tsession\tworkspace\tcurrent_objective"
        )?;
        for agent in self.workspace.agents() {
            writeln!(
                writer,
                "{}\t{}\t{}\t{}\t{}",
                agent.agent_id,
                agent.status,
                agent.session,
                agent.workspace.display(),
                agent.current_objective
            )?;
        }
        Ok(())
    }

    pub(crate) fn agents_list<W: Write>(&self, json: bool, writer: &mut W) -> anyhow::Result<()> {
        if json {
            writeln!(
                writer,
                "{}",
                serde_json::to_string_pretty(&self.agents_list_output())?
            )?;
        } else {
            self.list_agents(writer)?;
        }
        Ok(())
    }

    pub(crate) fn agents_export<W: Write>(
        &self,
        output: &Path,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let path = resolve_user_path(&self.workspace.root, output);
        fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&self.agents_list_output())?
            ),
        )
        .with_context(|| format!("write manager agent registry {}", path.display()))?;
        writeln!(writer, "Wrote agent registry {}", path.display())?;
        Ok(())
    }

    pub(crate) fn agents_validate_schema<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        let value = serde_json::to_value(self.agents_list_output())?;
        validate_agent_registry_schema(&value)?;
        writeln!(
            writer,
            "agent registry validates against schemas/agent_registry.schema.json"
        )?;
        Ok(())
    }

    pub(crate) fn agents_source_audit<W: Write>(
        &self,
        output: Option<&Path>,
        compare: Option<&Path>,
        json: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let compare_path = compare.map(|path| resolve_user_path(&self.workspace.root, path));
        let audit = audit_source_roots(&self.workspace, compare_path.as_deref())?;
        if let Some(output) = output {
            let path = resolve_user_path(&self.workspace.root, output);
            fs::write(
                &path,
                format!("{}\n", serde_json::to_string_pretty(&audit)?),
            )
            .with_context(|| format!("write source audit {}", path.display()))?;
            writeln!(writer, "Wrote source audit {}", path.display())?;
        }
        if json {
            writeln!(writer, "{}", serde_json::to_string_pretty(&audit)?)?;
        } else if let Some(comparison) = &audit.comparison {
            writeln!(
                writer,
                "source audit changed={} unchanged_roots={} changed_roots={} new_roots={} missing_roots={}",
                comparison.changed,
                comparison.unchanged_roots,
                comparison.changed_roots.len(),
                comparison.new_roots.len(),
                comparison.missing_roots.len()
            )?;
        } else if output.is_none() {
            writeln!(writer, "source audit roots={}", audit.roots.len())?;
        }
        Ok(())
    }

    pub(crate) fn agents_status<W: Write>(
        &self,
        agent_id: &str,
        json: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let agent = self.workspace.agent(agent_id)?;
        if json {
            writeln!(
                writer,
                "{}",
                serde_json::to_string_pretty(&ManagerAgentOutput::from(agent))?
            )?;
        } else {
            writeln!(writer, "agent_id={}", agent.agent_id)?;
            writeln!(writer, "matter_id={}", agent.matter_id)?;
            writeln!(writer, "status={}", agent.status)?;
            writeln!(writer, "automation_mode={}", agent.automation_mode())?;
            writeln!(writer, "session={}", agent.session)?;
            writeln!(writer, "workspace={}", agent.workspace.display())?;
            writeln!(writer, "current_objective={}", agent.current_objective)?;
        }
        Ok(())
    }

    pub(crate) fn agents_pause<W: Write>(
        &self,
        agent_id: &str,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        self.workspace.agent(agent_id)?;
        if self
            .workspace
            .registry_path
            .extension()
            .and_then(|extension| extension.to_str())
            == Some("json")
        {
            self.pause_agent_in_json_registry(agent_id)?;
            self.log_event(format!("paused manager agent {agent_id}"))?;
            writeln!(writer, "Paused {agent_id}")?;
            return Ok(());
        }
        let contents = fs::read_to_string(&self.workspace.registry_path).with_context(|| {
            format!(
                "read manager registry {}",
                self.workspace.registry_path.display()
            )
        })?;
        let mut found = false;
        let mut updated = Vec::new();
        for line in contents.lines() {
            if line.trim().is_empty() || line.starts_with('#') || line.starts_with("agent_id\t") {
                updated.push(line.to_string());
                continue;
            }
            let mut fields = line.splitn(8, '\t').map(str::to_string).collect::<Vec<_>>();
            if fields.len() == 8 && fields[0] == agent_id {
                fields[6] = "paused".to_string();
                found = true;
                updated.push(fields.join("\t"));
            } else {
                updated.push(line.to_string());
            }
        }
        if !found {
            bail!("unknown manager agent `{agent_id}`");
        }
        fs::write(
            &self.workspace.registry_path,
            format!("{}\n", updated.join("\n")),
        )
        .with_context(|| {
            format!(
                "write manager registry {}",
                self.workspace.registry_path.display()
            )
        })?;
        self.log_event(format!("paused manager agent {agent_id}"))?;
        writeln!(writer, "Paused {agent_id}")?;
        Ok(())
    }

    fn pause_agent_in_json_registry(&self, agent_id: &str) -> anyhow::Result<()> {
        let contents = fs::read_to_string(&self.workspace.registry_path).with_context(|| {
            format!(
                "read manager registry {}",
                self.workspace.registry_path.display()
            )
        })?;
        let mut registry: serde_json::Value =
            serde_json::from_str(&contents).with_context(|| {
                format!(
                    "parse manager registry {}",
                    self.workspace.registry_path.display()
                )
            })?;
        let agents = registry
            .get_mut("agents")
            .and_then(serde_json::Value::as_array_mut)
            .context("manager JSON registry must contain an agents array")?;
        let mut found = false;
        for agent in agents {
            if agent.get("agentId").and_then(serde_json::Value::as_str) == Some(agent_id) {
                agent["lifecycleState"] = serde_json::Value::String("paused".to_string());
                found = true;
                break;
            }
        }
        if !found {
            bail!("unknown manager agent `{agent_id}`");
        }
        fs::write(
            &self.workspace.registry_path,
            format!("{}\n", serde_json::to_string_pretty(&registry)?),
        )
        .with_context(|| {
            format!(
                "write manager registry {}",
                self.workspace.registry_path.display()
            )
        })?;
        Ok(())
    }

    pub(crate) fn dashboard<W: Write>(
        &self,
        output: Option<&Path>,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let contents = self.dashboard_markdown();
        if let Some(output) = output {
            let path = resolve_user_path(&self.workspace.root, output);
            fs::write(&path, &contents)
                .with_context(|| format!("write manager dashboard {}", path.display()))?;
            writeln!(writer, "Wrote dashboard {}", path.display())?;
        } else {
            write!(writer, "{contents}")?;
        }
        Ok(())
    }

    pub(crate) fn observers_list<W: Write>(
        &self,
        json: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let output = self.observer_list_output();
        if json {
            writeln!(writer, "{}", serde_json::to_string_pretty(&output)?)?;
        } else {
            writeln!(
                writer,
                "agent_id\tobserver_window_status\tsession_id\tworkspace"
            )?;
            for observer in output.observers {
                writeln!(
                    writer,
                    "{}\t{}\t{}\t{}",
                    observer.agent_id,
                    observer.observer_window_status,
                    observer.session_id,
                    observer.workspace.display()
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn observers_open<W: Write>(
        &self,
        agent_id: &str,
        dry_run: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let agent = self.workspace.agent(agent_id)?;
        let command = self.observer_attach_command(agent);
        if dry_run {
            writeln!(writer, "{command}")?;
            return Ok(());
        }
        let launcher = self.write_observer_launcher(agent, &command)?;
        let output = Command::new("open")
            .arg(&launcher)
            .output()
            .with_context(|| format!("open observer launcher {}", launcher.display()))?;
        if !output.status.success() {
            bail!("open observer launcher failed: {}", stderr_text(&output));
        }
        writeln!(writer, "Opened observer launcher {}", launcher.display())?;
        Ok(())
    }

    fn observer_list_output(&self) -> ManagerObserversListOutput {
        let sessions = self.live_session_names();
        ManagerObserversListOutput {
            observers: self
                .workspace
                .agents()
                .iter()
                .map(|agent| ManagerObserverOutput {
                    agent_id: agent.agent_id.clone(),
                    session_id: agent.session.clone(),
                    workspace: agent.workspace.clone(),
                    observer_window_status: observer_status(agent, &sessions).to_string(),
                    attach_command: self.observer_attach_command(agent),
                })
                .collect(),
        }
    }

    fn live_session_names(&self) -> BTreeSet<String> {
        self.tmux_output(["list-sessions", "-F", "#{session_name}"])
            .map(|output| {
                output
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(ToString::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    fn observer_attach_command(&self, agent: &ManagerAgent) -> String {
        format!(
            "tmux -L {} attach-session -t {}",
            self.tmux_socket, agent.session
        )
    }

    fn write_observer_launcher(
        &self,
        agent: &ManagerAgent,
        command: &str,
    ) -> anyhow::Result<PathBuf> {
        let dir = self.workspace.root.join(".codex-manager/observers");
        fs::create_dir_all(&dir)
            .with_context(|| format!("create observer launcher directory {}", dir.display()))?;
        let path = dir.join(format!("{}.command", sanitize_filename(&agent.agent_id)));
        fs::write(
            &path,
            format!(
                "#!/usr/bin/env bash\ncd {}\nexec {}\n",
                shell_quote(&agent.workspace),
                command
            ),
        )
        .with_context(|| format!("write observer launcher {}", path.display()))?;
        let mut permissions = fs::metadata(&path)
            .with_context(|| format!("read observer launcher metadata {}", path.display()))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions)
            .with_context(|| format!("chmod observer launcher {}", path.display()))?;
        Ok(path)
    }

    fn dashboard_markdown(&self) -> String {
        let mut contents = String::new();
        contents.push_str("# Manager Dashboard\n\n");
        contents.push_str(&format!("Updated: {}\n\n", current_timestamp()));
        contents.push_str("## Agent Registry\n\n");
        contents.push_str("| Agent ID | Lifecycle State | Automation Mode | Observer Window | Session | Current Objective |\n");
        contents.push_str("| --- | --- | --- | --- | --- | --- |\n");
        for agent in self.workspace.agents() {
            contents.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} |\n",
                markdown_cell(&agent.agent_id),
                markdown_cell(&agent.status),
                markdown_cell(agent.automation_mode()),
                markdown_cell(agent.observer_window_status()),
                markdown_cell(&agent.session),
                markdown_cell(&agent.current_objective)
            ));
        }
        contents.push_str("\n## Event Rollup\n\n");
        let event_rollup = self.event_rollup();
        contents.push_str(&format!(
            "- Structured events: {}\n",
            event_rollup.structured_events
        ));
        contents.push_str(&format!(
            "- Acknowledgements: {}\n",
            event_rollup.acknowledgements
        ));
        if event_rollup.event_type_counts.is_empty() {
            contents.push_str("- Event types: none\n");
        } else {
            contents.push_str("- Event types:\n");
            for (event_type, count) in event_rollup.event_type_counts {
                contents.push_str(&format!("  - {}: {}\n", markdown_cell(&event_type), count));
            }
        }
        if event_rollup.decision_counts.is_empty() {
            contents.push_str("- Decisions: none\n");
        } else {
            contents.push_str("- Decisions:\n");
            for (decision, count) in event_rollup.decision_counts {
                contents.push_str(&format!("  - {}: {}\n", markdown_cell(&decision), count));
            }
        }
        contents
    }

    fn event_rollup(&self) -> ManagerEventRollup {
        let ping_dir = self.workspace.root.join("pings");
        let mut rollup = ManagerEventRollup::default();
        let Ok(entries) = fs::read_dir(&ping_dir) else {
            return rollup;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
                continue;
            };
            rollup.structured_events += 1;
            let event_type = value
                .get("eventType")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            *rollup
                .event_type_counts
                .entry(event_type.to_string())
                .or_default() += 1;
        }

        let ack_dir = ping_dir.join("acks");
        let Ok(entries) = fs::read_dir(&ack_dir) else {
            return rollup;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("ack") {
                continue;
            }
            let Ok(contents) = fs::read_to_string(&path) else {
                continue;
            };
            rollup.acknowledgements += 1;
            let decision = key_value_line(&contents, "decision").unwrap_or("unknown");
            *rollup
                .decision_counts
                .entry(decision.to_string())
                .or_default() += 1;
        }
        rollup
    }

    fn agents_list_output(&self) -> ManagerAgentsListOutput {
        ManagerAgentsListOutput {
            agents: self
                .workspace
                .agents()
                .iter()
                .map(ManagerAgentOutput::from)
                .collect(),
        }
    }

    pub(crate) fn list_sessions<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        match self.tmux_output(["list-sessions"]) {
            Ok(output) => write!(writer, "{output}")?,
            Err(err) => writeln!(
                writer,
                "No tmux sessions found on socket `{}`: {err:#}",
                self.tmux_socket
            )?,
        }
        Ok(())
    }

    pub(crate) fn start_active<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        for agent in self.workspace.active_agents() {
            self.start_agent_by_ref(agent, writer)?;
        }
        Ok(())
    }

    pub(crate) fn start_agent<W: Write>(
        &self,
        agent_id: &str,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let agent = self.workspace.agent(agent_id)?;
        self.start_agent_by_ref(agent, writer)
    }

    pub(crate) fn stop_agent<W: Write>(
        &self,
        agent_id: &str,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let agent = self.workspace.agent(agent_id)?;
        if !self.session_exists(&agent.session)? {
            writeln!(
                writer,
                "Session is not running on tmux socket {}: {}",
                self.tmux_socket, agent.session
            )?;
            return Ok(());
        }

        let output = self
            .tmux_command()
            .arg("kill-session")
            .arg("-t")
            .arg(&agent.session)
            .output()
            .context("stop tmux specialist session")?;
        if !output.status.success() {
            bail!("tmux kill-session failed: {}", stderr_text(&output));
        }

        self.log_event(format!(
            "stopped {} in tmux socket {} session {}",
            agent.agent_id, self.tmux_socket, agent.session
        ))?;
        writeln!(
            writer,
            "Stopped {} on tmux socket {}: {}",
            agent.agent_id, self.tmux_socket, agent.session
        )?;
        Ok(())
    }

    pub(crate) fn restart_agent<W: Write>(
        &self,
        agent_id: &str,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        self.stop_agent(agent_id, writer)?;
        self.start_agent(agent_id, writer)
    }

    pub(crate) fn capture_agent<W: Write>(
        &self,
        agent_id: &str,
        lines: usize,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let output = self.capture(agent_id, lines)?;
        write!(writer, "{output}")?;
        Ok(())
    }

    pub(crate) fn check_agent<W: Write>(
        &self,
        agent_id: &str,
        lines: usize,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let output = self.capture(agent_id, lines)?;
        write!(writer, "{output}")?;
        print_status_hint(writer, &output)?;
        Ok(())
    }

    pub(crate) fn cycle<W: Write>(&self, lines: usize, writer: &mut W) -> anyhow::Result<()> {
        writeln!(writer, "== pings ==")?;
        let pings = self.pings()?;
        if pings.is_empty() {
            writeln!(writer, "none")?;
        } else {
            for ping in pings {
                writeln!(writer, "{}", ping.display())?;
            }
        }

        for agent in self.workspace.active_agents() {
            writeln!(writer)?;
            writeln!(writer, "== {} ==", agent.agent_id)?;
            match self.capture(&agent.agent_id, lines) {
                Ok(output) => {
                    for line in last_lines(&output, 30) {
                        writeln!(writer, "{line}")?;
                    }
                    print_status_hint(writer, &output)?;
                }
                Err(err) => {
                    writeln!(writer, "{err:#}")?;
                    writeln!(writer, "status_hint=error")?;
                    writeln!(
                        writer,
                        "note=tmux session is missing or could not be captured."
                    )?;
                }
            }
        }
        Ok(())
    }

    pub(crate) fn process_events<W: Write>(
        &self,
        event_dir: &Path,
        dry_run: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let event_dir = resolve_user_path(&self.workspace.root, event_dir);
        let ack_dir = event_dir.join("acks");
        fs::create_dir_all(&ack_dir)
            .with_context(|| format!("create event ack dir {}", ack_dir.display()))?;
        let mut events = fs::read_dir(&event_dir)
            .with_context(|| format!("read event dir {}", event_dir.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().and_then(|extension| extension.to_str()) == Some("json")
            })
            .collect::<Vec<_>>();
        events.sort();

        if events.is_empty() {
            writeln!(writer, "No structured JSON events.")?;
            return Ok(());
        }

        for event_path in events {
            self.process_event_file(&event_path, &ack_dir, dry_run, writer)?;
        }
        Ok(())
    }

    fn process_event_file<W: Write>(
        &self,
        event_path: &Path,
        ack_dir: &Path,
        dry_run: bool,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        let event = match fs::read_to_string(event_path)
            .with_context(|| format!("read event {}", event_path.display()))
            .and_then(|contents| {
                serde_json::from_str::<ManagerSpecialistEvent>(&contents)
                    .with_context(|| format!("parse event {}", event_path.display()))
            }) {
            Ok(event) => event,
            Err(_) => {
                writeln!(
                    writer,
                    "event={} decision=error reason=invalid-json",
                    event_path.display()
                )?;
                return Ok(());
            }
        };
        let ack_file = ack_dir.join(format!(
            "{}.ack",
            sanitize_event_ack_component(&event.event_id)
        ));
        if ack_file.is_file() {
            let decision = event_decision(&event);
            let delivery_agent_id = if decision.should_send {
                self.resolve_event_delivery_agent_id(&event.agent_id)?
            } else {
                event.agent_id.clone()
            };
            writeln!(
                writer,
                "event_id={} agent_id={} decision=already-acknowledged",
                event.event_id, delivery_agent_id
            )?;
            return Ok(());
        }

        let decision = event_decision(&event);
        let delivery_agent_id = if decision.should_send {
            self.resolve_event_delivery_agent_id(&event.agent_id)?
        } else {
            event.agent_id.clone()
        };
        if decision.should_send && !dry_run {
            self.worker_prompt(
                WorkerPromptRequest {
                    agent_id: &delivery_agent_id,
                    prompt: decision.prompt,
                    context_set: None,
                    idempotency_key: Some(&event.event_id),
                    dry_run: false,
                    json: false,
                    ack_json: false,
                },
                &mut std::io::sink(),
            )?;
        }
        let decision_name = if decision.should_send && dry_run {
            decision.dry_run_name
        } else {
            decision.name
        };
        let reason = format!(
            "policy_rule={} approval_gate={} needs_user={} context_level={}",
            decision.policy_rule,
            event.approval_gate.as_deref().unwrap_or("needs-user"),
            event.needs_user.unwrap_or(true),
            decision.context_level.unwrap_or_default()
        );
        fs::write(
            &ack_file,
            format!(
                "event_id={}\nagent_id={}\nevent_type={}\napproval_gate={}\ncontext_level={}\ndecision={}\npolicy_rule={}\nreason={}\n",
                event.event_id,
                delivery_agent_id,
                event.event_type,
                event.approval_gate.as_deref().unwrap_or("needs-user"),
                decision.context_level.unwrap_or_default(),
                decision_name,
                decision.policy_rule,
                reason
            ),
        )
        .with_context(|| format!("write event ack {}", ack_file.display()))?;
        self.log_event(format!(
            "processed event {} for {}: {} ({})",
            event.event_id, event.agent_id, decision_name, reason
        ))?;
        writeln!(
            writer,
            "event_id={} agent_id={} decision={} reason={}",
            event.event_id, delivery_agent_id, decision_name, reason
        )?;
        Ok(())
    }

    fn resolve_event_delivery_agent_id(&self, event_agent_id: &str) -> anyhow::Result<String> {
        if self.workspace.agent(event_agent_id).is_ok() {
            return Ok(event_agent_id.to_string());
        }

        if let Some(agent) = self
            .workspace
            .agents()
            .iter()
            .rev()
            .find(|agent| agent.matter_id == event_agent_id && agent.is_active())
        {
            return Ok(agent.agent_id.clone());
        }

        if let Some(agent) = self
            .workspace
            .agents()
            .iter()
            .rev()
            .find(|agent| agent.matter_id == event_agent_id && agent.status != "closed")
        {
            return Ok(agent.agent_id.clone());
        }

        if let Some(agent) = self
            .workspace
            .agents()
            .iter()
            .rev()
            .find(|agent| agent.matter_id == event_agent_id)
        {
            return Ok(agent.agent_id.clone());
        }

        bail!("unknown manager event agent `{event_agent_id}`");
    }

    pub(crate) fn list_pings<W: Write>(&self, writer: &mut W) -> anyhow::Result<()> {
        let pings = self.pings()?;
        if pings.is_empty() {
            writeln!(writer, "none")?;
        } else {
            for ping in pings {
                writeln!(writer, "{}", ping.display())?;
            }
        }
        Ok(())
    }

    pub(crate) fn start_agent_by_ref<W: Write>(
        &self,
        agent: &ManagerAgent,
        writer: &mut W,
    ) -> anyhow::Result<()> {
        if !agent.workspace.is_dir() {
            bail!(
                "workspace does not exist for {}: {}",
                agent.agent_id,
                agent.workspace.display()
            );
        }
        if !self.specialist_codex_bin.is_file() {
            bail!(
                "specialist Codex binary does not exist: {}",
                self.specialist_codex_bin.display()
            );
        }
        if self.session_exists(&agent.session)? {
            writeln!(
                writer,
                "Session already exists on tmux socket {}: {}",
                self.tmux_socket, agent.session
            )?;
            return Ok(());
        }

        let specialist_codex_bin = shell_quote(&self.specialist_codex_bin);
        let run_cmd = format!(
            "env -u CODEX_CI -u CODEX_SANDBOX_NETWORK_DISABLED -u CODEX_THREAD_ID {specialist_codex_bin} resume --last --specialist --full-auto --no-alt-screen || exec env -u CODEX_CI -u CODEX_SANDBOX_NETWORK_DISABLED -u CODEX_THREAD_ID {specialist_codex_bin} --specialist --full-auto --no-alt-screen",
        );
        let output = self
            .tmux_command()
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&agent.session)
            .arg("-c")
            .arg(&agent.workspace)
            .arg(run_cmd)
            .output()
            .context("start tmux specialist session")?;
        if !output.status.success() {
            bail!("tmux new-session failed: {}", stderr_text(&output));
        }

        self.log_event(format!(
            "started {} in tmux socket {} session {}",
            agent.agent_id, self.tmux_socket, agent.session
        ))?;
        writeln!(
            writer,
            "Started {} on tmux socket {}: {}",
            agent.agent_id, self.tmux_socket, agent.session
        )?;
        Ok(())
    }

    pub(crate) fn capture(&self, agent_id: &str, lines: usize) -> anyhow::Result<String> {
        let agent = self.workspace.agent(agent_id)?;
        let target = self.tmux_target_for(&agent.session)?;
        let start_line = format!("-{lines}");
        self.tmux_output(["capture-pane", "-pt", &target, "-S", &start_line])
    }

    pub(crate) fn tmux_target_for(&self, session_name: &str) -> anyhow::Result<String> {
        let output = self.tmux_output(["list-sessions", "-F", "#{session_name}\t#{session_id}"])?;
        for line in output.lines() {
            if let Some((name, session_id)) = line.split_once('\t')
                && name == session_name
            {
                return Ok(session_id.to_string());
            }
        }
        bail!("no live tmux session named exactly: {session_name}");
    }

    fn session_exists(&self, session_name: &str) -> anyhow::Result<bool> {
        let status = self
            .tmux_command()
            .arg("has-session")
            .arg("-t")
            .arg(session_name)
            .stderr(Stdio::null())
            .status()
            .context("query tmux session")?;
        Ok(status.success())
    }

    pub(crate) fn pings(&self) -> anyhow::Result<Vec<PathBuf>> {
        let ping_dir = self.workspace.root.join("pings");
        if !ping_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut pings = Vec::new();
        for entry in fs::read_dir(&ping_dir).context("read manager ping directory")? {
            let entry = entry.context("read manager ping entry")?;
            let path = entry.path();
            if path.is_file()
                && path.file_name().and_then(|name| name.to_str()) != Some("README.md")
            {
                pings.push(path);
            }
        }
        pings.sort();
        Ok(pings)
    }

    pub(crate) fn log_event(&self, message: String) -> anyhow::Result<()> {
        let path = self.workspace.root.join("events.log.md");
        let stamp = current_timestamp();
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("open manager event log {}", path.display()))?;
        writeln!(file, "- {stamp}: {message}")
            .with_context(|| format!("write manager event log {}", path.display()))?;
        Ok(())
    }

    fn tmux_output<const N: usize>(&self, args: [&str; N]) -> anyhow::Result<String> {
        let output = self
            .tmux_command()
            .args(args)
            .output()
            .context("run tmux command")?;
        if !output.status.success() {
            bail!("{}", stderr_text(&output));
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    pub(crate) fn tmux_command(&self) -> Command {
        let mut command = Command::new("tmux");
        command.arg("-L").arg(&self.tmux_socket);
        command
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagerObserversListOutput {
    observers: Vec<ManagerObserverOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagerObserverOutput {
    agent_id: String,
    session_id: String,
    workspace: PathBuf,
    observer_window_status: String,
    attach_command: String,
}

#[derive(Default)]
struct ManagerEventRollup {
    structured_events: usize,
    acknowledgements: usize,
    event_type_counts: BTreeMap<String, usize>,
    decision_counts: BTreeMap<String, usize>,
}

fn observer_status(agent: &ManagerAgent, sessions: &BTreeSet<String>) -> &'static str {
    if !agent.workspace.is_dir() {
        "stale"
    } else if sessions.contains(&agent.session) {
        "open"
    } else {
        "missing"
    }
}

fn validate_agent_registry_schema(value: &serde_json::Value) -> anyhow::Result<()> {
    let agents = value
        .get("agents")
        .and_then(serde_json::Value::as_array)
        .context("registry must contain an agents array")?;
    if agents.is_empty() {
        bail!("registry agents array must not be empty");
    }
    for (index, agent) in agents.iter().enumerate() {
        validate_required_string(agent, index, "agentId")?;
        validate_required_string(agent, index, "matterId")?;
        validate_required_string(agent, index, "name")?;
        validate_required_string(agent, index, "workspace")?;
        validate_required_string(agent, index, "sessionId")?;
        validate_required_string(agent, index, "role")?;
        validate_required_string(agent, index, "lifecycleState")?;
        validate_string_array(agent, index, "sourceRoots")?;
        validate_required_field(agent, index, "currentObjective")?;
        let automation_mode = validate_required_string(agent, index, "automationMode")?;
        if !matches!(
            automation_mode,
            "stopped" | "observing" | "actively-managing"
        ) {
            bail!("agents[{index}].automationMode has invalid value `{automation_mode}`");
        }
        let observer_window_status =
            validate_required_string(agent, index, "observerWindowStatus")?;
        if !matches!(
            observer_window_status,
            "unknown" | "open" | "stale" | "missing"
        ) {
            bail!(
                "agents[{index}].observerWindowStatus has invalid value `{observer_window_status}`"
            );
        }
    }
    Ok(())
}

fn validate_required_field<'a>(
    agent: &'a serde_json::Value,
    index: usize,
    field: &str,
) -> anyhow::Result<&'a serde_json::Value> {
    agent
        .get(field)
        .with_context(|| format!("agents[{index}].{field} is required"))
}

fn validate_required_string<'a>(
    agent: &'a serde_json::Value,
    index: usize,
    field: &str,
) -> anyhow::Result<&'a str> {
    let value = validate_required_field(agent, index, field)?
        .as_str()
        .with_context(|| format!("agents[{index}].{field} must be a string"))?;
    if value.is_empty() {
        bail!("agents[{index}].{field} must not be empty");
    }
    Ok(value)
}

fn validate_string_array(
    agent: &serde_json::Value,
    index: usize,
    field: &str,
) -> anyhow::Result<()> {
    let values = validate_required_field(agent, index, field)?
        .as_array()
        .with_context(|| format!("agents[{index}].{field} must be an array"))?;
    for (value_index, value) in values.iter().enumerate() {
        if !value.is_string() {
            bail!("agents[{index}].{field}[{value_index}] must be a string");
        }
    }
    Ok(())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagerAgentsListOutput {
    agents: Vec<ManagerAgentOutput>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ManagerAgentOutput {
    agent_id: String,
    matter_id: String,
    name: String,
    workspace: PathBuf,
    source_roots: Vec<PathBuf>,
    session_id: String,
    role: String,
    lifecycle_state: String,
    automation_mode: String,
    observer_window_status: String,
    current_objective: String,
}

impl From<&ManagerAgent> for ManagerAgentOutput {
    fn from(agent: &ManagerAgent) -> Self {
        Self {
            agent_id: agent.agent_id.clone(),
            matter_id: agent.matter_id.clone(),
            name: agent.name.clone(),
            workspace: agent.workspace.clone(),
            source_roots: source_roots_for_agent(agent),
            session_id: agent.session.clone(),
            role: agent.role.clone(),
            lifecycle_state: agent.status.clone(),
            automation_mode: agent.automation_mode().to_string(),
            observer_window_status: agent.observer_window_status().to_string(),
            current_objective: agent.current_objective.clone(),
        }
    }
}

fn source_roots_for_agent(agent: &ManagerAgent) -> Vec<PathBuf> {
    let Ok(workspace) = load_workspace(&agent.workspace, None, None) else {
        return Vec::new();
    };
    let mut roots = workspace
        .roles
        .values()
        .filter(|role| role.access == RoleAccess::ReadOnly)
        .map(|role| role.path.clone())
        .collect::<Vec<_>>();
    roots.sort();
    roots
}

fn resolve_current_dir_path(path: PathBuf) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path)
    } else {
        let current_dir = std::env::current_dir().context("resolve current directory")?;
        Ok(resolve_user_path(&current_dir, &path))
    }
}

fn print_status_hint<W: Write>(writer: &mut W, output: &str) -> anyhow::Result<()> {
    writeln!(writer)?;
    writeln!(writer, "--- manager heuristic ---")?;
    writeln!(writer, "status_hint={}", status_hint(output))?;
    writeln!(
        writer,
        "note=Codex CLI suggested text at the prompt is not user-control by itself."
    )?;
    Ok(())
}

pub(crate) fn status_hint(output: &str) -> &'static str {
    let lower = output.to_ascii_lowercase();
    if lower.contains("the application panicked")
        || lower.contains("panicked at")
        || lower.contains("plugin cache root should be absolute")
        || lower.contains("operation not permitted")
        || lower.contains("traceback")
        || lower.contains("stream disconnected")
        || lower.contains("failed to resolve cwd")
    {
        "error"
    } else if output.contains("Working (")
        || output.contains("Waiting for background terminal")
        || output.contains("Reconnecting")
    {
        "working"
    } else if output.lines().any(|line| line.starts_with("› ")) {
        "waiting-at-prompt"
    } else if lower.contains("error") || lower.contains("failed") || lower.contains("reconnecting")
    {
        "watch-or-error"
    } else {
        "unknown"
    }
}

pub(crate) fn last_lines(text: &str, limit: usize) -> Vec<&str> {
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(limit);
    lines[start..].to_vec()
}

fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn markdown_cell(value: &str) -> String {
    value.replace('|', "\\|").replace('\n', " ")
}

fn sanitize_filename(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn sanitize_event_ack_component(value: &str) -> String {
    sanitize_filename(value)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagerSpecialistEvent {
    event_id: String,
    agent_id: String,
    #[serde(default)]
    event_type: String,
    #[serde(default)]
    approval_gate: Option<String>,
    #[serde(default)]
    needs_user: Option<bool>,
    #[serde(default)]
    recommended_next: Option<String>,
    #[serde(default)]
    context_level: Option<String>,
    #[serde(default)]
    blocked_alternative: Option<ManagerBlockedAlternative>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagerBlockedAlternative {
    recommended_next: String,
    approval_gate: String,
    #[serde(default)]
    context_level: Option<String>,
}

struct ManagerEventDecision<'a> {
    name: &'static str,
    dry_run_name: &'static str,
    policy_rule: String,
    should_send: bool,
    prompt: &'a str,
    context_level: Option<&'a str>,
}

fn event_decision(event: &ManagerSpecialistEvent) -> ManagerEventDecision<'_> {
    let approval_gate = event.approval_gate.as_deref().unwrap_or("needs-user");
    let prompt = event.recommended_next.as_deref().unwrap_or_default().trim();
    if event.needs_user.unwrap_or(true) {
        return ManagerEventDecision {
            name: "escalated",
            dry_run_name: "escalated",
            policy_rule: "needs-user-requires-escalation".to_string(),
            should_send: false,
            prompt,
            context_level: event.context_level.as_deref(),
        };
    }
    if approval_gate == "blocked" {
        if let Some(alternative) = event.blocked_alternative.as_ref()
            && alternative.approval_gate == "plain-continuation"
            && !alternative.recommended_next.trim().is_empty()
        {
            return ManagerEventDecision {
                name: "auto-continued",
                dry_run_name: "would-auto-continue",
                policy_rule: "blocked-non-dependent-alternative:plain-continuation".to_string(),
                should_send: true,
                prompt: alternative.recommended_next.trim(),
                context_level: alternative.context_level.as_deref(),
            };
        }
        return ManagerEventDecision {
            name: "escalated",
            dry_run_name: "escalated",
            policy_rule: "blocked-requires-manager-review".to_string(),
            should_send: false,
            prompt,
            context_level: event.context_level.as_deref(),
        };
    }
    if prompt.is_empty() {
        return ManagerEventDecision {
            name: "escalated",
            dry_run_name: "escalated",
            policy_rule: "missing-recommended-next".to_string(),
            should_send: false,
            prompt,
            context_level: event.context_level.as_deref(),
        };
    }
    if approval_gate == "plain-continuation" {
        return ManagerEventDecision {
            name: "auto-continued",
            dry_run_name: "would-auto-continue",
            policy_rule: "auto-continue-allowed-gate:plain-continuation".to_string(),
            should_send: true,
            prompt,
            context_level: event.context_level.as_deref(),
        };
    }
    if approval_gate == "broad-low-context" {
        if is_checkpoint_or_handoff_task(prompt) {
            return ManagerEventDecision {
                name: "checkpoint-handoff-requested",
                dry_run_name: "would-request-checkpoint-handoff",
                policy_rule: "broad-low-context-checkpoint-handoff".to_string(),
                should_send: true,
                prompt,
                context_level: event.context_level.as_deref(),
            };
        }
        return ManagerEventDecision {
            name: "escalated",
            dry_run_name: "escalated",
            policy_rule: "broad-low-context-broad-task-escalation".to_string(),
            should_send: false,
            prompt,
            context_level: event.context_level.as_deref(),
        };
    }
    ManagerEventDecision {
        name: "escalated",
        dry_run_name: "escalated",
        policy_rule: format!("approval-gate-requires-user:{approval_gate}"),
        should_send: false,
        prompt,
        context_level: event.context_level.as_deref(),
    }
}

fn is_checkpoint_or_handoff_task(prompt: &str) -> bool {
    let prompt = prompt.to_ascii_lowercase();
    prompt.contains("checkpoint")
        || prompt.contains("handoff")
        || prompt.contains("compression-safe")
}

fn key_value_line<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let (line_key, value) = line.split_once('=')?;
        if line_key == key { Some(value) } else { None }
    })
}

pub(crate) fn stderr_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("process exited with status {}", output.status)
    } else {
        stderr
    }
}

pub(crate) fn current_timestamp() -> String {
    let output = Command::new("date")
        .arg("+%Y-%m-%d %H:%M %Z")
        .output()
        .ok()
        .filter(|output| output.status.success());
    output
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown-time".to_string())
}

#[cfg(test)]
mod tests {
    use super::status_hint;
    use pretty_assertions::assert_eq;

    #[test]
    fn status_hint_detects_prompt_and_errors() {
        assert_eq!(status_hint("› Continue"), "waiting-at-prompt");
        assert_eq!(status_hint("The application panicked (crashed)"), "error");
        assert_eq!(status_hint("Working (45s)"), "working");
    }
}
