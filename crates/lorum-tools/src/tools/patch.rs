use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::ToolOutput;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "edit".to_string(),
        description: "Edit files using diff-like patches. Supports creating, updating, and \
            deleting files. For updates, provide hunks with context lines and +/- changes."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "File path to operate on"
                },
                "op": {
                    "type": "string",
                    "enum": ["create", "update", "delete"],
                    "description": "Operation: create, update, or delete"
                },
                "diff": {
                    "type": "string",
                    "description": "For create: full file content. For update: one or more diff hunks."
                },
                "rename": {
                    "type": "string",
                    "description": "New path to move the file to (only for update operations)"
                }
            },
            "required": ["path", "op"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let op = args
        .get("op")
        .and_then(|v| v.as_str())
        .unwrap_or("update");
    match op {
        "create" => {
            let bytes = args
                .get("diff")
                .and_then(|v| v.as_str())
                .map(|s| s.len());
            ToolCallSummary {
                headline: format!("create {path}"),
                detail: bytes.map(|n| format!("{n} bytes")),
                body: None,
            }
        }
        "delete" => ToolCallSummary {
            headline: format!("delete {path}"),
            detail: None,
            body: None,
        },
        _ => {
            let diff = args.get("diff").and_then(|v| v.as_str());
            let hunk_count = diff.map(|d| parse_hunks(d).len()).unwrap_or(0);
            ToolCallSummary {
                headline: format!("edit {path}"),
                detail: Some(format!("{hunk_count} hunk(s)")),
                body: diff.map(|d| d.to_string()),
            }
        }
    }
}

pub fn format_result(_is_error: bool, result: &Value) -> String {
    result.as_str().unwrap_or("").to_string()
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let raw_path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolOutput::err("missing required parameter: path"),
    };
    let op = match args.get("op").and_then(Value::as_str) {
        Some(o) => o,
        None => return ToolOutput::err("missing required parameter: op"),
    };

    let path = super::read::resolve_path(raw_path, cwd);

    match op {
        "create" => execute_create(&path, &args).await,
        "delete" => execute_delete(&path).await,
        "update" => execute_update(&path, &args).await,
        _ => ToolOutput::err(format!("unknown op: {op}. Expected create, update, or delete")),
    }
}

async fn execute_create(path: &Path, args: &Value) -> ToolOutput {
    let diff = match args.get("diff").and_then(Value::as_str) {
        Some(d) => d,
        None => return ToolOutput::err("missing required parameter: diff (file content for create)"),
    };

    if path.exists() {
        return ToolOutput::err(format!(
            "file already exists: {}. Use op=update to modify it.",
            path.display()
        ));
    }

    if let Some(parent) = path.parent() {
        if let Err(err) = tokio::fs::create_dir_all(parent).await {
            return ToolOutput::err(format!(
                "failed to create directory {}: {err}",
                parent.display()
            ));
        }
    }

    match tokio::fs::write(path, diff).await {
        Ok(()) => ToolOutput::ok(format!("created {}", path.display())),
        Err(err) => ToolOutput::err(format!("failed to write {}: {err}", path.display())),
    }
}

async fn execute_delete(path: &Path) -> ToolOutput {
    if !path.exists() {
        return ToolOutput::err(format!("file not found: {}", path.display()));
    }

    match tokio::fs::remove_file(path).await {
        Ok(()) => ToolOutput::ok(format!("deleted {}", path.display())),
        Err(err) => ToolOutput::err(format!("failed to delete {}: {err}", path.display())),
    }
}

async fn execute_update(path: &Path, args: &Value) -> ToolOutput {
    let diff = match args.get("diff").and_then(Value::as_str) {
        Some(d) => d,
        None => return ToolOutput::err("missing required parameter: diff (hunks for update)"),
    };

    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(err) => return ToolOutput::err(format!("failed to read {}: {err}", path.display())),
    };

    let hunks = parse_hunks(diff);
    if hunks.is_empty() {
        return ToolOutput::err("no hunks found in diff");
    }

    let mut lines: Vec<String> = content.lines().map(String::from).collect();

    // Track an offset as hunks may shift line positions
    let mut offset: isize = 0;

    for (i, hunk) in hunks.iter().enumerate() {
        match apply_hunk(&lines, hunk, offset) {
            Ok((new_lines, new_offset)) => {
                lines = new_lines;
                offset = new_offset;
            }
            Err(err) => {
                return ToolOutput::err(format!("hunk {} failed: {}", i + 1, err));
            }
        }
    }

    // Reconstruct file content (preserve trailing newline if original had one)
    let mut result = lines.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }

    if let Err(err) = tokio::fs::write(path, &result).await {
        return ToolOutput::err(format!("failed to write {}: {err}", path.display()));
    }

    // Handle rename if provided
    if let Some(new_path_str) = args.get("rename").and_then(Value::as_str) {
        let new_path = super::read::resolve_path(new_path_str, path.parent().unwrap_or(Path::new("/")));

        if let Some(parent) = new_path.parent() {
            if let Err(err) = tokio::fs::create_dir_all(parent).await {
                return ToolOutput::err(format!(
                    "failed to create directory for rename {}: {err}",
                    parent.display()
                ));
            }
        }

        if let Err(err) = tokio::fs::rename(path, &new_path).await {
            return ToolOutput::err(format!(
                "edited but failed to rename {} -> {}: {err}",
                path.display(),
                new_path.display()
            ));
        }

        return ToolOutput::ok(format!(
            "edited and renamed {} -> {}",
            path.display(),
            new_path.display()
        ));
    }

    let hunk_count = hunks.len();
    ToolOutput::ok(format!(
        "edited {} ({hunk_count} hunk(s) applied)",
        path.display()
    ))
}

