use std::path::Path;

use ignore::WalkBuilder;
use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use regex::RegexBuilder;
use serde_json::{json, Value};

use crate::ToolOutput;

const MAX_MATCHES: usize = 500;
const MAX_LINE_LENGTH: usize = 500;

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "grep".to_string(),
        description: "Search file contents using a regular expression pattern. \
            Returns matching lines with file paths and line numbers."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern to search for"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in. Defaults to the working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Glob pattern to filter files (e.g. '*.rs', '*.ts')"
                },
                "type": {
                    "type": "string",
                    "description": "File type filter (e.g. 'js', 'py', 'rust', 'ts', 'go', 'java', 'c', 'cpp', 'rb', 'php', 'swift', 'kt')"
                },
                "i": {
                    "type": "boolean",
                    "description": "Case-insensitive search"
                },
                "multiline": {
                    "type": "boolean",
                    "description": "Enable multiline matching (dot matches newline)"
                },
                "pre": {
                    "type": "integer",
                    "description": "Lines of context before each match (like grep -B)"
                },
                "post": {
                    "type": "integer",
                    "description": "Lines of context after each match (like grep -A)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of matches to return (default 500)"
                },
                "offset": {
                    "type": "integer",
                    "description": "Skip the first N matches before collecting results"
                },
                "gitignore": {
                    "type": "boolean",
                    "description": "Respect .gitignore files (default true)"
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let pattern = args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    let path = args.get("path").and_then(|v| v.as_str());
    let include = args
        .get("glob")
        .or_else(|| args.get("include"))
        .and_then(|v| v.as_str());
    let file_type = args.get("type").and_then(|v| v.as_str());
    let mut parts = Vec::new();
    if let Some(p) = path {
        parts.push(format!("in {p}"));
    }
    if let Some(i) = include {
        parts.push(format!("({i})"));
    }
    if let Some(t) = file_type {
        parts.push(format!("[type:{t}]"));
    }
    if args.get("i").and_then(|v| v.as_bool()).unwrap_or(false) {
        parts.push("-i".to_string());
    }
    ToolCallSummary {
        headline: format!("grep /{pattern}/"),
        detail: if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        },
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error || text == "no matches found" {
        return text.to_string();
    }
    let count = text
        .lines()
        .filter(|l| *l != "--")
        .count();
    format!("{count} matches")
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let pattern = match args.get("pattern").and_then(Value::as_str) {
        Some(p) => p,
        None => return ToolOutput::err("missing required parameter: pattern"),
    };

    let search_path = args
        .get("path")
        .and_then(Value::as_str)
        .map(|p| {
            let path = Path::new(p);
            if path.is_absolute() {
                path.to_path_buf()
            } else {
                cwd.join(p)
            }
        })
        .unwrap_or_else(|| cwd.to_path_buf());

    let include_glob = args
        .get("glob")
        .or_else(|| args.get("include"))
        .and_then(Value::as_str)
        .map(String::from);

    let file_type = args
        .get("type")
        .and_then(Value::as_str)
        .map(String::from);

    let case_insensitive = args
        .get("i")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let multiline = args
        .get("multiline")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let pre_context = args
        .get("pre")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    let post_context = args
        .get("post")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(MAX_MATCHES);

    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;

    let gitignore = args
        .get("gitignore")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let pattern_owned = pattern.to_string();
    let result = tokio::task::spawn_blocking(move || {
        grep_recursive(
            &search_path,
            &pattern_owned,
            include_glob.as_deref(),
            file_type.as_deref(),
            case_insensitive,
            multiline,
            pre_context,
            post_context,
            limit,
            offset,
            gitignore,
        )
    })
    .await;

    match result {
        Ok(Ok(output)) => {
            if output.is_empty() {
                ToolOutput::ok("no matches found")
            } else {
                ToolOutput::ok(output)
            }
        }
        Ok(Err(err)) => ToolOutput::err(err),
        Err(err) => ToolOutput::err(format!("grep task failed: {err}")),
    }
}

