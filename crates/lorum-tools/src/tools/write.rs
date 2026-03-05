use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "write".to_string(),
        description: "Write content to a file. Creates the file and parent directories if they \
            don't exist. Overwrites existing content."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "The content to write to the file"
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let bytes = args
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| s.len());
    ToolCallSummary {
        headline: format!("write {path}"),
        detail: bytes.map(|n| format!("{n} bytes")),
        body: None,
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    result.as_str().unwrap_or("").to_string()
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let file_path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolOutput::err("missing required parameter: path"),
    };
    let content = match args.get("content").and_then(Value::as_str) {
        Some(c) => c,
        None => return ToolOutput::err("missing required parameter: content"),
    };

    let path = super::read::resolve_path(file_path, cwd);

    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            return ToolOutput::err(format!(
                "failed to create directory {}: {err}",
                parent.display()
            ));
        }
    }

    match tokio::fs::write(&path, content).await {
        Ok(()) => {
            let line_count = content.lines().count();
            ToolOutput::ok(format!(
                "wrote {} lines to {}",
                line_count,
                path.display()
            ))
        }
        Err(err) => ToolOutput::err(format!("failed to write {}: {err}", path.display())),
    }
}
