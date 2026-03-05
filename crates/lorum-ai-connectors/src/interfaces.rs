use async_trait::async_trait;
use lorum_ai_contract::{ProviderContext, ProviderError, ProviderRequest, StopReason, TokenUsage};

// ---------------------------------------------------------------------------
// FrameSink — incremental frame delivery from transport to adapter
// ---------------------------------------------------------------------------

pub trait FrameSink<F>: Send {
    fn push_frame(&mut self, frame: F) -> Result<(), ProviderError>;
}

pub struct CollectingFrameSink<F> {
    pub frames: Vec<F>,
}

impl<F> Default for CollectingFrameSink<F> {
    fn default() -> Self {
        Self { frames: Vec::new() }
    }
}

impl<F: Send> FrameSink<F> for CollectingFrameSink<F> {
    fn push_frame(&mut self, frame: F) -> Result<(), ProviderError> {
        self.frames.push(frame);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Frame types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum AnthropicFrame {
    MessageStart {
        message_id: String,
    },
    TextStart {
        block_id: String,
    },
    TextDelta {
        block_id: String,
        delta: String,
    },
    TextEnd {
        block_id: String,
    },
    ThinkingStart {
        block_id: String,
    },
    ThinkingDelta {
        block_id: String,
        delta: String,
    },
    ThinkingEnd {
        block_id: String,
    },
    ToolCallStart {
        block_id: String,
        call_id: String,
        name: String,
    },
    ToolCallDelta {
        block_id: String,
        delta: String,
    },
    ToolCallEnd {
        block_id: String,
    },
    MessageDone {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum OpenAiResponsesFrame {
    ResponseStart {
        message_id: String,
    },
    TextStart {
        block_id: String,
    },
    TextDelta {
        block_id: String,
        delta: String,
    },
    TextEnd {
        block_id: String,
    },
    ReasoningStart {
        block_id: String,
    },
    ReasoningDelta {
        block_id: String,
        delta: String,
    },
    ReasoningEnd {
        block_id: String,
    },
    FunctionCallStart {
        block_id: String,
        call_id: String,
        name: String,
    },
    FunctionCallDelta {
        block_id: String,
        delta: String,
    },
    FunctionCallEnd {
        block_id: String,
    },
    Completed {
        stop_reason: StopReason,
        usage: TokenUsage,
    },
    Error {
        code: String,
        message: String,
        retryable: bool,
    },
}

// ---------------------------------------------------------------------------
// Transport traits — push frames incrementally via FrameSink
// ---------------------------------------------------------------------------

#[async_trait]
pub trait AnthropicTransport: Send + Sync {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        sink: &mut dyn FrameSink<AnthropicFrame>,
    ) -> Result<(), ProviderError>;
}

#[async_trait]
pub trait OpenAiResponsesTransport: Send + Sync {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<(), ProviderError>;
}

#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: usize,
}

impl RetryPolicy {
    pub fn new(max_attempts: usize) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
        }
    }

    pub fn should_retry(&self, attempt: usize, error: &ProviderError) -> bool {
        attempt < self.max_attempts
            && matches!(
                error,
                ProviderError::RateLimited { .. } | ProviderError::Transport { .. }
            )
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self { max_attempts: 2 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexTransportMode {
    WebSocket,
    Sse,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderSessionState {
    pub provider_session_id: Option<String>,
    pub websocket_disabled: bool,
}

#[async_trait]
pub trait ProviderSessionStateStore: Send + Sync {
    async fn get(
        &self,
        session_id: &str,
        provider_id: &str,
    ) -> Result<Option<ProviderSessionState>, ProviderError>;

    async fn set(
        &self,
        session_id: &str,
        provider_id: &str,
        state: ProviderSessionState,
    ) -> Result<(), ProviderError>;

    async fn clear(&self, session_id: &str, provider_id: &str) -> Result<(), ProviderError>;
}

#[derive(Debug, Clone, PartialEq)]
pub struct CodexTransportMeta {
    pub provider_session_id: Option<String>,
    pub reused_provider_session: bool,
}

#[async_trait]
pub trait CodexWebSocketTransport: Send + Sync {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        state: Option<ProviderSessionState>,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError>;
}

#[async_trait]
pub trait CodexSseTransport: Send + Sync {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        state: Option<ProviderSessionState>,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError>;
}
