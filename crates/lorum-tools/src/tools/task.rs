use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

const VALID_AGENTS: &[&str] = &[
    "explore", "plan", "reviewer", "task", "designer", "oracle", "librarian",
];

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "task".to_string(),
        description: "Launch subagents to parallelize workflows.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "agent": {
                    "type": "string",
                    "enum": VALID_AGENTS,
                    "description": "Agent type to launch"
                },
                "tasks": {
                    "type": "array",
                    "description": "Tasks to assign to the agent",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": {
                                "type": "string",
                                "description": "Unique task identifier"
                            },
                            "description": {
                                "type": "string",
                                "description": "Task description"
                            },
                            "assignment": {
                                "type": "string",
                                "description": "Specific assignment details"
                            }
                        },
                        "required": ["id", "description"]
                    }
                },
                "context": {
                    "type": "string",
                    "description": "Shared background context for all tasks"
                },
                "schema": {
                    "type": "object",
                    "description": "Expected output schema"
                },
                "isolated": {
                    "type": "boolean",
                    "description": "Run in isolated environment"
                }
            },
            "required": ["agent", "tasks"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let agent = args
        .get("agent")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let task_count = args
        .get("tasks")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    ToolCallSummary {
        headline: format!("task ({agent})"),
        detail: Some(format!("{task_count} task(s)")),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    crate::display_preview(text, 200)
}
