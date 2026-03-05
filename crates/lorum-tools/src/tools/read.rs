use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde_json::{json, Value};

use crate::cid::line_cid;
use crate::ToolOutput;

const DEFAULT_LIMIT: u64 = 2000;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "read".to_string(),
        description: "Read the contents of a file, or list the contents of a directory. \
            For files, returns the content with line numbers. \
            Use offset and limit to read specific ranges of large files. \
            For directories, returns a listing of entries with sizes and modification times."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute path to the file or directory to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (1-based). Defaults to 1."
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read. Defaults to 2000."
                }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let raw_path = match args.get("path").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolOutput::err("missing required parameter: path"),
    };

    let path = resolve_path(raw_path, cwd);

    // Directory listing
    if path.is_dir() {
        return list_directory(&path).await;
    }

    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_LIMIT) as usize;

    let content = match tokio::fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(err) => return ToolOutput::err(format!("failed to read {}: {err}", path.display())),
    };

    let lines: Vec<&str> = content.lines().collect();
    let total_lines = lines.len();
    let start = (offset - 1).min(total_lines);
    let end = (start + limit).min(total_lines);

    let mut output = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let line_no = start + i + 1;
        let cid = line_cid(line_no, line);
        output.push_str(&format!("{line_no}#{cid}\t{line}\n"));
    }

    if end < total_lines {
        output.push_str(&format!(
            "\n[truncated: showing lines {}-{} of {}. Use offset to read more.]\n",
            start + 1,
            end,
            total_lines
        ));
    }

    ToolOutput::ok(output)
}

async fn list_directory(path: &Path) -> ToolOutput {
    let mut entries = Vec::new();
    let mut read_dir = match tokio::fs::read_dir(path).await {
        Ok(rd) => rd,
        Err(err) => {
            return ToolOutput::err(format!(
                "failed to read directory {}: {err}",
                path.display()
            ))
        }
    };

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let meta = entry.metadata().await;
        entries.push((name, meta));
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut output = String::new();
    for (name, meta_result) in &entries {
        match meta_result {
            Ok(meta) => {
                let size = meta.len();
                let modified = meta
                    .modified()
                    .map(format_system_time)
                    .unwrap_or_else(|_| "unknown".to_string());
                let kind = if meta.is_dir() { "dir " } else { "file" };
                output.push_str(&format!(
                    "{kind}  {size:>10}  {modified}  {name}\n"
                ));
            }
            Err(_) => {
                output.push_str(&format!("???   {:>10}  {:>19}  {name}\n", "?", "?"));
            }
        }
    }

    if entries.is_empty() {
        output.push_str("(empty directory)\n");
    }

    ToolOutput::ok(output)
}

fn format_system_time(time: std::time::SystemTime) -> String {
    let duration = time
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Convert to a simple date-time string without external crate
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;

    // Approximate year/month/day from days since epoch
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02}"
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Simple Gregorian calendar conversion
    let mut year = 1970;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_lengths: [u64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1;
    for &ml in &month_lengths {
        if days < ml {
            break;
        }
        days -= ml;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    year.is_multiple_of(4) && !year.is_multiple_of(100) || year.is_multiple_of(400)
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let path = args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let mut parts = Vec::new();
    if let Some(offset) = args.get("offset").and_then(|v| v.as_u64()) {
        parts.push(format!("from line {offset}"));
    }
    if let Some(limit) = args.get("limit").and_then(|v| v.as_u64()) {
        parts.push(format!("limit {limit}"));
    }
    ToolCallSummary {
        headline: format!("read {path}"),
        detail: if parts.is_empty() {
            None
        } else {
            Some(parts.join(", "))
        },
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        return crate::display_preview(text, 200);
    }
    let line_count = text.lines().count();
    format!("{line_count} lines")
}

pub fn resolve_path(file_path: &str, cwd: &Path) -> std::path::PathBuf {
    let p = Path::new(file_path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    }
}
