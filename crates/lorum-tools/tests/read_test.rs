use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::{ToolCallDisplay, ToolExecutor};
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn read_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "read".to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn read_file_with_default_params() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("hello.txt");
    std::fs::write(&file, "line one\nline two\nline three\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call("t1", json!({ "path": file.to_str().unwrap() })))
        .await;

    assert_eq!(result.tool_call_id, "t1");
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("line one"));
    assert!(text.contains("line two"));
    assert!(text.contains("line three"));
    // Check CID format: LINE#ID\tcontent
    // Each line should have format like "1#XX\tline one"
    for line in text.lines() {
        if line.starts_with('[') {
            continue; // skip truncation notice
        }
        assert!(line.contains('#'), "line should have CID: {line}");
        assert!(line.contains('\t'), "line should have tab separator: {line}");
        // Verify the tag format: digits, #, 2 alphanumeric chars, tab
        let tab_pos = line.find('\t').unwrap();
        let tag = &line[..tab_pos];
        let hash_pos = tag.find('#').unwrap();
        let line_no_str = &tag[..hash_pos];
        let cid = &tag[hash_pos + 1..];
        assert!(
            line_no_str.parse::<usize>().is_ok(),
            "line number should be numeric: {line_no_str}"
        );
        assert_eq!(cid.len(), 2, "CID should be 2 chars: {cid}");
        assert!(
            cid.chars().all(|c| c.is_ascii_alphanumeric()),
            "CID should be alphanumeric: {cid}"
        );
    }
}

#[tokio::test]
async fn read_file_with_offset_and_limit() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("lines.txt");
    let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    std::fs::write(&file, &content).unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call(
            "t2",
            json!({ "path": file.to_str().unwrap(), "offset": 10, "limit": 5 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Should contain lines 10-14
    assert!(text.contains("line 10"));
    assert!(text.contains("line 14"));
    // Should NOT contain line 9 or line 15
    assert!(!text.contains("\tline 9\n"));
    assert!(!text.contains("\tline 15\n"));
    // Should show truncation notice
    assert!(text.contains("[truncated:"));
}

#[tokio::test]
async fn read_file_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nonexistent.txt");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call(
            "t3",
            json!({ "path": missing.to_str().unwrap() }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("failed to read"));
}

#[tokio::test]
async fn read_directory_listing() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("subdir");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(dir.path().join("alpha.txt"), "aaa").unwrap();
    std::fs::write(dir.path().join("beta.txt"), "bbbbbb").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call(
            "t4",
            json!({ "path": dir.path().to_str().unwrap() }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("alpha.txt"));
    assert!(text.contains("beta.txt"));
    assert!(text.contains("subdir"));
    assert!(text.contains("dir"));
    assert!(text.contains("file"));
    let alpha_pos = text.find("alpha.txt").unwrap();
    let beta_pos = text.find("beta.txt").unwrap();
    let subdir_pos = text.find("subdir").unwrap();
    assert!(alpha_pos < beta_pos);
    assert!(beta_pos < subdir_pos);
}

#[tokio::test]
async fn read_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("empty.txt");
    std::fs::write(&file, "").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call(
            "t5",
            json!({ "path": file.to_str().unwrap() }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.is_empty() || text.trim().is_empty());
}

#[tokio::test]
async fn read_with_offset_beyond_file_length() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("short.txt");
    std::fs::write(&file, "only one line\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call(
            "t6",
            json!({ "path": file.to_str().unwrap(), "offset": 999 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.is_empty());
}

#[tokio::test]
async fn read_missing_path_parameter() {
    let dir = tempfile::tempdir().unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry.execute(&read_call("t7", json!({}))).await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: path"));
}

#[tokio::test]
async fn format_call_basic() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let summary = registry.format_call("read", &json!({ "path": "/tmp/foo.rs" }));
    assert_eq!(summary.headline, "read /tmp/foo.rs");
    assert!(summary.detail.is_none());
}

#[tokio::test]
async fn format_call_with_offset_and_limit() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let summary = registry.format_call(
        "read",
        &json!({ "path": "/src/main.rs", "offset": 50, "limit": 100 }),
    );
    assert_eq!(summary.headline, "read /src/main.rs");
    let detail = summary.detail.unwrap();
    assert!(detail.contains("from line 50"));
    assert!(detail.contains("limit 100"));
}

#[tokio::test]
async fn format_call_missing_path() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let summary = registry.format_call("read", &json!({}));
    assert_eq!(summary.headline, "read <unknown>");
}

#[tokio::test]
async fn format_result_success() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result_value = serde_json::Value::String("line1\nline2\nline3\n".to_string());
    let formatted = registry.format_result("read", false, &result_value);
    assert!(formatted.headline.contains("3 lines"));
}

#[tokio::test]
async fn format_result_error() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result_value =
        serde_json::Value::String("failed to read /missing: No such file".to_string());
    let formatted = registry.format_result("read", true, &result_value);
    assert!(formatted.headline.contains("failed to read"));
}

#[tokio::test]
async fn read_empty_directory() {
    let dir = tempfile::tempdir().unwrap();
    let empty_dir = dir.path().join("empty_dir");
    std::fs::create_dir(&empty_dir).unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call(
            "t8",
            json!({ "path": empty_dir.to_str().unwrap() }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("(empty directory)"));
}

#[tokio::test]
async fn read_relative_path_resolves_against_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("relative.txt");
    std::fs::write(&file, "content here\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&read_call("t9", json!({ "path": "relative.txt" })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("content here"));
}

#[tokio::test]
async fn read_cid_is_deterministic() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("det.txt");
    std::fs::write(&file, "hello\nworld\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    // Read twice
    let r1 = registry
        .execute(&read_call("t1", json!({ "path": file.to_str().unwrap() })))
        .await;
    let r2 = registry
        .execute(&read_call("t2", json!({ "path": file.to_str().unwrap() })))
        .await;

    let t1 = r1.result.as_str().unwrap();
    let t2 = r2.result.as_str().unwrap();
    assert_eq!(t1, t2, "CIDs should be deterministic across reads");
}

#[tokio::test]
async fn read_cid_changes_when_content_changes() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("mutable.txt");
    std::fs::write(&file, "original\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    let r1 = registry
        .execute(&read_call("t1", json!({ "path": file.to_str().unwrap() })))
        .await;
    let t1 = r1.result.as_str().unwrap().to_string();

    // Change file content
    std::fs::write(&file, "modified\n").unwrap();

    let r2 = registry
        .execute(&read_call("t2", json!({ "path": file.to_str().unwrap() })))
        .await;
    let t2 = r2.result.as_str().unwrap();

    assert_ne!(t1, t2, "CIDs should change when content changes");
}
