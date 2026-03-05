use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "render-mermaid".to_string(),
        description: "Render a Mermaid diagram to an image file. Requires the mmdc \
            (mermaid-cli) binary to be installed."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "mermaid": {
                    "type": "string",
                    "description": "Mermaid graph source text"
                },
                "config": {
                    "type": "object",
                    "description": "Optional JSON render configuration"
                }
            },
            "required": ["mermaid"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let mermaid = args
        .get("mermaid")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "render-mermaid".to_string(),
        detail: Some(crate::display_preview(mermaid, 40)),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        return crate::display_preview(text, 200);
    }
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let mermaid_source = match args.get("mermaid").and_then(Value::as_str) {
        Some(s) => s,
        None => return ToolOutput::err("missing required parameter: mermaid"),
    };

    // Write mermaid source to a temp file
    let tmp_dir = std::env::temp_dir();
    let input_path = tmp_dir.join(format!("lorum_mermaid_{}.mmd", std::process::id()));
    let output_path = tmp_dir.join(format!("lorum_mermaid_{}.png", std::process::id()));

    if let Err(err) = tokio::fs::write(&input_path, mermaid_source).await {
        return ToolOutput::err(format!("failed to write temp file: {err}"));
    }

    // Build mmdc command
    let mut cmd = Command::new("mmdc");
    cmd.arg("-i")
        .arg(&input_path)
        .arg("-o")
        .arg(&output_path)
        .arg("-e")
        .arg("png")
        .current_dir(cwd);

    // If config is provided, write it to a temp file and pass it
    let config_path = tmp_dir.join(format!("lorum_mermaid_config_{}.json", std::process::id()));
    let has_config = if let Some(config) = args.get("config") {
        match tokio::fs::write(&config_path, config.to_string()).await {
            Ok(_) => {
                cmd.arg("-c").arg(&config_path);
                true
            }
            Err(err) => {
                let _ = tokio::fs::remove_file(&input_path).await;
                return ToolOutput::err(format!("failed to write config file: {err}"));
            }
        }
    } else {
        false
    };

    let result = cmd.output().await;

    // Clean up temp files
    let _ = tokio::fs::remove_file(&input_path).await;
    if has_config {
        let _ = tokio::fs::remove_file(&config_path).await;
    }

    match result {
        Ok(output) => {
            if output.status.success() {
                ToolOutput::ok(format!(
                    "Mermaid diagram rendered to {}",
                    output_path.display()
                ))
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                let _ = tokio::fs::remove_file(&output_path).await;
                ToolOutput::err(format!("mmdc failed: {stderr}"))
            }
        }
        Err(err) => {
            if err.kind() == std::io::ErrorKind::NotFound {
                ToolOutput::err(
                    "render-mermaid requires mermaid-cli. \
                     Install it: npm install -g @mermaid-js/mermaid-cli",
                )
            } else {
                ToolOutput::err(format!("failed to run mmdc: {err}"))
            }
        }
    }
}
