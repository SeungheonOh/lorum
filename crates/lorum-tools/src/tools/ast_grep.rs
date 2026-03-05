use std::path::Path;
use std::time::Duration;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "ast-grep".to_string(),
        description: "Structural code search using AST pattern matching via ast-grep (sg). \
            Requires the `sg` binary to be installed."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "patterns": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "AST patterns to match"
                },
                "path": {
                    "type": "string",
                    "description": "File, directory, or glob pattern to search"
                },
                "lang": {
                    "type": "string",
                    "description": "Language override (e.g., \"typescript\", \"rust\", \"python\")"
                },
                "selector": {
                    "type": "string",
                    "description": "Optional selector for contextual pattern mode"
                },
                "context": {
                    "type": "integer",
                    "description": "Context lines around matches. Defaults to 0."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max matches to return. Defaults to 50."
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip first N matches"
                }
            },
            "required": ["patterns"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let first_pattern = args
        .get("patterns")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let lang = args.get("lang").and_then(|v| v.as_str());
    let detail = match lang {
        Some(l) => format!("{} ({})", crate::display_preview(first_pattern, 80), l),
        None => crate::display_preview(first_pattern, 80),
    };
    ToolCallSummary {
        headline: "ast-grep".to_string(),
        detail: Some(detail),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        return crate::display_preview(text, 200);
    }
    let lines: Vec<&str> = text.lines().collect();
    let match_count = lines.len();
    if match_count == 0 {
        return "no matches".to_string();
    }
    format!("{} match line(s)", match_count)
}

pub async fn execute(args: Value, cwd: &Path, _default_timeout: Duration) -> ToolOutput {
    let patterns = match args.get("patterns").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => arr,
        Some(_) => return ToolOutput::err("patterns array must not be empty"),
        None => return ToolOutput::err("missing required parameter: patterns"),
    };

    let path = args.get("path").and_then(Value::as_str);
    let lang = args.get("lang").and_then(Value::as_str);
    let selector = args.get("selector").and_then(Value::as_str);
    let context = args.get("context").and_then(Value::as_u64).unwrap_or(0);
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;

    let resolved_path = path.map(|p| {
        let p = Path::new(p);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    });

    let mut combined_output = String::new();

    for pattern_val in patterns {
        let pattern = match pattern_val.as_str() {
            Some(p) => p,
            None => continue,
        };

        let mut cmd = Command::new("sg");
        cmd.arg("--pattern").arg(pattern);

        if let Some(l) = lang {
            cmd.arg("--lang").arg(l);
        }
        if let Some(s) = selector {
            cmd.arg("--selector").arg(s);
        }
        if context > 0 {
            cmd.arg("-A").arg(context.to_string());
            cmd.arg("-B").arg(context.to_string());
        }
        if let Some(ref p) = resolved_path {
            cmd.arg(p);
        }

        cmd.current_dir(cwd);

        let result = cmd.output().await;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if !stdout.is_empty() {
                    if !combined_output.is_empty() {
                        combined_output.push('\n');
                    }
                    combined_output.push_str(&stdout);
                }
                if !stderr.is_empty() && !output.status.success() {
                    if !combined_output.is_empty() {
                        combined_output.push('\n');
                    }
                    combined_output.push_str("STDERR:\n");
                    combined_output.push_str(&stderr);
                }
            }
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("No such file or directory") || msg.contains("not found") {
                    return ToolOutput::err(
                        "ast-grep (sg) binary not found. Install it: npm install -g @ast-grep/cli",
                    );
                }
                return ToolOutput::err(format!("failed to execute sg: {err}"));
            }
        }
    }

    if combined_output.is_empty() {
        return ToolOutput::ok("no matches");
    }

    // Apply offset and limit
    let lines: Vec<&str> = combined_output.lines().collect();
    let after_offset: Vec<&str> = lines.into_iter().skip(offset).collect();
    let limited: Vec<&str> = after_offset.into_iter().take(limit).collect();
    let text = limited.join("\n");

    ToolOutput::ok(text)
}
