use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lorum_ai_contract::{
    AssistantEventSink, AssistantMessage, ProviderAdapter, ProviderError, ProviderFinal,
};

use crate::interfaces::{
    CodexSseTransport, CodexTransportMeta, CodexTransportMode, CodexWebSocketTransport,
    FrameSink, OpenAiResponsesFrame, ProviderSessionState, ProviderSessionStateStore, RetryPolicy,
};
use crate::openai_responses::OpenAiFrameProcessor;
use crate::shared::CollectingSink;

#[derive(Default)]
pub struct InMemoryProviderSessionStateStore {
    states: std::sync::Mutex<HashMap<(String, String), ProviderSessionState>>,
}

#[async_trait]
impl ProviderSessionStateStore for InMemoryProviderSessionStateStore {
    async fn get(
        &self,
        session_id: &str,
        provider_id: &str,
    ) -> Result<Option<ProviderSessionState>, ProviderError> {
        let guard = self.states.lock().map_err(|_| ProviderError::Transport {
            message: "session state lock poisoned".to_string(),
        })?;
        Ok(guard
            .get(&(session_id.to_string(), provider_id.to_string()))
            .cloned())
    }

    async fn set(
        &self,
        session_id: &str,
        provider_id: &str,
        state: ProviderSessionState,
    ) -> Result<(), ProviderError> {
        let mut guard = self.states.lock().map_err(|_| ProviderError::Transport {
            message: "session state lock poisoned".to_string(),
        })?;
        guard.insert((session_id.to_string(), provider_id.to_string()), state);
        Ok(())
    }

    async fn clear(&self, session_id: &str, provider_id: &str) -> Result<(), ProviderError> {
        let mut guard = self.states.lock().map_err(|_| ProviderError::Transport {
            message: "session state lock poisoned".to_string(),
        })?;
        guard.remove(&(session_id.to_string(), provider_id.to_string()));
        Ok(())
    }
}

pub struct OpenAiCodexResponsesAdapter {
    websocket: Option<Arc<dyn CodexWebSocketTransport>>,
    sse: Arc<dyn CodexSseTransport>,
    state_store: Arc<dyn ProviderSessionStateStore>,
    retry_policy: RetryPolicy,
    provider_id: String,
    disable_websocket_after_failure: bool,
}

impl OpenAiCodexResponsesAdapter {
    pub fn new(
        websocket: Option<Arc<dyn CodexWebSocketTransport>>,
        sse: Arc<dyn CodexSseTransport>,
        state_store: Arc<dyn ProviderSessionStateStore>,
    ) -> Self {
        Self {
            websocket,
            sse,
            state_store,
            retry_policy: RetryPolicy::default(),
            provider_id: "openai".to_string(),
            disable_websocket_after_failure: true,
        }
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }

    pub fn set_disable_websocket_after_failure(&mut self, disable: bool) {
        self.disable_websocket_after_failure = disable;
    }

    async fn stream_with_selected_transport(
        &self,
        request: &lorum_ai_contract::ProviderRequest,
        context: &lorum_ai_contract::ProviderContext,
        state: Option<ProviderSessionState>,
        use_websocket: bool,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<(CodexTransportMeta, CodexTransportMode), ProviderError> {
        if use_websocket {
            if let Some(websocket) = &self.websocket {
                let result = websocket
                    .stream_frames(request, context, state.clone(), sink)
                    .await
                    .map(|meta| (meta, CodexTransportMode::WebSocket));
                if result.is_ok() {
                    return result;
                }

                if !self.disable_websocket_after_failure {
                    return result;
                }
            }
        }

        self.sse
            .stream_frames(request, context, state, sink)
            .await
            .map(|meta| (meta, CodexTransportMode::Sse))
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiCodexResponsesAdapter {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn api_kind(&self) -> lorum_ai_contract::ApiKind {
        lorum_ai_contract::ApiKind::OpenAiCodexResponses
    }

    async fn stream(
        &self,
        request: lorum_ai_contract::ProviderRequest,
        context: lorum_ai_contract::ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError> {
        let provider_key = self.api_kind().as_str();
        let existing_state = self
            .state_store
            .get(&request.session_id, provider_key)
            .await?;

        let prefer_websocket = existing_state
            .as_ref()
            .map(|state| !state.websocket_disabled)
            .unwrap_or(true);

        let mut attempt = 1usize;
        let mut last_error = None;

        while attempt <= self.retry_policy.max_attempts {
            let mut processor = OpenAiFrameProcessor::new(sink, &request.model);

            let transport_result = self
                .stream_with_selected_transport(
                    &request,
                    &context,
                    existing_state.clone(),
                    prefer_websocket,
                    &mut processor,
                )
                .await;

            match transport_result {
                Ok((meta, mode)) => {
                    let transport_name = match mode {
                        CodexTransportMode::WebSocket => "websocket",
                        CodexTransportMode::Sse => "sse",
                    };
                    let final_msg =
                        processor.finalize(transport_name, meta.reused_provider_session)?;

                    let next_state = ProviderSessionState {
                        provider_session_id: meta.provider_session_id,
                        websocket_disabled: matches!(mode, CodexTransportMode::Sse)
                            && prefer_websocket,
                    };
                    self.state_store
                        .set(&request.session_id, provider_key, next_state)
                        .await?;

                    return Ok(final_msg);
                }
                Err(err) => {
                    if processor.has_received_frames()
                        || !self.retry_policy.should_retry(attempt, &err)
                    {
                        if self.disable_websocket_after_failure && prefer_websocket {
                            let _ = self
                                .state_store
                                .set(
                                    &request.session_id,
                                    provider_key,
                                    ProviderSessionState {
                                        provider_session_id: existing_state
                                            .as_ref()
                                            .and_then(|state| state.provider_session_id.clone()),
                                        websocket_disabled: true,
                                    },
                                )
                                .await;
                        }
                        return Err(err);
                    }
                    last_error = Some(err);
                    attempt += 1;
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ProviderError::Transport {
            message: "codex transport failed without explicit error".to_string(),
        }))
    }

    async fn complete(
        &self,
        request: lorum_ai_contract::ProviderRequest,
        context: lorum_ai_contract::ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        let mut sink = CollectingSink::default();
        let final_msg = self.stream(request, context, &mut sink).await?;
        Ok(final_msg.message)
    }
}
