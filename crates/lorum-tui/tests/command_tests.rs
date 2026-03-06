use std::collections::HashMap;

use lorum_ai_contract::{ApiKind, ModelRef};
use lorum_tui::commands::parse_model_selection;

#[test]
fn parse_model_selection_supports_preset_shorthand() {
    let mut presets = HashMap::new();
    presets.insert(
        "codex".to_string(),
        ModelRef {
            provider: "openai".to_string(),
            api: ApiKind::OpenAiCodexResponses,
            model: "gpt-5-codex".to_string(),
        },
    );

    let parsed = parse_model_selection("codex", &presets).expect("parse preset shorthand");
    assert_eq!(parsed.provider, "openai");
    assert_eq!(parsed.api, ApiKind::OpenAiCodexResponses);
    assert_eq!(parsed.model, "gpt-5-codex");
}

#[test]
fn parse_model_selection_supports_explicit_provider_api_model() {
    let parsed =
        parse_model_selection("openai openai_codex_responses gpt-5-codex", &HashMap::new())
            .expect("parse explicit model command");
    assert_eq!(parsed.provider, "openai");
    assert_eq!(parsed.api, ApiKind::OpenAiCodexResponses);
    assert_eq!(parsed.model, "gpt-5-codex");
}

#[test]
fn parse_model_selection_reports_unknown_single_token() {
    let err = parse_model_selection("unknown", &HashMap::new())
        .expect_err("single token without preset should fail");
    assert_eq!(err, "unknown preset 'unknown'");
}
