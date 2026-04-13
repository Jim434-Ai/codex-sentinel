use super::workspace::ManagerAgent;
use super::workspace::ManagerWorkspace;
use super::workspace::resolve_user_path;
use anyhow::Context;
use anyhow::bail;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

const DEFAULT_MANAGER_TMUX_SOCKET: &str = "codex-clean";

pub(crate) struct ManagerRuntime {
    workspace: ManagerWorkspace,
    tmux_socket: String,
    specialist_codex_bin: PathBuf,
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
        let current_dir = std::env::current_dir().context("resolve current directory")?;
        let specialist_codex_bin = if let Some(path) = specialist_codex_bin {
            resolve_user_path(&current_dir, &path)
        } else if let Some(path) = std::env::var_os("SPECIALIST_CODEX_BIN").map(PathBuf::from) {
            resolve_user_path(&current_dir, &path)
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

    fn start_agent_by_ref<W: Write>(
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

        let run_cmd = format!(
            "env -u CODEX_CI -u CODEX_SANDBOX_NETWORK_DISABLED -u CODEX_THREAD_ID {} resume --last --specialist --no-alt-screen",
            shell_quote(&self.specialist_codex_bin)
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

    fn capture(&self, agent_id: &str, lines: usize) -> anyhow::Result<String> {
        let agent = self.workspace.agent(agent_id)?;
        let target = self.tmux_target_for(&agent.session)?;
        let start_line = format!("-{lines}");
        self.tmux_output(["capture-pane", "-pt", &target, "-S", &start_line])
    }

    fn tmux_target_for(&self, session_name: &str) -> anyhow::Result<String> {
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
            .status()
            .context("query tmux session")?;
        Ok(status.success())
    }

    fn pings(&self) -> anyhow::Result<Vec<PathBuf>> {
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

    fn log_event(&self, message: String) -> anyhow::Result<()> {
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

    fn tmux_command(&self) -> Command {
        let mut command = Command::new("tmux");
        command.arg("-L").arg(&self.tmux_socket);
        command
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

fn status_hint(output: &str) -> &'static str {
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

fn last_lines(text: &str, limit: usize) -> Vec<&str> {
    let lines = text.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(limit);
    lines[start..].to_vec()
}

fn shell_quote(path: &Path) -> String {
    let value = path.to_string_lossy();
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn stderr_text(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        format!("process exited with status {}", output.status)
    } else {
        stderr
    }
}

fn current_timestamp() -> String {
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
