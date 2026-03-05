use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_agent_core::{ChatTurnEngine, RuntimeEventSink, TurnEngine, TurnError, TurnRequest};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent,
    ModelRef, ProviderAdapter, ProviderContext, ProviderError, ProviderFinal, ProviderInputMessage,
    ProviderRequest, StopReason, StreamTextDelta, TextContent, TokenUsage,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId};

#[derive(Default)]
struct RecordingSink {
    events: Vec<RuntimeEvent>,
}

impl RuntimeEventSink for RecordingSink {
    fn push(&mut self, event: RuntimeEvent) -> Result<(), TurnError> {
        self.events.push(event);
        Ok(())
    }
}

enum Behavior {
    Success,
    Error,
}

struct MockProvider {
    behavior: Behavior,
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
        sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 1,
            block_id: "text-1".to_string(),
            delta: "hello".to_string(),
        }))
        .map_err(|err| ProviderError::Transport {
            message: format!("sink rejected stream delta: {err}"),
        })?;

        match self.behavior {
            Behavior::Success => {
                let message = AssistantMessage {
                    message_id: "msg-123".to_string(),
                    model: sample_model(),
                    content: vec![AssistantContent::Text(TextContent {
                        text: "hello".to_string(),
                    })],
                    usage: TokenUsage::default(),
                    stop_reason: StopReason::Stop,
                };
                Ok(ProviderFinal {
                    message,
                    transport_details: None,
                })
            }
            Behavior::Error => Err(ProviderError::Transport {
                message: "network timeout".to_string(),
            }),
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
        },
        provider_context: ProviderContext {
            api_key: None,
            timeout_ms: 30_000,
        },
        cancellation_token: Some(Arc::new(AtomicBool::new(false))),
        starting_sequence_no: 1,
    }
}

fn terminal_count(events: &[RuntimeEvent]) -> usize {
    events
        .iter()
        .filter(|event| event.is_turn_terminal())
        .count()
}

#[test]
fn success_path_preserves_strict_monotonic_sequence_and_single_terminal() {
    block_on(async {
        let engine = ChatTurnEngine::new(MockProvider {
            behavior: Behavior::Success,
        });
        let mut sink = RecordingSink::default();

        engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect("turn should succeed");

        assert_eq!(terminal_count(&sink.events), 1);
        assert!(sink
            .events
            .last()
            .is_some_and(RuntimeEvent::is_turn_terminal));
        for pair in sink.events.windows(2) {
            assert!(pair[1].sequence_no() > pair[0].sequence_no());
        }
    });
}

#[test]
fn provider_error_path_emits_single_terminal_and_no_post_terminal_events() {
    block_on(async {
        let engine = ChatTurnEngine::new(MockProvider {
            behavior: Behavior::Error,
        });
        let mut sink = RecordingSink::default();

        let err = engine
            .run_turn(sample_turn_request(), &mut sink)
            .await
            .expect_err("turn should fail");

        assert!(matches!(
            err,
            TurnError::ProviderFailure(ProviderError::Transport { .. })
        ));
        assert_eq!(terminal_count(&sink.events), 1);
        assert!(matches!(
            sink.events.last(),
            Some(RuntimeEvent::RuntimeError { .. })
        ));
        for pair in sink.events.windows(2) {
            assert!(pair[1].sequence_no() > pair[0].sequence_no());
        }
    });
}
