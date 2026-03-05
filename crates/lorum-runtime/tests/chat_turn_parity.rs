use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent,
    ModelRef, ProviderAdapter, ProviderContext, ProviderError, ProviderFinal, ProviderRequest,
    StopReason, StreamDoneEvent, StreamTextDelta, TextContent, TokenUsage,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId, TurnTerminalReason};
use lorum_runtime::{
    ChatOnlyRuntime, ModelSelectRequest, RuntimeAuthResolver, RuntimeConfig, RuntimeController,
    RuntimeModelResolver, RuntimeProviderRegistry, RuntimeSubscriber, UserInputCommand,
};
use lorum_session::{InMemorySessionStore, SessionStore};

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
    default_model: Option<ModelRef>,
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
            return Ok(model.clone());
        }

        Ok(self.default_model.clone().unwrap_or(ModelRef {
            provider: "mock".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "base-model".to_string(),
        }))
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

#[derive(Clone)]
enum ProviderOutcome {
    Stop(StopReason),
    Error(ProviderError),
}

struct MockProvider {
    outcomes: Mutex<Vec<ProviderOutcome>>,
    seen_models: Mutex<Vec<ModelRef>>,
    next_id: AtomicU64,
}

impl MockProvider {
    fn new(outcomes: Vec<ProviderOutcome>) -> Self {
        Self {
            outcomes: Mutex::new(outcomes),
            seen_models: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
        }
    }

    fn seen_models(&self) -> Vec<ModelRef> {
        self.seen_models.lock().expect("lock seen models").clone()
    }
}

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
        self.seen_models
            .lock()
            .expect("lock seen models")
            .push(request.model.clone());

        let outcome = {
            let mut guard = self.outcomes.lock().expect("lock provider outcomes");
            if guard.is_empty() {
                ProviderOutcome::Stop(StopReason::Stop)
            } else {
                guard.remove(0)
            }
        };

        match outcome {
            ProviderOutcome::Stop(stop_reason) => {
                sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
                    sequence_no: 1,
                    block_id: "b1".to_string(),
                    delta: format!("echo:{}", request.session_id),
                }))
                .map_err(|e| ProviderError::Transport {
                    message: e.to_string(),
                })?;

                let msg_id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let message = AssistantMessage {
                    message_id: format!("msg-{msg_id}"),
                    model: request.model,
                    content: vec![AssistantContent::Text(TextContent {
                        text: "ok".to_string(),
                    })],
                    usage: TokenUsage::default(),
                    stop_reason,
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
            ProviderOutcome::Error(err) => Err(err),
        }
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        Err(ProviderError::InvalidResponse {
            message: "not used in parity tests".to_string(),
        })
    }
}

fn runtime_with_provider(
    model_resolver: Arc<dyn RuntimeModelResolver>,
    provider: Arc<dyn ProviderAdapter>,
    session_store: Arc<dyn SessionStore>,
) -> ChatOnlyRuntime {
    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("mock".to_string(), provider);

    ChatOnlyRuntime::new(
        RuntimeConfig {
            max_tool_turns: 0,
            timeout_ms: 30_000,
        },
        Arc::new(FixedAuthResolver),
        model_resolver,
        Arc::new(StaticProviderRegistry { providers }),
        session_store,
        None,
    )
}

fn submit_cmd(session_id: &str, turn_id: &str, prompt: &str) -> UserInputCommand {
    UserInputCommand {
        session_id: SessionId::from(session_id),
        turn_id: TurnId::from(turn_id),
        prompt: prompt.to_string(),
        system_prompt: None,
    }
}

fn replay_for_turn(events: &[RuntimeEvent], turn_id: &TurnId) -> Vec<RuntimeEvent> {
    events
        .iter()
        .filter(|ev| ev.turn_id() == Some(turn_id))
        .cloned()
        .collect()
}

