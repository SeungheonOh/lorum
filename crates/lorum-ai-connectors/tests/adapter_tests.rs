#![allow(clippy::type_complexity)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_connectors::{
    AnthropicAdapter, AnthropicFrame, AnthropicTransport, CodexSseTransport, CodexTransportMeta,
    CodexWebSocketTransport, FrameSink, InMemoryProviderSessionStateStore,
    OpenAiCodexResponsesAdapter, OpenAiResponsesAdapter, OpenAiResponsesFrame,
    OpenAiResponsesTransport, ProviderSessionState, ProviderSessionStateStore, RetryPolicy,
    ToolCallJsonAccumulator, coalesce_delta_events,
};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessageEvent, ModelRef,
    ProviderAdapter, ProviderContext, ProviderError, ProviderInputMessage, ProviderRequest,
    StopReason, StreamBoundaryEvent, StreamTextDelta, TokenUsage,
};

struct MockAnthropicTransport {
    responses: Mutex<VecDeque<Result<Vec<AnthropicFrame>, ProviderError>>>,
}

#[async_trait]
impl AnthropicTransport for MockAnthropicTransport {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        sink: &mut dyn FrameSink<AnthropicFrame>,
    ) -> Result<(), ProviderError> {
        let frames = self
            .responses
            .lock()
            .expect("lock anthropic responses")
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::Transport {
                    message: "missing mock response".to_string(),
                })
            })?;
        for frame in frames {
            sink.push_frame(frame)?;
        }
        Ok(())
    }
}

struct MockOpenAiTransport {
    responses: Mutex<VecDeque<Result<Vec<OpenAiResponsesFrame>, ProviderError>>>,
}

#[async_trait]
impl OpenAiResponsesTransport for MockOpenAiTransport {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<(), ProviderError> {
        let frames = self
            .responses
            .lock()
            .expect("lock openai responses")
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::Transport {
                    message: "missing mock response".to_string(),
                })
            })?;
        for frame in frames {
            sink.push_frame(frame)?;
        }
        Ok(())
    }
}

struct MockCodexWsTransport {
    responses:
        Mutex<VecDeque<Result<(Vec<OpenAiResponsesFrame>, CodexTransportMeta), ProviderError>>>,
    observed_states: Mutex<Vec<Option<ProviderSessionState>>>,
}

#[async_trait]
impl CodexWebSocketTransport for MockCodexWsTransport {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        state: Option<ProviderSessionState>,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError> {
        self.observed_states
            .lock()
            .expect("lock ws observed states")
            .push(state);
        let (frames, meta) = self
            .responses
            .lock()
            .expect("lock ws responses")
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::Transport {
                    message: "missing ws response".to_string(),
                })
            })?;
        for frame in frames {
            sink.push_frame(frame)?;
        }
        Ok(meta)
    }
}

struct MockCodexSseTransport {
    responses:
        Mutex<VecDeque<Result<(Vec<OpenAiResponsesFrame>, CodexTransportMeta), ProviderError>>>,
    observed_states: Mutex<Vec<Option<ProviderSessionState>>>,
}

