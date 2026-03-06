use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent,
    ModelRef, ProviderAdapter, ProviderContext, ProviderError, ProviderFinal, ProviderRequest,
    StopReason, StreamDoneEvent, StreamTextDelta, TextContent, ToolCall, ToolDefinition,
    TokenUsage,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId};
use lorum_runtime::{
    ChatOnlyRuntime, ModelSelectRequest, RuntimeAuthResolver, RuntimeConfig, RuntimeController,
    RuntimeError, RuntimeModelResolver, RuntimeProviderRegistry, RuntimeSubscriber,
    ToolDispatcher, ToolExecutionResult, ToolExecutor, UserInputCommand,
};
use lorum_session::{InMemorySessionStore, SessionStore};
use serde_json::Value;

struct FixedAuthResolver;

#[async_trait]
impl RuntimeAuthResolver for FixedAuthResolver {
    async fn get_api_key(
        &self,
        _provider: &str,
        _session_id: &SessionId,
    ) -> Result<Option<String>, String> {
        Ok(Some("test-key".to_string()))
    }
}

#[derive(Default)]
struct RecordingModelResolver {
    seen_overrides: Mutex<Vec<Option<ModelRef>>>,
}

#[async_trait]
impl RuntimeModelResolver for RecordingModelResolver {
    async fn resolve_model(
        &self,
        _session_id: &SessionId,
        override_model: Option<&ModelRef>,
    ) -> Result<ModelRef, String> {
        self.seen_overrides
            .lock()
            .expect("lock model resolver")
            .push(override_model.cloned());

        if let Some(model) = override_model {
            Ok(model.clone())
        } else {
            Ok(ModelRef {
                provider: "mock".to_string(),
                api: ApiKind::OpenAiResponses,
                model: "base-model".to_string(),
            })
        }
    }
}

struct StaticProviderRegistry {
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl RuntimeProviderRegistry for StaticProviderRegistry {
    fn get_provider(&self, provider_id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.providers.get(provider_id).cloned()
    }
}

struct MockProvider;

#[async_trait]
impl ProviderAdapter for MockProvider {
    fn provider_id(&self) -> &str {
        "mock"
    }

    fn api_kind(&self) -> ApiKind {
        ApiKind::OpenAiResponses
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        _context: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError> {
        sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 1,
            block_id: "b1".to_string(),
            delta: format!("echo:{}", request.session_id),
        }))
        .map_err(|e| ProviderError::Transport {
            message: e.to_string(),
        })?;

        let message = AssistantMessage {
            message_id: "msg-1".to_string(),
            model: request.model,
            content: vec![AssistantContent::Text(TextContent {
                text: "ok".to_string(),
            })],
            usage: TokenUsage::default(),
            stop_reason: StopReason::Stop,
        };

        sink.push(AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: 2,
            message: message.clone(),
        }))
        .map_err(|e| ProviderError::Transport {
            message: e.to_string(),
        })?;

        Ok(ProviderFinal {
            message,
            transport_details: None,
        })
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        Err(ProviderError::InvalidResponse {
            message: "not used in runtime tests".to_string(),
        })
    }
}

#[derive(Default)]
struct RecordingSubscriber {
    events: Mutex<Vec<RuntimeEvent>>,
}

impl RuntimeSubscriber for RecordingSubscriber {
    fn on_event(&self, event: &RuntimeEvent) {
        self.events
            .lock()
            .expect("lock subscriber")
            .push(event.clone());
    }
}

struct MockToolUseProvider {
    call_count: Mutex<u32>,
}

impl MockToolUseProvider {
    fn new() -> Self {
        Self {
            call_count: Mutex::new(0),
        }
    }
}

#[async_trait]
impl ProviderAdapter for MockToolUseProvider {
    fn provider_id(&self) -> &str {
        "mock"
    }

    fn api_kind(&self) -> ApiKind {
        ApiKind::OpenAiResponses
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        _context: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError> {
        let mut count = self.call_count.lock().expect("lock call_count");
        *count += 1;

        let message = AssistantMessage {
            message_id: format!("msg-{}", *count),
            model: request.model,
            content: vec![AssistantContent::ToolCall(ToolCall {
                id: format!("tc-{}", *count),
                name: "test_tool".to_string(),
                arguments: serde_json::json!({}),
            })],
            usage: TokenUsage::default(),
            stop_reason: StopReason::ToolUse,
        };

        sink.push(AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: 1,
            message: message.clone(),
        }))
        .map_err(|e| ProviderError::Transport {
            message: e.to_string(),
        })?;

