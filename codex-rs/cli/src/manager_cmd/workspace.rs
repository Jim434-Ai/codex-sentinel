use anyhow::Context;
use anyhow::bail;
use serde::Deserialize;
use serde::Serialize;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

const DEFAULT_AGENT_REGISTRY_JSON: &str = "agents.json";
const DEFAULT_AGENT_REGISTRY_TSV: &str = "agents.tsv";
const DEFAULT_MANAGER_CONFIG: &str = ".codex-manager/manager.toml";
const MANAGER_AGENTS_FILE: &str = "AGENTS.md";

pub(crate) struct ManagerWorkspace {
    pub(crate) root: PathBuf,
    pub(crate) registry_path: PathBuf,
    pub(crate) config: ManagerConfig,
    agents: Vec<ManagerAgent>,
}

impl ManagerWorkspace {
    pub(crate) fn load(explicit_root: Option<&Path>) -> anyhow::Result<Self> {
        let root = match explicit_root {
            Some(path) if path.is_absolute() => path.to_path_buf(),
            Some(path) => {
                let cwd = std::env::current_dir().context("resolve current directory")?;
                resolve_user_path(&cwd, path)
            }
            None => {
                let cwd = std::env::current_dir().context("resolve current directory")?;
                discover_manager_workspace(&cwd)?
            }
        };
        let registry_path = if root.join(DEFAULT_AGENT_REGISTRY_JSON).is_file() {
            root.join(DEFAULT_AGENT_REGISTRY_JSON)
        } else {
            root.join(DEFAULT_AGENT_REGISTRY_TSV)
        };
        let registry = fs::read_to_string(&registry_path)
            .with_context(|| format!("read manager registry {}", registry_path.display()))?;
        let agents = parse_agents(&registry, &registry_path, &root)?;
        let config = ManagerConfig::load(&root)?;

        Ok(Self {
            root,
            registry_path,
            config,
            agents,
        })
    }

    pub(crate) fn agents(&self) -> &[ManagerAgent] {
        &self.agents
    }

    pub(crate) fn agent(&self, agent_id: &str) -> anyhow::Result<&ManagerAgent> {
        self.agents
            .iter()
            .find(|agent| agent.agent_id == agent_id)
            .with_context(|| format!("unknown manager agent `{agent_id}`"))
    }

    pub(crate) fn active_agents(&self) -> impl Iterator<Item = &ManagerAgent> {
        self.agents.iter().filter(|agent| agent.is_active())
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManagerAgent {
    pub(crate) agent_id: String,
    pub(crate) matter_id: String,
    pub(crate) name: String,
    pub(crate) workspace: PathBuf,
    pub(crate) session: String,
    pub(crate) role: String,
    pub(crate) status: String,
    pub(crate) current_objective: String,
}

impl ManagerAgent {
    pub(crate) fn is_active(&self) -> bool {
        !matches!(self.status.as_str(), "closed" | "paused" | "user-control")
    }

    pub(crate) fn automation_mode(&self) -> &'static str {
        if self.is_active() {
            "actively-managing"
        } else if self.status == "closed" {
            "stopped"
        } else {
            "observing"
        }
    }

    pub(crate) fn observer_window_status(&self) -> &'static str {
        "unknown"
    }
}

#[derive(Debug, Default, Deserialize)]
pub(crate) struct ManagerConfig {
    pub(crate) tmux: Option<ManagerTmuxConfig>,
    pub(crate) paths: Option<ManagerPathsConfig>,
}

impl ManagerConfig {
    fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(DEFAULT_MANAGER_CONFIG);
        if !path.is_file() {
            return Ok(Self::default());
        }

