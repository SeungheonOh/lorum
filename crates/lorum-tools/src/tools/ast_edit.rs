use std::path::Path;
use std::time::Duration;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "ast-edit".to_string(),
        description: "Structural AST-aware code rewrites via ast-grep (sg). \
            Requires the `sg` binary to be installed."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "ops": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "pat": {
                                "type": "string",
                                "description": "AST pattern to match"
                            },
                            "out": {
                                "type": "string",
                                "description": "Replacement template"
                            }
                        },
                        "required": ["pat", "out"]
                    },
                    "description": "Rewrite operations, each with a pattern (pat) and replacement template (out)"
                },
                "path": {
                    "type": "string",
                    "description": "File, directory, or glob pattern"
                },
                "lang": {
                    "type": "string",
                    "description": "Language override"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max total replacements"
                },
                "selector": {
                    "type": "string",
                    "description": "Optional selector for contextual pattern mode"
                }
            },
            "required": ["ops"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let first_op = args
        .get("ops")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first());
    let detail = match first_op {
        Some(op) => {
            let pat = op.get("pat").and_then(|v| v.as_str()).unwrap_or("?");
            let out = op.get("out").and_then(|v| v.as_str()).unwrap_or("?");
            crate::display_preview(&format!("{pat} -> {out}"), 80)
        }
        None => "<no ops>".to_string(),
    };
    ToolCallSummary {
        headline: "ast-edit".to_string(),
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
    if lines.is_empty() || text == "no changes" {
        return "no changes".to_string();
    }
    format!("{} change line(s)", lines.len())
}

pub async fn execute(args: Value, cwd: &Path, _default_timeout: Duration) -> ToolOutput {
    let ops = match args.get("ops").and_then(Value::as_array) {
        Some(arr) if !arr.is_empty() => arr,
        Some(_) => return ToolOutput::err("ops array must not be empty"),
        None => return ToolOutput::err("missing required parameter: ops"),
    };

    let path = args.get("path").and_then(Value::as_str);
    let lang = args.get("lang").and_then(Value::as_str);
    let selector = args.get("selector").and_then(Value::as_str);
    let _limit = args.get("limit").and_then(Value::as_u64);

    let resolved_path = path.map(|p| {
        let p = Path::new(p);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            cwd.join(p)
        }
    });

    let mut combined_output = String::new();

    for op in ops {
        let pat = match op.get("pat").and_then(Value::as_str) {
            Some(p) => p,
            None => return ToolOutput::err("each op must have a 'pat' field"),
        };
        let out = match op.get("out").and_then(Value::as_str) {
            Some(o) => o,
            None => return ToolOutput::err("each op must have an 'out' field"),
        };

        let mut cmd = Command::new("sg");
        cmd.arg("--pattern").arg(pat);
        cmd.arg("--rewrite").arg(out);

        if let Some(l) = lang {
            cmd.arg("--lang").arg(l);
        }
        if let Some(s) = selector {
            cmd.arg("--selector").arg(s);
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
        return ToolOutput::ok("no changes");
    }

    ToolOutput::ok(combined_output)
}
