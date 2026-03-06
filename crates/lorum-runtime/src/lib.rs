pub mod agents;
pub mod subagent;

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock, RwLockReadGuard};

use async_trait::async_trait;
use lorum_agent_core::{ChatTurnEngine, RuntimeEventSink, TurnEngine, TurnError, TurnRequest};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, ModelRef, ProviderAdapter,
    ProviderContext, ProviderError, ProviderFinal, ProviderInputMessage, ProviderRequest, ToolCall,
    ToolChoice, ToolDefinition,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_session::{SessionError, SessionStore};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub max_tool_turns: u32,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub max_output_lines: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserInputCommand {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub prompt: String,
    pub system_prompt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelSelectRequest {
    pub session_id: SessionId,
    pub model: ModelRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriptionId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeError {
    #[error("model resolver failed: {message}")]
    ModelResolution { message: String },
    #[error("auth resolver failed: {message}")]
    AuthResolution { message: String },
    #[error("provider adapter not found for provider '{provider}'")]
    ProviderNotFound { provider: String },
    #[error("subscriber registry lock poisoned")]
    SubscriberRegistryPoisoned,
    #[error("model override lock poisoned")]
    ModelOverridePoisoned,
    #[error("session replay failed: {message}")]
    SessionReplay { message: String },
    #[error("session persist failed: {message}")]
    SessionPersist { message: String },
    #[error(transparent)]
    TurnEngine(#[from] TurnError),
}

pub struct ToolExecutionResult {
    pub tool_call_id: String,
    pub is_error: bool,
    pub result: Value,
}

#[async_trait]
pub trait ToolExecutor: Send + Sync {
    fn definitions(&self) -> Vec<ToolDefinition>;
    async fn execute(&self, tool_call: &ToolCall) -> ToolExecutionResult;
}

pub struct ToolCallSummary {
    pub headline: String,
    pub detail: Option<String>,
    /// Optional multi-line body content (diffs, file content, etc.)
    pub body: Option<String>,
}

#[derive(Debug, PartialEq)]
pub struct ToolResultSummary {
    pub headline: String,
    /// Optional multi-line body content (file content with CIDs, grep matches, etc.)
    pub body: Option<String>,
}

pub trait ToolCallDisplay: Send + Sync {
    fn format_call(&self, tool_name: &str, args: &Value) -> ToolCallSummary;
    fn format_result(&self, tool_name: &str, is_error: bool, result: &Value) -> ToolResultSummary;
}

#[async_trait]
pub trait RuntimeAuthResolver: Send + Sync {
    async fn get_api_key(
        &self,
        provider: &str,
        session_id: &SessionId,
    ) -> Result<Option<String>, String>;
}

#[async_trait]
pub trait RuntimeModelResolver: Send + Sync {
    async fn resolve_model(
        &self,
        session_id: &SessionId,
        override_model: Option<&ModelRef>,
    ) -> Result<ModelRef, String>;
}

pub trait RuntimeProviderRegistry: Send + Sync {
    fn get_provider(&self, provider_id: &str) -> Option<Arc<dyn ProviderAdapter>>;
}

pub trait RuntimeSubscriber: Send + Sync {
    fn on_event(&self, event: &RuntimeEvent);
}

#[async_trait]
pub trait RuntimeController: Send + Sync {
    async fn submit_user_input(&self, cmd: UserInputCommand) -> Result<(), RuntimeError>;
    async fn set_model(&self, req: ModelSelectRequest) -> Result<(), RuntimeError>;
    async fn subscribe(
        &self,
        subscriber: Arc<dyn RuntimeSubscriber>,
    ) -> Result<SubscriptionId, RuntimeError>;
}

pub struct AgentLoopParams<'a> {
    pub session_id: &'a SessionId,
    pub turn_id: TurnId,
    pub prompt: String,
    pub system_prompt: Option<String>,
    pub model: ModelRef,
    pub provider: Arc<dyn ProviderAdapter>,
    pub provider_context: ProviderContext,
    pub session_store: Arc<dyn SessionStore>,
    pub tool_dispatcher: Option<&'a ToolDispatcher>,
    pub subscribers: &'a [Arc<dyn RuntimeSubscriber>],
    pub config: RuntimeConfig,
    pub tool_choice: Option<ToolChoice>,
    pub tool_set: ToolSetContext,
}

pub struct AgentLoopResult {
    pub final_turn_id: TurnId,
    pub terminal_reason: TurnTerminalReason,
}

pub struct ToolSetContext {
    pub depth: u32,
    pub max_recursion_depth: u32,
    pub require_submit_result: bool,
}

impl Default for ToolSetContext {
    fn default() -> Self {
        Self {
            depth: 0,
            max_recursion_depth: 4,
            require_submit_result: false,
        }
    }
}

pub struct ToolDispatchContext {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub subscribers: Vec<Arc<dyn RuntimeSubscriber>>,
    pub session_store: Arc<dyn SessionStore>,
    pub tool_set: ToolSetContext,
}

/// A handler for one or more runtime-managed tools.
/// Each handler declares the tool names it owns and provides
/// context-sensitive definitions and execution.
#[async_trait]
pub trait RuntimeToolHandler: Send + Sync {
    /// Tool names this handler owns for routing and deduplication.
    /// Must be context-independent (same names regardless of depth/session).
    fn claimed_names(&self) -> &'static [&'static str];

    /// Tool definitions to expose. The handler decides which tools to
    /// include based on context (e.g., omit "task" at max recursion depth).
    fn definitions(&self, ctx: &ToolSetContext) -> Vec<ToolDefinition>;

    /// Execute a tool call that this handler owns.
    async fn execute(
        &self,
        tool_call: &ToolCall,
        ctx: &ToolDispatchContext,
    ) -> ToolExecutionResult;
}

/// Routes tool calls to registered RuntimeToolHandlers or falls back
/// to the regular ToolExecutor for standard tools.
pub struct ToolDispatcher {
    tool_executor: Arc<dyn ToolExecutor>,
    handlers: RwLock<Vec<Arc<dyn RuntimeToolHandler>>>,
}

impl ToolDispatcher {
    pub fn new(tool_executor: Arc<dyn ToolExecutor>) -> Self {
        Self {
            tool_executor,
            handlers: RwLock::new(Vec::new()),
        }
    }

    pub fn register(&self, handler: Arc<dyn RuntimeToolHandler>) {
        self.handlers
            .write()
            .expect("handler registry poisoned")
            .push(handler);
    }

    fn read_handlers(&self) -> RwLockReadGuard<'_, Vec<Arc<dyn RuntimeToolHandler>>> {
        self.handlers
            .read()
            .expect("handler registry poisoned")
    }

    pub fn definitions(&self, ctx: &ToolSetContext) -> Vec<ToolDefinition> {
        let handlers = self.read_handlers();

        let claimed: HashSet<&str> = handlers
            .iter()
            .flat_map(|h| h.claimed_names().iter().copied())
            .collect();

        let mut defs: Vec<ToolDefinition> = self
            .tool_executor
            .definitions()
            .into_iter()
            .filter(|d| !claimed.contains(d.name.as_str()))
            .collect();

        for handler in handlers.iter() {
            defs.extend(handler.definitions(ctx));
        }

        defs
    }

    pub async fn dispatch(
        &self,
        tool_call: &ToolCall,
        ctx: &ToolDispatchContext,
    ) -> ToolExecutionResult {
        // Clone the matching handler out so we don't hold the lock across await
        let handler = {
            let handlers = self.read_handlers();
            handlers
                .iter()
                .find(|h| h.claimed_names().contains(&tool_call.name.as_str()))
                .cloned()
        };

        if let Some(handler) = handler {
            return handler.execute(tool_call, ctx).await;
        }

        self.tool_executor.execute(tool_call).await
    }

    pub fn executor(&self) -> &dyn ToolExecutor {
        self.tool_executor.as_ref()
    }
}

