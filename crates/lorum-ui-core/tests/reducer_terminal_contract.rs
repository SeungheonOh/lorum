use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_ui_core::{DefaultUiReducer, UiError, UiReducer};

#[test]
fn duplicate_terminal_event_is_rejected() {
    let mut reducer = DefaultUiReducer::new();
    let turn_id = TurnId::from("turn-1");

    reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        })
        .unwrap();
    reducer
        .apply(&RuntimeEvent::TurnFinished {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        })
        .unwrap();

    let err = reducer
        .apply(&RuntimeEvent::RuntimeError {
            turn_id: turn_id.clone(),
            sequence_no: 3,
            code: "late".to_owned(),
            message: "already closed".to_owned(),
        })
        .unwrap_err();

    assert_eq!(err, UiError::DuplicateTerminal { turn_id });
}

#[test]
fn post_terminal_delta_is_rejected() {
    let mut reducer = DefaultUiReducer::new();
    let turn_id = TurnId::from("turn-2");

    reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        })
        .unwrap();
    reducer
        .apply(&RuntimeEvent::RuntimeError {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            code: "transport".to_owned(),
            message: "stream dropped".to_owned(),
        })
        .unwrap();

    let err = reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 3,
            delta: "late".to_owned(),
        })
        .unwrap_err();

    assert_eq!(err, UiError::PostTerminalEvent { turn_id });
}

#[test]
fn runtime_error_marks_turn_as_completed_with_error_reason() {
    let mut reducer = DefaultUiReducer::new();
    let turn_id = TurnId::from("turn-3");

    reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
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
            message: "stream dropped".to_owned(),
        })
        .unwrap();

    assert_eq!(reducer.state().completed_turns.len(), 1);
    assert_eq!(reducer.state().completed_turns[0].turn_id, turn_id);
    assert_eq!(
        reducer.state().completed_turns[0].reason,
        TurnTerminalReason::Error
    );
}