        Ok(ProviderFinal {
            message,
            transport_details: None,
        })
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        Err(ProviderError::InvalidResponse {
            message: "not used".to_string(),
        })
    }
}

struct SimpleToolExecutor;

#[async_trait]
impl ToolExecutor for SimpleToolExecutor {
    fn definitions(&self) -> Vec<ToolDefinition> {
        vec![ToolDefinition {
            name: "test_tool".to_string(),
            description: "A test tool".to_string(),
            parameters: serde_json::json!({}),
        }]
    }

    async fn execute(&self, tool_call: &ToolCall) -> ToolExecutionResult {
        ToolExecutionResult {
            tool_call_id: tool_call.id.clone(),
            is_error: false,
            result: serde_json::json!("tool result"),
        }
    }
}

fn sample_command() -> UserInputCommand {
    UserInputCommand {
        session_id: SessionId::from("session-1"),
        turn_id: TurnId::from("turn-1"),
        prompt: "hello".to_string(),
        system_prompt: None,
    }
}

fn runtime_with_registry(
    config: RuntimeConfig,
    registry: Arc<dyn RuntimeProviderRegistry>,
    model_resolver: Arc<dyn RuntimeModelResolver>,
    session_store: Arc<dyn SessionStore>,
) -> ChatOnlyRuntime {
    ChatOnlyRuntime::new(
        config,
        Arc::new(FixedAuthResolver),
        model_resolver,
        registry,
        session_store,
        None,
    )
}

fn runtime_with_tool_executor(
    config: RuntimeConfig,
    provider: Arc<dyn ProviderAdapter>,
    session_store: Arc<dyn SessionStore>,
    tool_executor: Option<Arc<dyn ToolExecutor>>,
) -> ChatOnlyRuntime {
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("mock".to_string(), provider);

    let dispatcher = tool_executor.map(|e| Arc::new(ToolDispatcher::new(e)));

    ChatOnlyRuntime::new(
        config,
        Arc::new(FixedAuthResolver),
        Arc::new(RecordingModelResolver::default()),
        Arc::new(StaticProviderRegistry { providers }),
        session_store,
        dispatcher,
    )
}

#[test]
fn successful_submit_persists_and_broadcasts_events() {
    block_on(async {
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("mock".to_string(), Arc::new(MockProvider));

        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let model_resolver = Arc::new(RecordingModelResolver::default());
        let runtime = runtime_with_registry(
            RuntimeConfig {
                max_tool_turns: 0,
                timeout_ms: 30_000,
                max_output_bytes: 500_000,
                max_output_lines: 5_000,
            },
            Arc::new(StaticProviderRegistry { providers }),
            model_resolver,
            Arc::clone(&session_store),
        );

        let subscriber = Arc::new(RecordingSubscriber::default());
        runtime
            .subscribe(subscriber.clone())
            .await
            .expect("subscriber should register");

        runtime
            .submit_user_input(sample_command())
            .await
            .expect("submit should succeed");

        let replayed = session_store
            .replay(&SessionId::from("session-1"))
            .expect("session replay should succeed");
        assert_eq!(replayed.len(), 4);
        assert!(matches!(
            replayed[0],
            RuntimeEvent::UserMessageReceived { .. }
        ));
        assert!(matches!(replayed[1], RuntimeEvent::TurnStarted { .. }));
        assert!(matches!(
            replayed[2],
            RuntimeEvent::AssistantStreamDelta { .. }
        ));
        assert!(matches!(replayed[3], RuntimeEvent::TurnFinished { .. }));

        let observed = subscriber.events.lock().expect("lock subscriber events");
        assert_eq!(observed.len(), 4);
        assert_eq!(*observed, replayed);
    });
}

#[test]
fn provider_missing_returns_explicit_error() {
    block_on(async {
        let runtime = runtime_with_registry(
            RuntimeConfig {
                max_tool_turns: 0,
                timeout_ms: 30_000,
                max_output_bytes: 500_000,
                max_output_lines: 5_000,
            },
            Arc::new(StaticProviderRegistry {
                providers: HashMap::new(),
            }),
            Arc::new(RecordingModelResolver::default()),
            Arc::new(InMemorySessionStore::new()),
        );

        let err = runtime
            .submit_user_input(sample_command())
            .await
            .expect_err("missing provider should fail");
        assert!(matches!(err, RuntimeError::ProviderNotFound { .. }));
    });
}

