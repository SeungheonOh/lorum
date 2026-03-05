use std::fmt::{Display, Formatter};
use std::str::FromStr;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiKind {
    #[serde(rename = "openai-completions")]
    OpenAiCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "openai-codex-responses")]
    OpenAiCodexResponses,
    #[serde(rename = "azure-openai-responses")]
    AzureOpenAiResponses,
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
    #[serde(rename = "bedrock-converse-stream")]
    BedrockConverseStream,
    #[serde(rename = "google-generative-ai")]
    GoogleGenerativeAi,
    #[serde(rename = "google-gemini-cli")]
    GoogleGeminiCli,
    #[serde(rename = "google-vertex")]
    GoogleVertex,
    #[serde(rename = "cursor-agent")]
    CursorAgent,
    #[serde(rename = "minimax-messages")]
    MiniMaxMessages,
}

impl ApiKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ApiKind::OpenAiCompletions => "openai-completions",
            ApiKind::OpenAiResponses => "openai-responses",
            ApiKind::OpenAiCodexResponses => "openai-codex-responses",
            ApiKind::AzureOpenAiResponses => "azure-openai-responses",
            ApiKind::AnthropicMessages => "anthropic-messages",
            ApiKind::BedrockConverseStream => "bedrock-converse-stream",
            ApiKind::GoogleGenerativeAi => "google-generative-ai",
            ApiKind::GoogleGeminiCli => "google-gemini-cli",
            ApiKind::GoogleVertex => "google-vertex",
            ApiKind::CursorAgent => "cursor-agent",
            ApiKind::MiniMaxMessages => "minimax-messages",
        }
    }
}

impl Display for ApiKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("unknown api kind: {value}")]
pub struct ParseApiKindError {
    pub value: String,
}

