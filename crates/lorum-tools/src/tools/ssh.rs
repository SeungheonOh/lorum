use std::path::Path;
use std::time::Duration;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "ssh".to_string(),
        description: "Run a command on a remote host via SSH.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to run on the remote host"
                },
                "host": {
                    "type": "string",
                    "description": "The host identifier (e.g., user@hostname or SSH config alias)"
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds. Defaults to 30."
                }
            },
            "required": ["command", "host"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let host = args
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let command = args
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: format!("ssh {host}"),
        detail: Some(format!("$ {}", crate::display_preview(command, 80))),
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
    format!(
        "{} ({} lines)",
        crate::display_preview(lines[0], 80),
        lines.len()
    )
}

pub async fn execute(args: Value, _cwd: &Path, default_timeout: Duration) -> ToolOutput {
    let host = match args.get("host").and_then(Value::as_str) {
        Some(h) => h,
        None => return ToolOutput::err("missing required parameter: host"),
    };

    let command = match args.get("command").and_then(Value::as_str) {
        Some(c) => c,
        None => return ToolOutput::err("missing required parameter: command"),
    };

    let timeout = args
        .get("timeout")
        .and_then(Value::as_u64)
        .map(Duration::from_secs)
        .unwrap_or(default_timeout);

    let result = tokio::time::timeout(
        timeout,
        Command::new("ssh").arg(host).arg(command).output(),
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

            let exit_code = output.status.code().unwrap_or(-1);
            if exit_code != 0 {
                text.push_str(&format!("\n\nexit code: {exit_code}"));
                ToolOutput::err(text)
            } else {
                ToolOutput::ok(text)
            }
        }
        Ok(Err(err)) => ToolOutput::err(format!("failed to execute ssh: {err}")),
        Err(_) => ToolOutput::err(format!(
            "ssh command timed out after {}s",
            timeout.as_secs()
        )),
    }
}