// --- Hunk parsing and application ---

#[derive(Debug, Clone)]
pub enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

#[derive(Debug, Clone)]
pub struct Hunk {
    pub anchor: Option<String>,
    pub lines: Vec<HunkLine>,
}

pub fn parse_hunks(diff: &str) -> Vec<Hunk> {
    let mut hunks = Vec::new();
    let mut current_anchor: Option<String> = None;
    let mut current_lines: Vec<HunkLine> = Vec::new();
    let mut in_hunk = false;

    for line in diff.lines() {
        if line.starts_with("@@") {
            // Flush previous hunk if any
            if in_hunk && !current_lines.is_empty() {
                hunks.push(Hunk {
                    anchor: current_anchor.take(),
                    lines: std::mem::take(&mut current_lines),
                });
            }
            in_hunk = true;

            // Extract anchor text: everything after "@@" (trimmed)
            let anchor_text = line.trim_start_matches('@').trim();
            current_anchor = if anchor_text.is_empty() {
                None
            } else {
                Some(anchor_text.to_string())
            };
        } else if in_hunk {
            if let Some(stripped) = line.strip_prefix('-') {
                current_lines.push(HunkLine::Remove(stripped.to_string()));
            } else if let Some(stripped) = line.strip_prefix('+') {
                current_lines.push(HunkLine::Add(stripped.to_string()));
            } else if let Some(stripped) = line.strip_prefix(' ') {
                current_lines.push(HunkLine::Context(stripped.to_string()));
            } else {
                // No prefix — treat as context
                current_lines.push(HunkLine::Context(line.to_string()));
            }
        }
    }

    // Flush final hunk
    if in_hunk && !current_lines.is_empty() {
        hunks.push(Hunk {
            anchor: current_anchor,
            lines: current_lines,
        });
    }

    hunks
}

pub fn apply_hunk(
    file_lines: &[String],
    hunk: &Hunk,
    _offset: isize,
) -> Result<(Vec<String>, isize), String> {
    // Build the "old lines" pattern: Context + Remove lines in order
    let old_lines: Vec<&str> = hunk
        .lines
        .iter()
        .filter_map(|l| match l {
            HunkLine::Context(s) => Some(s.as_str()),
            HunkLine::Remove(s) => Some(s.as_str()),
            HunkLine::Add(_) => None,
        })
        .collect();

    if old_lines.is_empty() {
        // Pure addition — need anchor or insert at start
        return apply_pure_addition(file_lines, hunk);
    }

    // Determine search region
    let search_start = if let Some(anchor) = &hunk.anchor {
        find_anchor_line(file_lines, anchor).unwrap_or(0)
    } else {
        0
    };

    // Find where old_lines match contiguously in the file
    let match_pos = find_contiguous_match(file_lines, &old_lines, search_start)
        .ok_or_else(|| {
            let preview: Vec<&str> = old_lines.iter().take(3).copied().collect();
            format!(
                "could not find matching lines in file. Expected: {:?}{}",
                preview,
                if old_lines.len() > 3 { "..." } else { "" }
            )
        })?;

    // Build replacement: Context lines (kept) + Add lines
    let mut replacement = Vec::new();
    for hunk_line in &hunk.lines {
        match hunk_line {
            HunkLine::Context(s) => replacement.push(s.clone()),
            HunkLine::Add(s) => replacement.push(s.clone()),
            HunkLine::Remove(_) => {} // removed — do not include
        }
    }

    // Construct new file
    let mut new_lines = Vec::with_capacity(file_lines.len());
    new_lines.extend_from_slice(&file_lines[..match_pos]);
    new_lines.extend(replacement);
    new_lines.extend_from_slice(&file_lines[match_pos + old_lines.len()..]);

    let offset_delta = new_lines.len() as isize - file_lines.len() as isize;
    Ok((new_lines, _offset + offset_delta))
}

fn apply_pure_addition(
    file_lines: &[String],
    hunk: &Hunk,
) -> Result<(Vec<String>, isize), String> {
    let add_lines: Vec<String> = hunk
        .lines
        .iter()
        .filter_map(|l| match l {
            HunkLine::Add(s) => Some(s.clone()),
            _ => None,
        })
        .collect();

    if add_lines.is_empty() {
        return Err("hunk has no add lines".to_string());
    }

    let insert_pos = if let Some(anchor) = &hunk.anchor {
        find_anchor_line(file_lines, anchor)
            .map(|p| p + 1)
            .unwrap_or(file_lines.len())
    } else {
        file_lines.len()
    };

    let mut new_lines = Vec::with_capacity(file_lines.len() + add_lines.len());
    new_lines.extend_from_slice(&file_lines[..insert_pos]);
    new_lines.extend(add_lines.clone());
    new_lines.extend_from_slice(&file_lines[insert_pos..]);

    Ok((new_lines, add_lines.len() as isize))
}

fn find_anchor_line(file_lines: &[String], anchor: &str) -> Option<usize> {
    file_lines
        .iter()
        .position(|line| line.contains(anchor))
}

fn find_contiguous_match(
    file_lines: &[String],
    pattern: &[&str],
    search_start: usize,
) -> Option<usize> {
    if pattern.is_empty() {
        return None;
    }
    let pat_len = pattern.len();
    if file_lines.len() < pat_len {
        return None;
    }

    // Search forward from search_start first, then wrap around
    let total = file_lines.len();
    for offset in 0..total {
        let idx = (search_start + offset) % total;
        if idx + pat_len > total {
            continue;
        }
        if file_lines[idx..idx + pat_len]
            .iter()
            .zip(pattern.iter())
            .all(|(file_line, &pat_line)| file_line == pat_line)
        {
            return Some(idx);
        }
    }

    None
}
