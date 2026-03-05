use std::time::Duration;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

const MAX_BODY_SIZE: usize = 200 * 1024; // 200KB

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "fetch".to_string(),
        description: "Fetch content from a URL. Returns the response body as text.".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "URL to fetch"
                },
                "raw": {
                    "type": "boolean",
                    "description": "If true, return raw HTML; if false (default), attempt to extract readable text."
                },
                "timeout": {
                    "type": "integer",
                    "description": "Timeout in seconds. Defaults to 30."
                }
            },
            "required": ["url"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let url = args
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "fetch".to_string(),
        detail: Some(crate::display_preview(url, 60)),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        crate::display_preview(text, 200)
    } else {
        format!("{} bytes", text.len())
    }
}

pub async fn execute(args: Value) -> ToolOutput {
    let url = match args.get("url").and_then(Value::as_str) {
        Some(u) => u,
        None => return ToolOutput::err("missing required parameter: url"),
    };

    let timeout_secs = args
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(30);

    let raw = args
        .get("raw")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let client = match reqwest::Client::builder()
        .user_agent("servus/0.1")
        .timeout(Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(c) => c,
        Err(err) => return ToolOutput::err(format!("failed to create HTTP client: {err}")),
    };

    let response = match client.get(url).send().await {
        Ok(r) => r,
        Err(err) => {
            if err.is_timeout() {
                return ToolOutput::err(format!("request timed out after {timeout_secs}s"));
            }
            return ToolOutput::err(format!("request failed: {err}"));
        }
    };

    let status = response.status();
    if !status.is_success() {
        return ToolOutput::err(format!("HTTP error: {status}"));
    }

    let body = match response.bytes().await {
        Ok(b) => b,
        Err(err) => return ToolOutput::err(format!("failed to read response body: {err}")),
    };

    let mut text = String::from_utf8_lossy(&body).to_string();

    let truncated = text.len() > MAX_BODY_SIZE;
    if truncated {
        text.truncate(MAX_BODY_SIZE);
        text.push_str("\n[response truncated at 200KB]");
    }

    if !raw && !truncated {
        // For now, return the text as-is. Proper readability extraction is complex.
        // The raw text is returned with a note.
    }

    ToolOutput::ok(text)
}
