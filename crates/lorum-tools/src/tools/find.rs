use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

const DEFAULT_LIMIT: usize = 1000;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "find".to_string(),
        description: "Find files matching a glob pattern. Returns matching file paths sorted \
            by modification time."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match files against (e.g. '**/*.rs')"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in. Defaults to the working directory."
                },
                "hidden": {
                    "type": "boolean",
                    "description": "Include hidden files and directories (starting with '.'). Defaults to true."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Defaults to 1000."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let path = args.get("path").and_then(|v| v.as_str());
    ToolCallSummary {
        headline: format!("find {pattern}"),
        detail: path.map(|p| format!("in {p}")),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error || text == "no files matched the pattern" {
        return text.to_string();
    }
    let count = text.lines().count();
    format!("{count} files")
}

fn has_hidden_component(path: &Path, base: &Path) -> bool {
    let relative = path.strip_prefix(base).unwrap_or(path);
    relative.components().any(|c| {
        c.as_os_str()
            .to_str()
            .map(|s| s.starts_with('.'))
            .unwrap_or(false)
    })
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let pattern = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolOutput::err("missing required parameter: pattern"),
    };

    let hidden = args
        .get("hidden")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_LIMIT);

    let search_dir = args
        .get("path")
        .and_then(Value::as_str)
        .map(|p| {
            let path = Path::new(p);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(p)
            }
        })
        .unwrap_or_else(|| cwd.to_path_buf());

    // Build the full glob pattern
    let full_pattern = search_dir.join(pattern);
    let full_pattern_str = full_pattern.to_string_lossy();

    // tokio::task::spawn_blocking for the glob operation
    let pattern_owned = full_pattern_str.to_string();
    let base_dir = search_dir.clone();
    let result = tokio::task::spawn_blocking(move || {
        let mut matches = Vec::new();
        let entries = match ::glob::glob(&pattern_owned) {
            Ok(entries) => entries,
            Err(err) => return Err(format!("invalid glob pattern: {err}")),
        };

        for entry in entries {
            match entry {
                Ok(path) => {
                    if !hidden && has_hidden_component(&path, &base_dir) {
                        continue;
                    }
                    matches.push(path.to_string_lossy().to_string());
                    if matches.len() >= limit {
                        break;
                    }
                }
                Err(err) => {
                    // Skip unreadable entries
                    eprintln!("find entry error: {err}");
                }
            }
        }

        Ok(matches)
    })
    .await;

    match result {
        Ok(Ok(matches)) => {
            if matches.is_empty() {
                ToolOutput::ok("no files matched the pattern")
            } else {
                let truncated = matches.len() >= limit;
                let mut output = matches.join("\n");
                if truncated {
                    output.push_str(&format!("\n\n[results truncated at {limit} matches]"));
                }
                ToolOutput::ok(output)
            }
        }
        Ok(Err(err)) => ToolOutput::err(err),
        Err(err) => ToolOutput::err(format!("find task failed: {err}")),
    }
}
