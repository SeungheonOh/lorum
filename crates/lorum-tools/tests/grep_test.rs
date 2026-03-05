use std::path::PathBuf;
use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::{ToolCallDisplay, ToolExecutor};
use lorum_tools::ToolRegistry;
use serde_json::json;

fn make_registry(cwd: PathBuf) -> ToolRegistry {
    ToolRegistry::new(cwd, Duration::from_secs(30))
}

fn grep_call(id: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: "grep".to_string(),
        arguments: args,
    }
}

// ---------------------------------------------------------------------------
// Basic pattern match
// ---------------------------------------------------------------------------

#[tokio::test]
async fn basic_pattern_match() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.rs"), "fn main() {}\nfn helper() {}\nstruct Foo;\n")
        .unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t1", json!({ "pattern": "fn " })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("hello.rs:1:fn main() {}"));
    assert!(text.contains("hello.rs:2:fn helper() {}"));
    assert!(!text.contains("struct Foo"));
}

#[tokio::test]
async fn basic_pattern_match_in_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("code.rs");
    std::fs::write(&file, "let x = 10;\nlet y = 20;\nlet z = 30;\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t2",
            json!({ "pattern": "y = 20", "path": file.to_str().unwrap() }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("code.rs:2:let y = 20;"));
}

// ---------------------------------------------------------------------------
// Case-insensitive search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn case_insensitive_search() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mixed.txt"),
        "Hello World\nhello world\nHELLO WORLD\nfoo bar\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    // Without -i: only exact case
    let result = registry
        .execute(&grep_call("t3a", json!({ "pattern": "hello" })))
        .await;
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    let count = text.lines().count();
    assert_eq!(count, 1, "without -i should match only lowercase: {text}");

    // With -i: all three
    let result = registry
        .execute(&grep_call("t3b", json!({ "pattern": "hello", "i": true })))
        .await;
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    let count = text.lines().count();
    assert_eq!(count, 3, "with -i should match all cases: {text}");
}

// ---------------------------------------------------------------------------
// Glob filtering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn glob_filtering() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.rs"), "fn run() {}\n").unwrap();
    std::fs::write(dir.path().join("app.ts"), "function run() {}\n").unwrap();
    std::fs::write(dir.path().join("readme.md"), "run this\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t4", json!({ "pattern": "run", "glob": "*.rs" })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("app.rs"));
    assert!(!text.contains("app.ts"));
    assert!(!text.contains("readme.md"));
}

// ---------------------------------------------------------------------------
// Type filtering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn type_filtering() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("index.js"), "const x = 1;\n").unwrap();
    std::fs::write(dir.path().join("lib.py"), "x = 1\n").unwrap();
    std::fs::write(dir.path().join("main.rs"), "let x = 1;\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    // Filter by "js"
    let result = registry
        .execute(&grep_call("t5a", json!({ "pattern": "x", "type": "js" })))
        .await;
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("index.js"));
    assert!(!text.contains("lib.py"));
    assert!(!text.contains("main.rs"));

    // Filter by "rust"
    let result = registry
        .execute(&grep_call("t5b", json!({ "pattern": "x", "type": "rust" })))
        .await;
    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("main.rs"));
    assert!(!text.contains("index.js"));
}

#[tokio::test]
async fn unknown_type_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), "content\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t5c", json!({ "pattern": "content", "type": "brainfuck" })))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("unknown file type"));
}

// ---------------------------------------------------------------------------
// Context lines (pre/post)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pre_context_lines() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ctx.txt"),
        "line1\nline2\nline3\nMATCH\nline5\nline6\nline7\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t6a", json!({ "pattern": "MATCH", "pre": 2 })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains(":2:line2"));
    assert!(text.contains(":3:line3"));
    assert!(text.contains(":4:MATCH"));
    // Should NOT include line5 (no post context)
    assert!(!text.contains(":5:line5"));
}

#[tokio::test]
async fn post_context_lines() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("ctx.txt"),
        "line1\nline2\nMATCH\nline4\nline5\nline6\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t6b", json!({ "pattern": "MATCH", "post": 2 })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(!text.contains(":2:line2")); // no pre context
    assert!(text.contains(":3:MATCH"));
    assert!(text.contains(":4:line4"));
    assert!(text.contains(":5:line5"));
    assert!(!text.contains(":6:line6"));
}

