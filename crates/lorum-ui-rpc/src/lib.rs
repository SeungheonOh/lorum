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
