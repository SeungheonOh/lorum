use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn replace_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "replace".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn single_replacement() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello world").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&replace_call(
            "t1",
            json!({
                "path": file.to_str().unwrap(),
                "old_text": "hello",
                "new_text": "goodbye"
            }),
        ))
        .await;

    assert_eq!(result.tool_call_id, "t1");
    assert!(!result.is_error);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "goodbye world");
}

#[tokio::test]
async fn replace_all_occurrences() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "aaa bbb aaa ccc aaa").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&replace_call(
            "t2",
            json!({
                "path": file.to_str().unwrap(),
                "old_text": "aaa",
                "new_text": "xxx",
                "all": true
            }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("3 occurrences"));
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "xxx bbb xxx ccc xxx");
}

#[tokio::test]
async fn old_text_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello world").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&replace_call(
            "t3",
            json!({
                "path": file.to_str().unwrap(),
                "old_text": "missing",
                "new_text": "replacement"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("old_text not found"));
}

#[tokio::test]
async fn multiple_matches_without_all_flag() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "foo bar foo baz foo").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&replace_call(
            "t4",
            json!({
                "path": file.to_str().unwrap(),
                "old_text": "foo",
                "new_text": "qux"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("3 times"));
    // File should be unchanged
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "foo bar foo baz foo");
}

#[tokio::test]
async fn file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nonexistent.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&replace_call(
            "t5",
            json!({
                "path": missing.to_str().unwrap(),
                "old_text": "hello",
                "new_text": "world"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("failed to read"));
}

#[tokio::test]
async fn replace_with_empty_string_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello beautiful world").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&replace_call(
            "t6",
            json!({
                "path": file.to_str().unwrap(),
                "old_text": " beautiful",
                "new_text": ""
            }),
        ))
        .await;

    assert!(!result.is_error);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "hello world");
}
