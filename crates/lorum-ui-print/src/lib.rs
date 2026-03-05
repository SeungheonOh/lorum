use std::error::Error;
use std::fmt::{Display, Formatter};

use lorum_domain::{RuntimeEvent, TurnTerminalReason};
use serde::Serialize;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_ABORTED: i32 = 10;
pub const EXIT_RUNTIME_ERROR: i32 = 20;

#[derive(Debug)]
pub enum PrintRenderError {
    Serialize(serde_json::Error),
}

impl Display for PrintRenderError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Serialize(err) => write!(f, "failed to serialize runtime event: {err}"),
        }
    }
}

impl Error for PrintRenderError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Serialize(err) => Some(err),
        }
    }
}

pub fn print_exit_code(events: &[RuntimeEvent]) -> i32 {
    for event in events.iter().rev() {
        match event {
            RuntimeEvent::RuntimeError { .. } => return EXIT_RUNTIME_ERROR,
            RuntimeEvent::TurnFinished { reason, .. } => {
                return match reason {
                    TurnTerminalReason::Done | TurnTerminalReason::ToolUse => EXIT_SUCCESS,
                    TurnTerminalReason::Aborted => EXIT_ABORTED,
                    TurnTerminalReason::Error => EXIT_RUNTIME_ERROR,
                };
            }
            _ => {}
        }
    }

    EXIT_SUCCESS
}

pub fn render_text(events: &[RuntimeEvent]) -> String {
    let mut transcript = String::new();

    for event in events {
        if let RuntimeEvent::AssistantStreamDelta { delta, .. } = event {
            transcript.push_str(delta);
        }
    }

    let summary = events
        .iter()
        .rev()
        .find_map(|event| match event {
            RuntimeEvent::TurnFinished { reason, .. } => Some(match reason {
                TurnTerminalReason::Done => "status: done".to_string(),
                TurnTerminalReason::ToolUse => "status: tool_use".to_string(),
                TurnTerminalReason::Aborted => "status: aborted".to_string(),
                TurnTerminalReason::Error => "status: error".to_string(),
            }),
            RuntimeEvent::RuntimeError { code, message, .. } => {
                Some(format!("status: runtime_error ({code}): {message}"))
            }
            _ => None,
        })
        .unwrap_or_else(|| "status: incomplete".to_string());

    if transcript.is_empty() {
        summary
    } else {
        format!("{transcript}\n{summary}")
    }
}

pub fn render_json_lines(events: &[RuntimeEvent]) -> Result<String, PrintRenderError> {
    #[derive(Serialize)]
    struct Envelope<'a> {
        event: &'a RuntimeEvent,
    }

    let mut lines = Vec::with_capacity(events.len());
    for event in events {
        let line =
            serde_json::to_string(&Envelope { event }).map_err(PrintRenderError::Serialize)?;
        lines.push(line);
    }

    Ok(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use lorum_domain::{MessageId, SessionId, TurnId};

    use super::*;

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
}
