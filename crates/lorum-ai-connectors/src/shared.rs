use lorum_ai_contract::{
    AssistantEventSink, AssistantMessageEvent, ProviderError, ProviderInputMessage,
};
use serde_json::Value;

#[derive(Default, Debug, Clone)]
pub struct ToolCallJsonAccumulator {
    buffer: String,
}

impl ToolCallJsonAccumulator {
    pub fn push_chunk(&mut self, chunk: &str) -> Option<Value> {
        self.buffer.push_str(chunk);
        serde_json::from_str::<Value>(&self.buffer).ok()
    }

    pub fn finalize(&self) -> Value {
        serde_json::from_str::<Value>(&self.buffer).unwrap_or(Value::Null)
    }

    pub fn current_buffer(&self) -> &str {
        &self.buffer
    }
}

pub fn coalesce_delta_events(events: Vec<AssistantMessageEvent>) -> Vec<AssistantMessageEvent> {
    let mut output: Vec<AssistantMessageEvent> = Vec::new();

    for event in events {
        if let Some(last) = output.last_mut() {
            match (last, &event) {
                (
                    AssistantMessageEvent::TextDelta(prev),
                    AssistantMessageEvent::TextDelta(curr),
                ) if prev.block_id == curr.block_id => {
                    prev.delta.push_str(&curr.delta);
                    prev.sequence_no = curr.sequence_no;
                    continue;
                }
                (
                    AssistantMessageEvent::ThinkingDelta(prev),
                    AssistantMessageEvent::ThinkingDelta(curr),
                ) if prev.block_id == curr.block_id => {
                    prev.delta.push_str(&curr.delta);
                    prev.sequence_no = curr.sequence_no;
                    continue;
                }
                (
                    AssistantMessageEvent::ToolCallDelta(prev),
                    AssistantMessageEvent::ToolCallDelta(curr),
                ) if prev.block_id == curr.block_id => {
                    prev.delta.push_str(&curr.delta);
                    prev.sequence_no = curr.sequence_no;
                    continue;
                }
                _ => {}
            }
        }

        output.push(event);
    }

    output
}

pub(crate) fn sanitize_tool_call_pairing(messages: &mut Vec<ProviderInputMessage>) {
    lorum_ai_contract::patch_orphaned_tool_calls(messages, "Tool call result unavailable");
}

pub(crate) fn normalize_provider_error(
    code: &str,
    message: &str,
    retryable: bool,
) -> ProviderError {
    let lowered = code.to_ascii_lowercase();
    if lowered.contains("rate") {
        return ProviderError::RateLimited {
            message: message.to_string(),
        };
    }
    if retryable {
        ProviderError::Transport {
            message: message.to_string(),
        }
    } else {
        ProviderError::InvalidResponse {
            message: format!("{code}: {message}"),
        }
    }
}

pub(crate) fn sink_to_provider_error(err: lorum_ai_contract::StreamSinkError) -> ProviderError {
    ProviderError::Transport {
        message: err.to_string(),
    }
}

#[derive(Default)]
pub(crate) struct CollectingSink {
    pub(crate) events: Vec<AssistantMessageEvent>,
}

impl AssistantEventSink for CollectingSink {
    fn push(
        &mut self,
        event: AssistantMessageEvent,
    ) -> Result<(), lorum_ai_contract::StreamSinkError> {
        self.events.push(event);
        Ok(())
    }
}
