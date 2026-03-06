mod anthropic;
mod codex;
mod interfaces;
mod openai_responses;
mod runtime_catalog;
mod shared;

pub use anthropic::AnthropicAdapter;
pub use codex::{InMemoryProviderSessionStateStore, OpenAiCodexResponsesAdapter};
pub use interfaces::{
    AnthropicFrame, AnthropicTransport, CodexSseTransport, CodexTransportMeta,
    CodexTransportMode, CodexWebSocketTransport, CollectingFrameSink, FrameSink,
    OpenAiResponsesFrame, OpenAiResponsesTransport, ProviderSessionState,
    ProviderSessionStateStore, RetryPolicy,
};
pub use openai_responses::OpenAiResponsesAdapter;
pub use runtime_catalog::{build_curl_provider_catalog, ProviderCatalog};
pub use shared::{coalesce_delta_events, ToolCallJsonAccumulator};

/// Internal helpers re-exported for integration tests.
#[doc(hidden)]
pub mod internals {
    pub use crate::runtime_catalog::{
        anthropic_prompt_parts, chatgpt_account_id_from_access_token, default_codex_model,
        map_openai_error, openai_codex_frames_from_events, openai_codex_input,
        openai_prompt_input, parse_sse_json_events,
    };
}
