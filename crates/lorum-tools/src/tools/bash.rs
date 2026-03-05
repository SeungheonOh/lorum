use std::path::Path;
use std::time::Duration;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "bash".to_string(),
        description: "Execute a bash command and return its output (stdout and stderr). \
            Commands run in a shell via `sh -c`. \
            Long-running commands will be killed after the timeout."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds. Defaults to 120."
                },
                "cwd": {
                    "type": "string",
                    "description": "Working directory override. If provided, use instead of the default cwd. Relative paths are resolved against the default cwd."
                },
                "head": {
                    "type": "integer",
                    "description": "Return only the first N lines of output."
                },
                "tail": {
                    "type": "integer",
                    "description": "Return only the last N lines of output."
                }
            },
            "required": ["command"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let cwd = args.get("cwd").and_then(|v| v.as_str());
    let detail = match cwd {
        Some(dir) => format!("$ {} (in {})", crate::display_preview(command, 80), dir),
        None => format!("$ {}", crate::display_preview(command, 80)),
    };
    ToolCallSummary {
        headline: "bash".to_string(),
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
    if lines.len() <= 3 {
        return crate::display_preview(text, 200);
    }
    format!("{} ({} lines)", crate::display_preview(lines[0], 80), lines.len())
}

pub async fn execute(args: Value, cwd: &Path, default_timeout: Duration) -> ToolOutput {
    let command = match args.get("command").and_then(Value::as_str) {
        Some(c) => c,
        None => return ToolOutput::err("missing required parameter: command"),
    };

    let timeout = args
        .get("timeout")
        .and_then(Value::as_u64)
        .map(Duration::from_secs)
        .unwrap_or(default_timeout);

    let head = args.get("head").and_then(Value::as_u64).map(|n| n as usize);
    let tail = args.get("tail").and_then(Value::as_u64).map(|n| n as usize);

    let effective_cwd = match args.get("cwd").and_then(Value::as_str) {
        Some(dir) => super::read::resolve_path(dir, cwd),
        None => cwd.to_path_buf(),
    };

    let result = tokio::time::timeout(
        timeout,
        Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&effective_cwd)
            .output(),
    )
    .await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut text = String::new();
            if !stdout.is_empty() {
                text.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str("STDERR:\n");
                text.push_str(&stderr);
            }

            if text.is_empty() {
                text.push_str("(no output)");
            }

            // Truncate very long output
            const MAX_OUTPUT: usize = 100_000;
            if text.len() > MAX_OUTPUT {
                text.truncate(MAX_OUTPUT);
                text.push_str("\n[output truncated]");
            }

            // Apply head/tail line truncation
            if head.is_some() || tail.is_some() {
                let mut lines: Vec<&str> = text.lines().collect();
                if let Some(h) = head {
                    if h < lines.len() {
                        lines.truncate(h);
                    }
                }
                if let Some(t) = tail {
                    if t < lines.len() {
                        lines = lines[lines.len() - t..].to_vec();
                    }
                }
                text = lines.join("\n");
                if !text.is_empty() {
                    text.push('\n');
                }
            }

            let exit_code = output.status.code().unwrap_or(-1);
            if exit_code != 0 {
                text.push_str(&format!("\n\nexit code: {exit_code}"));
                ToolOutput::err(text)
            } else {
                ToolOutput::ok(text)
            }
        }
        Ok(Err(err)) => ToolOutput::err(format!("failed to execute command: {err}")),
        Err(_) => ToolOutput::err(format!(
            "command timed out after {}s",
            timeout.as_secs()
        )),
    }
}
