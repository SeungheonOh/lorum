use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use lorum_agent_core::{ChatTurnEngine, RuntimeEventSink, TurnEngine, TurnError, TurnRequest};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, ModelRef, ProviderAdapter,
    ProviderContext, ProviderError, ProviderFinal, ProviderInputMessage, ProviderRequest, ToolCall,
    ToolDefinition,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_session::{SessionError, SessionStore};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub max_tool_turns: u32,
    pub timeout_ms: u64,
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

pub struct ChatOnlyRuntime {
    config: RuntimeConfig,
    auth_resolver: Arc<dyn RuntimeAuthResolver>,
    model_resolver: Arc<dyn RuntimeModelResolver>,
    provider_registry: Arc<dyn RuntimeProviderRegistry>,
    session_store: Arc<dyn SessionStore>,
    subscribers: RwLock<HashMap<SubscriptionId, Arc<dyn RuntimeSubscriber>>>,
    next_subscription_id: AtomicU64,
    model_overrides: RwLock<HashMap<SessionId, ModelRef>>,
    tool_executor: Option<Arc<dyn ToolExecutor>>,
}

impl ChatOnlyRuntime {
    pub fn new(
        config: RuntimeConfig,
        auth_resolver: Arc<dyn RuntimeAuthResolver>,
        model_resolver: Arc<dyn RuntimeModelResolver>,
        provider_registry: Arc<dyn RuntimeProviderRegistry>,
        session_store: Arc<dyn SessionStore>,
        tool_executor: Option<Arc<dyn ToolExecutor>>,
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
            tool_executor,
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

    fn persist_and_broadcast(
        &self,
        session_id: &SessionId,
        event: RuntimeEvent,
        subscribers: &[Arc<dyn RuntimeSubscriber>],
    ) -> Result<(), RuntimeError> {
        self.session_store
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
        &self,
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

            self.persist_and_broadcast(
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

        // Single replay to build initial history before the loop
        let session_events = self
            .session_store
            .replay(&cmd.session_id)
            .map_err(|e| RuntimeError::SessionReplay {
                message: e.to_string(),
            })?;
        let mut history = lorum_session::reconstruct_conversation(&session_events);

        // Append the new user message to local history
        history.push(ProviderInputMessage::User {
            content: cmd.prompt.clone(),
        });

        self.persist_and_broadcast(
            &cmd.session_id,
            RuntimeEvent::UserMessageReceived {
                turn_id: cmd.turn_id.clone(),
                session_id: cmd.session_id.clone(),
                sequence_no: 0,
                content: cmd.prompt.clone(),
            },
            &subscribers,
        )?;

        let mut current_turn_id = cmd.turn_id.clone();
        let mut tool_turns = 0u32;
        let mut starting_sequence_no = 1u64;

        loop {
            let tool_definitions = match &self.tool_executor {
                Some(executor) => executor.definitions(),
                None => vec![],
            };

            let provider_request = ProviderRequest {
                session_id: cmd.session_id.as_str().to_string(),
                model: model.clone(),
                system_prompt: cmd.system_prompt.clone(),
                input: history.clone(),
                tools: tool_definitions,
            };

            let request = TurnRequest {
                session_id: cmd.session_id.clone(),
                turn_id: current_turn_id.clone(),
                provider_request,
                provider_context: provider_context.clone(),
                cancellation_token: None,
                starting_sequence_no,
            };

            let mut sink = PersistAndBroadcastSink {
                session_id: cmd.session_id.clone(),
                session_store: Arc::clone(&self.session_store),
                subscribers: subscribers.clone(),
            };

            let engine = ChatTurnEngine::new(ProviderAdapterHandle {
                inner: Arc::clone(&provider),
            });
            let result = engine.run_turn(request, &mut sink).await?;

            if result.terminal_reason != TurnTerminalReason::ToolUse {
                break;
            }

            let tool_executor = match &self.tool_executor {
                Some(executor) if self.config.max_tool_turns > 0 => executor,
                _ => {
                    if let Some(ref msg) = result.assistant_message {
                        self.inject_synthetic_tool_results_for_orphaned_calls(
                            &cmd.session_id,
                            &current_turn_id,
                            msg,
                            "Tool execution is not available",
                            &mut history,
                            &subscribers,
                            &mut starting_sequence_no,
                        )?;
                    }
                    break;
                }
            };

            tool_turns += 1;
            if tool_turns > self.config.max_tool_turns {
                if let Some(ref msg) = result.assistant_message {
                    self.inject_synthetic_tool_results_for_orphaned_calls(
                        &cmd.session_id,
                        &current_turn_id,
                        msg,
                        "Maximum tool turns exceeded",
                        &mut history,
                        &subscribers,
                        &mut starting_sequence_no,
                    )?;
                }
                break;
            }

            // Extract tool calls directly from TurnResult — no session replay needed
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

            // Append assistant message to local history
            history.push(ProviderInputMessage::Assistant {
                message: assistant_message,
            });

            current_turn_id = TurnId::from(format!(
                "{}-cont-{}",
                cmd.turn_id.as_str(),
                tool_turns
            ));

            let mut seq = 1u64;
            for tool_call in &tool_calls {
                self.persist_and_broadcast(
                    &cmd.session_id,
                    RuntimeEvent::ToolExecutionStart {
                        turn_id: current_turn_id.clone(),
                        sequence_no: seq,
                        tool_call_id: tool_call.id.clone(),
                        tool_name: tool_call.name.clone(),
                        arguments: tool_call.arguments.clone(),
                    },
                    &subscribers,
                )?;
                seq += 1;

                let exec_result = tool_executor.execute(tool_call).await;

                self.persist_and_broadcast(
                    &cmd.session_id,
                    RuntimeEvent::ToolExecutionEnd {
                        turn_id: current_turn_id.clone(),
                        sequence_no: seq,
                        tool_call_id: tool_call.id.clone(),
                        tool_name: tool_call.name.clone(),
                        is_error: exec_result.is_error,
                    },
                    &subscribers,
                )?;
                seq += 1;

                // Append tool result to local history
                history.push(ProviderInputMessage::ToolResult {
                    tool_call_id: exec_result.tool_call_id.clone(),
                    is_error: exec_result.is_error,
                    result: exec_result.result.clone(),
                });

                self.persist_and_broadcast(
                    &cmd.session_id,
                    RuntimeEvent::ToolResultReceived {
                        turn_id: current_turn_id.clone(),
                        sequence_no: seq,
                        tool_call_id: exec_result.tool_call_id,
                        is_error: exec_result.is_error,
                        result: exec_result.result,
                    },
                    &subscribers,
                )?;
                seq += 1;
            }

            starting_sequence_no = seq;
        }

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
