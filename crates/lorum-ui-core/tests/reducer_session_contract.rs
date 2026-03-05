use lorum_domain::{RuntimeEvent, SessionId, TurnId};
use lorum_ui_core::{DefaultUiReducer, UiError, UiReducer};

#[test]
fn turn_started_with_non_active_session_is_rejected() {
    let mut reducer = DefaultUiReducer::new();

    reducer
        .apply(&RuntimeEvent::SessionSwitched {
            sequence_no: 1,
            from_session_id: SessionId::from("session-a"),
            to_session_id: SessionId::from("session-a"),
        })
        .unwrap();

    let turn_id = TurnId::from("turn-1");
    let err = reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            session_id: SessionId::from("session-b"),
        })
        .unwrap_err();

    assert_eq!(
        err,
        UiError::SessionMismatch {
            turn_id,
            expected_session: SessionId::from("session-a"),
            actual_session: SessionId::from("session-b"),
        }
    );
}

#[test]
fn session_switch_is_global_and_does_not_trip_turn_ordering() {
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
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            delta: "A".to_owned(),
        })
        .unwrap();

    reducer
        .apply(&RuntimeEvent::SessionSwitched {
            sequence_no: 3,
            from_session_id: SessionId::from("session-a"),
            to_session_id: SessionId::from("session-a"),
        })
        .unwrap();

    reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id,
            sequence_no: 3,
            delta: "B".to_owned(),
        })
        .unwrap();
}

#[test]
fn open_turn_events_after_switch_to_other_session_are_rejected() {
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
        .apply(&RuntimeEvent::SessionSwitched {
            sequence_no: 2,
            from_session_id: SessionId::from("session-a"),
            to_session_id: SessionId::from("session-b"),
        })
        .unwrap();

    let err = reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 3,
            delta: "late".to_owned(),
        })
        .unwrap_err();

    assert_eq!(
        err,
        UiError::SessionMismatch {
            turn_id,
            expected_session: SessionId::from("session-b"),
            actual_session: SessionId::from("session-a"),
        }
    );
}
