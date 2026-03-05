use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::{ToolCallDisplay, ToolExecutor};
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "test-1".to_string(),
        name: name.to_string(),
        arguments: args,
    }
}

/// Helper: read a file via the read tool and extract CID tags for given line numbers.
async fn read_tags(
    registry: &ToolRegistry,
    file_path: &str,
) -> Vec<(usize, String, String)> {
    let result = registry
        .execute(&call("read", json!({ "path": file_path })))
        .await;
    assert!(!result.is_error, "read failed: {:?}", result.result);
    let text = result.result.as_str().unwrap();

    let mut tags = Vec::new();
    for line in text.lines() {
        if line.starts_with('[') {
            continue;
        }
        let tab_pos = match line.find('\t') {
            Some(p) => p,
            None => continue,
        };
        let tag = &line[..tab_pos];
        let content = &line[tab_pos + 1..];
        let hash_pos = match tag.find('#') {
            Some(p) => p,
            None => continue,
        };
        let line_no: usize = tag[..hash_pos].parse().unwrap();
        tags.push((line_no, tag.to_string(), content.to_string()));
    }
    tags
}

// === Missing parameter tests ===

#[tokio::test]
async fn missing_path_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_registry(dir.path().to_path_buf());
    let result = reg
        .execute(&call(
            "hashline",
            json!({ "edits": [{ "op": "replace", "pos": "1#XX" }] }),
        ))
        .await;
    assert!(result.is_error);
    assert!(result.result.as_str().unwrap().contains("path"));
}

#[tokio::test]
async fn no_operations_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let result = reg
        .execute(&call(
            "hashline",
            json!({ "path": file.to_str().unwrap() }),
        ))
        .await;
    assert!(result.is_error);
    assert!(
        result.result.as_str().unwrap().contains("no operations"),
        "got: {}",
        result.result
    );
}

#[tokio::test]
async fn empty_edits_no_move_no_delete_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let result = reg
        .execute(&call(
            "hashline",
            json!({ "path": file.to_str().unwrap(), "edits": [] }),
        ))
        .await;
    assert!(result.is_error);
}

// === Replace operations ===

#[tokio::test]
async fn replace_single_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_beta = &tags[1].1; // line 2 = "beta"

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_beta,
                    "lines": ["BETA"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "alpha\nBETA\ngamma\n");
}

#[tokio::test]
async fn replace_range_of_lines() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\nd\ne\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;
    let tag_d = &tags[3].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_b,
                    "end": tag_d,
                    "lines": ["X", "Y"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\nX\nY\ne\n");
}

#[tokio::test]
async fn delete_single_line_with_null() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_b,
                    "lines": null
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\nc\n");
}

#[tokio::test]
async fn delete_range_with_empty_array() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\nd\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;
    let tag_c = &tags[2].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_b,
                    "end": tag_c,
                    "lines": []
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\nd\n");
}

#[tokio::test]
async fn clear_line_keeps_empty_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_b,
                    "lines": [""]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\n\nc\n");
}

// === Prepend and Append ===

#[tokio::test]
async fn prepend_before_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "prepend",
                    "pos": tag_b,
                    "lines": ["X", "Y"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\nX\nY\nb\nc\n");
}

#[tokio::test]
async fn append_after_line() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "append",
                    "pos": tag_b,
                    "lines": ["X", "Y"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\nb\nX\nY\nc\n");
}

// === Multiple edits ===

#[tokio::test]
async fn multiple_edits_in_single_call() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\nd\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_a = &tags[0].1;
    let tag_c = &tags[2].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [
                    { "op": "replace", "pos": tag_a, "lines": ["A"] },
                    { "op": "replace", "pos": tag_c, "lines": ["C"] }
                ]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "A\nb\nC\nd\n");
}

// === String shorthand ===

#[tokio::test]
async fn single_string_shorthand_for_lines() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_b,
                    "lines": "REPLACED"
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(content, "a\nREPLACED\nc\n");
}

// === Error cases ===

#[tokio::test]
async fn stale_cid_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\nworld\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());

    // Use a wrong CID
    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": "1#XX",
                    "lines": ["new"]
                }]
            }),
        ))
        .await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(
        text.contains("CID mismatch") || text.contains("Re-read"),
        "got: {text}"
    );
}

#[tokio::test]
async fn out_of_range_line_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": "99#XX",
                    "lines": ["new"]
                }]
            }),
        ))
        .await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("out of range"), "got: {text}");
}

#[tokio::test]
async fn end_before_pos_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "a\nb\nc\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_a = &tags[0].1;
    let tag_c = &tags[2].1;

    // pos=line3, end=line1 (reversed)
    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_c,
                    "end": tag_a,
                    "lines": ["x"]
                }]
            }),
        ))
        .await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("before pos"), "got: {text}");
}

#[tokio::test]
async fn invalid_tag_format_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": "not-a-tag",
                    "lines": ["new"]
                }]
            }),
        ))
        .await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("invalid tag"), "got: {text}");
}