#[test]
fn multi_turn_replay_preserves_per_turn_sequence_and_terminal_events() {
    block_on(async {
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let model_resolver = Arc::new(RecordingModelResolver::default());
        let provider = Arc::new(MockProvider::new(vec![
            ProviderOutcome::Stop(StopReason::Stop),
            ProviderOutcome::Stop(StopReason::Stop),
        ]));
        let runtime = runtime_with_provider(model_resolver, provider, Arc::clone(&session_store));

        let subscriber = Arc::new(RecordingSubscriber::default());
        runtime
            .subscribe(subscriber.clone())
            .await
            .expect("subscriber should register");

        runtime
            .submit_user_input(submit_cmd("session-parity", "turn-a", "hello"))
            .await
            .expect("first submit should succeed");
        runtime
            .submit_user_input(submit_cmd("session-parity", "turn-b", "world"))
            .await
            .expect("second submit should succeed");

        let replayed = session_store
            .replay(&SessionId::from("session-parity"))
            .expect("session replay should succeed");

        for turn in [TurnId::from("turn-a"), TurnId::from("turn-b")] {
            let turn_events = replay_for_turn(&replayed, &turn);
            assert_eq!(
                turn_events.len(),
                4,
                "each turn should emit exactly 4 events (user_msg + start + delta + finish)"
            );
            assert!(matches!(
                turn_events[0],
                RuntimeEvent::UserMessageReceived { .. }
            ));
            assert!(matches!(turn_events[1], RuntimeEvent::TurnStarted { .. }));
            assert!(matches!(
                turn_events[2],
                RuntimeEvent::AssistantStreamDelta { .. }
            ));
            assert!(matches!(
                turn_events[3],
                RuntimeEvent::TurnFinished {
                    reason: TurnTerminalReason::Done,
                    ..
                }
            ));
        }

        let observed = subscriber.events.lock().expect("lock subscriber events");
        assert_eq!(observed.as_slice(), replayed.as_slice());
    });
}

#[test]
fn abort_and_provider_error_have_deterministic_terminal_semantics() {
    block_on(async {
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let model_resolver = Arc::new(RecordingModelResolver::default());
        let provider = Arc::new(MockProvider::new(vec![
            ProviderOutcome::Stop(StopReason::Aborted),
            ProviderOutcome::Error(ProviderError::Transport {
                message: "forced failure".to_string(),
            }),
        ]));
        let runtime = runtime_with_provider(model_resolver, provider, Arc::clone(&session_store));

        runtime
            .submit_user_input(submit_cmd("session-terminals", "turn-abort", "abort me"))
            .await
            .expect("aborted stop reason still resolves to TurnFinished");

        let err = runtime
            .submit_user_input(submit_cmd("session-terminals", "turn-error", "fail me"))
            .await
            .expect_err("provider error must surface as runtime error");
        assert!(matches!(err, lorum_runtime::RuntimeError::TurnEngine(_)));

        let replayed = session_store
            .replay(&SessionId::from("session-terminals"))
            .expect("session replay should succeed");

        let aborted_events = replay_for_turn(&replayed, &TurnId::from("turn-abort"));
        assert!(matches!(
            aborted_events.last(),
            Some(RuntimeEvent::TurnFinished {
                reason: TurnTerminalReason::Aborted,
                ..
            })
        ));

        let error_events = replay_for_turn(&replayed, &TurnId::from("turn-error"));
        assert!(matches!(
            error_events[0],
            RuntimeEvent::UserMessageReceived { .. }
        ));
        assert!(matches!(
            error_events[1],
            RuntimeEvent::TurnStarted { .. }
        ));
        assert!(matches!(
            error_events.last(),
            Some(RuntimeEvent::RuntimeError { .. })
        ));
    });
}

#[test]
fn model_switch_updates_resolver_override_and_provider_request_model() {
    block_on(async {
        let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());
        let model_resolver = Arc::new(RecordingModelResolver {
            seen_overrides: Mutex::new(Vec::new()),
            default_model: Some(ModelRef {
                provider: "mock".to_string(),
                api: ApiKind::OpenAiResponses,
                model: "base-model".to_string(),
            }),
        });
        let provider = Arc::new(MockProvider::new(vec![
            ProviderOutcome::Stop(StopReason::Stop),
            ProviderOutcome::Stop(StopReason::Stop),
        ]));

        let runtime = runtime_with_provider(
            model_resolver.clone(),
            provider.clone(),
            Arc::clone(&session_store),
        );

        runtime
            .submit_user_input(submit_cmd("session-model", "turn-1", "first"))
            .await
            .expect("first submit should succeed");

        let override_model = ModelRef {
            provider: "mock".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "override-model".to_string(),
        };
        runtime
            .set_model(ModelSelectRequest {
                session_id: SessionId::from("session-model"),
                model: override_model.clone(),
            })
            .await
            .expect("set_model should succeed");

        runtime
            .submit_user_input(submit_cmd("session-model", "turn-2", "second"))
            .await
            .expect("second submit should succeed");

        let seen_overrides = model_resolver
            .seen_overrides
            .lock()
            .expect("lock seen overrides")
            .clone();
        assert_eq!(seen_overrides.len(), 2);
        assert_eq!(seen_overrides[0], None);
        assert_eq!(seen_overrides[1], Some(override_model.clone()));

        let seen_models = provider.seen_models();
        assert_eq!(seen_models.len(), 2);
        assert_eq!(seen_models[0].model, "base-model");
        assert_eq!(seen_models[1], override_model);
    });
}
