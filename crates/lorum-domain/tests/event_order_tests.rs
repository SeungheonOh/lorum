use lorum_domain::{
    validate_turn_event_order, EventOrderError, MessageId, RuntimeEvent, SessionId, TurnId,
    TurnTerminalReason,
};

#[test]
fn accepts_valid_interleaved_turns() {
    let events = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-2"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 2,
            delta: "hello".to_string(),
        },
        RuntimeEvent::SessionSwitched {
            sequence_no: 99,
            from_session_id: SessionId::from("session-a"),
            to_session_id: SessionId::from("session-b"),
        },
        RuntimeEvent::RuntimeError {
            turn_id: TurnId::from("turn-2"),
            sequence_no: 2,
            code: "timeout".to_string(),
            message: "network timeout".to_string(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 3,
            reason: TurnTerminalReason::Done,
            message_id: Some(MessageId::from("msg-1")),
            assistant_message: None,
        },
    ];

    assert_eq!(validate_turn_event_order(&events), Ok(()));
}

#[test]
fn rejects_sequence_regression() {
    let events = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 2,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 1,
            delta: "late".to_string(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 3,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        },
    ];

    assert!(matches!(
        validate_turn_event_order(&events),
        Err(EventOrderError::SequenceRegression { .. })
    ));
}

#[test]
fn rejects_missing_terminal() {
    let events = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 2,
            delta: "no terminal".to_string(),
        },
    ];

    assert!(matches!(
        validate_turn_event_order(&events),
        Err(EventOrderError::MissingTerminal { .. })
    ));
}

#[test]
fn rejects_duplicate_terminal() {
    let events = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 2,
            reason: TurnTerminalReason::Done,
            message_id: Some(MessageId::from("msg-1")),
            assistant_message: None,
        },
        RuntimeEvent::RuntimeError {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 3,
            code: "unexpected".to_string(),
            message: "second terminal".to_string(),
        },
    ];

    assert!(matches!(
        validate_turn_event_order(&events),
        Err(EventOrderError::DuplicateTerminal { .. })
    ));
}
