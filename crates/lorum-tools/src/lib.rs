use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use lorum_ai_contract::{ToolCall, ToolDefinition};
use lorum_runtime::{ToolCallDisplay, ToolCallSummary, ToolExecutionResult, ToolExecutor, ToolResultSummary};
use serde_json::Value;

pub mod cid;
mod tools;

/// Returns the `task` tool definition for use by the SubagentHandler.
pub fn task_definition() -> lorum_ai_contract::ToolDefinition {
    tools::task::definition()
}

/// Internal helpers re-exported for integration tests.
#[doc(hidden)]
pub mod internals {
    pub mod hashline {
        pub use crate::tools::hashline::apply_edits;
    }
    pub mod patch {
        pub use crate::tools::patch::{apply_hunk, parse_hunks, Hunk, HunkLine};
    }
}

fn truncate_body(text: &str, max_lines: usize) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= max_lines {
        return text.to_string();
    }
    let shown: String = lines[..max_lines].join("\n");
    format!("{shown}\n... ({} more lines)", lines.len() - max_lines)
}

pub(crate) fn display_preview(s: &str, max_chars: usize) -> String {
    let first_line = s.lines().next().unwrap_or(s);
    let more_lines = s.contains('\n');
    let mut chars = first_line.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    let needs_ellipsis = chars.next().is_some() || more_lines;
    if needs_ellipsis {
        format!("{truncated}...")
    } else {
        truncated
    }
}

pub struct ToolRegistry {
    cwd: PathBuf,
    timeout: Duration,
    tools: HashMap<String, ToolHandler>,
}

