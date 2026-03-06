use async_trait::async_trait;
use futures::executor::block_on;
use lorum_agent_core::{ChatTurnEngine, RuntimeEventSink, TurnEngine, TurnError, TurnRequest};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent,
    ModelRef, ProviderAdapter, ProviderContext, ProviderError, ProviderFinal, ProviderInputMessage,
    ProviderRequest, StopReason, StreamDoneEvent, StreamErrorEvent, StreamTextDelta,
    StreamThinkingDelta, StreamToolCallDelta, TextContent, TokenUsage,
};
use lorum_domain::{validate_turn_event_order, MessageId, RuntimeEvent, SessionId, TurnId, TurnTerminalReason};

#[derive(Default)]
struct RecordingRuntimeSink {
    events: Vec<RuntimeEvent>,
}

impl RuntimeEventSink for RecordingRuntimeSink {
    fn push(&mut self, event: RuntimeEvent) -> Result<(), TurnError> {
        self.events.push(event);
        Ok(())
    }
}

enum MockBehavior {
    Success { stop_reason: StopReason },
    SuccessWithoutDeltas { stop_reason: StopReason },
    Error,
}

struct MockProvider {
    behavior: MockBehavior,
}

#[async_trait]
impl ProviderAdapter for MockProvider {
    fn provider_id(&self) -> &str {
        "mock"
    }

    fn api_kind(&self) -> ApiKind {
        ApiKind::OpenAiResponses
    }

    async fn stream(
        &self,
        _request: ProviderRequest,
        _context: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError> {
        let emits_stream_deltas =
            !matches!(self.behavior, MockBehavior::SuccessWithoutDeltas { .. });
        if emits_stream_deltas {
            sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
                sequence_no: 1,
                block_id: "text-1".to_string(),
                delta: "hello".to_string(),
            }))
            .map_err(|err| ProviderError::Transport {
                message: format!("sink rejected text delta: {err}"),
            })?;

            sink.push(AssistantMessageEvent::ThinkingDelta(StreamThinkingDelta {
                sequence_no: 2,
                block_id: "thinking-1".to_string(),
                delta: "...".to_string(),
            }))
            .map_err(|err| ProviderError::Transport {
                message: format!("sink rejected thinking delta: {err}"),
            })?;

            sink.push(AssistantMessageEvent::ToolCallDelta(StreamToolCallDelta {
                sequence_no: 3,
                block_id: "tool-1".to_string(),
                delta: "{}".to_string(),
            }))
            .map_err(|err| ProviderError::Transport {
                message: format!("sink rejected tool delta: {err}"),
            })?;
        }
        match self.behavior {
            MockBehavior::Success { stop_reason }
            | MockBehavior::SuccessWithoutDeltas { stop_reason } => {
                let message = AssistantMessage {
                    message_id: "msg-123".to_string(),
                    model: sample_model(),
                    content: vec![AssistantContent::Text(TextContent {
                        text: "hello".to_string(),
                    })],
                    usage: TokenUsage::default(),
                    stop_reason,
                };

                sink.push(AssistantMessageEvent::Done(StreamDoneEvent {
                    sequence_no: 4,
                    message: message.clone(),
                }))
                .map_err(|err| ProviderError::Transport {
                    message: format!("sink rejected done event: {err}"),
                })?;

                Ok(ProviderFinal {
                    message,
                    transport_details: None,
                })
            }
            MockBehavior::Error => {
                sink.push(AssistantMessageEvent::Error(StreamErrorEvent {
                    sequence_no: 4,
                    code: "provider_transport".to_string(),
                    message: "network timeout".to_string(),
                    retryable: true,
                }))
                .map_err(|err| ProviderError::Transport {
                    message: format!("sink rejected error event: {err}"),
                })?;

                Err(ProviderError::Transport {
                    message: "network timeout".to_string(),
                })
            }
        }
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        Err(ProviderError::InvalidResponse {
            message: "not implemented in test mock".to_string(),
        })
    }
}

fn sample_model() -> ModelRef {
    ModelRef {
        provider: "mock".to_string(),
        api: ApiKind::OpenAiResponses,
        model: "test-model".to_string(),
    }
}

fn sample_turn_request() -> TurnRequest {
    TurnRequest {
        session_id: SessionId::from("session-1"),
        turn_id: TurnId::from("turn-1"),
        provider_request: ProviderRequest {
            session_id: "session-1".to_string(),
            model: sample_model(),
            system_prompt: None,
            input: vec![ProviderInputMessage::User {
                content: "hello".to_string(),
            }],
            tools: vec![],
            tool_choice: None,
        },
        provider_context: ProviderContext {
            api_key: None,
            timeout_ms: 30_000,
        },
        cancellation_token: None,
        starting_sequence_no: 1,
    }
}

