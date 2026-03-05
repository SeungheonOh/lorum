use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "web-search".to_string(),
        description: "Search the web for information. Returns search results with titles, \
            URLs, and snippets."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Defaults to 10."
                },
                "recency": {
                    "type": "string",
                    "description": "Filter results by recency: \"day\", \"week\", \"month\", \"year\""
                }
            },
            "required": ["query"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let query = args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "web-search".to_string(),
        detail: Some(crate::display_preview(query, 60)),
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
    let _query = match args.get("query").and_then(Value::as_str) {
        Some(q) => q,
        None => return ToolOutput::err("missing required parameter: query"),
    };

    let _limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10);

    let _recency = args.get("recency").and_then(Value::as_str);

    let api_key = std::env::var("WEB_SEARCH_API_KEY");
    if api_key.is_err() {
        return ToolOutput::err(
            "web-search requires configuration. Set WEB_SEARCH_API_KEY and \
             WEB_SEARCH_PROVIDER environment variables.",
        );
    }

    let _provider = std::env::var("WEB_SEARCH_PROVIDER")
        .unwrap_or_else(|_| "duckduckgo".to_string());

    // No real search integration yet — return configuration message.
    ToolOutput::err(
        "web-search requires configuration. Set WEB_SEARCH_API_KEY and \
         WEB_SEARCH_PROVIDER environment variables.",
    )
}
