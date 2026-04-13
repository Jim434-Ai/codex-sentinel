use anyhow::Context;
use anyhow::bail;
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

pub(super) const DEFAULT_PROMPT_QUEUE_DIR: &str = "worker-prompts";

const PROCESSED_PROMPT_DIR: &str = ".processed";

pub(super) fn next_queued_prompt(queue_dir: &Path) -> anyhow::Result<Option<PathBuf>> {
    if !queue_dir.is_dir() {
        return Ok(None);
    }

    let mut prompts = Vec::new();
    for entry in fs::read_dir(queue_dir)
        .with_context(|| format!("read prompt queue dir {}", queue_dir.display()))?
    {
        let entry = entry.context("read prompt queue entry")?;
        let path = entry.path();
        if path.is_file() {
            prompts.push(path);
        }
    }
    prompts.sort();
    Ok(prompts.into_iter().next())
}

pub(super) fn next_prompt_file_path(queue_dir: &Path) -> anyhow::Result<PathBuf> {
    let prefix = epoch_millis();
    for suffix in 0..1000_u16 {
        let path = queue_dir.join(format!("{prefix}-{suffix}.md"));
        if !path.exists() {
            return Ok(path);
        }
    }
    bail!(
        "could not allocate prompt queue file name in {}",
        queue_dir.display()
    )
}

pub(super) fn move_processed_prompt(
    prompt_queue_dir: &Path,
    agent_id: &str,
    prompt_path: &Path,
) -> anyhow::Result<()> {
    let processed_dir = prompt_queue_dir
        .join(PROCESSED_PROMPT_DIR)
        .join(sanitize_queue_component(agent_id));
    fs::create_dir_all(&processed_dir)
        .with_context(|| format!("create processed prompt dir {}", processed_dir.display()))?;
    let file_name = prompt_path
        .file_name()
        .context("queued prompt file must have a file name")?;
    let processed_path = processed_dir.join(file_name);
    fs::rename(prompt_path, &processed_path).with_context(|| {
        format!(
            "move queued prompt {} to {}",
            prompt_path.display(),
            processed_path.display()
        )
    })?;
    Ok(())
}

pub(super) fn sanitize_queue_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn epoch_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
