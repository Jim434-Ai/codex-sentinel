use codex_core::config::Config;
use codex_protocol::openai_models::InputModality;
use codex_protocol::openai_models::ModelPreset;
use codex_protocol::openai_models::ReasoningEffort;
use codex_protocol::openai_models::ReasoningEffortPreset;
use color_eyre::eyre::ContextCompat;
use color_eyre::eyre::Result;
use color_eyre::eyre::WrapErr;
use serde::Deserialize;
use serde::Serialize;
use std::time::Duration;

#[derive(Deserialize)]
struct OllamaTagsResponse {
    #[serde(default)]
    models: Vec<OllamaTagModel>,
}

#[derive(Deserialize)]
struct OllamaTagModel {
    name: String,
    #[serde(default)]
    details: OllamaTagDetails,
}

#[derive(Default, Deserialize)]
struct OllamaTagDetails {
    #[serde(default)]
    family: String,
    #[serde(default)]
    families: Vec<String>,
}

#[derive(Serialize)]
struct OllamaShowRequest {
    model: String,
}

#[derive(Default, Deserialize)]
struct OllamaShowResponse {
    #[serde(default)]
    capabilities: Vec<String>,
}

struct OllamaModelMetadata {
    tag: OllamaTagModel,
    show: Option<OllamaShowResponse>,
}

pub(crate) async fn embedded_ollama_model_presets(config: &Config) -> Result<Vec<ModelPreset>> {
    let base_url = config
        .model_provider
        .base_url
        .as_deref()
        .wrap_err("embedded ollama provider is missing a base_url")?;
    let host_root = ollama_host_root(base_url).trim_end_matches('/').to_string();
    let tags_url = format!("{host_root}/api/tags");
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(5))
        .build()
        .wrap_err("failed to create Ollama HTTP client for TUI model picker")?;
    let tags: OllamaTagsResponse = client
        .get(tags_url)
        .send()
        .await
        .wrap_err("failed to fetch embedded Ollama model list for TUI picker")?
        .error_for_status()
        .wrap_err("embedded Ollama model list request failed for TUI picker")?
        .json()
        .await
        .wrap_err("failed to decode embedded Ollama model list for TUI picker")?;

    let models = hydrate_ollama_models_with_show_metadata(&client, &host_root, tags.models).await;
    Ok(ollama_model_presets_from_metadata(
        models,
        config.model.as_deref(),
    ))
}

async fn hydrate_ollama_models_with_show_metadata(
    client: &reqwest::Client,
    host_root: &str,
    tags: Vec<OllamaTagModel>,
) -> Vec<OllamaModelMetadata> {
    let show_url = format!("{}/api/show", host_root.trim_end_matches('/'));
    let mut handles = Vec::new();
    for tag in tags {
        let client = client.clone();
        let show_url = show_url.clone();
        let model_name = tag.name.clone();
        handles.push(tokio::spawn(async move {
            let show = fetch_ollama_show_metadata(&client, &show_url, model_name.as_str()).await;
            if let Err(err) = &show {
                tracing::warn!("failed to load Ollama metadata for `{model_name}`: {err}");
            }
            OllamaModelMetadata {
                tag,
                show: show.ok(),
            }
        }));
    }

    let mut models = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(model) => models.push(model),
            Err(err) => tracing::warn!("failed to join Ollama metadata task: {err}"),
        }
    }
    models
}

async fn fetch_ollama_show_metadata(
    client: &reqwest::Client,
    show_url: &str,
    model_name: &str,
) -> Result<OllamaShowResponse> {
    client
        .post(show_url)
        .json(&OllamaShowRequest {
            model: model_name.to_string(),
        })
        .send()
        .await
        .wrap_err("failed to fetch embedded Ollama model metadata")?
        .error_for_status()
        .wrap_err("embedded Ollama model metadata request failed")?
        .json()
        .await
        .wrap_err("failed to decode embedded Ollama model metadata")
}

