use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::cid;
use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "hashline".to_string(),
        description: "Applies precise file edits using LINE#ID tags from read output. \
            Supports replace, prepend, and append operations on individual lines or ranges."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to edit"
                },
                "edits": {
                    "type": "array",
                    "description": "Array of edit operations",
                    "items": {
                        "type": "object",
                        "properties": {
                            "op": {
                                "type": "string",
                                "enum": ["replace", "prepend", "append"],
                                "description": "Operation type"
                            },
                            "pos": {
                                "type": "string",
                                "description": "Anchor line tag (LINE#ID from read output)"
                            },
                            "end": {
                                "type": "string",
                                "description": "End line tag for range replace (inclusive)"
                            },
                            "lines": {
                                "description": "Replacement content: array of strings, single string, [\"\"] to clear, null/[] to delete"
                            }
                        },
                        "required": ["op", "pos"]
                    }
                },
                "delete": {
                    "type": "boolean",
                    "description": "If true, delete the file"
                },
                "move": {
                    "type": "string",
                    "description": "New path to move/rename file to"
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");

    if args.get("delete").and_then(|v| v.as_bool()).unwrap_or(false) {
        return ToolCallSummary {
            headline: format!("hashline delete {path}"),
            detail: None,
            body: None,
        };
    }

    if let Some(move_to) = args.get("move").and_then(|v| v.as_str()) {
        return ToolCallSummary {
            headline: format!("hashline move {path}"),
            detail: Some(format!("-> {move_to}")),
            body: None,
        };
    }

    let edits = args.get("edits").and_then(|v| v.as_array());
    let edit_count = edits.map(|a| a.len()).unwrap_or(0);

    let body = edits.map(|arr| {
        let mut lines = Vec::new();
        for edit in arr {
            let op = edit.get("op").and_then(|v| v.as_str()).unwrap_or("?");
            let pos = edit.get("pos").and_then(|v| v.as_str()).unwrap_or("?");
            let end = edit.get("end").and_then(|v| v.as_str());
            let range = match end {
                Some(e) => format!("{pos}..{e}"),
                None => pos.to_string(),
            };
            lines.push(format!("@@ {op} {range}"));
            if let Some(new_lines) = edit.get("lines") {
                match new_lines {
                    serde_json::Value::Null => lines.push("- (delete)".to_string()),
                    serde_json::Value::Array(arr) if arr.is_empty() => {
                        lines.push("- (delete)".to_string());
                    }
                    serde_json::Value::Array(arr) => {
                        for l in arr {
                            let s = l.as_str().unwrap_or("");
                            lines.push(format!("+{s}"));
                        }
                    }
                    serde_json::Value::String(s) => {
                        lines.push(format!("+{s}"));
                    }
                    _ => {}
                }
            }
        }
        lines.join("\n")
    });

    ToolCallSummary {
        headline: format!("hashline {path}"),
        detail: Some(format!("{edit_count} edit(s)")),
        body,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        return crate::display_preview(text, 200);
    }
    crate::display_preview(text, 200)
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let raw_path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolOutput::err("missing required parameter: path"),
    };

    let path = super::read::resolve_path(raw_path, cwd);

    // Handle delete
    if args.get("delete").and_then(Value::as_bool).unwrap_or(false) {
        return match tokio::fs::remove_file(&path).await {
            Ok(()) => ToolOutput::ok(format!("deleted {}", path.display())),
            Err(err) => ToolOutput::err(format!("failed to delete {}: {err}", path.display())),
        };
    }

    // Handle move (with optional edits first)
    let move_to = args.get("move").and_then(Value::as_str).map(|p| {
        let mp = Path::new(p);
        if mp.is_absolute() {
            mp.to_path_buf()
        } else {
            cwd.join(mp)
        }
    });

    let edits = args.get("edits").and_then(Value::as_array);

    // If we have edits, apply them
    if let Some(edit_ops) = edits {
        if !edit_ops.is_empty() {
            // Read file content
            let content = match tokio::fs::read_to_string(&path).await {
                Ok(c) => c,
                Err(err) => {
                    return ToolOutput::err(format!(
                        "failed to read {}: {err}",
                        path.display()
                    ))
                }
            };

            let result = apply_edits(&content, edit_ops);
            match result {
                Ok(new_content) => {
                    // Write back
                    let write_path = move_to.as_ref().unwrap_or(&path);
                    if let Some(parent) = write_path.parent() {
                        if !parent.exists() {
                            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                                return ToolOutput::err(format!(
                                    "failed to create directory {}: {err}",
                                    parent.display()
                                ));
                            }
                        }
                    }
                    if let Err(err) = tokio::fs::write(write_path, &new_content).await {
                        return ToolOutput::err(format!(
                            "failed to write {}: {err}",
                            write_path.display()
                        ));
                    }

                    // If moving, delete original
                    if move_to.is_some() && *write_path != path {
                        let _ = tokio::fs::remove_file(&path).await;
                        return ToolOutput::ok(format!(
                            "applied {} edit(s), moved {} -> {}",
                            edit_ops.len(),
                            path.display(),
                            write_path.display()
                        ));
                    }

                    return ToolOutput::ok(format!(
                        "applied {} edit(s) to {}",
                        edit_ops.len(),
                        path.display()
                    ));
                }
                Err(err) => return ToolOutput::err(err),
            }
        }
    }

    // Move only (no edits)
    if let Some(move_path) = move_to {
        if let Some(parent) = move_path.parent() {
            if !parent.exists() {
                if let Err(err) = tokio::fs::create_dir_all(parent).await {
                    return ToolOutput::err(format!(
                        "failed to create directory {}: {err}",
                        parent.display()
                    ));
                }
            }
        }
        return match tokio::fs::rename(&path, &move_path).await {
            Ok(()) => ToolOutput::ok(format!(
                "moved {} -> {}",
                path.display(),
                move_path.display()
            )),
            Err(err) => ToolOutput::err(format!(
                "failed to move {} -> {}: {err}",
                path.display(),
                move_path.display()
            )),
        };
    }

    // No edits, no delete, no move
    ToolOutput::err("no operations specified: provide edits, delete, or move")
}

