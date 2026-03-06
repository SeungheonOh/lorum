use std::collections::HashMap;

use lorum_ai_contract::AssistantMessage;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_inner(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

id_newtype!(SessionId);
id_newtype!(TurnId);
id_newtype!(MessageId);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnTerminalReason {
    Done,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum RuntimeEvent {
    UserMessageReceived {
        turn_id: TurnId,
        session_id: SessionId,
        sequence_no: u64,
        content: String,
    },
    TurnStarted {
        turn_id: TurnId,
        sequence_no: u64,
        session_id: SessionId,
    },
    AssistantStreamDelta {
        turn_id: TurnId,
        sequence_no: u64,
        delta: String,
    },
    AssistantThinkingDelta {
        turn_id: TurnId,
        sequence_no: u64,
        delta: String,
    },
    TurnFinished {
        turn_id: TurnId,
        sequence_no: u64,
        reason: TurnTerminalReason,
        message_id: Option<MessageId>,
        assistant_message: Option<AssistantMessage>,
    },
    RuntimeError {
        turn_id: TurnId,
        sequence_no: u64,
        code: String,
        message: String,
    },
    ToolExecutionStart {
        turn_id: TurnId,
        sequence_no: u64,
        tool_call_id: String,
        tool_name: String,
        arguments: Value,
    },
    ToolExecutionEnd {
        turn_id: TurnId,
        sequence_no: u64,
        tool_call_id: String,
        tool_name: String,
        is_error: bool,
    },
    ToolResultReceived {
        turn_id: TurnId,
        sequence_no: u64,
        tool_call_id: String,
        is_error: bool,
        result: Value,
    },
    SessionSwitched {
        sequence_no: u64,
        from_session_id: SessionId,
        to_session_id: SessionId,
    },
}

impl RuntimeEvent {
    pub fn sequence_no(&self) -> u64 {
        match self {
            RuntimeEvent::UserMessageReceived { sequence_no, .. }
            | RuntimeEvent::TurnStarted { sequence_no, .. }
            | RuntimeEvent::AssistantStreamDelta { sequence_no, .. }
            | RuntimeEvent::AssistantThinkingDelta { sequence_no, .. }
            | RuntimeEvent::TurnFinished { sequence_no, .. }
            | RuntimeEvent::RuntimeError { sequence_no, .. }
            | RuntimeEvent::ToolExecutionStart { sequence_no, .. }
            | RuntimeEvent::ToolExecutionEnd { sequence_no, .. }
            | RuntimeEvent::ToolResultReceived { sequence_no, .. }
            | RuntimeEvent::SessionSwitched { sequence_no, .. } => *sequence_no,
        }
    }

    pub fn turn_id(&self) -> Option<&TurnId> {
        match self {
            RuntimeEvent::UserMessageReceived { turn_id, .. }
            | RuntimeEvent::TurnStarted { turn_id, .. }
            | RuntimeEvent::AssistantStreamDelta { turn_id, .. }
            | RuntimeEvent::AssistantThinkingDelta { turn_id, .. }
            | RuntimeEvent::TurnFinished { turn_id, .. }
            | RuntimeEvent::RuntimeError { turn_id, .. }
            | RuntimeEvent::ToolExecutionStart { turn_id, .. }
            | RuntimeEvent::ToolExecutionEnd { turn_id, .. }
            | RuntimeEvent::ToolResultReceived { turn_id, .. } => Some(turn_id),
            RuntimeEvent::SessionSwitched { .. } => None,
        }
    }

    pub fn is_turn_terminal(&self) -> bool {
        matches!(
            self,
            RuntimeEvent::TurnFinished { .. } | RuntimeEvent::RuntimeError { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiCommand {
    Submit {
        session_id: SessionId,
        turn_id: TurnId,
        prompt: String,
    },
    Cancel {
        turn_id: TurnId,
    },
    SwitchSession {
        session_id: SessionId,
    },
    SetModel {
        model: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum UiNotification {
    Info { message: String },
    Warning { message: String },
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EventOrderError {
    #[error("sequence regression for turn {turn_id:?}: {previous} -> {current}")]
    SequenceRegression {
        turn_id: TurnId,
        previous: u64,
        current: u64,
    },
    #[error("duplicate terminal event for turn {turn_id:?}")]
    DuplicateTerminal { turn_id: TurnId },
    #[error("event observed after terminal event for turn {turn_id:?}")]
    EventAfterTerminal { turn_id: TurnId },
    #[error("missing terminal event for turn {turn_id:?}")]
    MissingTerminal { turn_id: TurnId },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TurnState {
    last_sequence_no: u64,
    terminal_seen: bool,
}

pub fn validate_turn_event_order(events: &[RuntimeEvent]) -> Result<(), EventOrderError> {
    let mut turn_states: HashMap<TurnId, TurnState> = HashMap::new();

    for event in events {
        let Some(turn_id) = event.turn_id() else {
            continue;
        };

        let sequence_no = event.sequence_no();
        let is_terminal = event.is_turn_terminal();

        let state = turn_states.entry(turn_id.clone()).or_insert(TurnState {
            last_sequence_no: sequence_no,
            terminal_seen: false,
        });

        if sequence_no < state.last_sequence_no {
            return Err(EventOrderError::SequenceRegression {
                turn_id: turn_id.clone(),
                previous: state.last_sequence_no,
                current: sequence_no,
            });
        }

        if state.terminal_seen {
            if is_terminal {
                return Err(EventOrderError::DuplicateTerminal {
                    turn_id: turn_id.clone(),
                });
            }

            return Err(EventOrderError::EventAfterTerminal {
                turn_id: turn_id.clone(),
            });
        }

        state.last_sequence_no = sequence_no;

        if is_terminal {
            state.terminal_seen = true;
        }
    }

    if let Some((turn_id, _)) = turn_states.iter().find(|(_, state)| !state.terminal_seen) {
        return Err(EventOrderError::MissingTerminal {
            turn_id: turn_id.clone(),
        });
    }

    Ok(())
}
