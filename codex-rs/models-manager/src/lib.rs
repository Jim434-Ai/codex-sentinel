pub(crate) mod cache;
pub mod collaboration_mode_presets;
pub(crate) mod config;
pub mod manager;
pub mod model_info;
pub mod model_presets;

pub use codex_app_server_protocol::AuthMode;
pub use codex_login::AuthManager;
pub use codex_login::CodexAuth;
pub use codex_model_provider_info::ModelProviderInfo;
pub use codex_model_provider_info::WireApi;
pub use config::ModelsManagerConfig;

/// Load the bundled model catalog shipped with `codex-models-manager`.
pub fn bundled_models_response()
-> std::result::Result<codex_protocol::openai_models::ModelsResponse, serde_json::Error> {
    serde_json::from_str(include_str!("../models.json"))
}

/// Convert the client version string to a whole version string (e.g. "1.2.3-alpha.4" -> "1.2.3").
pub fn client_version_to_whole() -> String {
    format!(
        "{}.{}.{}",
        env!("CARGO_PKG_VERSION_MAJOR"),
        env!("CARGO_PKG_VERSION_MINOR"),
        env!("CARGO_PKG_VERSION_PATCH")
    )
}

#[cfg(test)]
mod tests {
    use super::client_version_to_whole;

    #[test]
    fn client_version_satisfies_current_model_catalog_gate() {
        let client_version = parse_version(&client_version_to_whole());
        assert!(
            client_version >= (0, 98, 0),
            "client_version sent to /models must satisfy current target model minimum"
        );
    }

    fn parse_version(version: &str) -> (u32, u32, u32) {
        let parts = version
            .split('.')
            .map(|part| part.parse::<u32>().expect("numeric version part"))
            .collect::<Vec<_>>();
        assert_eq!(parts.len(), 3);
        (parts[0], parts[1], parts[2])
    }
}