#[tokio::test]
async fn pre_and_post_context_with_separator() {
    let dir = tempfile::tempdir().unwrap();
    // Two matches far apart should get a "--" separator between groups
    std::fs::write(
        dir.path().join("sep.txt"),
        "a1\na2\nFIRST\na4\na5\na6\na7\na8\na9\nSECOND\na11\na12\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t6c",
            json!({ "pattern": "FIRST|SECOND", "pre": 1, "post": 1 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Should contain the separator
    assert!(text.contains("\n--\n"), "expected -- separator in:\n{text}");
    // First group: lines 2-4
    assert!(text.contains(":2:a2"));
    assert!(text.contains(":3:FIRST"));
    assert!(text.contains(":4:a4"));
    // Second group: lines 9-11
    assert!(text.contains(":9:a9"));
    assert!(text.contains(":10:SECOND"));
    assert!(text.contains(":11:a11"));
}

#[tokio::test]
async fn overlapping_context_merges_groups() {
    let dir = tempfile::tempdir().unwrap();
    // Two matches close together should merge into one group (no separator)
    std::fs::write(
        dir.path().join("merge.txt"),
        "a\nb\nMATCH1\nc\nMATCH2\nd\ne\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t6d",
            json!({ "pattern": "MATCH", "pre": 1, "post": 1 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Should NOT contain separator because groups overlap
    assert!(!text.contains("--"), "should not have separator in:\n{text}");
    // Should have a continuous range from b to d
    assert!(text.contains(":2:b"));
    assert!(text.contains(":3:MATCH1"));
    assert!(text.contains(":4:c"));
    assert!(text.contains(":5:MATCH2"));
    assert!(text.contains(":6:d"));
}

// ---------------------------------------------------------------------------
// No matches found
// ---------------------------------------------------------------------------

#[tokio::test]
async fn no_matches_found() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "hello world\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t7", json!({ "pattern": "zzzzz_not_here" })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert_eq!(text, "no matches found");
}

// ---------------------------------------------------------------------------
// Invalid regex error
// ---------------------------------------------------------------------------

#[tokio::test]
async fn invalid_regex_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("file.txt"), "content\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t8", json!({ "pattern": "[invalid" })))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("invalid regex"), "expected invalid regex error, got: {text}");
}

// ---------------------------------------------------------------------------
// Missing pattern parameter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn missing_pattern_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());
    let result = registry.execute(&grep_call("t9", json!({}))).await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("missing required parameter: pattern"));
}

// ---------------------------------------------------------------------------
// Limit and offset
// ---------------------------------------------------------------------------

