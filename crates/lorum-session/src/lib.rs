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

    lorum_ai_contract::patch_orphaned_tool_calls(
        &mut result,
        "Tool result unavailable: session recovered from incomplete state",
    );

    result
}