#[test]
fn set_model_override_is_used_by_model_resolver() {
    block_on(async {
        let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
        providers.insert("mock".to_string(), Arc::new(MockProvider));

        let model_resolver = Arc::new(RecordingModelResolver::default());
        let runtime = runtime_with_registry(
            RuntimeConfig {
                max_tool_turns: 0,
                timeout_ms: 30_000,
                max_output_bytes: 500_000,
                max_output_lines: 5_000,
            },
            Arc::new(StaticProviderRegistry { providers }),
            model_resolver.clone(),
            Arc::new(InMemorySessionStore::new()),
        );

        let override_model = ModelRef {
            provider: "mock".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "override-model".to_string(),
        };

        runtime
            .set_model(ModelSelectRequest {
                session_id: SessionId::from("session-1"),
                model: override_model.clone(),
            })
            .await
            .expect("set_model should succeed");

        runtime
            .submit_user_input(sample_command())
            .await
            .expect("submit should succeed");

        let seen = model_resolver
            .seen_overrides
            .lock()
            .expect("lock seen overrides");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], Some(override_model));
    });
}

#[test]
fn no_executor_injects_synthetic_results() {
    block_on(async {
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let runtime = runtime_with_tool_executor(
            RuntimeConfig {
                max_tool_turns: 5,
                timeout_ms: 30_000,
                max_output_bytes: 500_000,
                max_output_lines: 5_000,
            },
            Arc::new(MockToolUseProvider::new()),
            Arc::clone(&session_store),
            None,
        );

        runtime
            .submit_user_input(sample_command())
            .await
            .expect("submit should succeed");

        let events = session_store
            .replay(&SessionId::from("session-1"))
            .expect("replay");

        let tool_results: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::ToolResultReceived { .. }))
            .collect();

        assert_eq!(tool_results.len(), 1);
        match &tool_results[0] {
            RuntimeEvent::ToolResultReceived {
                tool_call_id,
                is_error,
                result,
                ..
            } => {
                assert_eq!(tool_call_id, "tc-1");
                assert!(is_error);
                assert_eq!(
                    result,
                    &Value::String("Tool execution is not available".to_string())
                );
            }
            _ => unreachable!(),
        }
    });
}

#[test]
fn max_tool_turns_exceeded_injects_synthetic_results() {
    block_on(async {
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let runtime = runtime_with_tool_executor(
            RuntimeConfig {
                max_tool_turns: 1,
                timeout_ms: 30_000,
                max_output_bytes: 500_000,
                max_output_lines: 5_000,
            },
            Arc::new(MockToolUseProvider::new()),
            Arc::clone(&session_store),
            Some(Arc::new(SimpleToolExecutor)),
        );

        runtime
            .submit_user_input(sample_command())
            .await
            .expect("submit should succeed");

        let events = session_store
            .replay(&SessionId::from("session-1"))
            .expect("replay");

        let tool_results: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RuntimeEvent::ToolResultReceived { .. }))
            .collect();

        // 1 real result (from the first tool turn) + 1 synthetic (from exceeding max)
        assert_eq!(tool_results.len(), 2);
        match &tool_results[1] {
            RuntimeEvent::ToolResultReceived {
                is_error, result, ..
            } => {
                assert!(is_error);
                assert_eq!(
                    result,
                    &Value::String("Maximum tool turns exceeded".to_string())
                );
            }
            _ => unreachable!(),
        }
    });
}

#[test]
fn subsequent_submit_succeeds_after_synthetic_injection() {
    block_on(async {
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let runtime = runtime_with_tool_executor(
            RuntimeConfig {
                max_tool_turns: 0,
                timeout_ms: 30_000,
                max_output_bytes: 500_000,
                max_output_lines: 5_000,
            },
            Arc::new(MockToolUseProvider::new()),
            Arc::clone(&session_store),
            None,
        );

        // First submit -- will inject synthetic results for orphaned tool calls
        runtime
            .submit_user_input(sample_command())
            .await
            .expect("first submit should succeed");

        // Second submit -- should not fail due to orphaned tool calls
        let cmd2 = UserInputCommand {
            session_id: SessionId::from("session-1"),
            turn_id: TurnId::from("turn-2"),
            prompt: "follow up".to_string(),
            system_prompt: None,
        };
        runtime
            .submit_user_input(cmd2)
            .await
            .expect("second submit should succeed after synthetic injection");
    });
}
