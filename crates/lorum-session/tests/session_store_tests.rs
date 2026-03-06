use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantMessage, ModelRef, ProviderInputMessage, StopReason,
    TokenUsage, ToolCall,
};
use lorum_domain::{MessageId, RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_session::{
    reconstruct_conversation, InMemorySessionStore, SessionError, SessionStore,
};

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

#[test]
fn reconstruct_with_orphaned_tool_calls_injects_synthetic_results() {
    let events = vec![
        RuntimeEvent::UserMessageReceived {
            turn_id: TurnId::from("t1"),
            session_id: SessionId::from("s1"),
            sequence_no: 1,
            content: "hi".to_string(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("t1"),
            sequence_no: 2,
            reason: TurnTerminalReason::ToolUse,
            message_id: Some(MessageId::from("m1")),
            assistant_message: Some(AssistantMessage {
                message_id: "m1".to_string(),
                model: ModelRef {
                    provider: "mock".to_string(),
                    api: ApiKind::OpenAiResponses,
                    model: "test".to_string(),
                },
                content: vec![AssistantContent::ToolCall(ToolCall {
                    id: "tc-orphan".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({}),
                })],
                usage: TokenUsage::default(),
                stop_reason: StopReason::ToolUse,
            }),
        },
    ];

    let result = reconstruct_conversation(&events);

    assert_eq!(result.len(), 3);
    match &result[2] {
        ProviderInputMessage::ToolResult {
            tool_call_id,
            is_error,
            ..
        } => {
            assert_eq!(tool_call_id, "tc-orphan");
            assert!(is_error);
        }
        other => panic!("expected synthetic ToolResult, got {:?}", other),
    }
}

#[test]
fn reconstruct_with_matched_tool_calls_unchanged() {
    let events = vec![
        RuntimeEvent::UserMessageReceived {
            turn_id: TurnId::from("t1"),
            session_id: SessionId::from("s1"),
            sequence_no: 1,
            content: "hi".to_string(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("t1"),
            sequence_no: 2,
            reason: TurnTerminalReason::ToolUse,
            message_id: Some(MessageId::from("m1")),
            assistant_message: Some(AssistantMessage {
                message_id: "m1".to_string(),
                model: ModelRef {
                    provider: "mock".to_string(),
                    api: ApiKind::OpenAiResponses,
                    model: "test".to_string(),
                },
                content: vec![AssistantContent::ToolCall(ToolCall {
                    id: "tc-1".to_string(),
                    name: "read".to_string(),
                    arguments: serde_json::json!({}),
                })],
                usage: TokenUsage::default(),
                stop_reason: StopReason::ToolUse,
            }),
        },
        RuntimeEvent::ToolResultReceived {
            turn_id: TurnId::from("t1"),
            sequence_no: 3,
            tool_call_id: "tc-1".to_string(),
            is_error: false,
            result: serde_json::json!("file content"),
        },
    ];

    let result = reconstruct_conversation(&events);

    // User + Assistant + ToolResult = 3 messages, no synthetics injected
    assert_eq!(result.len(), 3);
    match &result[2] {
        ProviderInputMessage::ToolResult {
            tool_call_id,
            is_error,
            result,
        } => {
            assert_eq!(tool_call_id, "tc-1");
            assert!(!is_error);
            assert_eq!(result, &serde_json::json!("file content"));
        }
        other => panic!("expected original ToolResult, got {:?}", other),
    }
}
