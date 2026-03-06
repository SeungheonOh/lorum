use std::collections::HashSet;
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolChoice {
    Auto,
    Required,
    Specific { name: String },
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
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

/// Scan `messages` for assistant tool calls that have no corresponding `ToolResult`
/// and inject a synthetic error result immediately after each orphaned assistant message.
pub fn patch_orphaned_tool_calls(messages: &mut Vec<ProviderInputMessage>, reason: &str) {
    // 1. Collect all tool_call_ids that already have a ToolResult
    let matched_ids: HashSet<String> = messages
        .iter()
        .filter_map(|m| match m {
            ProviderInputMessage::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect();

    // 2. Walk messages, find orphaned tool call IDs and their insertion points
    let mut insertions: Vec<(usize, Vec<ProviderInputMessage>)> = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if let ProviderInputMessage::Assistant { message } = msg {
            let orphans: Vec<&ToolCall> = message
                .content
                .iter()
                .filter_map(|c| match c {
                    AssistantContent::ToolCall(tc) if !matched_ids.contains(&tc.id) => Some(tc),
                    _ => None,
                })
                .collect();

            if !orphans.is_empty() {
                let synthetics: Vec<ProviderInputMessage> = orphans
                    .into_iter()
                    .map(|tc| ProviderInputMessage::ToolResult {
                        tool_call_id: tc.id.clone(),
                        is_error: true,
                        result: Value::String(reason.to_string()),
                    })
                    .collect();
                insertions.push((idx + 1, synthetics));
            }
        }
    }

    // 3. Insert in reverse order to keep indices stable
    for (insert_at, synthetics) in insertions.into_iter().rev() {
        for (offset, synthetic) in synthetics.into_iter().enumerate() {
            messages.insert(insert_at + offset, synthetic);
        }
    }
}
