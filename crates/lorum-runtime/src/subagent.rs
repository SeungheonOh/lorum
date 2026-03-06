use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use lorum_ai_contract::{ToolCall, ToolChoice, ToolDefinition};
use lorum_domain::{RuntimeEvent, SessionId, TurnId};
use lorum_session::{SessionMetadata, SessionStore};
use serde_json::{json, Value};

use crate::agents::AgentDefinition;
use crate::{
    persist_and_broadcast, run_agent_loop, AgentLoopParams, RuntimeAuthResolver, RuntimeConfig,
    RuntimeError, RuntimeModelResolver, RuntimeProviderRegistry, RuntimeSubscriber,
    RuntimeToolHandler, ToolDispatchContext, ToolDispatcher, ToolExecutionResult, ToolSetContext,
};

const SUBAGENT_SYSTEM_PROMPT_WRAPPER: &str = "\
You are a subagent working on a specific task. Follow these rules strictly:
1. Complete the assigned task as thoroughly as possible.
2. You MUST call the submit_result tool exactly once when you are done.
3. Use submit_result with {\"result\": {\"data\": <your result>}} for success.
4. Use submit_result with {\"result\": {\"error\": \"<description>\"}} for failure.
5. Do NOT end your turn without calling submit_result.";

const SUBMIT_RESULT_REMINDER: &str = "\
You have not called the submit_result tool yet. You MUST call submit_result \
before finishing. Call it now with your result.";

const MAX_SUBMIT_RETRIES: u32 = 3;

pub const WARNING_MISSING_SUBMIT: &str =
    "SYSTEM WARNING: Subagent exited without calling submit_result tool after 3 reminders.";

pub const WARNING_NULL_DATA: &str =
    "SYSTEM WARNING: Subagent called submit_result with null data.";

pub const DEFAULT_MAX_OUTPUT_BYTES: usize = 500_000;
pub const DEFAULT_MAX_OUTPUT_LINES: usize = 5_000;

#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone)]
pub struct SingleTaskResult {
    pub task_id: String,
    pub status: TaskStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
}

pub struct SubagentExecutor {
    auth_resolver: Arc<dyn RuntimeAuthResolver>,
    model_resolver: Arc<dyn RuntimeModelResolver>,
    provider_registry: Arc<dyn RuntimeProviderRegistry>,
    session_store: Arc<dyn SessionStore>,
    config: RuntimeConfig,
}

impl SubagentExecutor {
    pub fn new(
        auth_resolver: Arc<dyn RuntimeAuthResolver>,
        model_resolver: Arc<dyn RuntimeModelResolver>,
        provider_registry: Arc<dyn RuntimeProviderRegistry>,
        session_store: Arc<dyn SessionStore>,
        config: RuntimeConfig,
    ) -> Self {
        Self {
            auth_resolver,
            model_resolver,
            provider_registry,
            session_store,
            config,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute_task(
        &self,
        parent_session_id: &SessionId,
        parent_subscribers: &[Arc<dyn RuntimeSubscriber>],
        agent_def: &AgentDefinition,
        task_id: &str,
        task_description: &str,
        task_assignment: Option<&str>,
        context: Option<&str>,
        _schema: Option<&Value>,
        depth: u32,
        dispatcher: &ToolDispatcher,
    ) -> Result<SingleTaskResult, RuntimeError> {
        let child_session_id =
            SessionId::from(format!("{}-task-{}", parent_session_id.as_str(), task_id));

        self.session_store
            .create_session(
                &child_session_id,
                SessionMetadata {
                    parent_session_id: Some(parent_session_id.clone()),
                    agent_type: Some(agent_def.name.to_string()),
                    task_id: Some(task_id.to_string()),
                    depth,
                    spawned_by_tool_call_id: None,
                },
            )
            .map_err(|e| RuntimeError::SessionPersist {
                message: e.to_string(),
            })?;

        let system_prompt = format!(
            "{}\n\n--- Agent Role ---\n{}\n",
            SUBAGENT_SYSTEM_PROMPT_WRAPPER, agent_def.system_prompt,
        );

        let user_prompt = compose_user_prompt(task_description, task_assignment, context);

        let model = self
            .model_resolver
            .resolve_model(&child_session_id, None)
            .await
            .map_err(|message| RuntimeError::ModelResolution { message })?;

        let api_key = self
            .auth_resolver
            .get_api_key(&model.provider, &child_session_id)
            .await
            .map_err(|message| RuntimeError::AuthResolution { message })?;

        let provider = self
            .provider_registry
            .get_provider(&model.provider)
            .ok_or_else(|| RuntimeError::ProviderNotFound {
                provider: model.provider.clone(),
            })?;

        let provider_context = lorum_ai_contract::ProviderContext {
            api_key,
            timeout_ms: self.config.timeout_ms,
        };

        let turn_id = TurnId::from(format!("{}-turn-0", child_session_id.as_str()));

        let _result = run_agent_loop(AgentLoopParams {
            session_id: &child_session_id,
            turn_id,
            prompt: user_prompt,
            system_prompt: Some(system_prompt.clone()),
            model: model.clone(),
            provider: Arc::clone(&provider),
            provider_context: provider_context.clone(),
            session_store: Arc::clone(&self.session_store),
            tool_dispatcher: Some(dispatcher),
            subscribers: parent_subscribers,
            config: self.config,
            tool_choice: None,
            tool_set: ToolSetContext {
                depth,
                max_recursion_depth: self.config.max_tool_turns,
                require_submit_result: true,
            },
        })
        .await?;

        // Scan for submit_result; if missing, run reminder loop
        let mut submit_payload = self.scan_child_submit(&child_session_id)?;

        if submit_payload.is_none() {
            for retry in 0..MAX_SUBMIT_RETRIES {
                let reminder_turn_id = TurnId::from(format!(
                    "{}-reminder-{}",
                    child_session_id.as_str(),
                    retry
                ));

                let _retry_result = run_agent_loop(AgentLoopParams {
                    session_id: &child_session_id,
                    turn_id: reminder_turn_id,
                    prompt: SUBMIT_RESULT_REMINDER.to_string(),
                    system_prompt: Some(system_prompt.clone()),
                    model: model.clone(),
                    provider: Arc::clone(&provider),
                    provider_context: provider_context.clone(),
                    session_store: Arc::clone(&self.session_store),
                    tool_dispatcher: Some(dispatcher),
                    subscribers: parent_subscribers,
                    config: self.config,
                    tool_choice: Some(ToolChoice::Specific {
                        name: "submit_result".to_string(),
                    }),
                    tool_set: ToolSetContext {
                        depth,
                        max_recursion_depth: self.config.max_tool_turns,
                        require_submit_result: true,
                    },
                })
                .await?;

                submit_payload = self.scan_child_submit(&child_session_id)?;
                if submit_payload.is_some() {
                    break;
                }
            }
        }

        let raw_output = extract_raw_output(&self.scan_child_events(&child_session_id)?);
        let finalized = finalize_subagent_output(submit_payload.as_ref(), &raw_output, &self.config);

        Ok(SingleTaskResult {
            task_id: task_id.to_string(),
            status: finalized.status,
            output: finalized.output,
            error: finalized.error,
        })
    }

    fn scan_child_submit(&self, child_session_id: &SessionId) -> Result<Option<Value>, RuntimeError> {
        let events = self.scan_child_events(child_session_id)?;
        Ok(scan_for_submit_result(&events))
    }

    fn scan_child_events(&self, child_session_id: &SessionId) -> Result<Vec<RuntimeEvent>, RuntimeError> {
        self.session_store
            .replay(child_session_id)
            .map_err(|e| RuntimeError::SessionReplay {
                message: e.to_string(),
            })
    }
}

fn compose_user_prompt(
    description: &str,
    assignment: Option<&str>,
    context: Option<&str>,
) -> String {
    let mut parts = Vec::new();

    if let Some(ctx) = context {
        parts.push(format!("<context>\n{ctx}\n</context>"));
    }

    let goal = if let Some(assign) = assignment {
        format!("{description}\n\n{assign}")
    } else {
        description.to_string()
    };
    parts.push(format!("<goal>\n{goal}\n</goal>"));

    parts.join("\n\n")
}

fn scan_for_submit_result(events: &[RuntimeEvent]) -> Option<Value> {
    let mut submit_tool_call_ids: HashSet<String> = HashSet::new();

    for event in events {
        if let RuntimeEvent::ToolExecutionStart {
            tool_name,
            tool_call_id,
            ..
        } = event
        {
            if tool_name == "submit_result" {
                submit_tool_call_ids.insert(tool_call_id.clone());
            }
        }
    }

    for event in events.iter().rev() {
        if let RuntimeEvent::ToolResultReceived {
            tool_call_id,
            is_error,
            result,
            ..
        } = event
        {
            if submit_tool_call_ids.contains(tool_call_id) && !is_error {
                return Some(result.clone());
            }
        }
    }

    None
}

// === Output finalization ===

pub struct FinalizedOutput {
    pub status: TaskStatus,
    pub output: Option<Value>,
    pub error: Option<String>,
}

/// Extract raw text output from child session events (assistant text content).
fn extract_raw_output(events: &[RuntimeEvent]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for event in events {
        if let RuntimeEvent::AssistantStreamDelta { delta, .. } = event {
            parts.push(delta.clone());
        }
    }
    parts.join("")
}

/// Finalize subagent output using the decision table from the spec.
fn finalize_subagent_output(
    submit_payload: Option<&Value>,
    raw_output: &str,
    config: &RuntimeConfig,
) -> FinalizedOutput {
    match submit_payload {
        Some(payload) => {
            // Check for error result
            if let Some(error) = payload.get("error").and_then(|v| v.as_str()) {
                return FinalizedOutput {
                    status: TaskStatus::Failed,
                    output: None,
                    error: Some(error.to_string()),
                };
            }

            // Check for null data
            let data = payload.get("data");
            if data.is_some_and(|v| v.is_null()) {
                return FinalizedOutput {
                    status: TaskStatus::Completed,
                    output: Some(Value::String(format!(
                        "{}\n{}",
                        WARNING_NULL_DATA,
                        truncate_output(raw_output, config)
                    ))),
                    error: None,
                };
            }

            // Success with data
            let output = data.map(|v| {
                let text = serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string());
                Value::String(truncate_output(&text, config))
            });

            FinalizedOutput {
                status: TaskStatus::Completed,
                output,
                error: None,
            }
        }
        None => {
            // No submit_result — try to salvage raw output
            let trimmed = raw_output.trim();
            if !trimmed.is_empty() {
                FinalizedOutput {
                    status: TaskStatus::Completed,
                    output: Some(Value::String(format!(
                        "{}\n{}",
                        WARNING_MISSING_SUBMIT,
                        truncate_output(trimmed, config)
                    ))),
                    error: None,
                }
            } else {
                FinalizedOutput {
                    status: TaskStatus::Failed,
                    output: None,
                    error: Some(WARNING_MISSING_SUBMIT.to_string()),
                }
            }
        }
    }
}

fn truncate_output(text: &str, config: &RuntimeConfig) -> String {
    let max_bytes = config.max_output_bytes;
    let max_lines = config.max_output_lines;

    let mut result = String::new();

    for (line_count, line) in text.lines().enumerate() {
        if line_count >= max_lines {
            result.push_str(&format!(
                "\n... (truncated at {} lines)",
                max_lines
            ));
            return result;
        }

        if !result.is_empty() {
            result.push('\n');
        }

        if result.len() + line.len() > max_bytes {
            let remaining = max_bytes.saturating_sub(result.len());
            if remaining > 0 {
                result.push_str(&line[..remaining.min(line.len())]);
            }
            result.push_str(&format!(
                "\n... (truncated at {} bytes)",
                max_bytes
            ));
            return result;
        }

        result.push_str(line);
    }

    result
}

// === RuntimeToolHandler implementations ===

pub struct SubagentHandler {
    executor: Arc<SubagentExecutor>,
    agents: Vec<AgentDefinition>,
    max_recursion_depth: u32,
    dispatcher: Arc<ToolDispatcher>,
    task_definition: ToolDefinition,
}

impl SubagentHandler {
    pub fn new(
        executor: Arc<SubagentExecutor>,
        agents: Vec<AgentDefinition>,
        max_recursion_depth: u32,
        dispatcher: Arc<ToolDispatcher>,
        task_definition: ToolDefinition,
    ) -> Self {
        Self {
            executor,
            agents,
            max_recursion_depth,
            dispatcher,
            task_definition,
        }
    }

    fn find_agent(&self, name: &str) -> Option<&AgentDefinition> {
        self.agents.iter().find(|a| a.name == name)
    }

    fn validate_task_args(&self, args: &Value) -> Result<(), String> {
        let agent = args
            .get("agent")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing required parameter: agent".to_string())?;

        if self.find_agent(agent).is_none() {
            let valid: Vec<&str> = self.agents.iter().map(|a| a.name).collect();
            return Err(format!(
                "invalid agent type '{agent}'. Must be one of: {}",
                valid.join(", ")
            ));
        }

        let tasks = args
            .get("tasks")
            .and_then(|v| v.as_array())
            .ok_or_else(|| "missing required parameter: tasks".to_string())?;

        if tasks.is_empty() {
            return Err("tasks array must not be empty".to_string());
        }

        let mut seen_ids: HashSet<String> = HashSet::new();
        for task in tasks {
            let id = task
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "each task must have a non-empty id".to_string())?;

            if id.is_empty() {
                return Err("each task must have a non-empty id".to_string());
            }

            if id.len() > 48 {
                return Err(format!(
                    "task id '{id}' exceeds max length of 48 characters"
                ));
            }

            let normalized = id.to_lowercase();
            if !seen_ids.insert(normalized) {
                return Err(format!("duplicate task id: '{id}'"));
            }

            if task.get("description").and_then(|v| v.as_str()).is_none() {
                return Err(format!("task '{id}' must have a description"));
            }
        }

        Ok(())
    }

    async fn dispatch_task(
        &self,
        tool_call: &ToolCall,
        ctx: &ToolDispatchContext,
    ) -> ToolExecutionResult {
        let args = &tool_call.arguments;

        if let Err(msg) = self.validate_task_args(args) {
            return ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                is_error: true,
                result: Value::String(msg),
            };
        }

        let agent_name = args["agent"].as_str().unwrap();
        let agent_def = self.find_agent(agent_name).unwrap();
        let tasks = args["tasks"].as_array().unwrap();
        let context = args.get("context").and_then(|v| v.as_str());
        let schema = args.get("schema");

        let depth = ctx.tool_set.depth + 1;

        let mut results: Vec<SingleTaskResult> = Vec::with_capacity(tasks.len());

        for task_item in tasks {
            let task_id = task_item["id"].as_str().unwrap();
            let description = task_item["description"].as_str().unwrap();
            let assignment = task_item.get("assignment").and_then(|v| v.as_str());

            let child_session_id = SessionId::from(format!(
                "{}-task-{}",
                ctx.session_id.as_str(),
                task_id
            ));

            let _ = persist_and_broadcast(
                ctx.session_store.as_ref(),
                &ctx.session_id,
                RuntimeEvent::SubagentSpawned {
                    turn_id: ctx.turn_id.clone(),
                    sequence_no: 0,
                    session_id: ctx.session_id.clone(),
                    child_session_id: child_session_id.clone(),
                    tool_call_id: tool_call.id.clone(),
                    agent_type: agent_name.to_string(),
                    task_id: task_id.to_string(),
                },
                &ctx.subscribers,
            );

            let result = self
                .executor
                .execute_task(
                    &ctx.session_id,
                    &ctx.subscribers,
                    agent_def,
                    task_id,
                    description,
                    assignment,
                    context,
                    schema,
                    depth,
                    &self.dispatcher,
                )
                .await;

            let (task_result, status_str) = match result {
                Ok(task_result) => {
                    let status = match task_result.status {
                        TaskStatus::Completed => "completed",
                        TaskStatus::Failed => "failed",
                        TaskStatus::Aborted => "aborted",
                    };
                    (task_result, status.to_string())
                }
                Err(e) => {
                    let tr = SingleTaskResult {
                        task_id: task_id.to_string(),
                        status: TaskStatus::Failed,
                        output: None,
                        error: Some(e.to_string()),
                    };
                    (tr, "failed".to_string())
                }
            };

            let _ = persist_and_broadcast(
                ctx.session_store.as_ref(),
                &ctx.session_id,
                RuntimeEvent::SubagentCompleted {
                    turn_id: ctx.turn_id.clone(),
                    sequence_no: 0,
                    session_id: ctx.session_id.clone(),
                    child_session_id,
                    tool_call_id: tool_call.id.clone(),
                    agent_type: agent_name.to_string(),
                    status: status_str,
                },
                &ctx.subscribers,
            );

            results.push(task_result);
        }

        format_task_results(tool_call.id.clone(), &results)
    }
}

#[async_trait]
impl RuntimeToolHandler for SubagentHandler {
    fn claimed_names(&self) -> &'static [&'static str] {
        &["task"]
    }

    fn definitions(&self, ctx: &ToolSetContext) -> Vec<ToolDefinition> {
        if ctx.depth >= self.max_recursion_depth {
            return vec![];
        }
        vec![self.task_definition.clone()]
    }

    async fn execute(
        &self,
        tool_call: &ToolCall,
        ctx: &ToolDispatchContext,
    ) -> ToolExecutionResult {
        self.dispatch_task(tool_call, ctx).await
    }
}

