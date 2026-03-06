use std::collections::HashMap;

use async_trait::async_trait;
use lorum_domain::{MessageId, RuntimeEvent, SessionId, TurnId, TurnTerminalReason, UiCommand};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UiState {
    pub active_session: Option<SessionId>,
    pub active_model: Option<String>,
    pub turn_buffers: HashMap<TurnId, TurnBuffer>,
    pub turn_states: HashMap<TurnId, TurnRuntimeState>,
    pub completed_turns: Vec<CompletedTurn>,
    pub last_error: Option<RuntimeErrorSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBuffer {
    pub session_id: SessionId,
    pub assistant_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnRuntimeState {
    pub session_id: SessionId,
    pub last_sequence_no: u64,
    pub terminal_seen: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeErrorSnapshot {
    pub code: String,
    pub message: String,
    pub turn_id: TurnId,
    pub sequence_no: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletedTurn {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub assistant_text: String,
    pub reason: TurnTerminalReason,
    pub message_id: Option<MessageId>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum UiError {
    #[error("assistant delta received before turn was started: {turn_id:?}")]
    DeltaBeforeTurnStarted { turn_id: TurnId },
    #[error("turn finished without an active buffer: {turn_id:?}")]
    FinishUnknownTurn { turn_id: TurnId },
    #[error("runtime error received before turn was started: {turn_id:?}")]
    RuntimeErrorUnknownTurn { turn_id: TurnId },
    #[error("sequence regression for turn {turn_id:?}: {previous} -> {current}")]
    SequenceRegression {
        turn_id: TurnId,
        previous: u64,
        current: u64,
    },
    #[error("duplicate terminal event for turn {turn_id:?}")]
    DuplicateTerminal { turn_id: TurnId },
    #[error("event observed after terminal event for turn {turn_id:?}")]
    PostTerminalEvent { turn_id: TurnId },
    #[error(
        "session mismatch for turn {turn_id:?}: expected {expected_session:?} but got {actual_session:?}"
    )]
    SessionMismatch {
        turn_id: TurnId,
        expected_session: SessionId,
        actual_session: SessionId,
    },
}

pub trait UiReducer {
    fn apply(&mut self, ev: &RuntimeEvent) -> Result<(), UiError>;
}

#[derive(Debug, Default)]
pub struct DefaultUiReducer {
    state: UiState,
}

impl DefaultUiReducer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &UiState {
        &self.state
    }

    fn enforce_active_session(
        &mut self,
        turn_id: &TurnId,
        event_session: &SessionId,
    ) -> Result<(), UiError> {
        // Deterministic policy: once active_session is set, every turn-scoped event must
        // belong to that exact session. SessionSwitched is global and is the only event
        // that mutates active_session after initialization.
        match &self.state.active_session {
            Some(active_session) if active_session != event_session => {
                Err(UiError::SessionMismatch {
                    turn_id: turn_id.clone(),
                    expected_session: active_session.clone(),
                    actual_session: event_session.clone(),
                })
            }
            Some(_) => Ok(()),
            None => {
                self.state.active_session = Some(event_session.clone());
                Ok(())
            }
        }
    }

    fn validate_non_start_turn_event(
        &mut self,
        turn_id: &TurnId,
        sequence_no: u64,
        is_terminal: bool,
    ) -> Result<SessionId, UiError> {
        let Some(turn_state) = self.state.turn_states.get_mut(turn_id) else {
            return Err(match is_terminal {
                true => UiError::FinishUnknownTurn {
                    turn_id: turn_id.clone(),
                },
                false => UiError::DeltaBeforeTurnStarted {
                    turn_id: turn_id.clone(),
                },
            });
        };

        if sequence_no < turn_state.last_sequence_no {
            return Err(UiError::SequenceRegression {
                turn_id: turn_id.clone(),
                previous: turn_state.last_sequence_no,
                current: sequence_no,
            });
        }

        if turn_state.terminal_seen {
            return Err(if is_terminal {
                UiError::DuplicateTerminal {
                    turn_id: turn_id.clone(),
                }
            } else {
                UiError::PostTerminalEvent {
                    turn_id: turn_id.clone(),
                }
            });
        }

        let session_id = turn_state.session_id.clone();
        turn_state.last_sequence_no = sequence_no;
        if is_terminal {
            turn_state.terminal_seen = true;
        }

        Ok(session_id)
    }
}

impl UiReducer for DefaultUiReducer {
    fn apply(&mut self, ev: &RuntimeEvent) -> Result<(), UiError> {
        match ev {
            RuntimeEvent::TurnStarted {
                turn_id,
                sequence_no,
                session_id,
            } => {
                self.enforce_active_session(turn_id, session_id)?;

                if let Some(turn_state) = self.state.turn_states.get(turn_id) {
                    if *sequence_no < turn_state.last_sequence_no {
                        return Err(UiError::SequenceRegression {
                            turn_id: turn_id.clone(),
                            previous: turn_state.last_sequence_no,
                            current: *sequence_no,
                        });
                    }

                    return Err(if turn_state.terminal_seen {
                        UiError::PostTerminalEvent {
                            turn_id: turn_id.clone(),
                        }
                    } else {
                        UiError::DeltaBeforeTurnStarted {
                            turn_id: turn_id.clone(),
                        }
                    });
                }

                self.state.turn_buffers.insert(
                    turn_id.clone(),
                    TurnBuffer {
                        session_id: session_id.clone(),
                        assistant_text: String::new(),
                    },
                );
                self.state.turn_states.insert(
                    turn_id.clone(),
                    TurnRuntimeState {
                        session_id: session_id.clone(),
                        last_sequence_no: *sequence_no,
                        terminal_seen: false,
                    },
                );
            }
            RuntimeEvent::AssistantStreamDelta {
                turn_id,
                sequence_no,
                delta,
            } => {
                let session_id =
                    self.validate_non_start_turn_event(turn_id, *sequence_no, false)?;
                self.enforce_active_session(turn_id, &session_id)?;

                let Some(buffer) = self.state.turn_buffers.get_mut(turn_id) else {
                    return Err(UiError::DeltaBeforeTurnStarted {
                        turn_id: turn_id.clone(),
                    });
                };
                buffer.assistant_text.push_str(delta);
            }
            RuntimeEvent::TurnFinished {
                turn_id,
                sequence_no,
                reason,
                message_id,
                ..
            } => {
                let session_id = self.validate_non_start_turn_event(turn_id, *sequence_no, true)?;
                self.enforce_active_session(turn_id, &session_id)?;

                let Some(buffer) = self.state.turn_buffers.remove(turn_id) else {
                    return Err(UiError::FinishUnknownTurn {
                        turn_id: turn_id.clone(),
                    });
                };

                self.state.completed_turns.push(CompletedTurn {
                    turn_id: turn_id.clone(),
                    session_id: buffer.session_id,
                    assistant_text: buffer.assistant_text,
                    reason: *reason,
                    message_id: message_id.clone(),
                });
            }
            RuntimeEvent::RuntimeError {
                turn_id,
                sequence_no,
                code,
                message,
            } => {
                let Some(turn_state) = self.state.turn_states.get(turn_id) else {
                    return Err(UiError::RuntimeErrorUnknownTurn {
                        turn_id: turn_id.clone(),
                    });
                };

                self.enforce_active_session(turn_id, &turn_state.session_id.clone())?;
                self.validate_non_start_turn_event(turn_id, *sequence_no, true)?;

                self.state.last_error = Some(RuntimeErrorSnapshot {
                    code: code.clone(),
                    message: message.clone(),
                    turn_id: turn_id.clone(),
                    sequence_no: *sequence_no,
                });

                if let Some(buffer) = self.state.turn_buffers.remove(turn_id) {
                    self.state.completed_turns.push(CompletedTurn {
                        turn_id: turn_id.clone(),
                        session_id: buffer.session_id,
                        assistant_text: buffer.assistant_text,
                        reason: TurnTerminalReason::Error,
                        message_id: None,
                    });
                }
            }
            RuntimeEvent::UserMessageReceived { .. }
            | RuntimeEvent::AssistantThinkingDelta { .. }
            | RuntimeEvent::ToolExecutionStart { .. }
            | RuntimeEvent::ToolExecutionEnd { .. }
            | RuntimeEvent::ToolResultReceived { .. } => {}
            RuntimeEvent::SessionSwitched { to_session_id, .. } => {
                self.state.active_session = Some(to_session_id.clone());
            }
        }

        Ok(())
    }
}

#[async_trait]
pub trait UiCommandSink {
    async fn send(&mut self, command: UiCommand) -> Result<(), UiError>;
}
