use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn ast_grep_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "ast-grep".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn missing_patterns_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&ast_grep_call("t1", json!({})))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(
        text.contains("missing required parameter: patterns"),
        "expected patterns error, got: {text}"
    );
}
