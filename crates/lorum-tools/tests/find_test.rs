use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn find_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "find".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn find_files_matching_pattern() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("alpha.rs"), "fn main() {}").unwrap();
    std::fs::write(dir.path().join("beta.rs"), "fn test() {}").unwrap();
    std::fs::write(dir.path().join("gamma.txt"), "hello").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&find_call("t1", json!({ "pattern": "*.rs" })))
        .await;

    assert_eq!(result.tool_call_id, "t1");
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("alpha.rs"));
    assert!(text.contains("beta.rs"));
    assert!(!text.contains("gamma.txt"));
}

#[tokio::test]
async fn find_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), "hello").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&find_call("t2", json!({ "pattern": "*.rs" })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert_eq!(text, "no files matched the pattern");
}

#[tokio::test]
async fn find_with_hidden_files_enabled() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".hidden"), "secret").unwrap();
    std::fs::write(dir.path().join("visible.txt"), "public").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&find_call(
            "t3",
            json!({ "pattern": "*", "hidden": true }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains(".hidden"));
    assert!(text.contains("visible.txt"));
}

#[tokio::test]
async fn find_with_hidden_files_disabled() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".hidden"), "secret").unwrap();
    std::fs::write(dir.path().join("visible.txt"), "public").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&find_call(
            "t4",
            json!({ "pattern": "*", "hidden": false }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(!text.contains(".hidden"));
    assert!(text.contains("visible.txt"));
}

#[tokio::test]
async fn find_with_path_restriction() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("sub");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("inner.rs"), "fn inner() {}").unwrap();
    std::fs::write(dir.path().join("outer.rs"), "fn outer() {}").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&find_call(
            "t5",
            json!({ "pattern": "*.rs", "path": "sub" }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("inner.rs"));
    assert!(!text.contains("outer.rs"));
}

#[tokio::test]
async fn find_with_limit() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..10 {
        std::fs::write(dir.path().join(format!("file_{i}.txt")), "data").unwrap();
    }

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&find_call(
            "t6",
            json!({ "pattern": "*.txt", "limit": 3 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Count returned file paths (non-empty lines before the truncation notice)
    let file_lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('['))
        .collect();
    assert_eq!(file_lines.len(), 3);
    assert!(text.contains("[results truncated at 3 matches]"));
}
