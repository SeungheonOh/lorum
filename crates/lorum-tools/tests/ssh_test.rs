use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn ssh_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "ssh".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn missing_host_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&ssh_call("t1", json!({ "command": "ls" })))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(
        text.contains("missing required parameter: host"),
        "expected host error, got: {text}"
    );
}

#[tokio::test]
async fn missing_command_parameter() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&ssh_call("t2", json!({ "host": "example.com" })))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(
        text.contains("missing required parameter: command"),
        "expected command error, got: {text}"
    );
}
