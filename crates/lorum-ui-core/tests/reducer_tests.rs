use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_ui_core::{
    CompletedTurn, DefaultUiReducer, RuntimeErrorSnapshot, UiReducer,
};

#[test]
fn accumulates_stream_and_completes_turn() {
    let mut reducer = DefaultUiReducer::new();
    let turn_id = TurnId::from("turn-1");
    let session_id = SessionId::from("session-a");

    reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        })
        .unwrap();

    reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            delta: "Hello".to_owned(),
        })
        .unwrap();

    reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 3,
            delta: " world".to_owned(),
        })
        .unwrap();

    reducer
        .apply(&RuntimeEvent::TurnFinished {
            turn_id: turn_id.clone(),
            sequence_no: 4,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        })
        .unwrap();

    let state = reducer.state();
    assert!(state.turn_buffers.is_empty());
    assert_eq!(state.completed_turns.len(), 1);
    assert_eq!(
        state.turn_states.get(&turn_id).map(|s| s.terminal_seen),
        Some(true)
    );
    assert_eq!(
        state.completed_turns[0],
        CompletedTurn {
            turn_id,
            session_id,
            assistant_text: "Hello world".to_owned(),
            reason: TurnTerminalReason::Done,
            message_id: None,
        }
    );
}

#[test]
fn runtime_error_updates_structured_error_and_terminates_turn() {
    let mut reducer = DefaultUiReducer::new();
    let turn_id = TurnId::from("turn-1");
    let session_id = SessionId::from("session-a");

    reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id,
        })
        .unwrap();
    reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            delta: "partial".to_owned(),
        })
        .unwrap();

    reducer
        .apply(&RuntimeEvent::RuntimeError {
            turn_id: turn_id.clone(),
            sequence_no: 3,
            code: "transport".to_owned(),
            message: "stream aborted".to_owned(),
        })
        .unwrap();

    assert_eq!(
        reducer.state().last_error,
        Some(RuntimeErrorSnapshot {
            code: "transport".to_owned(),
            message: "stream aborted".to_owned(),
            turn_id,
            sequence_no: 3,
        })
    );
    assert!(reducer.state().turn_buffers.is_empty());
    assert_eq!(reducer.state().completed_turns.len(), 1);
    assert_eq!(
        reducer.state().completed_turns[0].reason,
        TurnTerminalReason::Error
    );
}