type ToolHandler = Box<
    dyn Fn(Value, &Path, Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolOutput> + Send + '_>>
        + Send
        + Sync,
>;

struct ToolOutput {
    is_error: bool,
    content: Value,
}

impl ToolOutput {
    fn ok(content: impl Into<Value>) -> Self {
        Self {
            is_error: false,
            content: content.into(),
        }
    }

    fn err(message: impl Into<String>) -> Self {
        Self {
            is_error: true,
            content: Value::String(message.into()),
        }
    }
}

impl ToolRegistry {
    pub fn new(cwd: PathBuf, timeout: Duration) -> Self {
        let mut registry = Self {
            cwd,
            timeout,
            tools: HashMap::new(),
        };
        registry.register_builtins();
        registry
    }

    fn register_builtins(&mut self) {
        // File operations
        self.register("read", |args, cwd, _timeout| {
            Box::pin(tools::read::execute(args, cwd))
        });
        self.register("write", |args, cwd, _timeout| {
            Box::pin(tools::write::execute(args, cwd))
        });
        self.register("edit", |args, cwd, _timeout| {
            Box::pin(tools::patch::execute(args, cwd))
        });
        self.register("replace", |args, cwd, _timeout| {
            Box::pin(tools::replace::execute(args, cwd))
        });
        self.register("hashline", |args, cwd, _timeout| {
            Box::pin(tools::hashline::execute(args, cwd))
        });

        // Search & navigation
        self.register("grep", |args, cwd, _timeout| {
            Box::pin(tools::grep::execute(args, cwd))
        });
        self.register("find", |args, cwd, _timeout| {
            Box::pin(tools::find::execute(args, cwd))
        });

        // Execution
        self.register("bash", |args, cwd, timeout| {
            Box::pin(tools::bash::execute(args, cwd, timeout))
        });
        self.register("ssh", |args, cwd, timeout| {
            Box::pin(tools::ssh::execute(args, cwd, timeout))
        });

        // Browser & web
        self.register("browser", |args, cwd, _timeout| {
            Box::pin(tools::browser::execute(args, cwd))
        });
        self.register("web-search", |args, cwd, _timeout| {
            Box::pin(tools::web_search::execute(args, cwd))
        });
        self.register("fetch", |args, _cwd, _timeout| {
            Box::pin(tools::fetch::execute(args))
        });

        // Agent orchestration
        // "task" is handled by SubagentHandler via ToolDispatcher
        self.register("await", |args, cwd, _timeout| {
            Box::pin(tools::await_tool::execute(args, cwd))
        });
        self.register("cancel-job", |args, cwd, _timeout| {
            Box::pin(tools::cancel_job::execute(args, cwd))
        });

        // State & flow control
        self.register("checkpoint", |args, cwd, _timeout| {
            Box::pin(tools::checkpoint::execute(args, cwd))
        });
        self.register("rewind", |args, cwd, _timeout| {
            Box::pin(tools::rewind::execute(args, cwd))
        });
        self.register("resolve", |args, cwd, _timeout| {
            Box::pin(tools::resolve::execute(args, cwd))
        });
        self.register("ask", |args, cwd, _timeout| {
            Box::pin(tools::ask::execute(args, cwd))
        });
        self.register("todo-write", |args, cwd, _timeout| {
            Box::pin(tools::todo_write::execute(args, cwd))
        });
        self.register("exit-plan-mode", |args, cwd, _timeout| {
            Box::pin(tools::exit_plan_mode::execute(args, cwd))
        });

        // Specialty tools
        self.register("calculator", |args, _cwd, _timeout| {
            Box::pin(tools::calculator::execute(args))
        });
        self.register("gemini-image", |args, cwd, _timeout| {
            Box::pin(tools::gemini_image::execute(args, cwd))
        });
        self.register("render-mermaid", |args, cwd, _timeout| {
            Box::pin(tools::render_mermaid::execute(args, cwd))
        });
        self.register("ast-grep", |args, cwd, timeout| {
            Box::pin(tools::ast_grep::execute(args, cwd, timeout))
        });
        self.register("ast-edit", |args, cwd, timeout| {
            Box::pin(tools::ast_edit::execute(args, cwd, timeout))
        });
    }

    fn register<F>(&mut self, name: &str, handler: F)
    where
        F: Fn(Value, &Path, Duration) -> std::pin::Pin<Box<dyn std::future::Future<Output = ToolOutput> + Send + '_>>
            + Send
            + Sync
            + 'static,
    {
        self.tools.insert(name.to_string(), Box::new(handler));
    }
}

#[async_trait]
impl ToolExecutor for ToolRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![
            // File operations
            tools::read::definition(),
            tools::write::definition(),
            tools::patch::definition(),
            tools::replace::definition(),
            tools::hashline::definition(),
            // Search & navigation
            tools::grep::definition(),
            tools::find::definition(),
            // Execution
            tools::bash::definition(),
            tools::ssh::definition(),
            // Browser & web
            tools::browser::definition(),
            tools::web_search::definition(),
            tools::fetch::definition(),
            // Agent orchestration
            // "task" definition is provided by SubagentHandler via ToolDispatcher
            tools::await_tool::definition(),
            tools::cancel_job::definition(),
            // State & flow control
            tools::checkpoint::definition(),
            tools::rewind::definition(),
            tools::resolve::definition(),
            tools::ask::definition(),
            tools::todo_write::definition(),
            tools::exit_plan_mode::definition(),
            // Specialty tools
            tools::calculator::definition(),
            tools::gemini_image::definition(),
            tools::render_mermaid::definition(),
            tools::ast_grep::definition(),
            tools::ast_edit::definition(),
        ]
    }

    async fn execute(&self, tool_call: &ToolCall) -> ToolExecutionResult {
        let output = match self.tools.get(&tool_call.name) {
            Some(handler) => handler(tool_call.arguments.clone(), &self.cwd, self.timeout).await,
            None => ToolOutput::err(format!("unknown tool: {}", tool_call.name)),
        };

        ToolExecutionResult {
            tool_call_id: tool_call.id.clone(),
            is_error: output.is_error,
            result: output.content,
        }
    }
}