#[tokio::test]
async fn limit_restricts_results() {
    let dir = tempfile::tempdir().unwrap();
    let content: String = (1..=20).map(|i| format!("match line {i}\n")).collect();
    std::fs::write(dir.path().join("many.txt"), &content).unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t10a",
            json!({ "pattern": "match", "limit": 5 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Should have 5 match lines plus the truncation notice
    let match_lines: Vec<&str> = text.lines().filter(|l| l.contains("match line")).collect();
    assert_eq!(match_lines.len(), 5, "expected 5 lines, got:\n{text}");
    assert!(text.contains("[results truncated at 5 matches]"));
}

#[tokio::test]
async fn offset_skips_first_n_matches() {
    let dir = tempfile::tempdir().unwrap();
    let content: String = (1..=10).map(|i| format!("line {i}\n")).collect();
    std::fs::write(dir.path().join("offset.txt"), &content).unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t10b",
            json!({ "pattern": "line", "offset": 7 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Should skip first 7 matches, showing lines 8-10
    let match_lines: Vec<&str> = text.lines().collect();
    assert_eq!(match_lines.len(), 3, "expected 3 lines after offset 7, got:\n{text}");
    assert!(text.contains("line 8"));
    assert!(text.contains("line 9"));
    assert!(text.contains("line 10"));
    assert!(!text.contains(":1:line 1"));
}

#[tokio::test]
async fn offset_and_limit_combined() {
    let dir = tempfile::tempdir().unwrap();
    let content: String = (1..=20).map(|i| format!("item {i}\n")).collect();
    std::fs::write(dir.path().join("combo.txt"), &content).unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t10c",
            json!({ "pattern": "item", "offset": 5, "limit": 3 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // Skip 5, take 3 => items 6, 7, 8
    let match_lines: Vec<&str> = text.lines().filter(|l| l.contains("item")).collect();
    assert_eq!(match_lines.len(), 3, "expected 3 lines, got:\n{text}");
    assert!(text.contains("item 6"));
    assert!(text.contains("item 7"));
    assert!(text.contains("item 8"));
    // Should be truncated since there are more matches
    assert!(text.contains("[results truncated at 3 matches]"));
}

// ---------------------------------------------------------------------------
// Multiline search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn multiline_search() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("multi.txt"),
        "start\nmiddle\nend\nother\n",
    )
    .unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    // Without multiline, dot doesn't match newline so this pattern won't span lines
    // We'll search for a pattern that specifically needs multi_line flag
    // multi_line makes ^ and $ match start/end of lines
    let result = registry
        .execute(&grep_call(
            "t11",
            json!({ "pattern": "^middle$", "multiline": true }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("middle"), "multiline should match: {text}");
}

// ---------------------------------------------------------------------------
// Gitignore support
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gitignore_respected_by_default() {
    let dir = tempfile::tempdir().unwrap();

    // Initialize a git repo so ignore crate picks up .gitignore
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    std::fs::write(dir.path().join(".gitignore"), "ignored_dir/\n").unwrap();
    let ignored_dir = dir.path().join("ignored_dir");
    std::fs::create_dir(&ignored_dir).unwrap();
    std::fs::write(ignored_dir.join("secret.txt"), "SECRET_TOKEN\n").unwrap();
    std::fs::write(dir.path().join("visible.txt"), "SECRET_TOKEN\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    // Default: gitignore=true
    let result = registry
        .execute(&grep_call("t12a", json!({ "pattern": "SECRET_TOKEN" })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("visible.txt"), "should find visible.txt: {text}");
    assert!(
        !text.contains("secret.txt"),
        "should NOT find ignored secret.txt: {text}"
    );
}

#[tokio::test]
async fn gitignore_disabled() {
    let dir = tempfile::tempdir().unwrap();

    // Initialize a git repo so ignore crate picks up .gitignore
    std::process::Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();

    std::fs::write(dir.path().join(".gitignore"), "ignored_dir/\n").unwrap();
    let ignored_dir = dir.path().join("ignored_dir");
    std::fs::create_dir(&ignored_dir).unwrap();
    std::fs::write(ignored_dir.join("secret.txt"), "SECRET_TOKEN\n").unwrap();
    std::fs::write(dir.path().join("visible.txt"), "SECRET_TOKEN\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());

    // gitignore=false
    let result = registry
        .execute(&grep_call(
            "t12b",
            json!({ "pattern": "SECRET_TOKEN", "gitignore": false }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("visible.txt"), "should find visible.txt: {text}");
    assert!(
        text.contains("secret.txt"),
        "with gitignore=false should find secret.txt: {text}"
    );
}

// ---------------------------------------------------------------------------
// Path does not exist
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nonexistent_path_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does_not_exist");

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t13",
            json!({ "pattern": "foo", "path": missing.to_str().unwrap() }),
        ))
        .await;

    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("path does not exist"));
}

// ---------------------------------------------------------------------------
// Relative path resolution
// ---------------------------------------------------------------------------

#[tokio::test]
async fn relative_path_resolves_against_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("subdir");
    std::fs::create_dir(&sub).unwrap();
    std::fs::write(sub.join("target.txt"), "find_me_here\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t14",
            json!({ "pattern": "find_me_here", "path": "subdir" }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("find_me_here"));
}

// ---------------------------------------------------------------------------
// format_call / format_result
// ---------------------------------------------------------------------------

#[tokio::test]
async fn format_call_basic_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let summary = registry.format_call("grep", &json!({ "pattern": "TODO" }));
    assert_eq!(summary.headline, "grep /TODO/");
    assert!(summary.detail.is_none());
}

#[tokio::test]
async fn format_call_with_path_and_glob() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let summary = registry.format_call(
        "grep",
        &json!({ "pattern": "fn", "path": "src/", "glob": "*.rs" }),
    );
    assert_eq!(summary.headline, "grep /fn/");
    let detail = summary.detail.unwrap();
    assert!(detail.contains("in src/"));
    assert!(detail.contains("(*.rs)"));
}

#[tokio::test]
async fn format_call_with_type_and_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let summary = registry.format_call(
        "grep",
        &json!({ "pattern": "test", "type": "py", "i": true }),
    );
    assert_eq!(summary.headline, "grep /test/");
    let detail = summary.detail.unwrap();
    assert!(detail.contains("[type:py]"));
    assert!(detail.contains("-i"));
}

#[tokio::test]
async fn format_result_counts_matches_excluding_separators() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let result_text = "file.rs:1:fn main()\n--\nfile.rs:10:fn test()";
    let formatted = registry.format_result(
        "grep",
        false,
        &serde_json::Value::String(result_text.to_string()),
    );
    assert_eq!(formatted.headline, "2 matches");
}

#[tokio::test]
async fn format_result_error() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let formatted = registry.format_result(
        "grep",
        true,
        &serde_json::Value::String("invalid regex: unclosed bracket".to_string()),
    );
    assert_eq!(formatted.headline, "invalid regex: unclosed bracket");
}

#[tokio::test]
async fn format_result_no_matches() {
    let dir = tempfile::tempdir().unwrap();
    let registry = make_registry(dir.path().to_path_buf());

    let formatted = registry.format_result(
        "grep",
        false,
        &serde_json::Value::String("no matches found".to_string()),
    );
    assert_eq!(formatted.headline, "no matches found");
}

// ---------------------------------------------------------------------------
// Single file search
// ---------------------------------------------------------------------------

#[tokio::test]
async fn search_single_file_directly() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("single.txt");
    std::fs::write(&file, "alpha\nbeta\ngamma\nalpha again\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t15",
            json!({ "pattern": "alpha", "path": file.to_str().unwrap() }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(text.contains(":1:alpha"));
    assert!(text.contains(":4:alpha again"));
}

// ---------------------------------------------------------------------------
// Backward compatibility: "include" parameter still works
// ---------------------------------------------------------------------------

#[tokio::test]
async fn include_parameter_backward_compat() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("code.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("notes.txt"), "fn note\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    // Use old "include" parameter name
    let result = registry
        .execute(&grep_call(
            "t16",
            json!({ "pattern": "fn", "include": "*.rs" }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("code.rs"));
    assert!(!text.contains("notes.txt"));
}

// ---------------------------------------------------------------------------
// Line truncation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn long_lines_are_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let long_line = "x".repeat(600);
    std::fs::write(dir.path().join("long.txt"), format!("{long_line}\n")).unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call("t17", json!({ "pattern": "x" })))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // The match line content should be truncated to 500 chars + "..."
    assert!(text.contains("..."));
    // The content portion (after path:line:) should not be 600 chars
    let line = text.lines().next().unwrap();
    let content_start = line.find(':').unwrap() + 1;
    let content_start = line[content_start..].find(':').unwrap() + content_start + 1;
    let content = &line[content_start..];
    assert!(content.len() <= 503, "content should be truncated: len={}", content.len());
}

// ---------------------------------------------------------------------------
// Context lines at file boundaries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn context_at_start_of_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("start.txt"), "MATCH\nline2\nline3\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t18",
            json!({ "pattern": "MATCH", "pre": 3, "post": 1 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // pre=3 but match is on line 1, so no pre-context lines exist
    assert!(text.contains(":1:MATCH"));
    assert!(text.contains(":2:line2"));
    // No line before line 1
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
}

#[tokio::test]
async fn context_at_end_of_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("end.txt"), "line1\nline2\nMATCH\n").unwrap();

    let registry = make_registry(dir.path().to_path_buf());
    let result = registry
        .execute(&grep_call(
            "t19",
            json!({ "pattern": "MATCH", "pre": 1, "post": 5 }),
        ))
        .await;

    assert!(!result.is_error);
    let text = result.result.as_str().unwrap();
    // post=5 but match is on last line, so no post-context lines exist
    assert!(text.contains(":2:line2"));
    assert!(text.contains(":3:MATCH"));
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2);
}
