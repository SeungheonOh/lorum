use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ToolOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Checkpoint {
    id: String,
    goal: String,
    timestamp: u64,
}

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "checkpoint".to_string(),
        description: "Create a context checkpoint before exploratory work. \
            Use rewind to replace exploration with a concise report."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "goal": {
                    "type": "string",
                    "description": "Explanation of the investigation"
                }
            },
            "required": ["goal"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let goal = args
        .get("goal")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "checkpoint".to_string(),
        detail: Some(crate::display_preview(goal, 60)),
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
    let goal = match args.get("goal").and_then(Value::as_str) {
        Some(g) => g,
        None => return ToolOutput::err("missing required parameter: goal"),
    };

    let path = checkpoint_path(cwd);

    // Check if a checkpoint already exists
    if path.exists() {
        return ToolOutput::err("a checkpoint is already active");
    }

    let dir = cwd.join(".servus");
    if !dir.exists() {
        if let Err(e) = tokio::fs::create_dir_all(&dir).await {
            return ToolOutput::err(format!("failed to create .servus directory: {e}"));
        }
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let id = format!("chk-{timestamp}");

    let checkpoint = Checkpoint {
        id: id.clone(),
        goal: goal.to_string(),
        timestamp,
    };

    let content = match serde_json::to_string_pretty(&checkpoint) {
        Ok(c) => c,
        Err(e) => return ToolOutput::err(format!("failed to serialize checkpoint: {e}")),
    };

    if let Err(e) = tokio::fs::write(&path, content).await {
        return ToolOutput::err(format!("failed to write checkpoint: {e}"));
    }

    ToolOutput::ok(format!("checkpoint created: {id}"))
}