/// Resolve a `LINE#ID` tag against the given lines, returning the 0-based index.
fn resolve_tag(tag: &str, lines: &[&str]) -> Result<usize, String> {
    match cid::validate_tag(tag, lines) {
        Some(idx) => Ok(idx),
        None => {
            // Try to give a helpful error
            match cid::parse_tag(tag) {
                Some((line_no, given_cid)) => {
                    if line_no == 0 || line_no > lines.len() {
                        Err(format!(
                            "tag '{tag}': line {line_no} is out of range (file has {} lines)",
                            lines.len()
                        ))
                    } else {
                        let expected = cid::line_cid(line_no, lines[line_no - 1]);
                        Err(format!(
                            "tag '{tag}': CID mismatch at line {line_no} (expected #{expected}, got #{given_cid}). \
                             Re-read the file to get fresh tags."
                        ))
                    }
                }
                None => Err(format!(
                    "invalid tag format '{tag}': expected LINE#ID (e.g. '23#ZX')"
                )),
            }
        }
    }
}

/// Parse the `lines` field from an edit operation into a vector of line strings.
///
/// - `null` or `[]` → delete (empty vec with delete=true)
/// - `[""]` → clear content but keep one empty line
/// - `["line1", "line2"]` → multiple lines
/// - `"single line"` → shorthand for one line
fn parse_lines_field(value: Option<&Value>) -> (Vec<String>, bool) {
    match value {
        None | Some(Value::Null) => (Vec::new(), true), // delete
        Some(Value::Array(arr)) if arr.is_empty() => (Vec::new(), true), // delete
        Some(Value::Array(arr)) => {
            let lines: Vec<String> = arr
                .iter()
                .map(|v| v.as_str().unwrap_or("").to_string())
                .collect();
            (lines, false)
        }
        Some(Value::String(s)) => (vec![s.clone()], false),
        Some(_) => (Vec::new(), true),
    }
}

/// Apply a list of edit operations to file content.
///
/// Edits are applied in reverse order of their position in the file to avoid
/// invalidating line indices of earlier edits.
pub fn apply_edits(content: &str, edits: &[Value]) -> Result<String, String> {
    let lines: Vec<&str> = content.lines().collect();

    // Parse all edits and resolve tags first
    let mut resolved = Vec::new();
    for (i, edit) in edits.iter().enumerate() {
        let op = edit
            .get("op")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("edit[{i}]: missing 'op' field"))?;
        let pos_tag = edit
            .get("pos")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("edit[{i}]: missing 'pos' field"))?;

        let pos_idx = resolve_tag(pos_tag, &lines)
            .map_err(|e| format!("edit[{i}]: {e}"))?;

        let end_idx = if let Some(end_tag) = edit.get("end").and_then(Value::as_str) {
            let idx = resolve_tag(end_tag, &lines)
                .map_err(|e| format!("edit[{i}]: end {e}"))?;
            if idx < pos_idx {
                return Err(format!(
                    "edit[{i}]: end tag '{end_tag}' (line {}) is before pos tag '{pos_tag}' (line {})",
                    idx + 1,
                    pos_idx + 1
                ));
            }
            Some(idx)
        } else {
            None
        };

        let (new_lines, is_delete) = parse_lines_field(edit.get("lines"));

        resolved.push(ResolvedEdit {
            op: op.to_string(),
            pos_idx,
            end_idx,
            new_lines,
            is_delete,
        });
    }

    // Sort edits by position, reversed, so we apply from bottom to top
    resolved.sort_by(|a, b| b.pos_idx.cmp(&a.pos_idx));

    // Apply edits
    let mut result_lines: Vec<String> = lines.iter().map(|s| s.to_string()).collect();

    for edit in &resolved {
        match edit.op.as_str() {
            "replace" => {
                let start = edit.pos_idx;
                let end = edit.end_idx.unwrap_or(start);
                let range_len = end - start + 1;

                if edit.is_delete {
                    // Delete the lines
                    result_lines.drain(start..start + range_len);
                } else {
                    // Replace the range with new lines
                    result_lines.splice(start..start + range_len, edit.new_lines.clone());
                }
            }
            "prepend" => {
                let idx = edit.pos_idx;
                // Insert before the anchor line
                for (i, line) in edit.new_lines.iter().enumerate() {
                    result_lines.insert(idx + i, line.clone());
                }
            }
            "append" => {
                let idx = edit.pos_idx;
                // Insert after the anchor line
                for (i, line) in edit.new_lines.iter().enumerate() {
                    result_lines.insert(idx + 1 + i, line.clone());
                }
            }
            other => {
                return Err(format!("unknown edit op: '{other}'"));
            }
        }
    }

    // Reconstruct with trailing newline if original had one
    let mut output = result_lines.join("\n");
    if content.ends_with('\n') {
        output.push('\n');
    }
    Ok(output)
}

struct ResolvedEdit {
    op: String,
    pos_idx: usize,
    end_idx: Option<usize>,
    new_lines: Vec<String>,
    is_delete: bool,
}
