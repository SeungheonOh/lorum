use std::error::Error;
use std::fmt::{Display, Formatter};

use lorum_domain::RuntimeEvent;
use serde::{Deserialize, Serialize};

pub const RPC_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RpcMode {
    Chat,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RpcEnvelope {
    Ready {
        protocol_version: u32,
        mode: RpcMode,
    },
    Event {
        event: RuntimeEvent,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug)]
pub enum RpcEncodeError {
    Json(serde_json::Error),
}

impl Display for RpcEncodeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcEncodeError::Json(err) => write!(f, "json encode failed: {err}"),
        }
    }
}

impl Error for RpcEncodeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            RpcEncodeError::Json(err) => Some(err),
        }
    }
}

pub fn ready_envelope() -> RpcEnvelope {
    RpcEnvelope::Ready {
        protocol_version: RPC_PROTOCOL_VERSION,
        mode: RpcMode::Chat,
    }
}

pub fn event_envelope(ev: RuntimeEvent) -> RpcEnvelope {
    RpcEnvelope::Event { event: ev }
}

pub fn encode_envelope_json(env: &RpcEnvelope) -> Result<String, RpcEncodeError> {
    serde_json::to_string(env).map_err(RpcEncodeError::Json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorum_domain::{SessionId, TurnId, TurnTerminalReason};

    fn sample_runtime_event() -> RuntimeEvent {
        RuntimeEvent::TurnFinished {
            turn_id: TurnId::from("turn-1"),
            sequence_no: 7,
            reason: TurnTerminalReason::Done,
            message_id: None,
            assistant_message: None,
        }
    }

    #[test]
    fn ready_envelope_serializes_with_type_and_protocol_version() {
        let encoded = encode_envelope_json(&ready_envelope()).expect("ready envelope encodes");
        let value: serde_json::Value =
            serde_json::from_str(&encoded).expect("ready envelope is json");

        assert_eq!(value["type"], "ready");
        assert_eq!(value["protocol_version"], RPC_PROTOCOL_VERSION);
        assert_eq!(value["mode"], "chat");
    }

    #[test]
    fn event_envelope_preserves_runtime_event_payload() {
        let original_event = sample_runtime_event();
        let envelope = event_envelope(original_event.clone());
        let encoded = encode_envelope_json(&envelope).expect("event envelope encodes");
        let decoded: RpcEnvelope = serde_json::from_str(&encoded).expect("event envelope decodes");

        assert_eq!(
            decoded,
            RpcEnvelope::Event {
                event: original_event
            }
        );
    }

    #[test]
    fn error_envelope_serializes_and_decodes() {
        let envelope = RpcEnvelope::Error {
            code: "invalid_request".to_string(),
            message: "prompt is empty".to_string(),
        };

        let encoded = encode_envelope_json(&envelope).expect("error envelope encodes");
        let decoded: RpcEnvelope = serde_json::from_str(&encoded).expect("error envelope decodes");

        assert_eq!(decoded, envelope);
    }

    #[test]
    fn all_envelope_variants_roundtrip_deserialize() {
        let envelopes = vec![
            ready_envelope(),
            event_envelope(RuntimeEvent::TurnStarted {
                turn_id: TurnId::from("turn-2"),
                sequence_no: 1,
                session_id: SessionId::from("session-a"),
            }),
            RpcEnvelope::Error {
                code: "transport".to_owned(),
                message: "stream closed".to_owned(),
            },
        ];

        for envelope in envelopes {
            let encoded = encode_envelope_json(&envelope).expect("envelope encodes");
            let decoded: RpcEnvelope =
                serde_json::from_str(&encoded).expect("envelope roundtrip decodes");
            assert_eq!(decoded, envelope);
        }
    }
}
