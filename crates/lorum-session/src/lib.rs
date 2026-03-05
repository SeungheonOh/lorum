use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::RwLock;

use lorum_ai_contract::ProviderInputMessage;
use lorum_domain::RuntimeEvent;
pub use lorum_domain::SessionId;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    #[error("session store lock poisoned")]
    LockPoisoned,
    #[error("unknown session: {session_id:?}")]
    UnknownSession { session_id: SessionId },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwitchResult {
    pub from_session_id: SessionId,
    pub to_session_id: SessionId,
    pub from_event_count: usize,
    pub to_event_count: usize,
}

pub trait SessionStore: Send + Sync {
    fn append(&self, session_id: &SessionId, ev: RuntimeEvent) -> Result<(), SessionError>;
    fn replay(&self, session_id: &SessionId) -> Result<Vec<RuntimeEvent>, SessionError>;
    fn switch(&self, from: &SessionId, to: &SessionId) -> Result<SwitchResult, SessionError>;
}

#[derive(Debug)]
struct StoredEvent {
    insertion_order: u64,
    event: RuntimeEvent,
}

#[derive(Debug, Default)]
pub struct InMemorySessionStore {
    events: RwLock<HashMap<SessionId, Vec<StoredEvent>>>,
    next_insertion_order: AtomicU64,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SessionStore for InMemorySessionStore {
    fn append(&self, session_id: &SessionId, ev: RuntimeEvent) -> Result<(), SessionError> {
        let insertion_order = self
            .next_insertion_order
            .fetch_add(1, AtomicOrdering::Relaxed);
        let mut guard = self
            .events
            .write()
            .map_err(|_| SessionError::LockPoisoned)?;
        guard
            .entry(session_id.clone())
            .or_default()
            .push(StoredEvent {
                insertion_order,
                event: ev,
            });
        Ok(())
    }

    fn replay(&self, session_id: &SessionId) -> Result<Vec<RuntimeEvent>, SessionError> {
        let guard = self.events.read().map_err(|_| SessionError::LockPoisoned)?;
        let Some(stored) = guard.get(session_id) else {
            return Ok(Vec::new());
        };

        let mut sorted: Vec<&StoredEvent> = stored.iter().collect();
        sorted.sort_by_key(|entry| entry.insertion_order);

        Ok(sorted
            .into_iter()
            .map(|stored_event| stored_event.event.clone())
            .collect())
    }

    fn switch(&self, from: &SessionId, to: &SessionId) -> Result<SwitchResult, SessionError> {
        let guard = self.events.read().map_err(|_| SessionError::LockPoisoned)?;
        let Some(to_events) = guard.get(to) else {
            return Err(SessionError::UnknownSession {
                session_id: to.clone(),
            });
        };

        let from_event_count = guard.get(from).map_or(0, Vec::len);

        Ok(SwitchResult {
            from_session_id: from.clone(),
            to_session_id: to.clone(),
            from_event_count,
            to_event_count: to_events.len(),
        })
    }
}

pub fn reconstruct_conversation(events: &[RuntimeEvent]) -> Vec<ProviderInputMessage> {
    let mut result = Vec::new();

    for event in events {
        match event {
            RuntimeEvent::UserMessageReceived { content, .. } => {
                result.push(ProviderInputMessage::User {
                    content: content.clone(),
                });
            }
            RuntimeEvent::TurnFinished {
                assistant_message: Some(msg),
                ..
            } => {
                result.push(ProviderInputMessage::Assistant {
                    message: msg.clone(),
                });
            }
            RuntimeEvent::ToolResultReceived {
                tool_call_id,
                is_error,
                result: tool_result,
                ..
            } => {
                result.push(ProviderInputMessage::ToolResult {
                    tool_call_id: tool_call_id.clone(),
                    is_error: *is_error,
                    result: tool_result.clone(),
                });
            }
            _ => {}
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorum_domain::{MessageId, TurnId, TurnTerminalReason};

    #[test]
    fn append_replay_basic_behavior() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::from("session-a");
        let turn_id = TurnId::from("turn-1");

        let started = RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        };
        let finished = RuntimeEvent::TurnFinished {
            turn_id,
            sequence_no: 2,
            reason: TurnTerminalReason::Done,
            message_id: Some(MessageId::from("msg-1")),
            assistant_message: None,
        };

        store.append(&session_id, started.clone()).unwrap();
        store.append(&session_id, finished.clone()).unwrap();

        let replayed = store.replay(&session_id).unwrap();
        assert_eq!(replayed, vec![started, finished]);
    }

    #[test]
    fn replay_order_deterministic_for_interleaved_turns() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::from("session-a");

        let turn_a = TurnId::from("turn-a");
        let turn_b = TurnId::from("turn-b");

        let turn_b_started = RuntimeEvent::TurnStarted {
            turn_id: turn_b.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        };
        let turn_a_started = RuntimeEvent::TurnStarted {
            turn_id: turn_a.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        };
        let switched = RuntimeEvent::SessionSwitched {
            sequence_no: 1,
            from_session_id: SessionId::from("session-a"),
            to_session_id: SessionId::from("session-b"),
        };
        let turn_a_delta_1 = RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_a.clone(),
            sequence_no: 2,
            delta: "a-first".to_string(),
        };
        let turn_a_delta_2 = RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_a,
            sequence_no: 2,
            delta: "a-second".to_string(),
        };
        let turn_b_delta = RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_b,
            sequence_no: 2,
            delta: "b-only".to_string(),
        };

        // Intentionally appended in mixed order; replay must be stable and deterministic.
        store.append(&session_id, turn_b_started.clone()).unwrap();
        store.append(&session_id, turn_a_started.clone()).unwrap();
        store.append(&session_id, switched.clone()).unwrap();
        store.append(&session_id, turn_a_delta_1.clone()).unwrap();
        store.append(&session_id, turn_b_delta.clone()).unwrap();
        store.append(&session_id, turn_a_delta_2.clone()).unwrap();

        let replayed = store.replay(&session_id).unwrap();
        assert_eq!(
            replayed,
            vec![
                turn_b_started,
                turn_a_started,
                switched,
                turn_a_delta_1,
                turn_b_delta,
                turn_a_delta_2,
            ]
        );
    }

    #[test]
    fn switch_result_metadata() {
        let store = InMemorySessionStore::new();
        let from_session = SessionId::from("session-a");
        let to_session = SessionId::from("session-b");

        store
            .append(
                &from_session,
                RuntimeEvent::SessionSwitched {
                    sequence_no: 1,
                    from_session_id: from_session.clone(),
                    to_session_id: to_session.clone(),
                },
            )
            .unwrap();

        store
            .append(
                &to_session,
                RuntimeEvent::TurnStarted {
                    turn_id: TurnId::from("turn-2"),
                    sequence_no: 1,
                    session_id: to_session.clone(),
                },
            )
            .unwrap();

        store
            .append(
                &to_session,
                RuntimeEvent::TurnFinished {
                    turn_id: TurnId::from("turn-2"),
                    sequence_no: 2,
                    reason: TurnTerminalReason::Done,
                    message_id: None,
                    assistant_message: None,
                },
            )
            .unwrap();

        let result = store.switch(&from_session, &to_session).unwrap();

        assert_eq!(result.from_session_id, from_session);
        assert_eq!(result.to_session_id, to_session);
        assert_eq!(result.from_event_count, 1);
        assert_eq!(result.to_event_count, 2);
    }

    #[test]
    fn switching_to_unknown_session_returns_clear_error() {
        let store = InMemorySessionStore::new();
        let from_session = SessionId::from("session-a");
        let unknown = SessionId::from("session-unknown");

        store
            .append(
                &from_session,
                RuntimeEvent::SessionSwitched {
                    sequence_no: 1,
                    from_session_id: from_session.clone(),
                    to_session_id: SessionId::from("session-b"),
                },
            )
            .unwrap();

        let err = store.switch(&from_session, &unknown).unwrap_err();

        assert_eq!(
            err,
            SessionError::UnknownSession {
                session_id: unknown
            }
        );
    }

    #[test]
    fn replay_unknown_session_returns_empty() {
        let store = InMemorySessionStore::new();
        let session_id = SessionId::from("missing");

        let replayed = store.replay(&session_id).unwrap();
        assert!(replayed.is_empty());
    }
}
