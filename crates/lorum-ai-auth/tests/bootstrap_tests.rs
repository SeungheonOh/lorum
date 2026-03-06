use lorum_ai_auth::{
    default_env_keys_for_provider, oauth_default_model_preset, oauth_provider_configuration_error,
    supported_oauth_providers,
};

#[test]
fn default_env_key_mapping_is_defined_for_supported_providers() {
    assert_eq!(
        default_env_keys_for_provider("openai"),
        vec!["OPENAI_API_KEY", "OMP_OPENAI_API_KEY"]
    );
    assert_eq!(
        default_env_keys_for_provider("anthropic"),
        vec!["ANTHROPIC_API_KEY", "OMP_ANTHROPIC_API_KEY"]
    );
    assert!(default_env_keys_for_provider("unknown").is_empty());
}

#[test]
fn supported_oauth_provider_list_is_stable() {
    assert_eq!(supported_oauth_providers(), vec!["openai"]);
}

#[test]
fn oauth_provider_configuration_error_is_defined_for_openai() {
    assert_eq!(
        oauth_provider_configuration_error("openai"),
        Some(
            "OPENAI_OAUTH_CLIENT_ID (or OMP_OPENAI_CLIENT_ID) is required for /login openai"
                .to_string()
        )
    );
    assert!(oauth_provider_configuration_error("unknown").is_none());
}

#[test]
fn oauth_default_model_preset_is_defined_for_openai() {
    assert_eq!(
        oauth_default_model_preset("openai"),
        Some("codex".to_string())
    );
    assert!(oauth_default_model_preset("unknown").is_none());
}
