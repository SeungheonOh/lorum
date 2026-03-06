use lorum_tools::cid::line_cid;
use lorum_tools::internals::hashline::apply_edits;
use serde_json::json;

fn make_content(lines: &[&str]) -> String {
    let mut s: String = lines.join("\n");
    s.push('\n');
    s
}

fn make_tag(line_no: usize, content: &str) -> String {
    let cid = line_cid(line_no, content);
    format!("{line_no}#{cid}")
}

#[test]
fn replace_single_line() {
    let content = make_content(&["alpha", "beta", "gamma"]);
    let tag = make_tag(2, "beta");
    let edits = vec![json!({
        "op": "replace",
        "pos": tag,
        "lines": ["BETA"]
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "alpha\nBETA\ngamma\n");
}

#[test]
fn replace_range() {
    let content = make_content(&["a", "b", "c", "d", "e"]);
    let pos = make_tag(2, "b");
    let end = make_tag(4, "d");
    let edits = vec![json!({
        "op": "replace",
        "pos": pos,
        "end": end,
        "lines": ["X", "Y"]
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\nX\nY\ne\n");
}

#[test]
fn delete_single_line() {
    let content = make_content(&["a", "b", "c"]);
    let tag = make_tag(2, "b");
    let edits = vec![json!({
        "op": "replace",
        "pos": tag,
        "lines": null
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\nc\n");
}

#[test]
fn delete_range() {
    let content = make_content(&["a", "b", "c", "d"]);
    let pos = make_tag(2, "b");
    let end = make_tag(3, "c");
    let edits = vec![json!({
        "op": "replace",
        "pos": pos,
        "end": end,
        "lines": []
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\nd\n");
}

#[test]
fn clear_line_keeps_empty_line() {
    let content = make_content(&["a", "b", "c"]);
    let tag = make_tag(2, "b");
    let edits = vec![json!({
        "op": "replace",
        "pos": tag,
        "lines": [""]
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\n\nc\n");
}

#[test]
fn prepend_lines() {
    let content = make_content(&["a", "b", "c"]);
    let tag = make_tag(2, "b");
    let edits = vec![json!({
        "op": "prepend",
        "pos": tag,
        "lines": ["X", "Y"]
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\nX\nY\nb\nc\n");
}

#[test]
fn append_lines() {
    let content = make_content(&["a", "b", "c"]);
    let tag = make_tag(2, "b");
    let edits = vec![json!({
        "op": "append",
        "pos": tag,
        "lines": ["X", "Y"]
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\nb\nX\nY\nc\n");
}

#[test]
fn single_string_shorthand() {
    let content = make_content(&["a", "b", "c"]);
    let tag = make_tag(2, "b");
    let edits = vec![json!({
        "op": "replace",
        "pos": tag,
        "lines": "REPLACED"
    })];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "a\nREPLACED\nc\n");
}

#[test]
fn multiple_edits_applied_bottom_up() {
    let content = make_content(&["a", "b", "c", "d"]);
    let tag1 = make_tag(1, "a");
    let tag3 = make_tag(3, "c");
    let edits = vec![
        json!({ "op": "replace", "pos": tag1, "lines": ["A"] }),
        json!({ "op": "replace", "pos": tag3, "lines": ["C"] }),
    ];
    let result = apply_edits(&content, &edits).unwrap();
    assert_eq!(result, "A\nb\nC\nd\n");
}

#[test]
fn wrong_cid_returns_error() {
    let content = make_content(&["a", "b", "c"]);
    let edits = vec![json!({
        "op": "replace",
        "pos": "2#XX",
        "lines": ["new"]
    })];
    let result = apply_edits(&content, &edits);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("CID mismatch"), "got: {err}");
}

#[test]
fn end_before_pos_returns_error() {
    let content = make_content(&["a", "b", "c"]);
    let pos = make_tag(3, "c");
    let end = make_tag(1, "a");
    let edits = vec![json!({
        "op": "replace",
        "pos": pos,
        "end": end,
        "lines": ["x"]
    })];
    let result = apply_edits(&content, &edits);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.contains("before pos"), "got: {err}");
}