#[async_trait]
impl CodexSseTransport for MockCodexSseTransport {
    async fn stream_frames(
        &self,
        _request: &ProviderRequest,
        _context: &ProviderContext,
        state: Option<ProviderSessionState>,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError> {
        self.observed_states
            .lock()
            .expect("lock sse observed states")
            .push(state);
        let (frames, meta) = self
            .responses
            .lock()
            .expect("lock sse responses")
            .pop_front()
            .unwrap_or_else(|| {
                Err(ProviderError::Transport {
                    message: "missing sse response".to_string(),
                })
            })?;
        for frame in frames {
            sink.push_frame(frame)?;
        }
        Ok(meta)
    }
}

fn codex_ok_frames(message_id: &str) -> Vec<OpenAiResponsesFrame> {
    vec![
        OpenAiResponsesFrame::ResponseStart {
            message_id: message_id.to_string(),
        },
        OpenAiResponsesFrame::Completed {
            stop_reason: StopReason::Stop,
            usage: TokenUsage::default(),
        },
    ]
}

fn codex_ok_result(
    message_id: &str,
    session_id: Option<&str>,
    reused: bool,
) -> (Vec<OpenAiResponsesFrame>, CodexTransportMeta) {
    (
        codex_ok_frames(message_id),
        CodexTransportMeta {
            provider_session_id: session_id.map(ToString::to_string),
            reused_provider_session: reused,
        },
    )
}

#[derive(Default)]
struct RecordingSink {
    events: Vec<AssistantMessageEvent>,
}

impl AssistantEventSink for RecordingSink {
    fn push(
        &mut self,
        event: AssistantMessageEvent,
    ) -> Result<(), lorum_ai_contract::StreamSinkError> {
        self.events.push(event);
        Ok(())
    }
}

fn sample_request() -> ProviderRequest {
    ProviderRequest {
        session_id: "session-1".to_string(),
        model: ModelRef {
            provider: "openai".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "gpt-5.2".to_string(),
        },
        system_prompt: None,
        input: vec![ProviderInputMessage::User {
            content: "hello".to_string(),
        }],
        tools: vec![],
        tool_choice: None,
    }
}

fn sample_context() -> ProviderContext {
    ProviderContext {
        api_key: Some("key".to_string()),
        timeout_ms: 30_000,
    }
}

#[test]
fn json_accumulator_parses_chunked_json() {
    let mut accumulator = ToolCallJsonAccumulator::default();
    assert!(accumulator.push_chunk("{\"path\"").is_none());
    let parsed = accumulator
        .push_chunk(":\"src\"}")
        .expect("json should parse");
    assert_eq!(parsed["path"], "src");
}

#[test]
fn coalescer_merges_text_deltas() {
    let events = vec![
        AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 1,
            block_id: "b1".to_string(),
            delta: "a".to_string(),
        }),
        AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 2,
            block_id: "b1".to_string(),
            delta: "b".to_string(),
        }),
    ];

    let merged = coalesce_delta_events(events);
    assert_eq!(merged.len(), 1);
    match &merged[0] {
        AssistantMessageEvent::TextDelta(delta) => {
            assert_eq!(delta.delta, "ab");
            assert_eq!(delta.sequence_no, 2);
        }
        _ => panic!("expected text delta"),
    }
}

#[test]
fn anthropic_adapter_streams_full_lifecycle() {
    block_on(async {
        let transport = Arc::new(MockAnthropicTransport {
            responses: Mutex::new(VecDeque::from([Ok(vec![
                AnthropicFrame::MessageStart {
                    message_id: "m1".to_string(),
                },
                AnthropicFrame::TextStart {
                    block_id: "b1".to_string(),
                },
                AnthropicFrame::TextDelta {
                    block_id: "b1".to_string(),
                    delta: "hello".to_string(),
                },
                AnthropicFrame::TextEnd {
                    block_id: "b1".to_string(),
                },
                AnthropicFrame::MessageDone {
                    stop_reason: StopReason::Stop,
                    usage: TokenUsage {
                        input_tokens: 1,
                        output_tokens: 2,
                        ..TokenUsage::default()
                    },
                },
            ])])),
        });

        let adapter = AnthropicAdapter::new(transport);
        let mut sink = RecordingSink::default();
        let final_msg = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await
            .expect("stream must succeed");

        assert_eq!(sink.events.len(), 5);
        assert_eq!(final_msg.message.stop_reason, StopReason::Stop);
        assert_eq!(final_msg.message.content.len(), 1);
        assert_eq!(final_msg.message.usage.output_tokens, 2);
    });
}

#[test]
fn anthropic_adapter_retries_retryable_error() {
    block_on(async {
        let transport = Arc::new(MockAnthropicTransport {
            responses: Mutex::new(VecDeque::from([
                Err(ProviderError::Transport {
                    message: "timeout".to_string(),
                }),
                Ok(vec![
                    AnthropicFrame::MessageStart {
                        message_id: "m1".to_string(),
                    },
                    AnthropicFrame::MessageDone {
                        stop_reason: StopReason::Stop,
                        usage: TokenUsage::default(),
                    },
                ]),
            ])),
        });

        let adapter = AnthropicAdapter::new(transport).with_retry_policy(RetryPolicy::new(2));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await;
        assert!(result.is_ok());
    });
}

#[test]
fn anthropic_adapter_does_not_retry_non_retryable_error() {
    block_on(async {
        let transport = Arc::new(MockAnthropicTransport {
            responses: Mutex::new(VecDeque::from([Err(ProviderError::Auth {
                message: "bad key".to_string(),
            })])),
        });

        let adapter = AnthropicAdapter::new(transport).with_retry_policy(RetryPolicy::new(3));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await;

        assert!(matches!(result, Err(ProviderError::Auth { .. })));
    });
}

