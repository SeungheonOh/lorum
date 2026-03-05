use lorum_domain::{MessageId, RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_session::{InMemorySessionStore, SessionStore};

fn replay_twice(
    store: &dyn SessionStore,
    session_id: &SessionId,
) -> (Vec<RuntimeEvent>, Vec<RuntimeEvent>) {
    let first = store
        .replay(session_id)
        .expect("first replay should succeed");
    let second = store
        .replay(session_id)
        .expect("second replay should succeed");
    (first, second)
}

#[test]
fn replay_is_deterministic_across_repeated_reads() {
    let store = InMemorySessionStore::new();
    let session_id = SessionId::from("session-a");
    let turn_id = TurnId::from("turn-1");

    let events = vec![
        RuntimeEvent::TurnStarted {
            turn_id: turn_id.clone(),
            sequence_no: 1,
            session_id: session_id.clone(),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: turn_id.clone(),
            sequence_no: 2,
            delta: "hello".to_string(),
        },
        RuntimeEvent::TurnFinished {
            turn_id,
            sequence_no: 3,
            reason: TurnTerminalReason::Done,
            message_id: Some(MessageId::from("msg-1")),
            assistant_message: None,
        },
    ];

    for event in &events {
        store
            .append(&session_id, event.clone())
            .expect("append should succeed");
    }

    let (first, second) = replay_twice(&store, &session_id);
    assert_eq!(first, events);
    assert_eq!(second, events);
}

#[test]
fn replay_unknown_session_returns_empty_vector() {
    let store = InMemorySessionStore::new();
    let replayed = store
        .replay(&SessionId::from("missing"))
        .expect("unknown session replay should not error");

    assert!(replayed.is_empty());
}

#[test]
fn switch_from_a_to_b_keeps_metadata_and_b_replay_stable() {
    let store = InMemorySessionStore::new();
    let session_a = SessionId::from("session-a");
    let session_b = SessionId::from("session-b");
    let turn_b = TurnId::from("turn-b");

    let a_event = RuntimeEvent::SessionSwitched {
        sequence_no: 1,
        from_session_id: session_a.clone(),
        to_session_id: session_b.clone(),
    };
    let b_started = RuntimeEvent::TurnStarted {
        turn_id: turn_b.clone(),
        sequence_no: 1,
        session_id: session_b.clone(),
    };
    let b_finished = RuntimeEvent::TurnFinished {
        turn_id: turn_b,
        sequence_no: 2,
        reason: TurnTerminalReason::Done,
        message_id: None,
        assistant_message: None,
    };

    store
        .append(&session_a, a_event)
        .expect("append to session A should succeed");
    store
        .append(&session_b, b_started.clone())
        .expect("append first event to session B should succeed");
    store
        .append(&session_b, b_finished.clone())
        .expect("append second event to session B should succeed");

    let switched = store
        .switch(&session_a, &session_b)
        .expect("switch to existing session should succeed");

    assert_eq!(switched.from_session_id, session_a);
    assert_eq!(switched.to_session_id, session_b.clone());
    assert_eq!(switched.from_event_count, 1);
    assert_eq!(switched.to_event_count, 2);

    let (first, second) = replay_twice(&store, &session_b);
    assert_eq!(first, vec![b_started, b_finished]);
    assert_eq!(second, first);
}

#[test]
fn interleaved_turn_replay_is_complete_and_ordered_with_terminal_events() {
    let store = InMemorySessionStore::new();
    let session_id = SessionId::from("session-z");
    let turn_a = TurnId::from("turn-a");
    let turn_b = TurnId::from("turn-b");

    let switched = RuntimeEvent::SessionSwitched {
        sequence_no: 1,
        from_session_id: SessionId::from("session-x"),
        to_session_id: session_id.clone(),
    };

    let turn_a_started = RuntimeEvent::TurnStarted {
        turn_id: turn_a.clone(),
        sequence_no: 1,
        session_id: session_id.clone(),
    };
    let turn_a_delta_1 = RuntimeEvent::AssistantStreamDelta {
        turn_id: turn_a.clone(),
        sequence_no: 2,
        delta: "a-1".to_string(),
    };
    let turn_a_delta_2 = RuntimeEvent::AssistantStreamDelta {
        turn_id: turn_a.clone(),
        sequence_no: 2,
        delta: "a-2".to_string(),
    };
    let turn_a_finished = RuntimeEvent::TurnFinished {
        turn_id: turn_a,
        sequence_no: 3,
        reason: TurnTerminalReason::Done,
        message_id: Some(MessageId::from("msg-a")),
        assistant_message: None,
    };

    let turn_b_started = RuntimeEvent::TurnStarted {
        turn_id: turn_b.clone(),
        sequence_no: 1,
        session_id: session_id.clone(),
    };
    let turn_b_delta = RuntimeEvent::AssistantStreamDelta {
        turn_id: turn_b.clone(),
        sequence_no: 2,
        delta: "b-1".to_string(),
    };
    let turn_b_finished = RuntimeEvent::TurnFinished {
        turn_id: turn_b,
        sequence_no: 3,
        reason: TurnTerminalReason::Done,
        message_id: Some(MessageId::from("msg-b")),
        assistant_message: None,
    };

    let expected = vec![
        turn_b_started.clone(),
        turn_a_delta_2.clone(),
        switched.clone(),
        turn_a_started.clone(),
        turn_b_delta.clone(),
        turn_a_delta_1.clone(),
        turn_b_finished.clone(),
        turn_a_finished.clone(),
    ];

    for event in expected.clone() {
        store
            .append(&session_id, event)
            .expect("interleaved append should succeed");
    }

    let (first, second) = replay_twice(&store, &session_id);
    assert_eq!(first, expected);
    assert_eq!(second, first);
}
