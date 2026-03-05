use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn write_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "write".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn write_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("new_file.txt");
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&write_call(
            "t1",
            json!({
                "path": file.to_str().unwrap(),
                "content": "hello world\n"
            }),
        ))
        .await;

    assert_eq!(result.tool_call_id, "t1");
    assert!(!result.is_error);
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(written, "hello world\n");
}

#[tokio::test]
async fn write_creates_parent_directories() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("a").join("b").join("c").join("file.txt");
    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&write_call(
            "t2",
            json!({
                "path": nested.to_str().unwrap(),
                "content": "deep content"
            }),
        ))
        .await;

    assert!(!result.is_error);
    let written = std::fs::read_to_string(&nested).unwrap();
    assert_eq!(written, "deep content");
}

#[tokio::test]
async fn overwrite_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("existing.txt");
    std::fs::write(&file, "old content").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    let result = registry
        .execute(&write_call(
            "t3",
            json!({
                "path": file.to_str().unwrap(),
                "content": "new content"
            }),
        ))
        .await;

    assert!(!result.is_error);
    let written = std::fs::read_to_string(&file).unwrap();
    assert_eq!(written, "new content");
}
