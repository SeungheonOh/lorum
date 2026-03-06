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

fn provider_error_code(error: &ProviderError) -> String {
    match error {
        ProviderError::Auth { .. } => "auth",
        ProviderError::RateLimited { .. } => "rate_limited",
        ProviderError::Transport { .. } => "transport",
        ProviderError::InvalidResponse { .. } => "invalid_response",
        ProviderError::Aborted => "aborted",
    }
    .to_string()
}