#[test]
fn normal_stream_produces_start_delta_finish() {
    block_on(async {
        let provider = MockProvider {
            behavior: MockBehavior::Success {
                stop_reason: StopReason::Stop,
            },
        };
        let engine = ChatTurnEngine::new(provider);
        let mut sink = RecordingRuntimeSink::default();

        let result = engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect("turn should succeed");

        assert_eq!(result.turn_id, TurnId::from("turn-1"));
        assert_eq!(result.message_id, Some(MessageId::from("msg-123")));
        assert_eq!(result.terminal_reason, TurnTerminalReason::Done);
        assert_eq!(result.event_count, 4);

        assert!(matches!(sink.events[0], RuntimeEvent::TurnStarted { .. }));
        assert!(matches!(
            sink.events[1],
            RuntimeEvent::AssistantStreamDelta { .. }
        ));
        assert!(matches!(
            sink.events[2],
            RuntimeEvent::AssistantThinkingDelta { .. }
        ));
        assert!(matches!(sink.events[3], RuntimeEvent::TurnFinished { .. }));

        let sequence: Vec<u64> = sink.events.iter().map(RuntimeEvent::sequence_no).collect();
        assert_eq!(sequence, vec![1, 2, 3, 4]);
        assert_eq!(validate_turn_event_order(&sink.events), Ok(()));
    });
}

#[test]
fn done_event_with_message_text_emits_fallback_runtime_delta() {
    block_on(async {
        let provider = MockProvider {
            behavior: MockBehavior::SuccessWithoutDeltas {
                stop_reason: StopReason::Stop,
            },
        };
        let engine = ChatTurnEngine::new(provider);
        let mut sink = RecordingRuntimeSink::default();

        let result = engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect("turn should succeed");

        assert_eq!(result.event_count, 3);
        assert!(matches!(sink.events[0], RuntimeEvent::TurnStarted { .. }));
        assert!(matches!(
            sink.events[1],
            RuntimeEvent::AssistantStreamDelta { .. }
        ));
        assert!(matches!(sink.events[2], RuntimeEvent::TurnFinished { .. }));

        match &sink.events[1] {
            RuntimeEvent::AssistantStreamDelta { delta, .. } => assert_eq!(delta, "hello"),
            _ => panic!("second event should be assistant stream delta"),
        }
        assert_eq!(validate_turn_event_order(&sink.events), Ok(()));
    });
}

#[test]
fn provider_error_emits_runtime_error_terminal_path() {
    block_on(async {
        let provider = MockProvider {
            behavior: MockBehavior::Error,
        };
        let engine = ChatTurnEngine::new(provider);
        let mut sink = RecordingRuntimeSink::default();

        let err = engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect_err("turn should fail");

        assert!(matches!(
            err,
            TurnError::ProviderFailure(ProviderError::Transport { .. })
        ));
        assert!(matches!(sink.events[0], RuntimeEvent::TurnStarted { .. }));
        assert!(matches!(
            sink.events.last(),
            Some(RuntimeEvent::RuntimeError { .. })
        ));
        assert_eq!(validate_turn_event_order(&sink.events), Ok(()));
    });
}

#[test]
fn sequence_is_monotonic_and_stop_reason_mapping_is_deterministic() {
    block_on(async {
        let provider = MockProvider {
            behavior: MockBehavior::Success {
                stop_reason: StopReason::Aborted,
            },
        };
        let engine = ChatTurnEngine::new(provider);
        let mut sink = RecordingRuntimeSink::default();

        let result = engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect("turn should succeed");

        assert_eq!(result.terminal_reason, TurnTerminalReason::Aborted);
        assert_eq!(sink.events.len(), 4);

        for window in sink.events.windows(2) {
            assert!(window[1].sequence_no() > window[0].sequence_no());
        }

        assert_eq!(validate_turn_event_order(&sink.events), Ok(()));
    });
}

#[test]
fn maps_stop_reason_error_to_error_terminal_reason() {
    block_on(async {
        let provider = MockProvider {
            behavior: MockBehavior::Success {
                stop_reason: StopReason::Error,
            },
        };
        let engine = ChatTurnEngine::new(provider);
        let mut sink = RecordingRuntimeSink::default();

        let result = engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect("turn should succeed");

        assert_eq!(result.terminal_reason, TurnTerminalReason::Error);
        assert!(matches!(
            sink.events.last(),
            Some(RuntimeEvent::TurnFinished {
                reason: TurnTerminalReason::Error,
                ..
            })
        ));
    });
}