pub struct ChatOnlyRuntime {
    config: RuntimeConfig,
    auth_resolver: Arc<dyn RuntimeAuthResolver>,
    model_resolver: Arc<dyn RuntimeModelResolver>,
    provider_registry: Arc<dyn RuntimeProviderRegistry>,
    session_store: Arc<dyn SessionStore>,
    subscribers: RwLock<HashMap<SubscriptionId, Arc<dyn RuntimeSubscriber>>>,
    next_subscription_id: AtomicU64,
    model_overrides: RwLock<HashMap<SessionId, ModelRef>>,
    tool_dispatcher: Option<Arc<ToolDispatcher>>,
}

impl ChatOnlyRuntime {
    pub fn new(
        config: RuntimeConfig,
        auth_resolver: Arc<dyn RuntimeAuthResolver>,
        model_resolver: Arc<dyn RuntimeModelResolver>,
        provider_registry: Arc<dyn RuntimeProviderRegistry>,
        session_store: Arc<dyn SessionStore>,
        tool_dispatcher: Option<Arc<ToolDispatcher>>,
    ) -> Self {
        Self {
            config,
            auth_resolver,
            model_resolver,
            provider_registry,
            session_store,
            subscribers: RwLock::new(HashMap::new()),
            next_subscription_id: AtomicU64::new(1),
            model_overrides: RwLock::new(HashMap::new()),
            tool_dispatcher,
        }
    }

