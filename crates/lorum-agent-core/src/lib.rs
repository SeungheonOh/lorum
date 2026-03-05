use async_trait::async_trait;
use lorum_ai_contract::{
    AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent, ProviderAdapter,
    ProviderContext, ProviderError, ProviderRequest, StopReason, StreamSinkError, StreamTextDelta,
    StreamThinkingDelta, StreamToolCallDelta,
};
use lorum_domain::{
    validate_turn_event_order, EventOrderError, MessageId, RuntimeEvent, SessionId, TurnId,
    TurnTerminalReason,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use thiserror::Error;

pub trait RuntimeEventSink: Send {
    fn push(&mut self, event: RuntimeEvent) -> Result<(), TurnError>;
}

#[derive(Debug, Clone)]
pub struct TurnRequest {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub provider_request: ProviderRequest,
    pub provider_context: ProviderContext,
    pub cancellation_token: Option<Arc<AtomicBool>>,
    pub starting_sequence_no: u64,
}
#[derive(Debug, Clone, PartialEq)]
pub struct TurnResult {
    pub turn_id: TurnId,
    pub message_id: Option<MessageId>,
    pub terminal_reason: TurnTerminalReason,
    pub event_count: usize,
    pub assistant_message: Option<AssistantMessage>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum TurnError {
    #[error("runtime sink failure: {0}")]
    SinkFailure(String),
    #[error(transparent)]
    ProviderFailure(#[from] ProviderError),
    #[error(transparent)]
    EventOrderFailure(#[from] EventOrderError),
}

#[async_trait]
pub trait TurnEngine: Send + Sync {
    async fn run_turn(
        &self,
        request: TurnRequest,
        sink: &mut dyn RuntimeEventSink,
    ) -> Result<TurnResult, TurnError>;
}

pub struct ChatTurnEngine<P>
where
    P: ProviderAdapter,
{
    provider: P,
}

impl<P> ChatTurnEngine<P>
where
    P: ProviderAdapter,
{
    pub fn new(provider: P) -> Self {
        Self { provider }
    }
}

#[async_trait]
impl<P> TurnEngine for ChatTurnEngine<P>
where
    P: ProviderAdapter,
{
    async fn run_turn(
        &self,
        request: TurnRequest,
        sink: &mut dyn RuntimeEventSink,
    ) -> Result<TurnResult, TurnError> {
        let TurnRequest {
            session_id,
            turn_id,
            provider_request,
            provider_context,
            cancellation_token,
            ..
        } = request;

        let mut emitted_events: Vec<RuntimeEvent> = Vec::new();
        let mut emission_guard = TurnEmissionGuard::new(turn_id.clone());
        let mut sequence_no = request.starting_sequence_no.max(1);

        push_runtime_event(
            sink,
            &mut emitted_events,
            &mut emission_guard,
            RuntimeEvent::TurnStarted {
                turn_id: turn_id.clone(),
                sequence_no,
                session_id,
            },
        )?;
        sequence_no += 1;

        if is_cancellation_requested(cancellation_token.as_ref()) {
            return finish_aborted_turn(
                sink,
                &mut emitted_events,
                &mut emission_guard,
                turn_id,
                sequence_no,
            );
        }

        let provider_result = {
            let mut adapter = AssistantToRuntimeEventAdapter::new(
                turn_id.clone(),
                sequence_no,
                sink,
                &mut emitted_events,
                &mut emission_guard,
                cancellation_token.as_ref(),
            );
            let result = self
                .provider
                .stream(provider_request, provider_context, &mut adapter)
                .await;
            sequence_no = adapter.next_sequence_no();
            result
        };

        match provider_result {
            Ok(provider_final) => {
                let terminal_reason = map_stop_reason(provider_final.message.stop_reason);
                let message_id = MessageId::from(provider_final.message.message_id.clone());
                let assistant_message = provider_final.message;

                push_runtime_event(
                    sink,
                    &mut emitted_events,
                    &mut emission_guard,
                    RuntimeEvent::TurnFinished {
                        turn_id: turn_id.clone(),
                        sequence_no,
                        reason: terminal_reason,
                        message_id: Some(message_id.clone()),
                        assistant_message: Some(assistant_message.clone()),
                    },
                )?;

                validate_turn_event_order(&emitted_events)?;

                Ok(TurnResult {
                    turn_id,
                    message_id: Some(message_id),
                    terminal_reason,
                    event_count: emitted_events.len(),
                    assistant_message: Some(assistant_message),
                })
            }
            Err(err) => {
                if is_cancellation_requested(cancellation_token.as_ref()) {
                    return finish_aborted_turn(
                        sink,
                        &mut emitted_events,
                        &mut emission_guard,
                        turn_id,
                        sequence_no,
                    );
                }

                push_runtime_event(
                    sink,
                    &mut emitted_events,
                    &mut emission_guard,
                    RuntimeEvent::RuntimeError {
                        turn_id,
                        sequence_no,
                        code: provider_error_code(&err),
                        message: err.to_string(),
                    },
                )?;

                validate_turn_event_order(&emitted_events)?;
                Err(TurnError::ProviderFailure(err))
            }
        }
    }
}

pub struct AssistantToRuntimeEventAdapter<'a> {
    turn_id: TurnId,
    next_sequence_no: u64,
    runtime_sink: &'a mut dyn RuntimeEventSink,
    emitted_events: &'a mut Vec<RuntimeEvent>,
    emission_guard: &'a mut TurnEmissionGuard,
    emitted_stream_delta: bool,
    cancellation_token: Option<&'a Arc<AtomicBool>>,
}

impl<'a> AssistantToRuntimeEventAdapter<'a> {
    fn new(
        turn_id: TurnId,
        next_sequence_no: u64,
        runtime_sink: &'a mut dyn RuntimeEventSink,
        emitted_events: &'a mut Vec<RuntimeEvent>,
        emission_guard: &'a mut TurnEmissionGuard,
        cancellation_token: Option<&'a Arc<AtomicBool>>,
    ) -> Self {
        Self {
            turn_id,
            next_sequence_no,
            runtime_sink,
            emitted_events,
            emission_guard,
            emitted_stream_delta: false,
            cancellation_token,
        }
    }

    fn next_sequence_no(&self) -> u64 {
        self.next_sequence_no
    }

    fn push_delta(&mut self, delta: String) -> Result<(), StreamSinkError> {
        if is_cancellation_requested(self.cancellation_token) {
            return Err(StreamSinkError::Rejected(cancellation_rejection_message()));
        }

        let event = RuntimeEvent::AssistantStreamDelta {
            turn_id: self.turn_id.clone(),
            sequence_no: self.next_sequence_no,
            delta,
        };

        push_runtime_event(
            self.runtime_sink,
            self.emitted_events,
            self.emission_guard,
            event,
        )
        .map_err(|err| StreamSinkError::Rejected(err.to_string()))?;

        self.next_sequence_no += 1;
        self.emitted_stream_delta = true;
        Ok(())
    }

    fn push_thinking_delta(&mut self, delta: String) -> Result<(), StreamSinkError> {
        if is_cancellation_requested(self.cancellation_token) {
            return Err(StreamSinkError::Rejected(cancellation_rejection_message()));
        }

        let event = RuntimeEvent::AssistantThinkingDelta {
            turn_id: self.turn_id.clone(),
            sequence_no: self.next_sequence_no,
            delta,
        };

        push_runtime_event(
            self.runtime_sink,
            self.emitted_events,
            self.emission_guard,
            event,
        )
        .map_err(|err| StreamSinkError::Rejected(err.to_string()))?;

        self.next_sequence_no += 1;
        Ok(())
    }
}
fn done_message_text(message_event: &AssistantMessageEvent) -> Option<String> {
    let AssistantMessageEvent::Done(done) = message_event else {
        return None;
    };

    let text = done
        .message
        .content
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

impl AssistantEventSink for AssistantToRuntimeEventAdapter<'_> {
    fn push(&mut self, event: AssistantMessageEvent) -> Result<(), StreamSinkError> {
        match event {
            AssistantMessageEvent::TextDelta(StreamTextDelta { delta, .. }) => {
                self.push_delta(delta)
            }
            AssistantMessageEvent::ThinkingDelta(StreamThinkingDelta { delta, .. }) => {
                self.push_thinking_delta(delta)
            }
            AssistantMessageEvent::ToolCallDelta(StreamToolCallDelta { .. }) => {
                // Tool call argument deltas are accumulated by the provider adapter
                // into AssistantContent::ToolCall entries. Don't emit them as stream
                // deltas since the raw JSON would confuse the UI.
                Ok(())
            }
            AssistantMessageEvent::Done(done) => {
                if self.emitted_stream_delta {
                    return Ok(());
                }
                let done_event = AssistantMessageEvent::Done(done);
                if let Some(text) = done_message_text(&done_event) {
                    self.push_delta(text)
                } else {
                    Ok(())
                }
            }
            AssistantMessageEvent::Start(_)
            | AssistantMessageEvent::TextStart(_)
            | AssistantMessageEvent::TextEnd(_)
            | AssistantMessageEvent::ThinkingStart(_)
            | AssistantMessageEvent::ThinkingEnd(_)
            | AssistantMessageEvent::ToolCallStart(_)
            | AssistantMessageEvent::ToolCallEnd(_)
            | AssistantMessageEvent::Error(_) => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InternalFailureClass {
    Auth,
    RateLimited,
    Transport,
    InvalidResponse,
    Aborted,
}

impl InternalFailureClass {
    fn code(self) -> &'static str {
        match self {
            Self::Auth => "auth",
            Self::RateLimited => "rate_limited",
            Self::Transport => "transport",
            Self::InvalidResponse => "invalid_response",
            Self::Aborted => "aborted",
        }
    }
}

#[derive(Debug)]
struct TurnEmissionGuard {
    turn_id: TurnId,
    last_sequence_no: u64,
    terminal_seen: bool,
}

impl TurnEmissionGuard {
    fn new(turn_id: TurnId) -> Self {
        Self {
            turn_id,
            last_sequence_no: 0,
            terminal_seen: false,
        }
    }

    fn validate_next(&self, event: &RuntimeEvent) -> Result<(), TurnError> {
        let sequence_no = event.sequence_no();
        if sequence_no <= self.last_sequence_no {
            return Err(TurnError::EventOrderFailure(
                EventOrderError::SequenceRegression {
                    turn_id: self.turn_id.clone(),
                    previous: self.last_sequence_no,
                    current: sequence_no,
                },
            ));
        }

        if self.terminal_seen {
            let err = if event.is_turn_terminal() {
                EventOrderError::DuplicateTerminal {
                    turn_id: self.turn_id.clone(),
                }
            } else {
                EventOrderError::EventAfterTerminal {
                    turn_id: self.turn_id.clone(),
                }
            };
            return Err(TurnError::EventOrderFailure(err));
        }

        Ok(())
    }

    fn record(&mut self, event: &RuntimeEvent) {
        self.last_sequence_no = event.sequence_no();
        if event.is_turn_terminal() {
            self.terminal_seen = true;
        }
    }
}

fn push_runtime_event(
    sink: &mut dyn RuntimeEventSink,
    emitted_events: &mut Vec<RuntimeEvent>,
    emission_guard: &mut TurnEmissionGuard,
    event: RuntimeEvent,
) -> Result<(), TurnError> {
    emission_guard.validate_next(&event)?;
    sink.push(event.clone())?;
    emission_guard.record(&event);
    emitted_events.push(event);
    Ok(())
}

fn finish_aborted_turn(
    sink: &mut dyn RuntimeEventSink,
    emitted_events: &mut Vec<RuntimeEvent>,
    emission_guard: &mut TurnEmissionGuard,
    turn_id: TurnId,
    sequence_no: u64,
) -> Result<TurnResult, TurnError> {
    push_runtime_event(
        sink,
        emitted_events,
        emission_guard,
        RuntimeEvent::TurnFinished {
            turn_id: turn_id.clone(),
            sequence_no,
            reason: TurnTerminalReason::Aborted,
            message_id: None,
            assistant_message: None,
        },
    )?;

    validate_turn_event_order(emitted_events)?;

    Ok(TurnResult {
        turn_id,
        message_id: None,
        terminal_reason: TurnTerminalReason::Aborted,
        event_count: emitted_events.len(),
        assistant_message: None,
    })
}

fn is_cancellation_requested(token: Option<&Arc<AtomicBool>>) -> bool {
    token.is_some_and(|flag| flag.load(Ordering::SeqCst))
}

fn cancellation_rejection_message() -> String {
    "turn cancelled".to_string()
}

fn map_stop_reason(reason: StopReason) -> TurnTerminalReason {
    match reason {
        StopReason::ToolUse => TurnTerminalReason::ToolUse,
        StopReason::Error => TurnTerminalReason::Error,
        StopReason::Aborted => TurnTerminalReason::Aborted,
        _ => TurnTerminalReason::Done,
    }
}

fn classify_provider_failure(error: &ProviderError) -> InternalFailureClass {
    match error {
        ProviderError::Auth { .. } => InternalFailureClass::Auth,
        ProviderError::RateLimited { .. } => InternalFailureClass::RateLimited,
        ProviderError::Transport { .. } => InternalFailureClass::Transport,
        ProviderError::InvalidResponse { .. } => InternalFailureClass::InvalidResponse,
        ProviderError::Aborted => InternalFailureClass::Aborted,
    }
}

fn provider_error_code(error: &ProviderError) -> String {
    classify_provider_failure(error).code().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use lorum_ai_contract::{
        ApiKind, AssistantContent, AssistantMessage, ModelRef, ProviderFinal, ProviderInputMessage,
        StreamDoneEvent, StreamErrorEvent, TextContent, TokenUsage,
    };
    use lorum_domain::validate_turn_event_order;

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
}
