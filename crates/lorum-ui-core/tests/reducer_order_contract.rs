use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_ui_core::{DefaultUiReducer, UiError, UiReducer};

#[test]
fn interleaved_monotonic_sequences_are_accepted() {
    let mut reducer = DefaultUiReducer::new();
    let session_id = SessionId::from("session-a");
    let turn_a = TurnId::from("turn-a");
    let turn_b = TurnId::from("turn-b");

    let events = [
        RuntimeEvent::TurnStarted {
            turn_id: turn_a.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        },
        RuntimeEvent::TurnStarted {
            turn_id: turn_b.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_b.clone(),
            sequence_no: 2,
            delta: "B".to_owned(),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_a.clone(),
            sequence_no: 2,
            delta: "A".to_owned(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: turn_a,
            sequence_no: 3,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        },
        RuntimeEvent::TurnFinished {
            turn_id: turn_b,
            sequence_no: 3,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        },
    ];

    for event in &events {
        reducer.apply(event).unwrap();
    }

    assert_eq!(reducer.state().completed_turns.len(), 2);
}

#[test]
fn sequence_regression_is_rejected() {
    let mut reducer = DefaultUiReducer::new();
    let session_id = SessionId::from("session-a");
    let turn_id = TurnId::from("turn-1");

    reducer
        .apply(&RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 10,
            session_id,
        })
        .unwrap();

    let err = reducer
        .apply(&RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 9,
            delta: "out-of-order".to_owned(),
        })
        .unwrap_err();

    assert_eq!(
        err,
        UiError::SequenceRegression {
            turn_id,
            previous: 10,
            current: 9,
        }
    );
}

#[test]
fn equivalent_event_streams_produce_identical_state() {
    let events = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 2,
            delta: "hello".to_owned(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 3,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        },
    ];

    let mut reducer_a = DefaultUiReducer::new();
    let mut reducer_b = DefaultUiReducer::new();

    for event in &events {
        reducer_a.apply(event).unwrap();
        reducer_b.apply(event).unwrap();
    }

    assert_eq!(reducer_a.state(), reducer_b.state());
}
