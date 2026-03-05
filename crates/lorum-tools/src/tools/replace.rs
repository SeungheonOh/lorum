use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "replace".to_string(),
        description: "Performs string replacements in files. The old_text must be found in the \
            file. By default replaces the first unique occurrence."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file to edit"
                },
                "old_text": {
                    "type": "string",
                    "description": "The exact text to find and replace. Must match in the file."
                },
                "new_text": {
                    "type": "string",
                    "description": "The replacement text"
                },
                "all": {
                    "type": "boolean",
                    "description": "Replace all occurrences instead of requiring a unique match. Defaults to false."
                }
            },
            "required": ["path", "old_text", "new_text"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let old = args
        .get("old_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let new = args
        .get("new_text")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let body = if !old.is_empty() || !new.is_empty() {
        let mut lines = Vec::new();
        for l in old.lines() {
            lines.push(format!("-{l}"));
        }
        for l in new.lines() {
            lines.push(format!("+{l}"));
        }
        Some(lines.join("\n"))
    } else {
        None
    };
    ToolCallSummary {
        headline: format!("replace {path}"),
        detail: Some(format!(
            "'{}' -> '{}'",
            crate::display_preview(old, 30),
            crate::display_preview(new, 30)
        )),
        body,
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
    let old_text = match args.get("old_text").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolOutput::err("missing required parameter: old_text"),
    };
    let new_text = match args.get("new_text").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolOutput::err("missing required parameter: new_text"),
    };
    let replace_all = args
        .get("all")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let path = super::read::resolve_path(file_path, cwd);

    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(err) => return ToolOutput::err(format!("failed to read {}: {err}", path.display())),
    };

    let match_count = content.matches(old_text).count();
    if match_count == 0 {
        return ToolOutput::err(format!(
            "old_text not found in {}. Make sure the string matches exactly.",
            path.display()
        ));
    }

    if replace_all {
        let new_content = content.replace(old_text, new_text);
        match tokio::fs::write(&path, &new_content).await {
            Ok(()) => ToolOutput::ok(format!(
                "replaced {match_count} occurrence{} in {}",
                if match_count == 1 { "" } else { "s" },
                path.display()
            )),
            Err(err) => ToolOutput::err(format!("failed to write {}: {err}", path.display())),
        }
    } else {
        if match_count > 1 {
            return ToolOutput::err(format!(
                "old_text found {match_count} times in {}. Provide more context to make the match unique, or set all=true.",
                path.display()
            ));
        }

        let new_content = content.replacen(old_text, new_text, 1);
        match tokio::fs::write(&path, &new_content).await {
            Ok(()) => ToolOutput::ok(format!("replaced in {}", path.display())),
            Err(err) => ToolOutput::err(format!("failed to write {}: {err}", path.display())),
        }
    }
}
