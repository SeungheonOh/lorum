use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_connectors::{
    AnthropicAdapter, AnthropicFrame, AnthropicTransport, CodexSseTransport, CodexTransportMeta,
    CodexWebSocketTransport, FrameSink, InMemoryProviderSessionStateStore,
    OpenAiCodexResponsesAdapter, OpenAiResponsesAdapter, OpenAiResponsesFrame,
    OpenAiResponsesTransport, ProviderSessionStateStore, RetryPolicy,
};
use lorum_ai_contract::{
    ApiKind, AssistantEventSink, AssistantMessageEvent, ModelRef, ProviderAdapter, ProviderContext,
    ProviderError, ProviderInputMessage, ProviderRequest, StopReason, StreamSinkError, TokenUsage,
};

fn smoke_enabled() -> bool {
    matches!(std::env::var("OMP_LIVE_SMOKE").as_deref(), Ok("1"))
}

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("missing required env var: {name}"))
}

fn sample_request(api: ApiKind, provider: &str, model: &str) -> ProviderRequest {
    ProviderRequest {
        session_id: "smoke-session".to_string(),
        model: ModelRef {
            provider: provider.to_string(),
            api,
            model: model.to_string(),
        },
        system_prompt: None,
        input: vec![ProviderInputMessage::User {
            content: "ping".to_string(),
        }],
        tools: vec![],
        tool_choice: None,
    }
}

fn sample_context(api_key: String) -> ProviderContext {
    ProviderContext {
        api_key: Some(api_key),
        timeout_ms: 30_000,
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Vec<AssistantMessageEvent>,
}

impl AssistantEventSink for RecordingSink {
    fn push(&mut self, event: AssistantMessageEvent) -> Result<(), StreamSinkError> {
        self.events.push(event);
        Ok(())
    }
}

struct StaticAnthropicTransport {
    frames: Vec<AnthropicFrame>,
}

#[async_trait]
impl AnthropicTransport for StaticAnthropicTransport {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        sink: &mut dyn FrameSink<AnthropicFrame>,
    ) -> Result<(), ProviderError> {
        for frame in &self.frames {
            sink.push_frame(frame.clone())?;
        }
        Ok(())
    }
}

struct StaticOpenAiTransport {
    frames: Vec<OpenAiResponsesFrame>,
}

#[async_trait]
impl OpenAiResponsesTransport for StaticOpenAiTransport {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<(), ProviderError> {
        for frame in &self.frames {
            sink.push_frame(frame.clone())?;
        }
        Ok(())
    }
}

struct StaticCodexWs {
    responses: Mutex<VecDeque<Result<(Vec<OpenAiResponsesFrame>, CodexTransportMeta), ProviderError>>>,
}

#[async_trait]
impl CodexWebSocketTransport for StaticCodexWs {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        _state: Option<lorum_ai_connectors::ProviderSessionState>,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError> {
        let (frames, meta) = self
            .responses
            .lock()
            .expect("lock ws responses")
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::Transport {
                    message: "no ws response".to_string(),
                })
            })?;
        for frame in frames {
            sink.push_frame(frame)?;
        }
        Ok(meta)
    }
}

struct StaticCodexSse;

#[async_trait]
impl CodexSseTransport for StaticCodexSse {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        _state: Option<lorum_ai_connectors::ProviderSessionState>,
        _sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError> {
        Err(ProviderError::Transport {
            message: "sse fallback not expected in smoke scaffold".to_string(),
        })
    }
}

#[test]
#[ignore = "requires OMP_LIVE_SMOKE=1 and provider secrets"]
fn anthropic_live_smoke_scaffold() {
    if !smoke_enabled() {
        return;
    }

    block_on(async {
        let api_key = require_env("OMP_SMOKE_ANTHROPIC_API_KEY");
        let transport = Arc::new(StaticAnthropicTransport {
            frames: vec![
                AnthropicFrame::MessageStart {
                    message_id: "anthropic-smoke".to_string(),
                },
                AnthropicFrame::MessageDone {
                    stop_reason: StopReason::Stop,
                    usage: TokenUsage::default(),
                },
            ],
        });

        let adapter = AnthropicAdapter::new(transport).with_retry_policy(RetryPolicy::new(1));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(
                sample_request(
                    ApiKind::AnthropicMessages,
                    "anthropic",
                    "claude-sonnet-4-20250514",
                ),
                sample_context(api_key),
                &mut sink,
            )
            .await
            .expect("anthropic smoke scaffold stream");

        assert_eq!(result.message.stop_reason, StopReason::Stop);
    });
}

#[test]
#[ignore = "requires OMP_LIVE_SMOKE=1 and provider secrets"]
fn openai_responses_live_smoke_scaffold() {
    if !smoke_enabled() {
        return;
    }

    block_on(async {
        let api_key = require_env("OMP_SMOKE_OPENAI_API_KEY");
        let transport = Arc::new(StaticOpenAiTransport {
            frames: vec![
                OpenAiResponsesFrame::ResponseStart {
                    message_id: "openai-smoke".to_string(),
                },
                OpenAiResponsesFrame::Completed {
                    stop_reason: StopReason::Stop,
                    usage: TokenUsage::default(),
                },
            ],
        });

        let adapter = OpenAiResponsesAdapter::new(transport).with_retry_policy(RetryPolicy::new(1));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(
                sample_request(ApiKind::OpenAiResponses, "openai", "gpt-5.2"),
                sample_context(api_key),
                &mut sink,
            )
            .await
            .expect("openai responses smoke scaffold stream");

        assert_eq!(result.message.stop_reason, StopReason::Stop);
    });
}

#[test]
#[ignore = "requires OMP_LIVE_SMOKE=1 and provider secrets"]
fn codex_live_smoke_scaffold() {
    if !smoke_enabled() {
        return;
    }

    block_on(async {
        let api_key = require_env("OMP_SMOKE_CODEX_ACCESS_TOKEN");
        let ws = Arc::new(StaticCodexWs {
            responses: Mutex::new(VecDeque::from([Ok((
                vec![
                    OpenAiResponsesFrame::ResponseStart {
                        message_id: "codex-smoke".to_string(),
                    },
                    OpenAiResponsesFrame::Completed {
                        stop_reason: StopReason::Stop,
                        usage: TokenUsage::default(),
                    },
                ],
                CodexTransportMeta {
                    provider_session_id: Some("provider-session-1".to_string()),
                    reused_provider_session: false,
                },
            ))])),
        });
        let sse = Arc::new(StaticCodexSse);
        let state_store = Arc::new(InMemoryProviderSessionStateStore::default());

        let adapter = OpenAiCodexResponsesAdapter::new(Some(ws), sse, state_store.clone())
            .with_retry_policy(RetryPolicy::new(1));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(
                sample_request(ApiKind::OpenAiCodexResponses, "openai", "codex-mini-latest"),
                sample_context(api_key),
                &mut sink,
            )
            .await
            .expect("codex smoke scaffold stream");

        assert_eq!(result.message.stop_reason, StopReason::Stop);
        let state = state_store
            .get("smoke-session", "openai-codex-responses")
            .await
            .expect("load session state")
            .expect("session state exists");
        assert_eq!(
            state.provider_session_id.as_deref(),
            Some("provider-session-1")
        );
    });
}