#[test]
fn openai_adapter_emits_tool_call_content() {
    block_on(async {
        let transport = Arc::new(MockOpenAiTransport {
            responses: Mutex::new(VecDeque::from([Ok(vec![
                OpenAiResponsesFrame::ResponseStart {
                    message_id: "m1".to_string(),
                },
                OpenAiResponsesFrame::FunctionCallStart {
                    block_id: "call".to_string(),
                    call_id: "tc1".to_string(),
                    name: "read".to_string(),
                },
                OpenAiResponsesFrame::FunctionCallDelta {
                    block_id: "call".to_string(),
                    delta: "{\"path\":\"src\"}".to_string(),
                },
                OpenAiResponsesFrame::FunctionCallEnd {
                    block_id: "call".to_string(),
                },
                OpenAiResponsesFrame::Completed {
                    stop_reason: StopReason::ToolUse,
                    usage: TokenUsage::default(),
                },
            ])])),
        });

        let adapter = OpenAiResponsesAdapter::new(transport);
        let mut sink = RecordingSink::default();
        let final_msg = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await
            .expect("stream must succeed");

        assert_eq!(final_msg.message.stop_reason, StopReason::ToolUse);
        assert!(final_msg
            .message
            .content
            .iter()
            .any(|block| matches!(block, AssistantContent::ToolCall(_))));
    });
}

#[test]
fn openai_complete_returns_message() {
    block_on(async {
        let transport = Arc::new(MockOpenAiTransport {
            responses: Mutex::new(VecDeque::from([Ok(vec![
                OpenAiResponsesFrame::ResponseStart {
                    message_id: "m1".to_string(),
                },
                OpenAiResponsesFrame::TextStart {
                    block_id: "b1".to_string(),
                },
                OpenAiResponsesFrame::TextDelta {
                    block_id: "b1".to_string(),
                    delta: "ok".to_string(),
                },
                OpenAiResponsesFrame::TextEnd {
                    block_id: "b1".to_string(),
                },
                OpenAiResponsesFrame::Completed {
                    stop_reason: StopReason::Stop,
                    usage: TokenUsage::default(),
                },
            ])])),
        });

        let adapter = OpenAiResponsesAdapter::new(transport);
        let message = adapter
            .complete(sample_request(), sample_context())
            .await
            .expect("complete must succeed");
        assert_eq!(message.message_id, "m1");
    });
}

#[test]
fn openai_error_frame_maps_to_provider_error() {
    block_on(async {
        let transport = Arc::new(MockOpenAiTransport {
            responses: Mutex::new(VecDeque::from([Ok(vec![
                OpenAiResponsesFrame::ResponseStart {
                    message_id: "m1".to_string(),
                },
                OpenAiResponsesFrame::Error {
                    code: "rate_limit".to_string(),
                    message: "too many".to_string(),
                    retryable: true,
                },
            ])])),
        });

        let adapter = OpenAiResponsesAdapter::new(transport);
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await;

        assert!(matches!(result, Err(ProviderError::RateLimited { .. })));
        assert!(sink
            .events
            .iter()
            .any(|event| matches!(event, AssistantMessageEvent::Error(_))));
    });
}

#[test]
fn codex_prefers_websocket_and_persists_session_state() {
    block_on(async {
        let ws = Arc::new(MockCodexWsTransport {
            responses: Mutex::new(VecDeque::from([Ok(codex_ok_result(
                "c1",
                Some("ps1"),
                false,
            ))])),
            observed_states: Mutex::new(Vec::new()),
        });
        let sse = Arc::new(MockCodexSseTransport {
            responses: Mutex::new(VecDeque::from([Ok(codex_ok_result(
                "unused",
                Some("sse"),
                false,
            ))])),
            observed_states: Mutex::new(Vec::new()),
        });
        let store = Arc::new(InMemoryProviderSessionStateStore::default());

        let adapter = OpenAiCodexResponsesAdapter::new(Some(ws.clone()), sse, store.clone())
            .with_retry_policy(RetryPolicy::new(1));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await
            .expect("codex stream");

        assert_eq!(
            result
                .transport_details
                .expect("transport details")
                .transport,
            "websocket"
        );

        let state = store
            .get("session-1", "openai-codex-responses")
            .await
            .expect("read state")
            .expect("state exists");
        assert_eq!(state.provider_session_id.as_deref(), Some("ps1"));
        assert!(!state.websocket_disabled);
        assert_eq!(ws.observed_states.lock().expect("obs").len(), 1);
    });
}

