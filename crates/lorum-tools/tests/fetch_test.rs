use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry() -> ToolRegistry {
    let dir = std::env::temp_dir();
    ToolRegistry::new(dir, Duration::from_secs(30))
}

fn fetch_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "fetch".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn missing_url_error() {
    let registry = make_registry();
    let result = registry.execute(&fetch_call("t1", json!({}))).await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: url"));
}

#[tokio::test]
async fn invalid_url_error() {
    let registry = make_registry();
    let result = registry
        .execute(&fetch_call("t2", json!({ "url": "not-a-valid-url" })))
        .await;
    assert!(result.is_error);
}