impl FromStr for ApiKind {
    type Err = ParseApiKindError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai-completions" => Ok(ApiKind::OpenAiCompletions),
            "openai-responses" => Ok(ApiKind::OpenAiResponses),
            "openai-codex-responses" => Ok(ApiKind::OpenAiCodexResponses),
            "azure-openai-responses" => Ok(ApiKind::AzureOpenAiResponses),
            "anthropic-messages" => Ok(ApiKind::AnthropicMessages),
            "bedrock-converse-stream" => Ok(ApiKind::BedrockConverseStream),
            "google-generative-ai" => Ok(ApiKind::GoogleGenerativeAi),
            "google-gemini-cli" => Ok(ApiKind::GoogleGeminiCli),
            "google-vertex" => Ok(ApiKind::GoogleVertex),
            "cursor-agent" => Ok(ApiKind::CursorAgent),
            "minimax-messages" => Ok(ApiKind::MiniMaxMessages),
            other => Err(ParseApiKindError {
                value: other.to_string(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub api: ApiKind,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TextContent {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingContent {
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantContent {
    Text(TextContent),
    Thinking(ThinkingContent),
    ToolCall(ToolCall),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

impl TokenUsage {
    pub fn computed_total_tokens(&self) -> u64 {
        self.total_tokens.unwrap_or(
            self.input_tokens
                + self.output_tokens
                + self.cache_read_tokens
                + self.cache_write_tokens,
        )
    }

    pub fn has_any_usage(&self) -> bool {
        self.input_tokens > 0
            || self.output_tokens > 0
            || self.cache_read_tokens > 0
            || self.cache_write_tokens > 0
            || self.total_tokens.is_some()
            || self.cost_usd.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssistantMessage {
    pub message_id: String,
    pub model: ModelRef,
    pub content: Vec<AssistantContent>,
    pub usage: TokenUsage,
    pub stop_reason: StopReason,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamTextDelta {
    pub sequence_no: u64,
    pub block_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamThinkingDelta {
    pub sequence_no: u64,
    pub block_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamToolCallDelta {
    pub sequence_no: u64,
    pub block_id: String,
    pub delta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamBoundaryEvent {
    pub sequence_no: u64,
    pub block_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamStartEvent {
    pub sequence_no: u64,
    pub message_id: String,
    pub model: ModelRef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamDoneEvent {
    pub sequence_no: u64,
    pub message: AssistantMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamErrorEvent {
    pub sequence_no: u64,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start(StreamStartEvent),
    TextStart(StreamBoundaryEvent),
    TextDelta(StreamTextDelta),
    TextEnd(StreamBoundaryEvent),
    ThinkingStart(StreamBoundaryEvent),
    ThinkingDelta(StreamThinkingDelta),
    ThinkingEnd(StreamBoundaryEvent),
    ToolCallStart(StreamBoundaryEvent),
    ToolCallDelta(StreamToolCallDelta),
    ToolCallEnd(StreamBoundaryEvent),
    Done(StreamDoneEvent),
    Error(StreamErrorEvent),
}

impl AssistantMessageEvent {
    pub fn sequence_no(&self) -> u64 {
        match self {
            AssistantMessageEvent::Start(v) => v.sequence_no,
            AssistantMessageEvent::TextStart(v) => v.sequence_no,
            AssistantMessageEvent::TextDelta(v) => v.sequence_no,
            AssistantMessageEvent::TextEnd(v) => v.sequence_no,
            AssistantMessageEvent::ThinkingStart(v) => v.sequence_no,
            AssistantMessageEvent::ThinkingDelta(v) => v.sequence_no,
            AssistantMessageEvent::ThinkingEnd(v) => v.sequence_no,
            AssistantMessageEvent::ToolCallStart(v) => v.sequence_no,
            AssistantMessageEvent::ToolCallDelta(v) => v.sequence_no,
            AssistantMessageEvent::ToolCallEnd(v) => v.sequence_no,
            AssistantMessageEvent::Done(v) => v.sequence_no,
            AssistantMessageEvent::Error(v) => v.sequence_no,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            AssistantMessageEvent::Done(_) | AssistantMessageEvent::Error(_)
        )
    }

    pub fn stop_reason(&self) -> Option<StopReason> {
        match self {
            AssistantMessageEvent::Done(v) => Some(v.message.stop_reason),
            AssistantMessageEvent::Error(_) => Some(StopReason::Error),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderRequest {
    pub session_id: String,
    pub model: ModelRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    pub input: Vec<ProviderInputMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolDefinition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum ProviderInputMessage {
    User {
        content: String,
    },
    Assistant {
        message: AssistantMessage,
    },
    ToolResult {
        tool_call_id: String,
        is_error: bool,
        result: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContext {
    pub api_key: Option<String>,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderTransportDetails {
    pub transport: String,
    pub reused_provider_session: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderFinal {
    pub message: AssistantMessage,
    pub transport_details: Option<ProviderTransportDetails>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProviderError {
    #[error("authentication failed: {message}")]
    Auth { message: String },
    #[error("rate limited: {message}")]
    RateLimited { message: String },
    #[error("transport failure: {message}")]
    Transport { message: String },
    #[error("invalid provider response: {message}")]
    InvalidResponse { message: String },
    #[error("provider request aborted")]
    Aborted,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum StreamSinkError {
    #[error("event sink is closed")]
    Closed,
    #[error("event sink rejected event: {0}")]
    Rejected(String),
}

pub trait AssistantEventSink: Send {
    fn push(&mut self, event: AssistantMessageEvent) -> Result<(), StreamSinkError>;
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn provider_id(&self) -> &str;

    fn api_kind(&self) -> ApiKind;

    async fn stream(
        &self,
        request: ProviderRequest,
        context: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError>;

    async fn complete(
        &self,
        request: ProviderRequest,
        context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError>;

    fn supports_stateful_transport(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_model_ref() -> ModelRef {
        ModelRef {
            provider: "openai".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "gpt-5.2".to_string(),
        }
    }

    fn sample_message(stop_reason: StopReason) -> AssistantMessage {
        AssistantMessage {
            message_id: "m-1".to_string(),
            model: sample_model_ref(),
            content: vec![AssistantContent::Text(TextContent {
                text: "hello".to_string(),
            })],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                total_tokens: None,
                cost_usd: None,
            },
            stop_reason,
        }
    }

    fn event_with_sequence(event: AssistantMessageEvent) -> AssistantMessageEvent {
        event
    }

    #[test]
    fn api_kind_roundtrip_strings() {
        let all = [
            ApiKind::OpenAiCompletions,
            ApiKind::OpenAiResponses,
            ApiKind::OpenAiCodexResponses,
            ApiKind::AzureOpenAiResponses,
            ApiKind::AnthropicMessages,
            ApiKind::BedrockConverseStream,
            ApiKind::GoogleGenerativeAi,
            ApiKind::GoogleGeminiCli,
            ApiKind::GoogleVertex,
            ApiKind::CursorAgent,
            ApiKind::MiniMaxMessages,
        ];

        for kind in all {
            let as_string = kind.to_string();
            let parsed: ApiKind = as_string.parse().expect("must parse");
            assert_eq!(parsed, kind);
        }
    }

    #[test]
    fn api_kind_parse_unknown_fails() {
        let err = "not-real".parse::<ApiKind>().expect_err("must fail");
        assert_eq!(err.value, "not-real");
        assert_eq!(err.to_string(), "unknown api kind: not-real");
    }

    #[test]
    fn stop_reason_json_roundtrip() {
        let all = [
            StopReason::Stop,
            StopReason::Length,
            StopReason::ToolUse,
            StopReason::Error,
            StopReason::Aborted,
        ];

        for reason in all {
            let json = serde_json::to_string(&reason).expect("serialize reason");
            let back: StopReason = serde_json::from_str(&json).expect("deserialize reason");
            assert_eq!(back, reason);
        }
    }

    #[test]
    fn token_usage_computed_total_prefers_explicit_total() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
            total_tokens: Some(99),
            cost_usd: None,
        };

        assert_eq!(usage.computed_total_tokens(), 99);
    }

    #[test]
    fn token_usage_computed_total_falls_back_to_sum() {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            cache_read_tokens: 3,
            cache_write_tokens: 4,
            total_tokens: None,
            cost_usd: None,
        };

        assert_eq!(usage.computed_total_tokens(), 10);
    }

    #[test]
    fn token_usage_has_any_usage_detects_cost() {
        let usage = TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: None,
            cost_usd: Some(0.001),
        };

        assert!(usage.has_any_usage());
    }

    #[test]
    fn token_usage_has_any_usage_detects_none() {
        let usage = TokenUsage::default();
        assert!(!usage.has_any_usage());
    }

    #[test]
    fn assistant_content_serializes_with_tagged_union() {
        let content = AssistantContent::ToolCall(ToolCall {
            id: "tc-1".to_string(),
            name: "grep".to_string(),
            arguments: serde_json::json!({"path":"src"}),
        });

        let json = serde_json::to_value(content).expect("serialize content");
        assert_eq!(json["type"], "tool_call");
    }

    #[test]
    fn assistant_event_sequence_number_is_extracted_for_each_variant() {
        let cases = vec![
            event_with_sequence(AssistantMessageEvent::Start(StreamStartEvent {
                sequence_no: 1,
                message_id: "m".to_string(),
                model: sample_model_ref(),
            })),
            event_with_sequence(AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                sequence_no: 2,
                block_id: "b".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::TextDelta(StreamTextDelta {
                sequence_no: 3,
                block_id: "b".to_string(),
                delta: "a".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::TextEnd(StreamBoundaryEvent {
                sequence_no: 4,
                block_id: "b".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::ThinkingStart(StreamBoundaryEvent {
                sequence_no: 5,
                block_id: "t".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::ThinkingDelta(StreamThinkingDelta {
                sequence_no: 6,
                block_id: "t".to_string(),
                delta: "b".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::ThinkingEnd(StreamBoundaryEvent {
                sequence_no: 7,
                block_id: "t".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::ToolCallStart(StreamBoundaryEvent {
                sequence_no: 8,
                block_id: "tc".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::ToolCallDelta(StreamToolCallDelta {
                sequence_no: 9,
                block_id: "tc".to_string(),
                delta: "{".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::ToolCallEnd(StreamBoundaryEvent {
                sequence_no: 10,
                block_id: "tc".to_string(),
            })),
            event_with_sequence(AssistantMessageEvent::Done(StreamDoneEvent {
                sequence_no: 11,
                message: sample_message(StopReason::Stop),
            })),
            event_with_sequence(AssistantMessageEvent::Error(StreamErrorEvent {
                sequence_no: 12,
                code: "transport".to_string(),
                message: "broken".to_string(),
                retryable: true,
            })),
        ];

        let observed: Vec<u64> = cases
            .iter()
            .map(AssistantMessageEvent::sequence_no)
            .collect();
        assert_eq!(observed, (1..=12).collect::<Vec<_>>());
    }

    #[test]
    fn assistant_event_terminal_detection_is_precise() {
        let done = AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: 1,
            message: sample_message(StopReason::Stop),
        });
        let err = AssistantMessageEvent::Error(StreamErrorEvent {
            sequence_no: 2,
            code: "x".to_string(),
            message: "y".to_string(),
            retryable: false,
        });
        let delta = AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 3,
            block_id: "b".to_string(),
            delta: "z".to_string(),
        });

        assert!(done.is_terminal());
        assert!(err.is_terminal());
        assert!(!delta.is_terminal());
    }

    #[test]
    fn assistant_event_stop_reason_uses_done_message_reason() {
        let event = AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: 1,
            message: sample_message(StopReason::ToolUse),
        });

        assert_eq!(event.stop_reason(), Some(StopReason::ToolUse));
    }

    #[test]
    fn assistant_event_stop_reason_maps_error_to_error() {
        let event = AssistantMessageEvent::Error(StreamErrorEvent {
            sequence_no: 1,
            code: "auth".to_string(),
            message: "bad key".to_string(),
            retryable: false,
        });

        assert_eq!(event.stop_reason(), Some(StopReason::Error));
    }

    #[test]
    fn assistant_event_stop_reason_non_terminal_is_none() {
        let event = AssistantMessageEvent::TextStart(StreamBoundaryEvent {
            sequence_no: 1,
            block_id: "b1".to_string(),
        });

        assert_eq!(event.stop_reason(), None);
    }

    #[test]
    fn provider_error_display_is_stable() {
        let err = ProviderError::Transport {
            message: "timeout".to_string(),
        };
        assert_eq!(err.to_string(), "transport failure: timeout");
    }

    #[test]
    fn provider_error_serialization_uses_kind_tag() {
        let err = ProviderError::RateLimited {
            message: "quota".to_string(),
        };
        let json = serde_json::to_value(err).expect("serialize provider error");
        assert_eq!(json["kind"], "rate_limited");
    }

    #[test]
    fn stream_error_event_roundtrip_json() {
        let event = AssistantMessageEvent::Error(StreamErrorEvent {
            sequence_no: 44,
            code: "rate_limited".to_string(),
            message: "try later".to_string(),
            retryable: true,
        });

        let json = serde_json::to_string(&event).expect("serialize event");
        let back: AssistantMessageEvent = serde_json::from_str(&json).expect("deserialize event");
        assert_eq!(back, event);
    }

    #[test]
    fn assistant_message_roundtrip_json() {
        let message = sample_message(StopReason::Stop);
        let json = serde_json::to_string(&message).expect("serialize message");
        let back: AssistantMessage = serde_json::from_str(&json).expect("deserialize message");
        assert_eq!(back, message);
    }

    #[test]
    fn provider_context_supports_optional_api_key() {
        let ctx = ProviderContext {
            api_key: None,
            timeout_ms: 30_000,
        };

        let json = serde_json::to_string(&ctx).expect("serialize context");
        assert!(json.contains("timeout_ms"));
    }

    #[test]
    fn model_ref_roundtrip_json() {
        let model = sample_model_ref();
        let json = serde_json::to_string(&model).expect("serialize model ref");
        let back: ModelRef = serde_json::from_str(&json).expect("deserialize model ref");
        assert_eq!(back, model);
    }

    #[test]
    fn assistant_message_event_json_contains_type_discriminator() {
        let event = AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 3,
            block_id: "b".to_string(),
            delta: "abc".to_string(),
        });

        let json = serde_json::to_value(event).expect("serialize event");
        assert_eq!(json["type"], "text_delta");
    }

    #[test]
    fn computed_total_tokens_handles_zero_explicit_total() {
        let usage = TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 1,
            cache_write_tokens: 1,
            total_tokens: Some(0),
            cost_usd: None,
        };

        assert_eq!(usage.computed_total_tokens(), 0);
    }

    #[test]
    fn usage_has_any_usage_when_total_tokens_set() {
        let usage = TokenUsage {
            total_tokens: Some(123),
            ..TokenUsage::default()
        };

        assert!(usage.has_any_usage());
    }

    #[test]
    fn assistant_message_can_hold_multiple_content_blocks() {
        let message = AssistantMessage {
            message_id: "m-2".to_string(),
            model: sample_model_ref(),
            content: vec![
                AssistantContent::Thinking(ThinkingContent {
                    text: "hmm".to_string(),
                }),
                AssistantContent::ToolCall(ToolCall {
                    id: "tc".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({ "path": "src/lib.rs" }),
                }),
            ],
            usage: TokenUsage::default(),
            stop_reason: StopReason::ToolUse,
        };

        assert_eq!(message.content.len(), 2);
    }

    #[test]
    fn provider_transport_details_roundtrip_json() {
        let details = ProviderTransportDetails {
            transport: "websocket".to_string(),
            reused_provider_session: true,
        };

        let json = serde_json::to_string(&details).expect("serialize details");
        let back: ProviderTransportDetails =
            serde_json::from_str(&json).expect("deserialize details");
        assert_eq!(back, details);
    }

    #[test]
    fn provider_final_roundtrip_json() {
        let final_msg = ProviderFinal {
            message: sample_message(StopReason::Stop),
            transport_details: Some(ProviderTransportDetails {
                transport: "sse".to_string(),
                reused_provider_session: false,
            }),
        };

        let json = serde_json::to_string(&final_msg).expect("serialize final");
        let back: ProviderFinal = serde_json::from_str(&json).expect("deserialize final");
        assert_eq!(back, final_msg);
    }

    #[test]
    fn provider_request_roundtrip_json() {
        let req = ProviderRequest {
            session_id: "s-1".to_string(),
            model: sample_model_ref(),
            system_prompt: Some("You are a helpful assistant.".to_string()),
            input: vec![ProviderInputMessage::User {
                content: "hello".to_string(),
            }],
            tools: vec![],
        };

        let json = serde_json::to_string(&req).expect("serialize request");
        let back: ProviderRequest = serde_json::from_str(&json).expect("deserialize request");
        assert_eq!(back, req);
    }
}
