use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn edit_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "edit".to_string(),
        arguments: args,
    }
}

// --- Create operations ---

#[tokio::test]
async fn create_new_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("new_file.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t1",
            json!({
                "path": file.to_str().unwrap(),
                "op": "create",
                "diff": "line one\nline two\nline three\n"
            }),
        ))
        .await;

    assert_eq!(result.tool_call_id, "t1");
    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "line one\nline two\nline three\n");
}

#[tokio::test]
async fn create_with_nested_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a/b/c/deep.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t2",
            json!({
                "path": file.to_str().unwrap(),
                "op": "create",
                "diff": "deep content"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "deep content");
}

#[tokio::test]
async fn create_fails_if_file_exists() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("existing.txt");
    std::fs::write(&file, "already here").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t3",
            json!({
                "path": file.to_str().unwrap(),
                "op": "create",
                "diff": "new content"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("already exists"));
    // Original file unchanged
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "already here");
}

// --- Delete operations ---

#[tokio::test]
async fn delete_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("to_delete.txt");
    std::fs::write(&file, "goodbye").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t4",
            json!({
                "path": file.to_str().unwrap(),
                "op": "delete"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    assert!(!file.exists());
}

#[tokio::test]
async fn delete_fails_if_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("ghost.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t5",
            json!({
                "path": missing.to_str().unwrap(),
                "op": "delete"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("file not found"));
}

// --- Update operations ---

#[tokio::test]
async fn update_simple_single_line_change() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("update.txt");
    std::fs::write(&file, "line 1\nline 2\nline 3\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t6",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n line 1\n-line 2\n+line two\n line 3"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "line 1\nline two\nline 3\n");
}

#[tokio::test]
async fn update_add_new_lines() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("add.txt");
    std::fs::write(&file, "first\nlast\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t7",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n first\n+middle\n last"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "first\nmiddle\nlast\n");
}

#[tokio::test]
async fn update_remove_lines() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("remove.txt");
    std::fs::write(&file, "keep\nremove me\nalso keep\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t8",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n keep\n-remove me\n also keep"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "keep\nalso keep\n");
}

#[tokio::test]
async fn update_multiple_hunks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("multi.txt");
    std::fs::write(&file, "aaa\nbbb\nccc\nddd\neee\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t9",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n aaa\n-bbb\n+BBB\n ccc\n@@\n ddd\n-eee\n+EEE"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "aaa\nBBB\nccc\nddd\nEEE\n");
}

#[tokio::test]
async fn update_with_anchor_text() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("anchor.txt");
    std::fs::write(
        &file,
        "fn foo() {\n    let x = 1;\n}\nfn bar() {\n    let y = 2;\n}\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t10",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@ fn bar\n     let y = 2;\n-}\n+    let z = 3;\n+}"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        content,
        "fn foo() {\n    let x = 1;\n}\nfn bar() {\n    let y = 2;\n    let z = 3;\n}\n"
    );
}

#[tokio::test]
async fn update_context_mismatch_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mismatch.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t11",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n wrong context\n-beta\n+BETA"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("could not find matching lines"));
    // File should be unchanged
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "alpha\nbeta\ngamma\n");
}

#[tokio::test]
async fn update_with_rename() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("original.txt");
    let renamed = dir.path().join("renamed.txt");
    std::fs::write(&original, "hello\nworld\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t12",
            json!({
                "path": original.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n-hello\n+goodbye\n world",
                "rename": renamed.to_str().unwrap()
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    assert!(!original.exists(), "original file should no longer exist");
    let content = std::fs::read_to_string(&renamed).unwrap();
    assert_eq!(content, "goodbye\nworld\n");
}

// --- Missing required params ---

#[tokio::test]
async fn missing_path_returns_error() {
    let dir = tempfile::tempdir().unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t13",
            json!({
                "op": "create",
                "diff": "content"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: path"));
}

#[tokio::test]
async fn missing_op_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t14",
            json!({
                "path": file.to_str().unwrap(),
                "diff": "content"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: op"));
}

#[tokio::test]
async fn create_missing_diff_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t15",
            json!({
                "path": file.to_str().unwrap(),
                "op": "create"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: diff"));
}

#[tokio::test]
async fn update_missing_diff_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "content").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t16",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update"
            }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: diff"));
}

// --- Edge cases ---

#[tokio::test]
async fn update_no_prefix_lines_treated_as_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("noprefix.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    // Lines without a prefix should be treated as context
    let result = registry
        .execute(&edit_call(
            "t17",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\nalpha\n-beta\n+BETA\ngamma"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "alpha\nBETA\ngamma\n");
}

#[tokio::test]
async fn update_file_without_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("no_newline.txt");
    std::fs::write(&file, "line 1\nline 2").unwrap(); // no trailing newline

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&edit_call(
            "t18",
            json!({
                "path": file.to_str().unwrap(),
                "op": "update",
                "diff": "@@\n line 1\n-line 2\n+line TWO"
            }),
        ))
        .await;

    assert!(!result.is_error, "expected success, got: {:?}", result.result);
    let content = std::fs::read_to_string(&file).unwrap();
    // Should preserve the lack of trailing newline
    assert_eq!(content, "line 1\nline TWO");
}