fn ollama_model_presets_from_metadata(
    mut models: Vec<OllamaModelMetadata>,
    default_model: Option<&str>,
) -> Vec<ModelPreset> {
    models.retain(is_likely_chat_or_generation_ollama_model);
    models.sort_by(|left, right| left.tag.name.cmp(&right.tag.name));
    let fallback_default = models.first().map(|model| model.tag.name.clone());

    models
        .into_iter()
        .map(|model| {
            let is_default = default_model.is_some_and(|name| name == model.tag.name.as_str())
                || default_model.is_none()
                    && fallback_default
                        .as_ref()
                        .is_some_and(|default| default == &model.tag.name);
            let supported_reasoning_efforts = ollama_reasoning_efforts(&model);

            ModelPreset {
                id: model.tag.name.clone(),
                model: model.tag.name.clone(),
                display_name: model.tag.name.clone(),
                description: ollama_model_description(&model),
                default_reasoning_effort: ReasoningEffort::None,
                supported_reasoning_efforts,
                supports_personality: false,
                additional_speed_tiers: Vec::new(),
                is_default,
                upgrade: None,
                show_in_picker: true,
                availability_nux: None,
                supported_in_api: true,
                input_modalities: ollama_input_modalities(&model),
            }
        })
        .collect()
}

fn is_likely_chat_or_generation_ollama_model(model: &OllamaModelMetadata) -> bool {
    let name = model.tag.name.to_ascii_lowercase();
    if ["embed", "embedding", "rerank", "ocr", "whisper", "clip"]
        .iter()
        .any(|non_chat_marker| name.contains(non_chat_marker))
    {
        return false;
    }
    if let Some(show) = &model.show
        && !show.capabilities.is_empty()
    {
        return show
            .capabilities
            .iter()
            .any(|capability| capability == "completion");
    }

    let mut families = Vec::new();
    if !model.tag.details.family.is_empty() {
        families.push(model.tag.details.family.to_ascii_lowercase());
    }
    families.extend(
        model
            .tag
            .details
            .families
            .iter()
            .map(|family| family.to_ascii_lowercase()),
    );
    if families.is_empty() {
        return true;
    }

    !families
        .iter()
        .all(|family| is_non_chat_ollama_family(family))
}

fn is_non_chat_ollama_family(family: &str) -> bool {
    matches!(
        family,
        "bert" | "nomic-bert" | "clip" | "deepseekocr" | "whisper"
    )
}

fn ollama_model_description(model: &OllamaModelMetadata) -> String {
    let Some(show) = &model.show else {
        return "Local Ollama model".to_string();
    };
    if show.capabilities.is_empty() {
        return "Local Ollama model".to_string();
    }

    let capabilities = show.capabilities.join(", ");
    if has_ollama_capability(model, "thinking") {
        format!(
            "Local Ollama model. Capabilities: {capabilities}. Thinking-capable; granular reasoning effort is not exposed."
        )
    } else {
        format!("Local Ollama model. Capabilities: {capabilities}.")
    }
}

fn ollama_reasoning_efforts(model: &OllamaModelMetadata) -> Vec<ReasoningEffortPreset> {
    vec![ReasoningEffortPreset {
        effort: ReasoningEffort::None,
        description: if has_ollama_capability(model, "thinking") {
            "This Ollama model reports thinking support, but Codex cannot safely map it to reasoning effort levels."
        } else {
            "Ollama local model selection does not expose Codex reasoning effort controls."
        }
        .to_string(),
    }]
}

fn has_ollama_capability(model: &OllamaModelMetadata, expected: &str) -> bool {
    model.show.as_ref().is_some_and(|show| {
        show.capabilities
            .iter()
            .any(|capability| capability == expected)
    })
}

fn ollama_input_modalities(model: &OllamaModelMetadata) -> Vec<InputModality> {
    let mut modalities = vec![InputModality::Text];
    if has_ollama_capability(model, "vision") {
        modalities.push(InputModality::Image);
    }
    modalities
}

