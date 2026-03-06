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