        let contents = fs::read_to_string(&path)
            .with_context(|| format!("read manager config {}", path.display()))?;
        toml::from_str(&contents)
            .with_context(|| format!("parse manager config {}", path.display()))
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct ManagerTmuxConfig {
    pub(crate) socket: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ManagerPathsConfig {
    pub(crate) specialist_codex_bin: Option<PathBuf>,
}

pub(crate) fn resolve_user_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn discover_manager_workspace(start_dir: &Path) -> anyhow::Result<PathBuf> {
    for candidate in start_dir.ancestors() {
        if candidate.join(MANAGER_AGENTS_FILE).is_file()
            && (candidate.join(DEFAULT_AGENT_REGISTRY_JSON).is_file()
                || candidate.join(DEFAULT_AGENT_REGISTRY_TSV).is_file())
        {
            return Ok(candidate.to_path_buf());
        }
    }

    bail!(
        "could not find manager workspace from {}; expected {} or {}, plus {}",
        start_dir.display(),
        DEFAULT_AGENT_REGISTRY_JSON,
        DEFAULT_AGENT_REGISTRY_TSV,
        MANAGER_AGENTS_FILE
    )
}

fn parse_agents(
    contents: &str,
    path: &Path,
    manager_root: &Path,
) -> anyhow::Result<Vec<ManagerAgent>> {
    if path.extension().and_then(|extension| extension.to_str()) == Some("json") {
        parse_agents_json(contents, path, manager_root)
    } else {
        parse_agents_tsv(contents, path, manager_root)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagerAgentRegistryJson {
    agents: Vec<ManagerAgentJson>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManagerAgentJson {
    agent_id: String,
    matter_id: String,
    name: String,
    workspace: PathBuf,
    session_id: String,
    role: String,
    lifecycle_state: String,
    current_objective: String,
}

fn parse_agents_json(
    contents: &str,
    path: &Path,
    manager_root: &Path,
) -> anyhow::Result<Vec<ManagerAgent>> {
    let registry: ManagerAgentRegistryJson = serde_json::from_str(contents)
        .with_context(|| format!("parse manager registry {}", path.display()))?;
    let agents = registry
        .agents
        .into_iter()
        .map(|agent| ManagerAgent {
            agent_id: agent.agent_id,
            matter_id: agent.matter_id,
            name: agent.name,
            workspace: resolve_user_path(manager_root, &agent.workspace),
            session: agent.session_id,
            role: agent.role,
            status: agent.lifecycle_state,
            current_objective: agent.current_objective,
        })
        .collect::<Vec<_>>();
    if agents.is_empty() {
        bail!("{} does not define any manager agents", path.display());
    }
    Ok(agents)
}

fn parse_agents_tsv(
    contents: &str,
    path: &Path,
    manager_root: &Path,
) -> anyhow::Result<Vec<ManagerAgent>> {
    let mut agents = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let line_number = line_index + 1;
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        if line_number == 1 && line.starts_with("agent_id\t") {
            continue;
        }

        let fields = line.splitn(8, '\t').collect::<Vec<_>>();
        if fields.len() != 8 {
            bail!(
                "{}:{line_number}: expected 8 tab-separated fields, got {}",
                path.display(),
                fields.len()
            );
        }

        agents.push(ManagerAgent {
            agent_id: fields[0].to_string(),
            matter_id: fields[1].to_string(),
            name: fields[2].to_string(),
            workspace: resolve_user_path(manager_root, Path::new(fields[3])),
            session: fields[4].to_string(),
            role: fields[5].to_string(),
            status: fields[6].to_string(),
            current_objective: fields[7].to_string(),
        });
    }

    if agents.is_empty() {
        bail!("{} does not define any manager agents", path.display());
    }

    Ok(agents)
}

#[cfg(test)]
mod tests {
    use super::parse_agents_tsv;
    use pretty_assertions::assert_eq;
    use std::path::Path;

    #[test]
    fn parses_existing_agents_tsv_shape() {
        let agents = parse_agents_tsv(
            "agent_id\tmatter_id\tname\tworkspace\tsession\trole\tstatus\tcurrent_objective\n\
             agent-003\tmatter\tVeterans\tspecialists/veterans\tagent-003\tspecialist\tworking\tContinue NOI work\n",
            Path::new("agents.tsv"),
            Path::new("/manager"),
        )
        .expect("parse agents");

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_id, "agent-003");
        assert_eq!(agents[0].matter_id, "matter");
        assert_eq!(agents[0].name, "Veterans");
        assert_eq!(
            agents[0].workspace,
            Path::new("/manager/specialists/veterans")
        );
        assert_eq!(agents[0].session, "agent-003");
        assert_eq!(agents[0].role, "specialist");
        assert_eq!(agents[0].status, "working");
        assert_eq!(agents[0].current_objective, "Continue NOI work");
        assert_eq!(agents[0].automation_mode(), "actively-managing");
    }
}
