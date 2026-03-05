use std::time::Duration;

use lorum_ai_contract::ToolCall;
use lorum_runtime::ToolExecutor;
use serde_json::json;

fn registry(dir: &std::path::Path) -> lorum_tools::ToolRegistry {
    lorum_tools::ToolRegistry::new(dir.to_path_buf(), Duration::from_secs(30))
}

fn call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "test-1".to_string(),
        name: name.to_string(),
        arguments: args,
    }
}

#[tokio::test]
async fn todo_write_missing_ops_error() {
    let dir = tempfile::tempdir().unwrap();
    let reg = registry(dir.path());
    let result = reg.execute(&call("todo-write", json!({}))).await;
    assert!(result.is_error);
    let text = result.result.as_str().unwrap();
    assert!(text.contains("ops"), "expected ops error, got: {text}");
}

#[tokio::test]
async fn todo_write_replace_creates_todo_list() {
    let dir = tempfile::tempdir().unwrap();
    let reg = registry(dir.path());
    let result = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "replace",
                    "phases": [
                        {
                            "name": "Setup",
                            "tasks": [
                                { "content": "Install deps" },
                                { "content": "Configure DB" }
                            ]
                        }
                    ]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "unexpected error: {:?}", result.result);

    let todos_path = dir.path().join(".servus").join("todos.json");
    assert!(todos_path.exists(), "todos.json should exist");

    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(todos["phases"].as_array().unwrap().len(), 1);
    assert_eq!(todos["phases"][0]["tasks"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn todo_write_update_task_status() {
    let dir = tempfile::tempdir().unwrap();
    let reg = registry(dir.path());

    // Create initial list
    let _ = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "replace",
                    "phases": [{
                        "name": "Work",
                        "tasks": [{ "content": "Do stuff" }]
                    }]
                }]
            }),
        ))
        .await;

    // Read the todos to find the task ID
    let todos_path = dir.path().join(".servus").join("todos.json");
    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    let task_id = todos["phases"][0]["tasks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Update the task status
    let result = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "update",
                    "task_id": task_id,
                    "status": "completed"
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "unexpected error: {:?}", result.result);

    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(todos["phases"][0]["tasks"][0]["status"], "completed");
}

#[tokio::test]
async fn todo_write_add_phase() {
    let dir = tempfile::tempdir().unwrap();
    let reg = registry(dir.path());

    // Create initial list
    let _ = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "replace",
                    "phases": [{
                        "name": "Phase 1",
                        "tasks": [{ "content": "Task A" }]
                    }]
                }]
            }),
        ))
        .await;

    // Add a phase
    let result = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "add_phase",
                    "name": "Phase 2",
                    "tasks": [{ "content": "Task B" }]
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "unexpected error: {:?}", result.result);

    let todos_path = dir.path().join(".servus").join("todos.json");
    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(todos["phases"].as_array().unwrap().len(), 2);
    assert_eq!(todos["phases"][1]["name"], "Phase 2");
}

#[tokio::test]
async fn todo_write_add_task_to_phase() {
    let dir = tempfile::tempdir().unwrap();
    let reg = registry(dir.path());

    // Create initial list
    let _ = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "replace",
                    "phases": [{
                        "name": "Work",
                        "tasks": [{ "content": "Task 1" }]
                    }]
                }]
            }),
        ))
        .await;

    // Find the phase ID
    let todos_path = dir.path().join(".servus").join("todos.json");
    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    let phase_id = todos["phases"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Add a task
    let result = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "add_task",
                    "phase_id": phase_id,
                    "content": "Task 2"
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "unexpected error: {:?}", result.result);

    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    let tasks = todos["phases"][0]["tasks"].as_array().unwrap();
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[1]["content"], "Task 2");
    assert_eq!(tasks[1]["status"], "pending");
}

#[tokio::test]
async fn todo_write_remove_task() {
    let dir = tempfile::tempdir().unwrap();
    let reg = registry(dir.path());

    // Create initial list
    let _ = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "replace",
                    "phases": [{
                        "name": "Work",
                        "tasks": [
                            { "content": "Task 1" },
                            { "content": "Task 2" }
                        ]
                    }]
                }]
            }),
        ))
        .await;

    let todos_path = dir.path().join(".servus").join("todos.json");
    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    let task_id = todos["phases"][0]["tasks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Remove the first task
    let result = reg
        .execute(&call(
            "todo-write",
            json!({
                "ops": [{
                    "type": "remove_task",
                    "task_id": task_id
                }]
            }),
        ))
        .await;
    assert!(!result.is_error, "unexpected error: {:?}", result.result);

    let content = std::fs::read_to_string(&todos_path).unwrap();
    let todos: serde_json::Value = serde_json::from_str(&content).unwrap();
    assert_eq!(todos["phases"][0]["tasks"].as_array().unwrap().len(), 1);
}