pub struct SubmitResultHandler;

fn submit_result_definition() -> ToolDefinition {
    ToolDefinition {
        name: "submit_result".to_string(),
        description: "Submit the final result of this task. You MUST call this exactly once \
            before finishing. Use data for success or error for failure."
            .to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "result": {
                    "type": "object",
                    "description": "The result payload",
                    "properties": {
                        "data": {
                            "description": "The structured result data (for success)"
                        },
                        "error": {
                            "type": "string",
                            "description": "Error message (for failure)"
                        }
                    }
                }
            },
            "required": ["result"]
        }),
    }
}

fn parse_submit_result_args(args: &Value) -> Result<Value, &'static str> {
    let result = args
        .get("result")
        .ok_or("result must be an object containing either data or error")?;

    if !result.is_object() {
        return Err("result must be an object containing either data or error");
    }

    let has_data = result.get("data").is_some();
    let has_error = result.get("error").is_some();

    if has_data && has_error {
        return Err("result cannot contain both data and error");
    }
    if !has_data && !has_error {
        return Err("result must contain either data or error");
    }
    if has_data && result.get("data").unwrap().is_null() {
        return Err("data is required when submit_result indicates success");
    }

    Ok(result.clone())
}

#[async_trait]
impl RuntimeToolHandler for SubmitResultHandler {
    fn claimed_names(&self) -> &'static [&'static str] {
        &["submit_result"]
    }

    fn definitions(&self, ctx: &ToolSetContext) -> Vec<ToolDefinition> {
        if !ctx.require_submit_result {
            return vec![];
        }
        vec![submit_result_definition()]
    }

    async fn execute(
        &self,
        tool_call: &ToolCall,
        ctx: &ToolDispatchContext,
    ) -> ToolExecutionResult {
        if !ctx.tool_set.require_submit_result {
            return ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                is_error: true,
                result: Value::String(
                    "submit_result is not available in this session".to_string(),
                ),
            };
        }

        match parse_submit_result_args(&tool_call.arguments) {
            Ok(result_value) => ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                is_error: false,
                result: result_value,
            },
            Err(msg) => ToolExecutionResult {
                tool_call_id: tool_call.id.clone(),
                is_error: true,
                result: Value::String(msg.to_string()),
            },
        }
    }
}

