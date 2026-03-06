use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent,
    ModelRef, ProviderAdapter, ProviderContext, ProviderError, ProviderFinal, ProviderInputMessage,
    ProviderRequest, StopReason, StreamDoneEvent, StreamTextDelta, TextContent, TokenUsage,
};
use lorum_domain::{RuntimeEvent, SessionId, TurnId};
use lorum_runtime::{
    ChatOnlyRuntime, ModelSelectRequest, RuntimeAuthResolver, RuntimeConfig, RuntimeController,
    RuntimeModelResolver, RuntimeProviderRegistry, RuntimeSubscriber, UserInputCommand,
};
use lorum_session::{InMemorySessionStore, SessionStore};

const SESSION_ID: &str = "demo-session";
const PROVIDER_ID: &str = "mock";

struct DemoAuthResolver {
    api_key: Option<String>,
}

#[async_trait]
impl RuntimeAuthResolver for DemoAuthResolver {
    async fn get_api_key(
        &self,
        _provider: &str,
        _session_id: &SessionId,
    ) -> Result<Option<String>, String> {
        Ok(self.api_key.clone())
    }
}

struct DemoModelResolver {
    default_model: ModelRef,
}

#[async_trait]
impl RuntimeModelResolver for DemoModelResolver {
    async fn resolve_model(
        &self,
        _session_id: &SessionId,
        override_model: Option<&ModelRef>,
    ) -> Result<ModelRef, String> {
        Ok(override_model
            .cloned()
            .unwrap_or_else(|| self.default_model.clone()))
    }
}

struct DemoProviderRegistry {
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
}

impl RuntimeProviderRegistry for DemoProviderRegistry {
    fn get_provider(&self, provider_id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.providers.get(provider_id).cloned()
    }
}

struct EchoProvider;

#[async_trait]
impl ProviderAdapter for EchoProvider {
    fn provider_id(&self) -> &str {
        PROVIDER_ID
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
        let prompt = request
            .input
            .iter()
            .rev()
            .find_map(|msg| match msg {
                ProviderInputMessage::User { content } => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_default();

        if let Some(message) = prompt.strip_prefix("/error ") {
            return Err(ProviderError::Transport {
                message: format!("simulated transport error: {message}"),
            });
        }

        let (reply_text, stop_reason) = if let Some(message) = prompt.strip_prefix("/abort ") {
            (format!("echo: {message}"), StopReason::Aborted)
        } else {
            (format!("echo: {prompt}"), StopReason::Stop)
        };

        sink.push(AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 1,
            block_id: "text-1".to_string(),
            delta: reply_text.clone(),
        }))
        .map_err(|err| ProviderError::Transport {
            message: format!("sink rejected delta: {err}"),
        })?;

        let message = AssistantMessage {
            message_id: "msg-1".to_string(),
            model: request.model,
            content: vec![AssistantContent::Text(TextContent { text: reply_text })],
            usage: TokenUsage::default(),
            stop_reason,
        };

        sink.push(AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: 2,
            message: message.clone(),
        }))
        .map_err(|err| ProviderError::Transport {
            message: format!("sink rejected done event: {err}"),
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
            message: "complete() is not used by this temp REPL".to_string(),
        })
    }
}

struct StdoutSubscriber;

impl RuntimeSubscriber for StdoutSubscriber {
    fn on_event(&self, event: &RuntimeEvent) {
        match event {
            RuntimeEvent::AssistantStreamDelta { turn_id, delta, .. } => {
                println!("[delta][{}] {}", turn_id.as_str(), delta);
            }
            RuntimeEvent::TurnFinished {
                turn_id,
                reason,
                message_id,
                ..
            } => {
                println!(
                    "[done][{}] reason={reason:?} message_id={}",
                    turn_id.as_str(),
                    message_id
                        .as_ref()
                        .map(|id| id.as_str())
                        .unwrap_or("<none>")
                );
            }
            RuntimeEvent::RuntimeError {
                turn_id,
                code,
                message,
                ..
            } => {
                println!("[runtime-error][{}] {code}: {message}", turn_id.as_str());
            }
            _ => {}
        }
    }
}

fn main() {
    let session_id = SessionId::from(SESSION_ID);
    let default_model = ModelRef {
        provider: PROVIDER_ID.to_string(),
        api: ApiKind::OpenAiResponses,
        model: "demo-model".to_string(),
    };

    let auth_resolver: Arc<dyn RuntimeAuthResolver> = Arc::new(DemoAuthResolver {
        api_key: Some("demo-api-key".to_string()),
    });
    let model_resolver: Arc<dyn RuntimeModelResolver> = Arc::new(DemoModelResolver {
        default_model: default_model.clone(),
    });

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert(PROVIDER_ID.to_string(), Arc::new(EchoProvider));
    let provider_registry: Arc<dyn RuntimeProviderRegistry> =
        Arc::new(DemoProviderRegistry { providers });

    let session_store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());

    let runtime = ChatOnlyRuntime::new(
        RuntimeConfig {
            max_tool_turns: 0,
            timeout_ms: 30_000,
            max_output_bytes: 500_000,
            max_output_lines: 5_000,
        },
        auth_resolver,
        model_resolver,
        provider_registry,
        Arc::clone(&session_store),
        None,
    );

    if let Err(err) = block_on(runtime.subscribe(Arc::new(StdoutSubscriber))) {
        eprintln!("failed to subscribe runtime events: {err}");
        return;
    }

    println!("Temporary chat REPL (Phase2A chat-only path, demo only).");
    println!("Commands: /quit, /model <name>, /history");
    println!("Simulation: /abort <msg> -> aborted terminal, /error <msg> -> transport error");

    let mut turn_no = 1_u64;
    loop {
        print!("temp-chat> ");
        if let Err(err) = io::stdout().flush() {
            eprintln!("failed to flush prompt: {err}");
            break;
        }

        let mut line = String::new();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(err) => {
                eprintln!("failed to read input: {err}");
                continue;
            }
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        if input == "/quit" {
            break;
        }

        if input == "/history" {
            match session_store.replay(&session_id) {
                Ok(events) => {
                    if events.is_empty() {
                        println!("(no events yet)");
                    } else {
                        for event in events {
                            println!("[history] {event:?}");
                        }
                    }
                }
                Err(err) => eprintln!("history replay failed: {err}"),
            }
            continue;
        }

        if let Some(model_name) = input.strip_prefix("/model ") {
            let model_name = model_name.trim();
            if model_name.is_empty() {
                eprintln!("usage: /model <name>");
                continue;
            }

            let req = ModelSelectRequest {
                session_id: session_id.clone(),
                model: ModelRef {
                    provider: PROVIDER_ID.to_string(),
                    api: ApiKind::OpenAiResponses,
                    model: model_name.to_string(),
                },
            };

            match block_on(runtime.set_model(req)) {
                Ok(()) => println!("model set to '{model_name}'"),
                Err(err) => eprintln!("set_model failed: {err}"),
            }
            continue;
        }

        let cmd = UserInputCommand {
            session_id: session_id.clone(),
            turn_id: TurnId::from(format!("turn-{turn_no}")),
            prompt: input.to_string(),
            system_prompt: None,
        };

        turn_no += 1;

        if let Err(err) = block_on(runtime.submit_user_input(cmd)) {
            eprintln!("submit failed: {err}");
        }
    }

    println!("bye");
}
