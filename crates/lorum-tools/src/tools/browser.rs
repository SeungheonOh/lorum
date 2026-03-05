use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "browser".to_string(),
        description: "Control a headless browser for web automation. Supports navigation, \
            clicking, typing, screenshots, and DOM queries."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Browser action: \"open\", \"goto\", \"close\", \"observe\", \"click\", \"click_id\", \"type\", \"type_id\", \"fill\", \"fill_id\", \"press\", \"scroll\", \"drag\", \"wait_for_selector\", \"evaluate\", \"get_text\", \"get_html\", \"get_attribute\", \"extract_readable\", \"screenshot\""
                },
                "url": {
                    "type": "string",
                    "description": "URL for goto action"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS or ARIA selector"
                },
                "element_id": {
                    "type": "integer",
                    "description": "Element ID from observe"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type"
                },
                "value": {
                    "type": "string",
                    "description": "Value to fill"
                },
                "key": {
                    "type": "string",
                    "description": "Key to press"
                },
                "delta_x": {
                    "type": "number",
                    "description": "Scroll delta X"
                },
                "delta_y": {
                    "type": "number",
                    "description": "Scroll delta Y"
                },
                "script": {
                    "type": "string",
                    "description": "JavaScript to evaluate"
                },
                "format": {
                    "type": "string",
                    "description": "Output format: \"markdown\" or \"text\""
                },
                "path": {
                    "type": "string",
                    "description": "Screenshot save path"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "Capture full page screenshot"
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let detail = if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
        Some(crate::display_preview(url, 80))
    } else {
        args.get("selector")
            .and_then(|v| v.as_str())
            .map(|selector| crate::display_preview(selector, 80))
    };
    ToolCallSummary {
        headline: format!("browser {action}"),
        detail,
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

pub async fn execute(args: Value, _cwd: &Path) -> ToolOutput {
    let action = match args.get("action").and_then(Value::as_str) {
        Some(a) => a,
        None => return ToolOutput::err("missing required parameter: action"),
    };

    // Validate action-specific required parameters
    match action {
        "goto" => {
            if args.get("url").and_then(Value::as_str).is_none() {
                return ToolOutput::err("goto action requires the 'url' parameter");
            }
        }
        "click" | "type" | "fill" => {
            if args.get("selector").and_then(Value::as_str).is_none() {
                return ToolOutput::err(format!(
                    "{action} action requires the 'selector' parameter"
                ));
            }
        }
        "click_id" | "type_id" | "fill_id" => {
            if args.get("element_id").and_then(Value::as_u64).is_none() {
                return ToolOutput::err(format!(
                    "{action} action requires the 'element_id' parameter"
                ));
            }
        }
        _ => {}
    }

    ToolOutput::err(
        "browser tool requires a headless browser runtime. \
         Install playwright: npm install -g playwright",
    )
}