fn format_task_results(tool_call_id: String, results: &[SingleTaskResult]) -> ToolExecutionResult {
    let items: Vec<Value> = results
        .iter()
        .map(|r| {
            let mut item = json!({
                "id": r.task_id,
                "status": match r.status {
                    TaskStatus::Completed => "completed",
                    TaskStatus::Failed => "failed",
                    TaskStatus::Aborted => "aborted",
                },
            });
            if let Some(ref output) = r.output {
                item["output"] = output.clone();
            }
            if let Some(ref error) = r.error {
                item["error"] = Value::String(error.clone());
            }
            item
        })
        .collect();

    let any_error = results.iter().any(|r| r.status != TaskStatus::Completed);

    ToolExecutionResult {
        tool_call_id,
        is_error: any_error,
        result: json!({ "results": items }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lorum_ai_contract::ModelRef;

    #[test]
    fn compose_user_prompt_with_all_parts() {
        let prompt =
            compose_user_prompt("find bugs", Some("check auth module"), Some("we use JWT"));
        assert!(prompt.contains("<context>"));
        assert!(prompt.contains("we use JWT"));
        assert!(prompt.contains("<goal>"));
        assert!(prompt.contains("find bugs"));
        assert!(prompt.contains("check auth module"));
    }

    #[test]
    fn compose_user_prompt_without_context() {
        let prompt = compose_user_prompt("find bugs", None, None);
        assert!(!prompt.contains("<context>"));
        assert!(prompt.contains("<goal>"));
        assert!(prompt.contains("find bugs"));
    }

    #[test]
    fn scan_finds_submit_result() {
        let events = vec![
            RuntimeEvent::ToolExecutionStart {
                turn_id: TurnId::from("t1"),
                sequence_no: 1,
                tool_call_id: "tc-1".to_string(),
                tool_name: "submit_result".to_string(),
                arguments: json!({}),
            },
            RuntimeEvent::ToolResultReceived {
                turn_id: TurnId::from("t1"),
                sequence_no: 3,
                tool_call_id: "tc-1".to_string(),
                is_error: false,
                result: json!({ "data": { "answer": 42 } }),
            },
        ];
        let found = scan_for_submit_result(&events);
        assert_eq!(found, Some(json!({ "data": { "answer": 42 } })));
    }

    #[test]
    fn scan_returns_none_when_no_submit() {
        let events = vec![RuntimeEvent::ToolExecutionStart {
            turn_id: TurnId::from("t1"),
            sequence_no: 1,
            tool_call_id: "tc-1".to_string(),
            tool_name: "read".to_string(),
            arguments: json!({}),
        }];
        assert_eq!(scan_for_submit_result(&events), None);
    }

    #[test]
    fn scan_ignores_error_submit_results() {
        let events = vec![
            RuntimeEvent::ToolExecutionStart {
                turn_id: TurnId::from("t1"),
                sequence_no: 1,
                tool_call_id: "tc-1".to_string(),
                tool_name: "submit_result".to_string(),
                arguments: json!({}),
            },
            RuntimeEvent::ToolResultReceived {
                turn_id: TurnId::from("t1"),
                sequence_no: 3,
                tool_call_id: "tc-1".to_string(),
                is_error: true,
                result: json!("validation error"),
            },
        ];
        assert_eq!(scan_for_submit_result(&events), None);
    }

    #[test]
    fn format_results_marks_error_when_any_failed() {
        let results = vec![
            SingleTaskResult {
                task_id: "a".to_string(),
                status: TaskStatus::Completed,
                output: Some(json!("ok")),
                error: None,
            },
            SingleTaskResult {
                task_id: "b".to_string(),
                status: TaskStatus::Failed,
                output: None,
                error: Some("boom".to_string()),
            },
        ];
        let exec = format_task_results("tc-1".to_string(), &results);
        assert!(exec.is_error);
        let items = exec.result["results"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["status"], "completed");
        assert_eq!(items[1]["status"], "failed");
    }

    #[test]
    fn validate_rejects_empty_tasks() {
        let handler = make_test_handler();
        let args = json!({ "agent": "explore", "tasks": [] });
        let err = handler.validate_task_args(&args).unwrap_err();
        assert_eq!(err, "tasks array must not be empty");
    }

    #[test]
    fn validate_rejects_duplicate_ids() {
        let handler = make_test_handler();
        let args = json!({
            "agent": "explore",
            "tasks": [
                { "id": "abc", "description": "first" },
                { "id": "ABC", "description": "second" }
            ]
        });
        let err = handler.validate_task_args(&args).unwrap_err();
        assert!(err.contains("duplicate"));
    }

    #[test]
    fn validate_rejects_unknown_agent() {
        let handler = make_test_handler();
        let args = json!({
            "agent": "nonexistent",
            "tasks": [{ "id": "a", "description": "test" }]
        });
        let err = handler.validate_task_args(&args).unwrap_err();
        assert!(err.contains("invalid agent type"));
    }

    #[test]
    fn validate_accepts_valid_args() {
        let handler = make_test_handler();
        let args = json!({
            "agent": "explore",
            "tasks": [
                { "id": "a", "description": "find stuff" },
                { "id": "b", "description": "find more" }
            ]
        });
        handler.validate_task_args(&args).unwrap();
    }

    #[test]
    fn parse_submit_result_success() {
        let args = json!({ "result": { "data": { "answer": 42 } } });
        let result = parse_submit_result_args(&args).unwrap();
        assert_eq!(result, json!({ "data": { "answer": 42 } }));
    }

    #[test]
    fn parse_submit_result_error() {
        let args = json!({ "result": { "error": "something broke" } });
        let result = parse_submit_result_args(&args).unwrap();
        assert_eq!(result, json!({ "error": "something broke" }));
    }

    #[test]
    fn parse_submit_result_rejects_both() {
        let args = json!({ "result": { "data": 1, "error": "oops" } });
        assert!(parse_submit_result_args(&args).is_err());
    }

    #[test]
    fn parse_submit_result_rejects_null_data() {
        let args = json!({ "result": { "data": null } });
        assert!(parse_submit_result_args(&args).is_err());
    }

    // === Finalization tests ===

    fn test_config() -> RuntimeConfig {
        RuntimeConfig {
            max_tool_turns: 5,
            timeout_ms: 30_000,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            max_output_lines: DEFAULT_MAX_OUTPUT_LINES,
        }
    }

    #[test]
    fn finalize_with_data_succeeds() {
        let payload = json!({ "data": { "answer": 42 } });
        let result = finalize_subagent_output(Some(&payload), "", &test_config());
        assert_eq!(result.status, TaskStatus::Completed);
        assert!(result.output.is_some());
        assert!(result.error.is_none());
        let text = result.output.unwrap();
        assert!(text.as_str().unwrap().contains("42"));
    }

    #[test]
    fn finalize_with_error_fails() {
        let payload = json!({ "error": "something broke" });
        let result = finalize_subagent_output(Some(&payload), "", &test_config());
        assert_eq!(result.status, TaskStatus::Failed);
        assert!(result.output.is_none());
        assert_eq!(result.error.unwrap(), "something broke");
    }

    #[test]
    fn finalize_with_null_data_warns() {
        let payload = json!({ "data": null });
        let result = finalize_subagent_output(Some(&payload), "some raw output", &test_config());
        assert_eq!(result.status, TaskStatus::Completed);
        let text = result.output.unwrap();
        let s = text.as_str().unwrap();
        assert!(s.starts_with(WARNING_NULL_DATA));
        assert!(s.contains("some raw output"));
    }

    #[test]
    fn finalize_missing_with_raw_output_salvages() {
        let result = finalize_subagent_output(None, "here is my answer", &test_config());
        assert_eq!(result.status, TaskStatus::Completed);
        let text = result.output.unwrap();
        let s = text.as_str().unwrap();
        assert!(s.starts_with(WARNING_MISSING_SUBMIT));
        assert!(s.contains("here is my answer"));
    }

    #[test]
    fn finalize_missing_empty_raw_output_fails() {
        let result = finalize_subagent_output(None, "   ", &test_config());
        assert_eq!(result.status, TaskStatus::Failed);
        assert!(result.output.is_none());
        assert_eq!(result.error.unwrap(), WARNING_MISSING_SUBMIT);
    }

    #[test]
    fn truncate_by_lines() {
        let text: String = (0..10).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let cfg = RuntimeConfig {
            max_output_lines: 3,
            ..test_config()
        };
        let result = truncate_output(&text, &cfg);
        assert!(result.contains("line 0"));
        assert!(result.contains("line 2"));
        assert!(result.contains("truncated at 3 lines"));
        assert!(!result.contains("line 3"));
    }

    #[test]
    fn truncate_by_bytes() {
        let text = "a".repeat(200);
        let cfg = RuntimeConfig {
            max_output_bytes: 50,
            ..test_config()
        };
        let result = truncate_output(&text, &cfg);
        assert!(result.contains("truncated at 50 bytes"));
        assert!(result.len() < 200);
    }

    #[test]
    fn extract_raw_output_from_events() {
        let events = vec![
            RuntimeEvent::AssistantStreamDelta {
                turn_id: TurnId::from("t1"),
                sequence_no: 1,
                delta: "hello ".to_string(),
            },
            RuntimeEvent::AssistantStreamDelta {
                turn_id: TurnId::from("t1"),
                sequence_no: 2,
                delta: "world".to_string(),
            },
        ];
        assert_eq!(extract_raw_output(&events), "hello world");
    }

    fn make_test_handler() -> SubagentHandler {
        SubagentHandler::new(
            Arc::new(SubagentExecutor::new(
                Arc::new(MockAuth),
                Arc::new(MockModel),
                Arc::new(MockProviderReg),
                Arc::new(lorum_session::InMemorySessionStore::new()),
                test_config(),
            )),
            crate::agents::builtin_agents(),
            2,
            Arc::new(ToolDispatcher::new(Arc::new(MockExecutor))),
            ToolDefinition {
                name: "task".to_string(),
                description: "test".to_string(),
                parameters: json!({}),
            },
        )
    }

    struct MockAuth;
    #[async_trait]
    impl RuntimeAuthResolver for MockAuth {
        async fn get_api_key(&self, _: &str, _: &SessionId) -> Result<Option<String>, String> {
            Ok(Some("key".to_string()))
        }
    }

    struct MockModel;
    #[async_trait]
    impl RuntimeModelResolver for MockModel {
        async fn resolve_model(
            &self,
            _: &SessionId,
            _: Option<&ModelRef>,
        ) -> Result<ModelRef, String> {
            Ok(ModelRef {
                provider: "mock".to_string(),
                api: lorum_ai_contract::ApiKind::OpenAiResponses,
                model: "test".to_string(),
            })
        }
    }

    struct MockProviderReg;
    impl crate::RuntimeProviderRegistry for MockProviderReg {
        fn get_provider(&self, _: &str) -> Option<Arc<dyn lorum_ai_contract::ProviderAdapter>> {
            None
        }
    }

    struct MockExecutor;
    #[async_trait]
    impl crate::ToolExecutor for MockExecutor {
        fn definitions(&self) -> Vec<ToolDefinition> {
            vec![]
        }
        async fn execute(&self, tc: &ToolCall) -> ToolExecutionResult {
            ToolExecutionResult {
                tool_call_id: tc.id.clone(),
                is_error: true,
                result: Value::String("not implemented".to_string()),
            }
        }
    }
}
