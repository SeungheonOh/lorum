use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "exit-plan-mode".to_string(),
        description: "Signal plan completion and request user approval.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Plan title"
                }
            },
            "required": ["title"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let title = args
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "exit-plan-mode".to_string(),
        detail: Some(title.to_string()),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, _cwd: &Path) -> ToolOutput {
    match args.get("title").and_then(Value::as_str) {
        Some(_) => {}
        None => return ToolOutput::err("missing required parameter: title"),
    };

    ToolOutput::err("plan mode is not active")
}
