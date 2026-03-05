use std::path::Path;

use lorum_ai_contract::ToolDefinition;
use lorum_runtime::ToolCallSummary;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::ToolOutput;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TodoList {
    phases: Vec<Phase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Phase {
    id: String,
    name: String,
    tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    content: String,
    status: String, // "pending", "in_progress", "completed", "abandoned"
}

impl TodoList {
    fn new() -> Self {
        Self { phases: Vec::new() }
    }

    fn next_task_id(&self) -> String {
        let max = self
            .phases
            .iter()
            .flat_map(|p| &p.tasks)
            .filter_map(|t| t.id.strip_prefix("task-").and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        format!("task-{}", max + 1)
    }

    fn next_phase_id(&self) -> String {
        let max = self
            .phases
            .iter()
            .filter_map(|p| p.id.strip_prefix("phase-").and_then(|n| n.parse::<u64>().ok()))
            .max()
            .unwrap_or(0);
        format!("phase-{}", max + 1)
    }
}

pub fn definition() -> ToolDefinition {
    ToolDefinition {
        name: "todo-write".to_string(),
        description: "Manage a phased task list. Supports creating, updating, and tracking tasks."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "ops": {
                    "type": "array",
                    "description": "Operations to perform on the todo list",
                    "items": {
                        "type": "object",
                        "properties": {
                            "type": {
                                "type": "string",
                                "enum": ["replace", "update", "add_phase", "add_task", "remove_task"],
                                "description": "Operation type"
                            },
                            "phases": {
                                "type": "array",
                                "description": "For 'replace': the new phases list",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "id": { "type": "string" },
                                        "name": { "type": "string" },
                                        "tasks": {
                                            "type": "array",
                                            "items": {
                                                "type": "object",
                                                "properties": {
                                                    "id": { "type": "string" },
                                                    "content": { "type": "string" },
                                                    "status": { "type": "string" }
                                                },
                                                "required": ["content"]
                                            }
                                        }
                                    },
                                    "required": ["name"]
                                }
                            },
                            "task_id": {
                                "type": "string",
                                "description": "For 'update' and 'remove_task': the task ID"
                            },
                            "status": {
                                "type": "string",
                                "description": "For 'update': the new status",
                                "enum": ["pending", "in_progress", "completed", "abandoned"]
                            },
                            "phase_id": {
                                "type": "string",
                                "description": "For 'add_task': the target phase ID. For 'add_phase': optional explicit ID."
                            },
                            "name": {
                                "type": "string",
                                "description": "For 'add_phase': the phase name"
                            },
                            "content": {
                                "type": "string",
                                "description": "For 'add_task': the task content"
                            }
                        },
                        "required": ["type"]
                    }
                }
            },
            "required": ["ops"],
            "additionalProperties": false
        }),
    }
}

pub fn format_call(args: &Value) -> ToolCallSummary {
    let detail = args
        .get("ops")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|op| op.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("<unknown>");
    ToolCallSummary {
        headline: "todo-write".to_string(),
        detail: Some(detail.to_string()),
        body: None,
    }
}

pub fn format_result(is_error: bool, result: &Value) -> String {
    let text = result.as_str().unwrap_or("");
    if is_error {
        return crate::display_preview(text, 200);
    }
    crate::display_preview(text, 200)
}

fn todos_path(cwd: &Path) -> std::path::PathBuf {
    cwd.join(".servus").join("todos.json")
}

async fn load_todos(cwd: &Path) -> TodoList {
    let path = todos_path(cwd);
    match tokio::fs::read_to_string(&path).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|_| TodoList::new()),
        Err(_) => TodoList::new(),
    }
}

async fn save_todos(cwd: &Path, todos: &TodoList) -> Result<(), String> {
    let dir = cwd.join(".servus");
    if !dir.exists() {
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| format!("failed to create .servus directory: {e}"))?;
    }
    let content =
        serde_json::to_string_pretty(todos).map_err(|e| format!("failed to serialize: {e}"))?;
    tokio::fs::write(todos_path(cwd), content)
        .await
        .map_err(|e| format!("failed to write todos: {e}"))
}

