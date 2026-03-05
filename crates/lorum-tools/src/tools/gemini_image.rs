use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "gemini-image".to_string(),
        description: "Generate or edit images using Gemini image models. Requires \
            GEMINI_API_KEY environment variable."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "subject": {
                    "type": "string",
                    "description": "Detailed image prompt describing what to generate or how to edit"
                },
                "input": {
                    "type": "array",
                    "description": "Input images for editing, each with a url field",
                    "items": {
                        "type": "object",
                        "properties": {
                            "url": {
                                "type": "string",
                                "description": "URL or path to the input image"
                            }
                        }
                    }
                }
            },
            "required": ["subject"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let subject = args
        .get("subject")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "gemini-image".to_string(),
        detail: Some(crate::display_preview(subject, 50)),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        return crate::display_preview(text, 200);
    }
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, _cwd: &Path) -> ToolOutput {
    let _subject = match args.get("subject").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolOutput::err("missing required parameter: subject"),
    };

    let api_key = std::env::var("GEMINI_API_KEY");
    if api_key.is_err() {
        return ToolOutput::err(
            "gemini-image requires GEMINI_API_KEY environment variable",
        );
    }

    // No real Gemini integration yet.
    ToolOutput::err("gemini-image integration not yet implemented")
}
