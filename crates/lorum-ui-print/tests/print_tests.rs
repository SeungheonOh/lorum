use lorum_domain::{MessageId, RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_ui_print::{print_exit_code, render_json_lines, render_text, EXIT_ABORTED, EXIT_RUNTIME_ERROR, EXIT_SUCCESS};

fn successful_events() -> Vec<RuntimeEvent> {
    vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 2,
            delta: "Hello".to_owned(),
        },
        RuntimeEvent::SessionSwitched {
            sequence_no: 3,
            from_session_id: SessionId::from("session-a"),
            to_session_id: SessionId::from("session-b"),
        },
        RuntimeEvent::AssistantStreamDelta {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 4,
            delta: " world".to_owned(),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 5,
            reason: TurnTerminalReason::Done,
            message_id: Some(MessageId::from("msg-1")),
            assistant_message: None,
        },
    ]
}

#[test]
fn successful_stream_has_zero_exit_and_text_deltas() {
    let events = successful_events();

    assert_eq!(print_exit_code(&events), EXIT_SUCCESS);

    let text = render_text(&events);
    assert!(text.contains("Hello world"));
    assert!(text.contains("status: done"));
    assert!(!text.contains("session-a"));
    assert!(!text.contains("session-b"));
}

#[test]
fn aborted_and_runtime_error_have_non_zero_exit_codes() {
    let aborted = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-2"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-2"),
            sequence_no: 2,
            reason: TurnTerminalReason::Aborted,
            message_id: None,
            assistant_message: None,
        },
    ];

    let errored = vec![
        RuntimeEvent::TurnStarted {
            turn_id: TurnId::from("turn-3"),
            sequence_no: 1,
            session_id: SessionId::from("session-a"),
        },
        RuntimeEvent::RuntimeError {
            turn_id: TurnId::from("turn-3"),
            sequence_no: 2,
            code: "transport".to_owned(),
            message: "stream dropped".to_owned(),
        },
    ];

    assert_ne!(print_exit_code(&aborted), EXIT_SUCCESS);
    assert_eq!(print_exit_code(&aborted), EXIT_ABORTED);

    assert_ne!(print_exit_code(&errored), EXIT_SUCCESS);
    assert_eq!(print_exit_code(&errored), EXIT_RUNTIME_ERROR);
}

#[test]
fn json_lines_roundtrip_count_matches_event_count() {
    let events = successful_events();

    let output = render_json_lines(&events).expect("json lines should serialize");

    let parsed: Vec<RuntimeEvent> = output
        .lines()
        .map(|line| {
            let envelope: serde_json::Value =
                serde_json::from_str(line).expect("line should be valid json");
            serde_json::from_value(envelope["event"].clone())
                .expect("event payload should parse")
        })
        .collect();

    assert_eq!(parsed.len(), events.len());
    assert_eq!(parsed, events);
}