/// Map common file type names to glob patterns.
fn type_to_glob(file_type: &str) -> Option<&'static str> {
    match file_type {
        "js" => Some("*.js"),
        "jsx" => Some("*.jsx"),
        "ts" => Some("*.ts"),
        "tsx" => Some("*.tsx"),
        "py" => Some("*.py"),
        "rust" | "rs" => Some("*.rs"),
        "go" => Some("*.go"),
        "java" => Some("*.java"),
        "c" => Some("*.c"),
        "cpp" | "cc" | "cxx" => Some("*.cpp"),
        "h" => Some("*.h"),
        "hpp" => Some("*.hpp"),
        "rb" => Some("*.rb"),
        "php" => Some("*.php"),
        "swift" => Some("*.swift"),
        "kt" => Some("*.kt"),
        "scala" => Some("*.scala"),
        "cs" => Some("*.cs"),
        "lua" => Some("*.lua"),
        "sh" | "bash" => Some("*.sh"),
        "zsh" => Some("*.zsh"),
        "json" => Some("*.json"),
        "yaml" | "yml" => Some("*.yaml"),
        "toml" => Some("*.toml"),
        "xml" => Some("*.xml"),
        "html" => Some("*.html"),
        "css" => Some("*.css"),
        "scss" => Some("*.scss"),
        "md" | "markdown" => Some("*.md"),
        "sql" => Some("*.sql"),
        "r" => Some("*.r"),
        "dart" => Some("*.dart"),
        "zig" => Some("*.zig"),
        "el" | "elisp" => Some("*.el"),
        "ex" | "elixir" => Some("*.ex"),
        "erl" | "erlang" => Some("*.erl"),
        "hs" | "haskell" => Some("*.hs"),
        "ml" | "ocaml" => Some("*.ml"),
        "clj" | "clojure" => Some("*.clj"),
        "vim" => Some("*.vim"),
        "proto" => Some("*.proto"),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn grep_recursive(
    search_path: &Path,
    pattern: &str,
    include_glob: Option<&str>,
    file_type: Option<&str>,
    case_insensitive: bool,
    multiline: bool,
    pre_context: usize,
    post_context: usize,
    limit: usize,
    offset: usize,
    gitignore: bool,
) -> Result<String, String> {
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .multi_line(multiline)
        .dot_matches_new_line(multiline)
        .build()
        .map_err(|e| format!("invalid regex: {e}"))?;

    // Resolve the effective glob: explicit glob wins, then type-derived glob
    let type_glob_str;
    let effective_glob = if let Some(g) = include_glob {
        Some(g)
    } else if let Some(t) = file_type {
        type_glob_str = type_to_glob(t)
            .ok_or_else(|| format!("unknown file type: {t}"))?;
        Some(type_glob_str)
    } else {
        None
    };

    let glob_pattern = effective_glob.map(|g| {
        glob::Pattern::new(g).map_err(|e| format!("invalid glob: {e}"))
    });
    let glob_pattern = match glob_pattern {
        Some(Ok(p)) => Some(p),
        Some(Err(e)) => return Err(e),
        None => None,
    };

    let has_context = pre_context > 0 || post_context > 0;
    let mut total_matched: usize = 0;
    let mut collected: usize = 0;
    let mut output_lines: Vec<String> = Vec::new();
    let mut truncated = false;

    if search_path.is_file() {
        search_file_with_context(
            search_path,
            &regex,
            &glob_pattern,
            has_context,
            pre_context,
            post_context,
            limit,
            offset,
            &mut total_matched,
            &mut collected,
            &mut output_lines,
            &mut truncated,
        );
    } else if search_path.is_dir() {
        let walker = WalkBuilder::new(search_path)
            .hidden(false)
            .git_ignore(gitignore)
            .build();

        for entry in walker {
            if truncated {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            if let Some(ref glob) = glob_pattern {
                let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !glob.matches(file_name) {
                    continue;
                }
            }
            search_file_with_context(
                path,
                &regex,
                &glob_pattern,
                has_context,
                pre_context,
                post_context,
                limit,
                offset,
                &mut total_matched,
                &mut collected,
                &mut output_lines,
                &mut truncated,
            );
        }
    } else {
        return Err(format!("path does not exist: {}", search_path.display()));
    }

    let mut output = output_lines.join("\n");
    if truncated {
        output.push_str(&format!("\n\n[results truncated at {limit} matches]"));
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn search_file_with_context(
    path: &Path,
    regex: &regex::Regex,
    _glob_pattern: &Option<glob::Pattern>,
    has_context: bool,
    pre_context: usize,
    post_context: usize,
    limit: usize,
    offset: usize,
    total_matched: &mut usize,
    collected: &mut usize,
    output_lines: &mut Vec<String>,
    truncated: &mut bool,
) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return, // skip binary/unreadable files
    };

    let lines: Vec<&str> = content.lines().collect();

    if !has_context {
        // Simple mode: no context lines
        for (line_idx, line) in lines.iter().enumerate() {
            if *collected >= limit {
                *truncated = true;
                return;
            }
            if regex.is_match(line) {
                *total_matched += 1;
                if *total_matched <= offset {
                    continue;
                }
                *collected += 1;
                let display_line = truncate_line(line);
                output_lines.push(format!("{}:{}:{}", path.display(), line_idx + 1, display_line));
            }
        }
    } else {
        // Context mode: collect matching line indices, group them, and emit with context
        let mut match_indices: Vec<usize> = Vec::new();
        for (line_idx, line) in lines.iter().enumerate() {
            if regex.is_match(line) {
                match_indices.push(line_idx);
            }
        }

        if match_indices.is_empty() {
            return;
        }

        // Build ranges for each match (with context), then merge overlapping ranges
        // Each range is (start_line_idx, end_line_idx_inclusive, Vec<match_line_indices>)
        let mut groups: Vec<(usize, usize, Vec<usize>)> = Vec::new();
        for &m_idx in &match_indices {
            let start = m_idx.saturating_sub(pre_context);
            let end = (m_idx + post_context).min(lines.len() - 1);
            if let Some(last) = groups.last_mut() {
                if start <= last.1 + 1 {
                    // Merge with previous group
                    last.1 = last.1.max(end);
                    last.2.push(m_idx);
                    continue;
                }
            }
            groups.push((start, end, vec![m_idx]));
        }

        let mut first_group_for_file = true;
        for (start, end, match_line_indices) in groups {
            // Check offset/limit per actual match lines
            // Determine which match lines in this group pass offset/limit
            let mut passing_matches: Vec<usize> = Vec::new();
            for &m_idx in &match_line_indices {
                *total_matched += 1;
                if *total_matched <= offset {
                    continue;
                }
                if *collected >= limit {
                    *truncated = true;
                    return;
                }
                *collected += 1;
                passing_matches.push(m_idx);
            }

            if passing_matches.is_empty() {
                continue;
            }

            // Emit separator between non-contiguous groups
            if !first_group_for_file {
                output_lines.push("--".to_string());
            }
            first_group_for_file = false;

            // Emit lines in the range
            // We need to figure out which context lines to include based on passing matches
            let ctx_start = passing_matches
                .iter()
                .map(|&m| m.saturating_sub(pre_context))
                .min()
                .unwrap_or(start);
            let ctx_end = passing_matches
                .iter()
                .map(|&m| (m + post_context).min(lines.len() - 1))
                .max()
                .unwrap_or(end);

            for (line_idx, line) in lines.iter().enumerate().take(ctx_end + 1).skip(ctx_start) {
                let display_line = truncate_line(line);
                output_lines.push(format!(
                    "{}:{}:{}",
                    path.display(),
                    line_idx + 1,
                    display_line
                ));
            }
        }
    }
}

fn truncate_line(line: &str) -> String {
    if line.len() > MAX_LINE_LENGTH {
        format!("{}...", &line[..MAX_LINE_LENGTH])
    } else {
        line.to_string()
    }
}