#[tokio::test]
async fn missing_op_field_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "pos": "1#XX"
                }]
            }),
        ))
        .await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("op"), "got: {text}");
}

#[tokio::test]
async fn unknown_op_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("test.txt");
    std::fs::write(&file, "hello\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag = &tags[0].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "bogus",
                    "pos": tag,
                    "lines": ["x"]
                }]
            }),
        ))
        .await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("unknown edit op"), "got: {text}");
}

// === Delete file ===

#[tokio::test]
async fn delete_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("doomed.txt");
    std::fs::write(&file, "goodbye\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "delete": true
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);
    assert!(!file.exists());
}

#[tokio::test]
async fn delete_nonexistent_file_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("ghost.txt");

    let reg = make_registry(dir.path().to_path_buf());
    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "delete": true
            }),
        ))
        .await;
    assert!(result.is_error);
}

// === Move/rename file ===

#[tokio::test]
async fn move_file() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.txt");
    let new = dir.path().join("new.txt");
    std::fs::write(&old, "content\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": old.to_str().unwrap(),
                "move": new.to_str().unwrap(),
                "edits": []
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);
    assert!(!old.exists());
    assert!(new.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "content\n");
}

#[tokio::test]
async fn move_with_edits() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old.txt");
    let new = dir.path().join("new.txt");
    std::fs::write(&old, "alpha\nbeta\n").unwrap();

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, old.to_str().unwrap()).await;
    let tag_beta = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": old.to_str().unwrap(),
                "move": new.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_beta,
                    "lines": ["BETA"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);
    assert!(!old.exists());
    assert!(new.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "alpha\nBETA\n");
}

// === File without trailing newline ===

#[tokio::test]
async fn edit_file_without_trailing_newline() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("no_newline.txt");
    std::fs::write(&file, "a\nb\nc").unwrap(); // no trailing newline

    let reg = make_registry(dir.path().to_path_buf());
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    let tag_b = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_b,
                    "lines": ["B"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    let content = std::fs::read_to_string(&file).unwrap();
    // Should preserve the no-trailing-newline behavior
    assert_eq!(content, "a\nB\nc");
}

// === Format functions ===

#[tokio::test]
async fn format_call_with_edits() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_registry(dir.path().to_path_buf());

    let summary = reg.format_call(
        "hashline",
        &json!({
            "path": "src/main.rs",
            "edits": [
                { "op": "replace", "pos": "1#XX" },
                { "op": "append", "pos": "5#YY" }
            ]
        }),
    );
    assert_eq!(summary.headline, "hashline src/main.rs");
    assert_eq!(summary.detail.unwrap(), "2 edit(s)");
}

#[tokio::test]
async fn format_call_delete() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_registry(dir.path().to_path_buf());

    let summary = reg.format_call(
        "hashline",
        &json!({
            "path": "src/main.rs",
            "delete": true
        }),
    );
    assert_eq!(summary.headline, "hashline delete src/main.rs");
}

#[tokio::test]
async fn format_call_move() {
    let dir = tempfile::tempdir().unwrap();
    let reg = make_registry(dir.path().to_path_buf());

    let summary = reg.format_call(
        "hashline",
        &json!({
            "path": "src/old.rs",
            "move": "src/new.rs",
            "edits": []
        }),
    );
    assert_eq!(summary.headline, "hashline move src/old.rs");
    assert_eq!(summary.detail.unwrap(), "-> src/new.rs");
}

// === Roundtrip: read -> hashline -> verify ===

#[tokio::test]
async fn roundtrip_read_hashline_verify() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("roundtrip.txt");
    std::fs::write(
        &file,
        "function hello() {\n  console.log('hi');\n  return 42;\n}\n",
    )
    .unwrap();

    let reg = make_registry(dir.path().to_path_buf());

    // Read the file
    let tags = read_tags(&reg, file.to_str().unwrap()).await;
    assert_eq!(tags.len(), 4);

    // Replace the console.log line (line 2)
    let tag_log = &tags[1].1;

    let result = reg
        .execute(&call(
            "hashline",
            json!({
                "path": file.to_str().unwrap(),
                "edits": [{
                    "op": "replace",
                    "pos": tag_log,
                    "lines": ["  console.log('hello world');"]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "error: {:?}", result.result);

    // Re-read and verify
    let content = std::fs::read_to_string(&file).unwrap();
    assert_eq!(
        content,
        "function hello() {\n  console.log('hello world');\n  return 42;\n}\n"
    );

    // Read again to verify new CIDs are generated
    let new_tags = read_tags(&reg, file.to_str().unwrap()).await;
    assert_eq!(new_tags.len(), 4);
    // Line 2 CID should be different since content changed
    assert_ne!(tags[1].1, new_tags[1].1);
    // Lines 1, 3, 4 should be the same since they didn't change
    assert_eq!(tags[0].1, new_tags[0].1);
    assert_eq!(tags[2].1, new_tags[2].1);
    assert_eq!(tags[3].1, new_tags[3].1);
}