    fn read_model_override(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<ModelRef>, RuntimeError> {
        let guard = self
            .model_overrides
            .read()
            .map_err(|_| RuntimeError::ModelOverridePoisoned)?;
        Ok(guard.get(session_id).cloned())
    }

    fn subscribers_snapshot(&self) -> Result<Vec<Arc<dyn RuntimeSubscriber>>, RuntimeError> {
        let guard = self
            .subscribers
            .read()
            .map_err(|_| RuntimeError::SubscriberRegistryPoisoned)?;
        Ok(guard.values().cloned().collect())
    }
}

#[async_trait]
impl RuntimeController for ChatOnlyRuntime {
    async fn submit_user_input(&self, cmd: UserInputCommand) -> Result<(), RuntimeError> {
        let model_override = self.read_model_override(&cmd.session_id)?;
        let model = self
            .model_resolver
            .resolve_model(&cmd.session_id, model_override.as_ref())
            .await
            .map_err(|message| RuntimeError::ModelResolution { message })?;

        let api_key = self
            .auth_resolver
            .get_api_key(&model.provider, &cmd.session_id)
            .await
            .map_err(|message| RuntimeError::AuthResolution { message })?;

        let provider = self
            .provider_registry
            .get_provider(&model.provider)
            .ok_or_else(|| RuntimeError::ProviderNotFound {
                provider: model.provider.clone(),
            })?;

        let provider_context = ProviderContext {
            api_key,
            timeout_ms: self.config.timeout_ms,
        };

        let subscribers = self.subscribers_snapshot()?;

        run_agent_loop(AgentLoopParams {
            session_id: &cmd.session_id,
            turn_id: cmd.turn_id,
            prompt: cmd.prompt,
            system_prompt: cmd.system_prompt,
            model,
            provider,
            provider_context,
            session_store: Arc::clone(&self.session_store),
            tool_dispatcher: self.tool_dispatcher.as_deref(),
            subscribers: &subscribers,
            config: self.config,
            tool_choice: None,
            tool_set: ToolSetContext::default(),
        })
        .await?;

        Ok(())
    }

    async fn set_model(&self, req: ModelSelectRequest) -> Result<(), RuntimeError> {
        let mut guard = self
            .model_overrides
            .write()
            .map_err(|_| RuntimeError::ModelOverridePoisoned)?;
        guard.insert(req.session_id, req.model);
        Ok(())
    }

