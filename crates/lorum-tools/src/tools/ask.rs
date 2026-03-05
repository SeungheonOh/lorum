use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "ask".to_string(),
        description: "Ask the user for clarification or input. Returns user's response.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "description": "List of questions to ask the user",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique identifier for the question"
                            },
                            "question": {
                                "type": "string",
                                "description": "The question text"
                            },
                            "options": {
                                "type": "array",
                                "description": "Optional list of selectable options",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {
                                            "type": "string",
                                            "description": "Display label for the option"
                                        }
                                    },
                                    "required": ["label"]
                                }
                            },
                            "recommended": {
                                "type": "integer",
                                "description": "Index of the recommended option"
                            },
                            "multi": {
                                "type": "boolean",
                                "description": "Whether multiple options can be selected"
                            }
                        },
                        "required": ["id", "question"]
                    }
                }
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let detail = args
        .get("questions")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|q| q.get("question"))
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "ask".to_string(),
        detail: Some(crate::display_preview(detail, 60)),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, _cwd: &Path) -> ToolOutput {
    let questions = match args.get("questions").and_then(Value::as_array) {
        Some(q) if !q.is_empty() => q,
        Some(_) => return ToolOutput::err("questions array must not be empty"),
        None => return ToolOutput::err("missing required parameter: questions"),
    };

    // Validate each question has id and question fields
    for (i, q) in questions.iter().enumerate() {
        if q.get("id").and_then(Value::as_str).is_none() {
            return ToolOutput::err(format!("question at index {i} is missing required field: id"));
        }
        if q.get("question").and_then(Value::as_str).is_none() {
            return ToolOutput::err(format!(
                "question at index {i} is missing required field: question"
            ));
        }
    }

    ToolOutput::err(
        "ask tool requires interactive mode. User input cannot be collected during tool execution.",
    )
}
