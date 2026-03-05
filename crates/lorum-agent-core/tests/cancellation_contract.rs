use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_agent_core::{ChatTurnEngine, RuntimeEventSink, TurnEngine, TurnError, TurnRequest};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent,
    ModelRef, ProviderAdapter, ProviderContext, ProviderError, ProviderFinal, ProviderInputMessage,
    ProviderRequest, StopReason, StreamTextDelta, TextContent, TokenUsage,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};

enum CancellationMode {
    None,
    OnFirstDelta,
    OnTerminal,
}

struct CancellationAwareSink {
    events: Vec<RuntimeEvent>,
    cancellation_token: Arc<AtomicBool>,
    mode: CancellationMode,
}

impl CancellationAwareSink {
    fn new(cancellation_token: Arc<AtomicBool>, mode: CancellationMode) -> Self {
        Self {
            events: Vec::new(),
            cancellation_token,
            mode,
        }
    }
}

impl RuntimeEventSink for CancellationAwareSink {
    fn push(&mut self, event: RuntimeEvent) -> Result<(), TurnError> {
        match (&self.mode, &event) {
            (CancellationMode::OnFirstDelta, RuntimeEvent::AssistantStreamDelta { .. })
                if !self.cancellation_token.load(Ordering::SeqCst) =>
            {
                self.cancellation_token.store(true, Ordering::SeqCst);
            }
            (CancellationMode::OnTerminal, RuntimeEvent::TurnFinished { .. })
                if !self.cancellation_token.load(Ordering::SeqCst) =>
            {
                self.cancellation_token.store(true, Ordering::SeqCst);
            }
            _ => {}
        }

        self.events.push(event);
        Ok(())
    }
}

enum Behavior {
    ReturnSuccess,
    AbortAfterSecondDelta,
}

struct MockProvider {
    called: Arc<AtomicBool>,
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
        self.called.store(true, Ordering::SeqCst);

        sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 1,
            block_id: "text-1".to_string(),
            delta: "first".to_string(),
        }))
        .map_err(|err| ProviderError::Transport {
            message: format!("sink rejected first delta: {err}"),
        })?;

        match self.behavior {
            Behavior::ReturnSuccess => Ok(provider_final("msg-123", StopReason::Stop)),
            Behavior::AbortAfterSecondDelta => {
                let second_push = sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
                    sequence_no: 2,
                    block_id: "text-1".to_string(),
                    delta: "second".to_string(),
                }));

                match second_push {
                    Ok(()) => Ok(provider_final("msg-123", StopReason::Stop)),
                    Err(_) => Err(ProviderError::Aborted),
                }
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

fn provider_final(message_id: &str, stop_reason: StopReason) -> ProviderFinal {
    ProviderFinal {
        message: AssistantMessage {
            message_id: message_id.to_string(),
            model: sample_model(),
            content: vec![AssistantContent::Text(TextContent {
                text: "hello".to_string(),
            })],
            usage: TokenUsage::default(),
            stop_reason,
        },
        transport_details: None,
    }
}

fn sample_model() -> ModelRef {
    ModelRef {
        provider: "mock".to_string(),
        api: ApiKind::OpenAiResponses,
        model: "test-model".to_string(),
    }
}

fn sample_turn_request(cancellation_token: Arc<AtomicBool>) -> TurnRequest {
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
        cancellation_token: Some(cancellation_token),
        starting_sequence_no: 1,
    }
}

#[test]
fn cancel_before_stream_emits_aborted_terminal_without_provider_call() {
    block_on(async {
        let cancellation_token = Arc::new(AtomicBool::new(true));
        let provider_called = Arc::new(AtomicBool::new(false));
        let engine = ChatTurnEngine::new(MockProvider {
            called: Arc::clone(&provider_called),
            behavior: Behavior::ReturnSuccess,
        });
        let mut sink =
            CancellationAwareSink::new(Arc::clone(&cancellation_token), CancellationMode::None);

        let result = engine
            .run_turn(
                sample_turn_request(Arc::clone(&cancellation_token)),
                &mut sink,
            )
            .await
            .expect("cancel before stream should not fail turn execution");

        assert!(!provider_called.load(Ordering::SeqCst));
        assert_eq!(result.terminal_reason, TurnTerminalReason::Aborted);
        assert_eq!(result.message_id, None);
        assert_eq!(sink.events.len(), 2);
        assert!(matches!(sink.events[0], RuntimeEvent::TurnStarted { .. }));
        assert!(matches!(
            sink.events[1],
            RuntimeEvent::TurnFinished {
                reason: TurnTerminalReason::Aborted,
                message_id: None,
                ..
            }
        ));
    });
}

#[test]
fn cancel_mid_stream_stops_additional_delta_emission_and_finishes_aborted() {
    block_on(async {
        let cancellation_token = Arc::new(AtomicBool::new(false));
        let provider_called = Arc::new(AtomicBool::new(false));
        let engine = ChatTurnEngine::new(MockProvider {
            called: Arc::clone(&provider_called),
            behavior: Behavior::AbortAfterSecondDelta,
        });
        let mut sink = CancellationAwareSink::new(
            Arc::clone(&cancellation_token),
            CancellationMode::OnFirstDelta,
        );

        let result = engine
            .run_turn(
                sample_turn_request(Arc::clone(&cancellation_token)),
                &mut sink,
            )
            .await
            .expect("cancel mid-stream should finish turn as aborted");

        assert!(provider_called.load(Ordering::SeqCst));
        assert_eq!(result.terminal_reason, TurnTerminalReason::Aborted);
        assert_eq!(
            sink.events
                .iter()
                .filter(|event| matches!(event, RuntimeEvent::AssistantStreamDelta { .. }))
                .count(),
            1
        );
        assert!(matches!(
            sink.events.last(),
            Some(RuntimeEvent::TurnFinished {
                reason: TurnTerminalReason::Aborted,
                message_id: None,
                ..
            })
        ));
    });
}

#[test]
fn cancel_after_terminal_has_no_effect_on_completed_turn() {
    block_on(async {
        let cancellation_token = Arc::new(AtomicBool::new(false));
        let provider_called = Arc::new(AtomicBool::new(false));
        let engine = ChatTurnEngine::new(MockProvider {
            called: Arc::clone(&provider_called),
            behavior: Behavior::ReturnSuccess,
        });
        let mut sink = CancellationAwareSink::new(
            Arc::clone(&cancellation_token),
            CancellationMode::OnTerminal,
        );

        let result = engine
            .run_turn(
                sample_turn_request(Arc::clone(&cancellation_token)),
                &mut sink,
            )
            .await
            .expect("turn should complete before cancellation request is observed");

        assert!(provider_called.load(Ordering::SeqCst));
        assert!(cancellation_token.load(Ordering::SeqCst));
        assert_eq!(result.terminal_reason, TurnTerminalReason::Done);
        assert!(matches!(
            sink.events.last(),
            Some(RuntimeEvent::TurnFinished {
                reason: TurnTerminalReason::Done,
                message_id: Some(_),
                ..
            })
        ));
    });
}