    async fn subscribe(
        &self,
        subscriber: Arc<dyn RuntimeSubscriber>,
    ) -> Result<SubscriptionId, RuntimeError> {
        let id = SubscriptionId(self.next_subscription_id.fetch_add(1, Ordering::Relaxed));
        let mut guard = self
            .subscribers
            .write()
            .map_err(|_| RuntimeError::SubscriberRegistryPoisoned)?;
        guard.insert(id, subscriber);
        Ok(id)
    }
}

/// Runs a complete agent loop: replay history, prompt provider,
/// execute tools, repeat until done or max_tool_turns.
pub async fn run_agent_loop(params: AgentLoopParams<'_>) -> Result<AgentLoopResult, RuntimeError> {
    let session_events = params
        .session_store
        .replay(params.session_id)
        .map_err(|e| RuntimeError::SessionReplay {
            message: e.to_string(),
        })?;
    let mut history = lorum_session::reconstruct_conversation(&session_events);

    history.push(ProviderInputMessage::User {
        content: params.prompt.clone(),
    });

    persist_and_broadcast(
        params.session_store.as_ref(),
        params.session_id,
        RuntimeEvent::UserMessageReceived {
            turn_id: params.turn_id.clone(),
            session_id: params.session_id.clone(),
            sequence_no: 0,
            content: params.prompt,
        },
        params.subscribers,
    )?;

    let mut current_turn_id = params.turn_id.clone();
    let mut tool_turns = 0u32;
    let mut starting_sequence_no = 1u64;
    #[allow(unused_assignments)]
    let mut terminal_reason = TurnTerminalReason::Done;

    loop {
        let tool_definitions = match params.tool_dispatcher {
            Some(dispatcher) => dispatcher.definitions(&params.tool_set),
            None => vec![],
        };

        let provider_request = ProviderRequest {
            session_id: params.session_id.as_str().to_string(),
            model: params.model.clone(),
            system_prompt: params.system_prompt.clone(),
            input: history.clone(),
            tools: tool_definitions,
            tool_choice: params.tool_choice.clone(),
        };

        let request = TurnRequest {
            session_id: params.session_id.clone(),
            turn_id: current_turn_id.clone(),
            provider_request,
            provider_context: params.provider_context.clone(),
            cancellation_token: None,
            starting_sequence_no,
        };

        let mut sink = PersistAndBroadcastSink {
            session_id: params.session_id.clone(),
            session_store: Arc::clone(&params.session_store),
            subscribers: params.subscribers.to_vec(),
        };

        let engine = ChatTurnEngine::new(ProviderAdapterHandle {
            inner: Arc::clone(&params.provider),
        });
        let result = engine.run_turn(request, &mut sink).await?;

        terminal_reason = result.terminal_reason;

        if result.terminal_reason != TurnTerminalReason::ToolUse {
            break;
        }

        let dispatcher = match params.tool_dispatcher {
            Some(d) if params.config.max_tool_turns > 0 => d,
            _ => {
                if let Some(ref msg) = result.assistant_message {
                    inject_synthetic_tool_results_for_orphaned_calls(
                        params.session_store.as_ref(),
                        params.session_id,
                        &current_turn_id,
                        msg,
                        "Tool execution is not available",
                        &mut history,
                        params.subscribers,
                        &mut starting_sequence_no,
                    )?;
                }
                break;
            }
        };

        tool_turns += 1;
        if tool_turns > params.config.max_tool_turns {
            if let Some(ref msg) = result.assistant_message {
                inject_synthetic_tool_results_for_orphaned_calls(
                    params.session_store.as_ref(),
                    params.session_id,
                    &current_turn_id,
                    msg,
                    "Maximum tool turns exceeded",
                    &mut history,
                    params.subscribers,
                    &mut starting_sequence_no,
                )?;
            }
            break;
        }

        let assistant_message = match result.assistant_message {
            Some(msg) => msg,
            None => break,
        };

        let tool_calls: Vec<ToolCall> = assistant_message
            .content
            .iter()
            .filter_map(|c| match c {
                AssistantContent::ToolCall(tc) => Some(tc.clone()),
                _ => None,
            })
            .collect();

        if tool_calls.is_empty() {
            break;
        }

        history.push(ProviderInputMessage::Assistant {
            message: assistant_message,
        });

        current_turn_id = TurnId::from(format!(
            "{}-cont-{}",
            params.turn_id.as_str(),
            tool_turns
        ));

        let dispatch_ctx = ToolDispatchContext {
            session_id: params.session_id.clone(),
            turn_id: current_turn_id.clone(),
            subscribers: params.subscribers.to_vec(),
            session_store: Arc::clone(&params.session_store),
            tool_set: ToolSetContext {
                depth: params.tool_set.depth,
                max_recursion_depth: params.tool_set.max_recursion_depth,
                require_submit_result: params.tool_set.require_submit_result,
            },
        };

        let mut seq = 1u64;
        for tool_call in &tool_calls {
            persist_and_broadcast(
                params.session_store.as_ref(),
                params.session_id,
                RuntimeEvent::ToolExecutionStart {
                    turn_id: current_turn_id.clone(),
                    sequence_no: seq,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    arguments: tool_call.arguments.clone(),
                },
                params.subscribers,
            )?;
            seq += 1;

            let exec_result = dispatcher.dispatch(tool_call, &dispatch_ctx).await;

            persist_and_broadcast(
                params.session_store.as_ref(),
                params.session_id,
                RuntimeEvent::ToolExecutionEnd {
                    turn_id: current_turn_id.clone(),
                    sequence_no: seq,
                    tool_call_id: tool_call.id.clone(),
                    tool_name: tool_call.name.clone(),
                    is_error: exec_result.is_error,
                },
                params.subscribers,
            )?;
            seq += 1;

            history.push(ProviderInputMessage::ToolResult {
                tool_call_id: exec_result.tool_call_id.clone(),
                is_error: exec_result.is_error,
                result: exec_result.result.clone(),
            });

            persist_and_broadcast(
                params.session_store.as_ref(),
                params.session_id,
                RuntimeEvent::ToolResultReceived {
                    turn_id: current_turn_id.clone(),
                    sequence_no: seq,
                    tool_call_id: exec_result.tool_call_id,
                    is_error: exec_result.is_error,
                    result: exec_result.result,
                },
                params.subscribers,
            )?;
            seq += 1;
        }

        starting_sequence_no = seq;
    }

    Ok(AgentLoopResult {
        final_turn_id: current_turn_id,
        terminal_reason,
    })
}

pub(crate) fn persist_and_broadcast(
    session_store: &dyn SessionStore,
    session_id: &SessionId,
    event: RuntimeEvent,
    subscribers: &[Arc<dyn RuntimeSubscriber>],
) -> Result<(), RuntimeError> {
    session_store
        .append(session_id, event.clone())
        .map_err(|e| RuntimeError::SessionPersist {
            message: e.to_string(),
        })?;
    for subscriber in subscribers {
        subscriber.on_event(&event);
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn inject_synthetic_tool_results_for_orphaned_calls(
    session_store: &dyn SessionStore,
    session_id: &SessionId,
    turn_id: &TurnId,
    assistant_message: &AssistantMessage,
    reason: &str,
    history: &mut Vec<ProviderInputMessage>,
    subscribers: &[Arc<dyn RuntimeSubscriber>],
    seq: &mut u64,
) -> Result<(), RuntimeError> {
    let tool_calls: Vec<&ToolCall> = assistant_message
        .content
        .iter()
        .filter_map(|c| match c {
            AssistantContent::ToolCall(tc) => Some(tc),
            _ => None,
        })
        .collect();

    for tc in tool_calls {
        let result = Value::String(reason.to_string());

        persist_and_broadcast(
            session_store,
            session_id,
            RuntimeEvent::ToolResultReceived {
                turn_id: turn_id.clone(),
                sequence_no: *seq,
                tool_call_id: tc.id.clone(),
                is_error: true,
                result: result.clone(),
            },
            subscribers,
        )?;
        *seq += 1;

        history.push(ProviderInputMessage::ToolResult {
            tool_call_id: tc.id.clone(),
            is_error: true,
            result,
        });
    }

    Ok(())
}

struct PersistAndBroadcastSink {
    session_id: SessionId,
    session_store: Arc<dyn SessionStore>,
    subscribers: Vec<Arc<dyn RuntimeSubscriber>>,
}

impl RuntimeEventSink for PersistAndBroadcastSink {
    fn push(&mut self, event: RuntimeEvent) -> Result<(), TurnError> {
        self.session_store
            .append(&self.session_id, event.clone())
            .map_err(map_session_error)?;

        for subscriber in &self.subscribers {
            subscriber.on_event(&event);
        }

        Ok(())
    }
}

fn map_session_error(err: SessionError) -> TurnError {
    TurnError::SinkFailure(err.to_string())
}

#[derive(Clone)]
struct ProviderAdapterHandle {
    inner: Arc<dyn ProviderAdapter>,
}

#[async_trait]
impl ProviderAdapter for ProviderAdapterHandle {
    fn provider_id(&self) -> &str {
        self.inner.provider_id()
    }

    fn api_kind(&self) -> ApiKind {
        self.inner.api_kind()
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        context: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError> {
        self.inner.stream(request, context, sink).await
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        self.inner.complete(request, context).await
    }

    fn supports_stateful_transport(&self) -> bool {
        self.inner.supports_stateful_transport()
    }
}