fn apply_replace(todos: &mut TodoList, op: &Value) -> Result<String, String> {
    let phases_val = op
        .get("phases")
        .and_then(Value::as_array)
        .ok_or_else(|| "replace op requires 'phases' array".to_string())?;

    let mut counter = 0u64;
    let mut phases = Vec::new();

    for (phase_counter, phase_val) in phases_val.iter().enumerate() {
        let phase_id = phase_val
            .get("id")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| format!("phase-{}", phase_counter + 1));
        let name = phase_val
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Unnamed Phase")
            .to_string();

        let mut tasks = Vec::new();
        if let Some(tasks_val) = phase_val.get("tasks").and_then(Value::as_array) {
            for task_val in tasks_val {
                counter += 1;
                let task_id = task_val
                    .get("id")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .unwrap_or_else(|| format!("task-{counter}"));
                let content = task_val
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let status = task_val
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("pending")
                    .to_string();
                tasks.push(Task {
                    id: task_id,
                    content,
                    status,
                });
            }
        }

        phases.push(Phase {
            id: phase_id,
            name,
            tasks,
        });
    }

    let task_count: usize = phases.iter().map(|p| p.tasks.len()).sum();
    todos.phases = phases;
    Ok(format!(
        "replaced: {} phases, {} tasks",
        todos.phases.len(),
        task_count
    ))
}

fn apply_update(todos: &mut TodoList, op: &Value) -> Result<String, String> {
    let task_id = op
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "update op requires 'task_id'".to_string())?;
    let status = op
        .get("status")
        .and_then(Value::as_str)
        .ok_or_else(|| "update op requires 'status'".to_string())?;

    let valid_statuses = ["pending", "in_progress", "completed", "abandoned"];
    if !valid_statuses.contains(&status) {
        return Err(format!(
            "invalid status '{status}'. Must be one of: {}",
            valid_statuses.join(", ")
        ));
    }

    for phase in &mut todos.phases {
        for task in &mut phase.tasks {
            if task.id == task_id {
                let old_status = task.status.clone();
                task.status = status.to_string();
                return Ok(format!(
                    "updated task '{task_id}': {old_status} -> {status}"
                ));
            }
        }
    }

    Err(format!("task '{task_id}' not found"))
}

fn apply_add_phase(todos: &mut TodoList, op: &Value) -> Result<String, String> {
    let name = op
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "add_phase op requires 'name'".to_string())?
        .to_string();
    let phase_id = op
        .get("phase_id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| todos.next_phase_id());

    todos.phases.push(Phase {
        id: phase_id.clone(),
        name: name.clone(),
        tasks: Vec::new(),
    });

    Ok(format!("added phase '{phase_id}': {name}"))
}

fn apply_add_task(todos: &mut TodoList, op: &Value) -> Result<String, String> {
    let phase_id = op
        .get("phase_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "add_task op requires 'phase_id'".to_string())?;
    let content = op
        .get("content")
        .and_then(Value::as_str)
        .ok_or_else(|| "add_task op requires 'content'".to_string())?
        .to_string();
    let task_id = op
        .get("task_id")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| todos.next_task_id());

    let phase = todos
        .phases
        .iter_mut()
        .find(|p| p.id == phase_id)
        .ok_or_else(|| format!("phase '{phase_id}' not found"))?;

    phase.tasks.push(Task {
        id: task_id.clone(),
        content: content.clone(),
        status: "pending".to_string(),
    });

    Ok(format!("added task '{task_id}' to phase '{phase_id}'"))
}

fn apply_remove_task(todos: &mut TodoList, op: &Value) -> Result<String, String> {
    let task_id = op
        .get("task_id")
        .and_then(Value::as_str)
        .ok_or_else(|| "remove_task op requires 'task_id'".to_string())?;

    for phase in &mut todos.phases {
        let before = phase.tasks.len();
        phase.tasks.retain(|t| t.id != task_id);
        if phase.tasks.len() < before {
            return Ok(format!("removed task '{task_id}'"));
        }
    }

    Err(format!("task '{task_id}' not found"))
}

pub async fn execute(args: Value, cwd: &Path) -> ToolOutput {
    let ops = match args.get("ops").and_then(Value::as_array) {
        Some(o) if !o.is_empty() => o.clone(),
        Some(_) => return ToolOutput::err("ops array must not be empty"),
        None => return ToolOutput::err("missing required parameter: ops"),
    };

    let mut todos = load_todos(cwd).await;
    let mut results = Vec::new();

    for op in &ops {
        let op_type = match op.get("type").and_then(Value::as_str) {
            Some(t) => t,
            None => {
                results.push("error: op missing 'type' field".to_string());
                continue;
            }
        };

        let result = match op_type {
            "replace" => apply_replace(&mut todos, op),
            "update" => apply_update(&mut todos, op),
            "add_phase" => apply_add_phase(&mut todos, op),
            "add_task" => apply_add_task(&mut todos, op),
            "remove_task" => apply_remove_task(&mut todos, op),
            other => Err(format!("unknown op type: '{other}'")),
        };

        match result {
            Ok(msg) => results.push(msg),
            Err(err) => return ToolOutput::err(err),
        }
    }

    if let Err(err) = save_todos(cwd, &todos).await {
        return ToolOutput::err(err);
    }

    let summary = results.join("; ");
    ToolOutput::ok(summary)
}
