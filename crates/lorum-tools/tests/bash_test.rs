use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn bash_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "bash".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn basic_command_execution() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&bash_call("t1", json!({ "command": "echo hello world" })))
        .await;

    assert_eq!(result.tool_call_id, "t1");
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("hello world"));
}

#[tokio::test]
async fn command_with_cwd_override() {
    let dir = tempfile::tempdir().unwrap();
    let subdir = dir.path().join("mysubdir");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(subdir.join("marker.txt"), "found it").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&bash_call(
            "t2",
            json!({
                "command": "cat marker.txt",
                "cwd": subdir.to_str().unwrap()
            }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("found it"));
}

#[tokio::test]
async fn head_parameter_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&bash_call(
            "t3",
            json!({
                "command": "printf 'line1\nline2\nline3\nline4\nline5\n'",
                "head": 3
            }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0], "line1");
    assert_eq!(lines[1], "line2");
    assert_eq!(lines[2], "line3");
}

#[tokio::test]
async fn tail_parameter_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&bash_call(
            "t4",
            json!({
                "command": "printf 'line1\nline2\nline3\nline4\nline5\n'",
                "tail": 2
            }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0], "line4");
    assert_eq!(lines[1], "line5");
}

#[tokio::test]
async fn command_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&bash_call(
            "t5",
            json!({
                "command": "sleep 60",
                "timeout": 1
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("timed out"));
}

#[tokio::test]
async fn non_zero_exit_code() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&bash_call("t6", json!({ "command": "exit 42" })))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("exit code: 42"));
}
