use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "resolve".to_string(),
        description: "Resolve a pending preview action by applying or discarding changes."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["apply", "discard"],
                    "description": "Whether to apply or discard the pending changes"
                },
                "reason": {
                    "type": "string",
                    "description": "Explanation for the decision"
                }
            },
            "required": ["action", "reason"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let reason = args
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    ToolCallSummary {
        headline: format!("resolve {action}"),
        detail: Some(crate::display_preview(reason, 60)),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, _cwd: &Path) -> ToolOutput {
    let action = match args.get("action").and_then(Value::as_str) {
        Some(a) => a,
        None => return ToolOutput::err("missing required parameter: action"),
    };

    if action != "apply" && action != "discard" {
        return ToolOutput::err(format!(
            "invalid action '{action}'. Must be 'apply' or 'discard'"
        ));
    }

    match args.get("reason").and_then(Value::as_str) {
        Some(_) => {}
        None => return ToolOutput::err("missing required parameter: reason"),
    };

    ToolOutput::err("no pending action to resolve")
}