fn ollama_host_root(base_url: &str) -> &str {
    base_url
        .trim_end_matches('/')
        .strip_suffix("/v1")
        .unwrap_or_else(|| base_url.trim_end_matches('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn ollama_model_presets_mark_the_selected_model_as_default() {
        let presets = ollama_model_presets_from_metadata(
            vec![
                ollama_model(
                    "qwen3.5:27b",
                    "qwen35",
                    &["qwen35"],
                    &["completion", "tools", "thinking"],
                ),
                ollama_model(
                    "nomic-embed-text:latest",
                    "nomic-bert",
                    &["nomic-bert"],
                    &["embedding"],
                ),
                ollama_model(
                    "gemma4:26b",
                    "gemma4",
                    &["gemma4"],
                    &["completion", "vision", "tools", "thinking"],
                ),
                ollama_model(
                    "deepseek-ocr:latest",
                    "deepseekocr",
                    &["deepseekocr"],
                    &["completion", "vision"],
                ),
            ],
            Some("qwen3.5:27b"),
        );

        assert_eq!(
            presets
                .iter()
                .map(|preset| preset.model.as_str())
                .collect::<Vec<_>>(),
            vec!["gemma4:26b", "qwen3.5:27b"]
        );
        assert_eq!(presets.iter().filter(|preset| preset.is_default).count(), 1);
        assert_eq!(
            presets
                .iter()
                .find(|preset| preset.model == "qwen3.5:27b")
                .expect("preset should exist")
                .default_reasoning_effort,
            ReasoningEffort::None
        );
        assert_eq!(
            presets
                .iter()
                .find(|preset| preset.model == "qwen3.5:27b")
                .expect("preset should exist")
                .supported_reasoning_efforts
                .iter()
                .map(|preset| preset.effort)
                .collect::<Vec<_>>(),
            vec![ReasoningEffort::None]
        );
        assert!(
            presets
                .iter()
                .find(|preset| preset.model == "qwen3.5:27b")
                .expect("preset should exist")
                .description
                .contains("Thinking-capable")
        );
    }

    #[test]
    fn ollama_model_presets_fall_back_to_the_first_model_when_none_is_selected() {
        let presets = ollama_model_presets_from_metadata(
            vec![
                ollama_model("qwen3.5:27b", "qwen35", &["qwen35"], &["completion"]),
                ollama_model("gemma4:26b", "gemma4", &["gemma4"], &["completion"]),
            ],
            None,
        );

        assert!(
            presets
                .iter()
                .find(|preset| preset.model == "gemma4:26b")
                .expect("preset should exist")
                .is_default
        );
        assert_eq!(presets.iter().filter(|preset| preset.is_default).count(), 1);
    }

    #[test]
    fn ollama_model_presets_keep_multimodal_generation_tags() {
        let presets = ollama_model_presets_from_metadata(
            vec![ollama_model(
                "openbmb/minicpm-v4.5:latest",
                "qwen3",
                &["qwen3", "clip"],
                &["completion", "vision"],
            )],
            None,
        );

        assert_eq!(
            presets
                .iter()
                .map(|preset| preset.model.as_str())
                .collect::<Vec<_>>(),
            vec!["openbmb/minicpm-v4.5:latest"]
        );
        assert_eq!(
            presets[0].input_modalities,
            vec![InputModality::Text, InputModality::Image]
        );
    }

    #[test]
    fn ollama_model_presets_do_not_overstate_gpt_oss_reasoning_levels() {
        let presets = ollama_model_presets_from_metadata(
            vec![ollama_model(
                "gpt-oss:20b",
                "gptoss",
                &["gptoss"],
                &["completion", "tools", "thinking"],
            )],
            None,
        );

        assert_eq!(presets[0].default_reasoning_effort, ReasoningEffort::None);
        assert_eq!(
            presets[0]
                .supported_reasoning_efforts
                .iter()
                .map(|preset| preset.effort)
                .collect::<Vec<_>>(),
            vec![ReasoningEffort::None]
        );
        assert_eq!(presets[0].input_modalities, vec![InputModality::Text]);
    }

    fn ollama_model(
        name: &str,
        family: &str,
        families: &[&str],
        capabilities: &[&str],
    ) -> OllamaModelMetadata {
        OllamaModelMetadata {
            tag: OllamaTagModel {
                name: name.to_string(),
                details: OllamaTagDetails {
                    family: family.to_string(),
                    families: families
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                },
            },
            show: Some(OllamaShowResponse {
                capabilities: capabilities
                    .iter()
                    .map(std::string::ToString::to_string)
                    .collect(),
            }),
        }
    }
}
