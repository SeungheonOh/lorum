use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "rewind".to_string(),
        description: "End active checkpoint and provide findings report.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "report": {
                    "type": "string",
                    "description": "Concise findings from the investigation"
                }
            },
            "required": ["report"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let report = args
        .get("report")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "rewind".to_string(),
        detail: Some(crate::display_preview(report, 60)),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    crate::display_preview(text, 200)
}

fn checkpoint_path(cwd: &Path) -> std::path::PathBuf {
    cwd.join(".servus").join("checkpoint.json")
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let report = match args.get("report").and_then(Value::as_str) {
        Some(r) => r,
        None => return ToolOutput::err("missing required parameter: report"),
    };

    let path = checkpoint_path(cwd);

    // Check if a checkpoint exists
    if !path.exists() {
        return ToolOutput::err("no active checkpoint to rewind");
    }

    // Delete the checkpoint file
    if let Err(e) = tokio::fs::remove_file(&path).await {
        return ToolOutput::err(format!("failed to remove checkpoint: {e}"));
    }

    ToolOutput::ok(format!("checkpoint rewound. report: {report}"))
}
