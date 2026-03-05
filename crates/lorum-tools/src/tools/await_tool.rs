use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "await".to_string(),
        description: "Wait for a background job to complete.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "jobId": {
                    "type": "string",
                    "description": "Job ID to wait for"
                }
            },
            "required": ["jobId"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let job_id = args
        .get("jobId")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "await".to_string(),
        detail: Some(job_id.to_string()),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, _cwd: &Path) -> ToolOutput {
    match args.get("jobId").and_then(Value::as_str) {
        Some(_) => {}
        None => return ToolOutput::err("missing required parameter: jobId"),
    };

    ToolOutput::err("no background jobs to await")
}