#[test]
fn codex_falls_back_to_sse_on_websocket_failure() {
    block_on(async {
        let ws = Arc::new(MockCodexWsTransport {
            responses: Mutex::new(VecDeque::from([Err(ProviderError::Transport {
                message: "ws down".to_string(),
            })])),
            observed_states: Mutex::new(Vec::new()),
        });
        let sse = Arc::new(MockCodexSseTransport {
            responses: Mutex::new(VecDeque::from([Ok(codex_ok_result(
                "c2",
                Some("ps2"),
                false,
            ))])),
            observed_states: Mutex::new(Vec::new()),
        });
        let store = Arc::new(InMemoryProviderSessionStateStore::default());

        let adapter = OpenAiCodexResponsesAdapter::new(Some(ws), sse, store.clone())
            .with_retry_policy(RetryPolicy::new(1));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await
            .expect("codex stream");

        assert_eq!(
            result
                .transport_details
                .expect("transport details")
                .transport,
            "sse"
        );

        let state = store
            .get("session-1", "openai-codex-responses")
            .await
            .expect("read state")
            .expect("state exists");
        assert!(state.websocket_disabled);
        assert_eq!(state.provider_session_id.as_deref(), Some("ps2"));
    });
}

#[test]
fn codex_skips_websocket_when_state_marks_disabled() {
    block_on(async {
        let ws = Arc::new(MockCodexWsTransport {
            responses: Mutex::new(VecDeque::from([Ok(codex_ok_result(
                "unused",
                Some("ws"),
                false,
            ))])),
            observed_states: Mutex::new(Vec::new()),
        });
        let sse = Arc::new(MockCodexSseTransport {
            responses: Mutex::new(VecDeque::from([Ok(codex_ok_result(
                "c3",
                Some("ps3"),
                true,
            ))])),
            observed_states: Mutex::new(Vec::new()),
        });
        let store = Arc::new(InMemoryProviderSessionStateStore::default());
        store
            .set(
                "session-1",
                "openai-codex-responses",
                ProviderSessionState {
                    provider_session_id: Some("existing".to_string()),
                    websocket_disabled: true,
                },
            )
            .await
            .expect("seed state");

        let adapter = OpenAiCodexResponsesAdapter::new(Some(ws.clone()), sse.clone(), store)
            .with_retry_policy(RetryPolicy::new(1));
        let mut sink = RecordingSink::default();
        let result = adapter
            .stream(sample_request(), sample_context(), &mut sink)
            .await
            .expect("codex stream");

        assert_eq!(
            result
                .transport_details
                .expect("transport details")
                .transport,
            "sse"
        );
        assert_eq!(ws.observed_states.lock().expect("obs").len(), 0);
        assert_eq!(sse.observed_states.lock().expect("obs").len(), 1);
    });
}

#[test]
fn codex_reuses_provider_session_state_on_second_call() {
    block_on(async {
        let ws = Arc::new(MockCodexWsTransport {
            responses: Mutex::new(VecDeque::from([
                Ok(codex_ok_result("first", Some("ps1"), false)),
                Ok(codex_ok_result("second", Some("ps1"), true)),
            ])),
            observed_states: Mutex::new(Vec::new()),
        });
        let sse = Arc::new(MockCodexSseTransport {
            responses: Mutex::new(VecDeque::new()),
            observed_states: Mutex::new(Vec::new()),
        });
        let store = Arc::new(InMemoryProviderSessionStateStore::default());

        let adapter = OpenAiCodexResponsesAdapter::new(Some(ws.clone()), sse, store)
            .with_retry_policy(RetryPolicy::new(1));
        let mut sink1 = RecordingSink::default();
        adapter
            .stream(sample_request(), sample_context(), &mut sink1)
            .await
            .expect("first stream");

        let mut sink2 = RecordingSink::default();
        let second = adapter
            .stream(sample_request(), sample_context(), &mut sink2)
            .await
            .expect("second stream");

        assert!(
            second
                .transport_details
                .expect("transport details")
                .reused_provider_session
        );

        let observed = ws.observed_states.lock().expect("observed states");
        assert_eq!(observed.len(), 2);
        assert!(observed[0].is_none());
        assert_eq!(
            observed[1]
                .as_ref()
                .and_then(|state| state.provider_session_id.as_deref()),
            Some("ps1")
        );
    });
}

#[test]
fn coalescer_keeps_non_consecutive_deltas_separate() {
    let events = vec![
        AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 1,
            block_id: "b1".to_string(),
            delta: "a".to_string(),
        }),
        AssistantMessageEvent::TextEnd(StreamBoundaryEvent {
            sequence_no: 2,
            block_id: "b1".to_string(),
        }),
        AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 3,
            block_id: "b1".to_string(),
            delta: "b".to_string(),
        }),
    ];

    let merged = coalesce_delta_events(events);
    assert_eq!(merged.len(), 3);
}