impl ToolCallDisplay for ToolRegistry {
    fn format_call(&self, tool_name: &str, args: &Value) -> ToolCallSummary {
        match tool_name {
            "read" => tools::read::format_call(args),
            "write" => tools::write::format_call(args),
            "edit" => tools::patch::format_call(args),
            "replace" => tools::replace::format_call(args),
            "hashline" => tools::hashline::format_call(args),
            "grep" => tools::grep::format_call(args),
            "find" => tools::find::format_call(args),
            "bash" => tools::bash::format_call(args),
            "ssh" => tools::ssh::format_call(args),
            "browser" => tools::browser::format_call(args),
            "web-search" => tools::web_search::format_call(args),
            "fetch" => tools::fetch::format_call(args),
            "task" => tools::task::format_call(args),
            "await" => tools::await_tool::format_call(args),
            "cancel-job" => tools::cancel_job::format_call(args),
            "checkpoint" => tools::checkpoint::format_call(args),
            "rewind" => tools::rewind::format_call(args),
            "resolve" => tools::resolve::format_call(args),
            "ask" => tools::ask::format_call(args),
            "todo-write" => tools::todo_write::format_call(args),
            "exit-plan-mode" => tools::exit_plan_mode::format_call(args),
            "calculator" => tools::calculator::format_call(args),
            "gemini-image" => tools::gemini_image::format_call(args),
            "render-mermaid" => tools::render_mermaid::format_call(args),
            "ast-grep" => tools::ast_grep::format_call(args),
            "ast-edit" => tools::ast_edit::format_call(args),
            _ => ToolCallSummary {
                headline: tool_name.to_string(),
                detail: Some(display_preview(&args.to_string(), 80)),
                body: None,
            },
        }
    }

    fn format_result(&self, tool_name: &str, is_error: bool, result: &Value) -> ToolResultSummary {
        let headline = match tool_name {
            "read" => tools::read::format_result(is_error, result),
            "write" => tools::write::format_result(is_error, result),
            "edit" => tools::patch::format_result(is_error, result),
            "replace" => tools::replace::format_result(is_error, result),
            "hashline" => tools::hashline::format_result(is_error, result),
            "grep" => tools::grep::format_result(is_error, result),
            "find" => tools::find::format_result(is_error, result),
            "bash" => tools::bash::format_result(is_error, result),
            "ssh" => tools::ssh::format_result(is_error, result),
            "browser" => tools::browser::format_result(is_error, result),
            "web-search" => tools::web_search::format_result(is_error, result),
            "fetch" => tools::fetch::format_result(is_error, result),
            "task" => tools::task::format_result(is_error, result),
            "await" => tools::await_tool::format_result(is_error, result),
            "cancel-job" => tools::cancel_job::format_result(is_error, result),
            "checkpoint" => tools::checkpoint::format_result(is_error, result),
            "rewind" => tools::rewind::format_result(is_error, result),
            "resolve" => tools::resolve::format_result(is_error, result),
            "ask" => tools::ask::format_result(is_error, result),
            "todo-write" => tools::todo_write::format_result(is_error, result),
            "exit-plan-mode" => tools::exit_plan_mode::format_result(is_error, result),
            "calculator" => tools::calculator::format_result(is_error, result),
            "gemini-image" => tools::gemini_image::format_result(is_error, result),
            "render-mermaid" => tools::render_mermaid::format_result(is_error, result),
            "ast-grep" => tools::ast_grep::format_result(is_error, result),
            "ast-edit" => tools::ast_edit::format_result(is_error, result),
            _ => {
                let preview = result.to_string();
                display_preview(&preview, 200)
            }
        };

        // For tools with rich content, extract body from the result
        let body = if !is_error {
            match tool_name {
                "read" | "grep" | "bash" | "ssh" => {
                    // Show the actual output content
                    let text = result.as_str().unwrap_or("");
                    if text.is_empty() {
                        None
                    } else {
                        Some(truncate_body(text, 50))
                    }
                }
                "edit" | "hashline" | "replace" => {
                    // Show the diff/result details from the result text
                    let text = result.as_str().unwrap_or("");
                    if text.is_empty() || text.lines().count() <= 1 {
                        None
                    } else {
                        Some(truncate_body(text, 30))
                    }
                }
                _ => None,
            }
        } else {
            None
        };

        ToolResultSummary { headline, body }
    }
}
