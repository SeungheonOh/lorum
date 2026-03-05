use std::collections::HashMap;
use std::env;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use futures_util::StreamExt;
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantEventSink, AssistantMessage, ModelRef, ProviderAdapter,
    ProviderContext, ProviderError, ProviderInputMessage, ProviderRequest, ToolDefinition,
};
use serde_json::Value;

use crate::{
    AnthropicAdapter, AnthropicFrame, AnthropicTransport, CodexSseTransport, CodexTransportMeta,
    InMemoryProviderSessionStateStore, OpenAiCodexResponsesAdapter, OpenAiResponsesAdapter,
    OpenAiResponsesFrame, OpenAiResponsesTransport, ProviderSessionState,
};
use crate::interfaces::FrameSink;

const DEFAULT_OPENAI_MODEL: &str = "gpt-4.1-mini";
const DEFAULT_CODEX_MODEL: &str = "gpt-5-codex";
const DEFAULT_ANTHROPIC_MODEL: &str = "claude-3-5-sonnet-latest";
const DEFAULT_MINIMAX_MODEL: &str = "MiniMax-M2.5";
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const MINIMAX_MESSAGES_URL: &str = "https://api.minimax.io/anthropic/v1/messages";
const OPENAI_CODEX_RESPONSES_URL: &str = "https://chatgpt.com/backend-api/codex/responses";
const OPENAI_CODEX_ACCOUNT_HEADER: &str = "chatgpt-account-id";
const OPENAI_CODEX_BETA_HEADER: &str = "OpenAI-Beta";
const OPENAI_CODEX_BETA_VALUE: &str = "responses=experimental";
const OPENAI_CODEX_ORIGINATOR_HEADER: &str = "originator";
const OPENAI_CODEX_ORIGINATOR_VALUE: &str = "pi";
const OPENAI_CODEX_SESSION_HEADER: &str = "session_id";
const OPENAI_CODEX_CONVERSATION_HEADER: &str = "conversation_id";
const OPENAI_AUTH_CLAIM_PATH: &str = "https://api.openai.com/auth";

#[derive(Clone)]
pub struct ProviderCatalog {
    providers: HashMap<String, Arc<dyn ProviderAdapter>>,
    model_presets: HashMap<String, ModelRef>,
    default_preset: String,
}

impl ProviderCatalog {
    pub fn provider(&self, id: &str) -> Option<Arc<dyn ProviderAdapter>> {
        self.providers.get(id).cloned()
    }

    pub fn default_model(&self) -> Option<ModelRef> {
        self.model_presets.get(&self.default_preset).cloned()
    }

    pub fn preset_model(&self, preset: &str) -> Option<ModelRef> {
        self.model_presets.get(preset).cloned()
    }

    pub fn all_presets(&self) -> &HashMap<String, ModelRef> {
        &self.model_presets
    }

    pub fn model_presets(&self) -> &HashMap<String, ModelRef> {
        &self.model_presets
    }

    pub fn into_providers(self) -> HashMap<String, Arc<dyn ProviderAdapter>> {
        self.providers
    }
}

pub fn build_curl_provider_catalog() -> ProviderCatalog {
    let client = reqwest::Client::new();

    let openai_transport = Arc::new(ReqwestOpenAiTransport {
        client: client.clone(),
    });
    let codex_sse = Arc::new(ReqwestCodexSseTransport {
        client: client.clone(),
    });
    let anthropic_transport = Arc::new(ReqwestAnthropicTransport {
        client: client.clone(),
    });
    let minimax_transport = Arc::new(ReqwestMiniMaxTransport { client });

    let openai_responses_adapter = OpenAiResponsesAdapter::new(openai_transport);
    let codex_adapter = OpenAiCodexResponsesAdapter::new(
        None,
        codex_sse,
        Arc::new(InMemoryProviderSessionStateStore::default()),
    );

    let openai_facade = OpenAiFacade {
        responses: openai_responses_adapter,
        codex: codex_adapter,
    };

    let anthropic_adapter = AnthropicAdapter::new(anthropic_transport);
    let minimax_adapter = AnthropicAdapter::new(minimax_transport)
        .with_provider_id("minimax")
        .with_api_kind(ApiKind::MiniMaxMessages);

    let mut providers: HashMap<String, Arc<dyn ProviderAdapter>> = HashMap::new();
    providers.insert("openai".to_string(), Arc::new(openai_facade));
    providers.insert("anthropic".to_string(), Arc::new(anthropic_adapter));
    providers.insert("minimax".to_string(), Arc::new(minimax_adapter));

    let openai_model = default_openai_model();
    let codex_model = default_codex_model();
    let anthropic_model = default_anthropic_model();
    let minimax_model = default_minimax_model();

    let mut model_presets: HashMap<String, ModelRef> = HashMap::new();
    model_presets.insert("openai".to_string(), openai_model);
    model_presets.insert("codex".to_string(), codex_model.clone());
    model_presets.insert("anthropic".to_string(), anthropic_model);
    model_presets.insert("minimax".to_string(), minimax_model);

    ProviderCatalog {
        providers,
        model_presets,
        default_preset: "codex".to_string(),
    }
}

struct OpenAiFacade {
    responses: OpenAiResponsesAdapter,
    codex: OpenAiCodexResponsesAdapter,
}

#[async_trait]
impl ProviderAdapter for OpenAiFacade {
    fn provider_id(&self) -> &str {
        "openai"
    }

    fn api_kind(&self) -> ApiKind {
        ApiKind::OpenAiResponses
    }

    async fn stream(
        &self,
        request: ProviderRequest,
        context: ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<lorum_ai_contract::ProviderFinal, ProviderError> {
        match request.model.api {
            ApiKind::OpenAiCodexResponses => self.codex.stream(request, context, sink).await,
            ApiKind::OpenAiResponses => self.responses.stream(request, context, sink).await,
            other => Err(ProviderError::InvalidResponse {
                message: format!("openai provider does not support api kind: {other}"),
            }),
        }
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        context: ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        match request.model.api {
            ApiKind::OpenAiCodexResponses => self.codex.complete(request, context).await,
            ApiKind::OpenAiResponses => self.responses.complete(request, context).await,
            other => Err(ProviderError::InvalidResponse {
                message: format!("openai provider does not support api kind: {other}"),
            }),
        }
    }

    fn supports_stateful_transport(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// reqwest-based transports — push frames incrementally via FrameSink
// ---------------------------------------------------------------------------

struct ReqwestOpenAiTransport {
    client: reqwest::Client,
}

#[async_trait]
impl OpenAiResponsesTransport for ReqwestOpenAiTransport {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<(), ProviderError> {
        let api_key = context.api_key.clone().ok_or_else(|| ProviderError::Auth {
            message: "missing api key in provider context".to_string(),
        })?;

        let mut payload = serde_json::json!({
            "model": request.model.model,
            "input": openai_prompt_input(request),
            "stream": true,
        });
        if let Some(ref instructions) = request.system_prompt {
            payload["instructions"] = Value::String(instructions.clone());
        }
        if !request.tools.is_empty() {
            payload["tools"] = openai_tool_definitions(&request.tools);
        }

        let mut emitter = OpenAiSseToFrameEmitter::new(sink);
        stream_sse_events(
            &self.client,
            OPENAI_RESPONSES_URL,
            &payload,
            &[("Authorization".to_string(), format!("Bearer {api_key}"))],
            context.timeout_ms,
            &mut |event| emitter.process_event(&event),
        )
        .await?;
        emitter.finalize()?;
        Ok(())
    }
}

struct ReqwestCodexSseTransport {
    client: reqwest::Client,
}

#[async_trait]
impl CodexSseTransport for ReqwestCodexSseTransport {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        state: Option<ProviderSessionState>,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<CodexTransportMeta, ProviderError> {
        let access_token = context.api_key.clone().ok_or_else(|| ProviderError::Auth {
            message: "missing api key in provider context".to_string(),
        })?;
        let account_id = chatgpt_account_id_from_access_token(&access_token)?;

        let instructions = request.system_prompt.clone().unwrap_or_default();
        let mut payload = serde_json::json!({
            "model": request.model.model,
            "instructions": instructions,
            "input": openai_codex_input(request),
            "store": false,
            "stream": true,
        });
        if !request.tools.is_empty() {
            payload["tools"] = openai_tool_definitions(&request.tools);
        }
        let headers = openai_codex_headers(&access_token, account_id, &request.session_id);

        let mut emitter = OpenAiSseToFrameEmitter::new(sink);
        stream_sse_events(
            &self.client,
            OPENAI_CODEX_RESPONSES_URL,
            &payload,
            &headers,
            context.timeout_ms,
            &mut |event| emitter.process_event(&event),
        )
        .await?;
        let response_id = emitter.finalize()?;

        let provider_session_id = response_id.or_else(|| {
            state
                .as_ref()
                .and_then(|prev| prev.provider_session_id.clone())
        });

        Ok(CodexTransportMeta {
            provider_session_id,
            reused_provider_session: state
                .as_ref()
                .and_then(|prev| prev.provider_session_id.as_ref())
                .is_some(),
        })
    }
}

struct ReqwestAnthropicTransport {
    client: reqwest::Client,
}

#[async_trait]
impl AnthropicTransport for ReqwestAnthropicTransport {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        sink: &mut dyn FrameSink<AnthropicFrame>,
    ) -> Result<(), ProviderError> {
        let api_key = context.api_key.clone().ok_or_else(|| ProviderError::Auth {
            message: "missing api key in provider context".to_string(),
        })?;

        let (system, messages) = anthropic_prompt_parts(request);
        let mut payload = serde_json::json!({
            "model": request.model.model,
            "max_tokens": 4096,
            "stream": true,
            "messages": messages,
        });
        if let Some(system_content) = system {
            payload["system"] = Value::String(system_content);
        }
        if !request.tools.is_empty() {
            payload["tools"] = anthropic_tool_definitions(&request.tools);
        }

        let mut emitter = AnthropicSseToFrameEmitter::new(sink);
        stream_sse_events(
            &self.client,
            "https://api.anthropic.com/v1/messages",
            &payload,
            &[
                ("x-api-key".to_string(), api_key),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ],
            context.timeout_ms,
            &mut |event| emitter.process_event(&event),
        )
        .await?;
        emitter.finalize()?;
        Ok(())
    }
}

struct ReqwestMiniMaxTransport {
    client: reqwest::Client,
}

#[async_trait]
impl AnthropicTransport for ReqwestMiniMaxTransport {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        sink: &mut dyn FrameSink<AnthropicFrame>,
    ) -> Result<(), ProviderError> {
        let api_key = context.api_key.clone().ok_or_else(|| ProviderError::Auth {
            message: "missing api key in provider context".to_string(),
        })?;

        let (system, messages) = anthropic_prompt_parts(request);
        let mut payload = serde_json::json!({
            "model": request.model.model,
            "max_tokens": 16384,
            "stream": true,
            "messages": messages,
        });
        if let Some(system_content) = system {
            payload["system"] = Value::String(system_content);
        }
        if !request.tools.is_empty() {
            payload["tools"] = anthropic_tool_definitions(&request.tools);
        }

        let mut emitter = AnthropicSseToFrameEmitter::new(sink);
        stream_sse_events(
            &self.client,
            MINIMAX_MESSAGES_URL,
            &payload,
            &[
                ("x-api-key".to_string(), api_key),
                ("anthropic-version".to_string(), "2023-06-01".to_string()),
            ],
            context.timeout_ms,
            &mut |event| emitter.process_event(&event),
        )
        .await?;
        emitter.finalize()?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Incremental SSE streaming — calls on_event for each parsed SSE event
// ---------------------------------------------------------------------------

async fn stream_sse_events<F>(
    client: &reqwest::Client,
    url: &str,
    payload: &Value,
    headers: &[(String, String)],
    timeout_ms: u64,
    on_event: &mut F,
) -> Result<(), ProviderError>
where
    F: FnMut(Value) -> Result<(), ProviderError> + Send,
{
    let timeout = Duration::from_millis(timeout_ms.max(1000));

    let mut builder = client
        .post(url)
        .timeout(timeout)
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream");

    for (name, value) in headers {
        builder = builder.header(name.as_str(), value.as_str());
    }

    builder = builder.body(payload.to_string());

    let response = builder.send().await.map_err(|err| {
        if err.is_timeout() {
            ProviderError::Transport {
                message: format!("request timed out after {timeout_ms}ms"),
            }
        } else if err.is_connect() {
            ProviderError::Transport {
                message: format!("connection failed: {err}"),
            }
        } else {
            ProviderError::Transport {
                message: format!("request failed: {err}"),
            }
        }
    })?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        let body = response.text().await.unwrap_or_default();
        let message = extract_error_message_from_body(&body)
            .unwrap_or_else(|| format!("authentication failed (HTTP {status})"));
        return Err(ProviderError::Auth { message });
    }
    if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let body = response.text().await.unwrap_or_default();
        let message = extract_error_message_from_body(&body)
            .unwrap_or_else(|| "rate limited".to_string());
        return Err(ProviderError::RateLimited { message });
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = extract_error_message_from_body(&body)
            .unwrap_or_else(|| format!("request failed (HTTP {status}): {body}"));
        return Err(ProviderError::InvalidResponse { message });
    }

    // Stream the response body and parse SSE events incrementally
    let mut current_data: Vec<String> = Vec::new();
    let mut line_buffer = String::new();
    let mut had_events = false;

    let mut stream = response.bytes_stream();
    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|err| ProviderError::Transport {
            message: format!("stream read error: {err}"),
        })?;

        let chunk_str = String::from_utf8_lossy(&chunk);
        line_buffer.push_str(&chunk_str);

        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos].trim_end_matches('\r').to_string();
            line_buffer = line_buffer[newline_pos + 1..].to_string();

            if line.is_empty() {
                if !current_data.is_empty() {
                    let data_payload = current_data.join("\n");
                    current_data.clear();
                    if data_payload.trim() == "[DONE]" {
                        continue;
                    }
                    match serde_json::from_str::<Value>(&data_payload) {
                        Ok(event) => {
                            had_events = true;
                            on_event(event)?;
                        }
                        Err(err) => {
                            return Err(ProviderError::InvalidResponse {
                                message: format!(
                                    "sse event json parse failed: {err}; event={data_payload}"
                                ),
                            });
                        }
                    }
                }
                continue;
            }

            if let Some(data_line) = line.strip_prefix("data:") {
                current_data.push(data_line.trim_start().to_string());
            }
        }
    }

    // Handle remaining data in buffer
    if !current_data.is_empty() {
        let data_payload = current_data.join("\n");
        if data_payload.trim() != "[DONE]" {
            if let Ok(event) = serde_json::from_str::<Value>(&data_payload) {
                had_events = true;
                on_event(event)?;
            }
        }
    }

    // Non-streaming fallback: try to parse accumulated buffer as single JSON
    if !had_events && !line_buffer.trim().is_empty() {
        match serde_json::from_str::<Value>(line_buffer.trim()) {
            Ok(single) => {
                if let Some(err_payload) = openai_error_payload_from_event(&single) {
                    return Err(map_openai_error(err_payload));
                }
                on_event(single)?;
            }
            Err(err) => {
                return Err(ProviderError::InvalidResponse {
                    message: format!("response parse failed: {err}; body={line_buffer}"),
                });
            }
        }
    }

    Ok(())
}

fn extract_error_message_from_body(body: &str) -> Option<String> {
    let json: Value = serde_json::from_str(body).ok()?;
    if let Some(msg) = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str)
    {
        return Some(msg.to_string());
    }
    if let Some(msg) = json.get("error").and_then(Value::as_str) {
        return Some(msg.to_string());
    }
    if let Some(msg) = json.get("detail").and_then(Value::as_str) {
        return Some(msg.to_string());
    }
    if let Some(msg) = json
        .get("detail")
        .and_then(|d| d.get("message"))
        .and_then(Value::as_str)
    {
        return Some(msg.to_string());
    }
    None
}

// ---------------------------------------------------------------------------
// OpenAI SSE → Frame incremental emitter
// ---------------------------------------------------------------------------

struct OpenAiSseToFrameEmitter<'a> {
    sink: &'a mut dyn FrameSink<OpenAiResponsesFrame>,
    response_id: Option<String>,
    emitted_start: bool,
    started_text: bool,
    started_reasoning: bool,
    saw_completed: bool,
    text_block_id: String,
    reasoning_block_id: String,
    function_call_index: u32,
    completed_usage: lorum_ai_contract::TokenUsage,
    completed_reason: lorum_ai_contract::StopReason,
    last_response_json: Option<Value>,
}

impl<'a> OpenAiSseToFrameEmitter<'a> {
    fn new(sink: &'a mut dyn FrameSink<OpenAiResponsesFrame>) -> Self {
        Self {
            sink,
            response_id: None,
            emitted_start: false,
            started_text: false,
            started_reasoning: false,
            saw_completed: false,
            text_block_id: "text-0".to_string(),
            reasoning_block_id: "reasoning-0".to_string(),
            function_call_index: 0,
            completed_usage: lorum_ai_contract::TokenUsage::default(),
            completed_reason: lorum_ai_contract::StopReason::Stop,
            last_response_json: None,
        }
    }

    fn ensure_start(&mut self, event: &Value) -> Result<(), ProviderError> {
        if self.emitted_start {
            return Ok(());
        }
        if let Some(id) = openai_response_id_from_event(event) {
            self.response_id = Some(id.clone());
            self.emitted_start = true;
            self.sink.push_frame(OpenAiResponsesFrame::ResponseStart {
                message_id: id,
            })?;
        }
        Ok(())
    }

    fn process_event(&mut self, event: &Value) -> Result<(), ProviderError> {
        if let Some(error_payload) = openai_error_payload_from_event(event) {
            return Err(map_openai_error(error_payload));
        }

        self.ensure_start(event)?;

        if let Some(resp) = event.get("response") {
            self.last_response_json = Some(resp.clone());
        }

        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "response.output_text.delta" | "response.refusal.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !delta.is_empty() {
                    if !self.started_text {
                        self.sink.push_frame(OpenAiResponsesFrame::TextStart {
                            block_id: self.text_block_id.clone(),
                        })?;
                        self.started_text = true;
                    }
                    self.sink.push_frame(OpenAiResponsesFrame::TextDelta {
                        block_id: self.text_block_id.clone(),
                        delta,
                    })?;
                }
            }

            "response.reasoning_summary_text.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !delta.is_empty() {
                    if !self.started_reasoning {
                        self.sink.push_frame(OpenAiResponsesFrame::ReasoningStart {
                            block_id: self.reasoning_block_id.clone(),
                        })?;
                        self.started_reasoning = true;
                    }
                    self.sink.push_frame(OpenAiResponsesFrame::ReasoningDelta {
                        block_id: self.reasoning_block_id.clone(),
                        delta,
                    })?;
                }
            }
            "response.reasoning_summary_text.done" => {
                if self.started_reasoning {
                    self.sink.push_frame(OpenAiResponsesFrame::ReasoningEnd {
                        block_id: self.reasoning_block_id.clone(),
                    })?;
                }
            }

            "response.function_call_arguments.delta" => {
                let delta = event
                    .get("delta")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let block_id = format!("call-{}", self.function_call_index);
                if !delta.is_empty() {
                    self.sink.push_frame(OpenAiResponsesFrame::FunctionCallDelta {
                        block_id,
                        delta,
                    })?;
                }
            }
            "response.output_item.added" => {
                if let Some(item) = event.get("item") {
                    let item_type = item.get("type").and_then(Value::as_str).unwrap_or("");
                    if item_type == "function_call" {
                        let block_id = format!("call-{}", self.function_call_index);
                        let call_id = item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        self.sink.push_frame(OpenAiResponsesFrame::FunctionCallStart {
                            block_id,
                            call_id,
                            name,
                        })?;
                    }
                }
            }
            "response.function_call_arguments.done" => {
                let block_id = format!("call-{}", self.function_call_index);
                self.sink
                    .push_frame(OpenAiResponsesFrame::FunctionCallEnd { block_id })?;
                self.function_call_index += 1;
            }

            "response.output_item.done" => {
                if self.started_text {
                    return Ok(());
                }
                if let Some(item) = event.get("item") {
                    let text = extract_openai_response_text(item);
                    if !text.is_empty() {
                        self.sink.push_frame(OpenAiResponsesFrame::TextStart {
                            block_id: self.text_block_id.clone(),
                        })?;
                        self.sink.push_frame(OpenAiResponsesFrame::TextDelta {
                            block_id: self.text_block_id.clone(),
                            delta: text,
                        })?;
                        self.started_text = true;
                    }
                }
            }
            "response.completed" | "response.done" => {
                let response = event.get("response").unwrap_or(event);
                let usage_json = response.get("usage").cloned().unwrap_or(Value::Null);
                self.completed_usage = parse_openai_usage(&usage_json);
                self.completed_reason = map_openai_stop_reason(
                    response
                        .get("status")
                        .and_then(Value::as_str)
                        .or_else(|| response.get("stop_reason").and_then(Value::as_str)),
                );
                // OpenAI Responses API uses "completed" status even for tool calls.
                // Detect function calls by checking if any were emitted during the stream.
                if self.function_call_index > 0 {
                    self.completed_reason = lorum_ai_contract::StopReason::ToolUse;
                }
                self.saw_completed = true;
            }
            _ => {}
        }

        Ok(())
    }

    fn finalize(&mut self) -> Result<Option<String>, ProviderError> {
        // Emit start if we never saw a response id
        if !self.emitted_start {
            let id = self
                .response_id
                .clone()
                .unwrap_or_else(|| "response".to_string());
            self.sink.push_frame(OpenAiResponsesFrame::ResponseStart {
                message_id: id,
            })?;
        }

        // Close unclosed reasoning block
        if self.started_reasoning {
            // Only close if not already closed by a done event
            // (process_event already closes on reasoning_summary_text.done)
        }

        // Close text block
        if self.started_text {
            self.sink.push_frame(OpenAiResponsesFrame::TextEnd {
                block_id: self.text_block_id.clone(),
            })?;
        }

        // Fallback usage from last response json
        if !self.saw_completed {
            if let Some(last_response) = &self.last_response_json {
                let usage_json = last_response.get("usage").cloned().unwrap_or(Value::Null);
                self.completed_usage = parse_openai_usage(&usage_json);
                self.completed_reason = map_openai_stop_reason(
                    last_response
                        .get("status")
                        .and_then(Value::as_str)
                        .or_else(|| last_response.get("stop_reason").and_then(Value::as_str)),
                );
            }
            if self.function_call_index > 0 {
                self.completed_reason = lorum_ai_contract::StopReason::ToolUse;
            }
        }

        self.sink.push_frame(OpenAiResponsesFrame::Completed {
            stop_reason: self.completed_reason,
            usage: self.completed_usage.clone(),
        })?;

        Ok(self.response_id.clone())
    }
}

// ---------------------------------------------------------------------------
// Anthropic SSE → Frame incremental emitter
// ---------------------------------------------------------------------------

struct AnthropicSseToFrameEmitter<'a> {
    sink: &'a mut dyn FrameSink<AnthropicFrame>,
    message_id: String,
    block_index: u32,
    current_block_id: String,
    current_block_type: String,
    usage: lorum_ai_contract::TokenUsage,
    stop_reason: lorum_ai_contract::StopReason,
    had_frames: bool,
    non_streaming_events: Vec<Value>,
}

impl<'a> AnthropicSseToFrameEmitter<'a> {
    fn new(sink: &'a mut dyn FrameSink<AnthropicFrame>) -> Self {
        Self {
            sink,
            message_id: String::new(),
            block_index: 0,
            current_block_id: String::new(),
            current_block_type: String::new(),
            usage: lorum_ai_contract::TokenUsage::default(),
            stop_reason: lorum_ai_contract::StopReason::Stop,
            had_frames: false,
            non_streaming_events: Vec::new(),
        }
    }

    fn process_event(&mut self, event: &Value) -> Result<(), ProviderError> {
        // Check for error
        if let Some(error) = event.get("error") {
            let msg = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("anthropic error");
            let kind = error
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("invalid_response");
            return Err(match kind {
                "authentication_error" => ProviderError::Auth {
                    message: msg.to_string(),
                },
                "rate_limit_error" => ProviderError::RateLimited {
                    message: msg.to_string(),
                },
                _ => ProviderError::InvalidResponse {
                    message: msg.to_string(),
                },
            });
        }

        let event_type = event.get("type").and_then(Value::as_str).unwrap_or("");
        match event_type {
            "message_start" => {
                self.had_frames = true;
                if let Some(msg) = event.get("message") {
                    self.message_id = msg
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("anthropic-message")
                        .to_string();
                    if let Some(u) = msg.get("usage") {
                        self.usage.input_tokens = u
                            .get("input_tokens")
                            .and_then(Value::as_u64)
                            .unwrap_or(0);
                    }
                }
                self.sink.push_frame(AnthropicFrame::MessageStart {
                    message_id: self.message_id.clone(),
                })?;
            }
            "content_block_start" => {
                self.had_frames = true;
                let block_id = format!("block-{}", self.block_index);
                self.block_index += 1;
                self.current_block_id = block_id.clone();

                let content_block = event.get("content_block").unwrap_or(event);
                let block_type = content_block
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("text");
                self.current_block_type = block_type.to_string();

                match block_type {
                    "thinking" => {
                        self.sink
                            .push_frame(AnthropicFrame::ThinkingStart { block_id })?;
                    }
                    "tool_use" => {
                        let call_id = content_block
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let name = content_block
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        self.sink.push_frame(AnthropicFrame::ToolCallStart {
                            block_id,
                            call_id,
                            name,
                        })?;
                    }
                    _ => {
                        self.sink
                            .push_frame(AnthropicFrame::TextStart { block_id })?;
                    }
                }
            }
            "content_block_delta" => {
                self.had_frames = true;
                let delta_obj = event.get("delta").unwrap_or(event);
                let delta_type = delta_obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("");

                match delta_type {
                    "thinking_delta" => {
                        let thinking = delta_obj
                            .get("thinking")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if !thinking.is_empty() {
                            self.sink.push_frame(AnthropicFrame::ThinkingDelta {
                                block_id: self.current_block_id.clone(),
                                delta: thinking,
                            })?;
                        }
                    }
                    "input_json_delta" => {
                        let partial = delta_obj
                            .get("partial_json")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if !partial.is_empty() {
                            self.sink.push_frame(AnthropicFrame::ToolCallDelta {
                                block_id: self.current_block_id.clone(),
                                delta: partial,
                            })?;
                        }
                    }
                    _ => {
                        let text = delta_obj
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if !text.is_empty() {
                            self.sink.push_frame(AnthropicFrame::TextDelta {
                                block_id: self.current_block_id.clone(),
                                delta: text,
                            })?;
                        }
                    }
                }
            }
            "content_block_stop" => {
                self.had_frames = true;
                match self.current_block_type.as_str() {
                    "thinking" => {
                        self.sink.push_frame(AnthropicFrame::ThinkingEnd {
                            block_id: self.current_block_id.clone(),
                        })?;
                    }
                    "tool_use" => {
                        self.sink.push_frame(AnthropicFrame::ToolCallEnd {
                            block_id: self.current_block_id.clone(),
                        })?;
                    }
                    _ => {
                        self.sink.push_frame(AnthropicFrame::TextEnd {
                            block_id: self.current_block_id.clone(),
                        })?;
                    }
                }
            }
            "message_delta" => {
                self.had_frames = true;
                if let Some(delta) = event.get("delta") {
                    self.stop_reason = match delta
                        .get("stop_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("end_turn")
                    {
                        "max_tokens" => lorum_ai_contract::StopReason::Length,
                        "tool_use" => lorum_ai_contract::StopReason::ToolUse,
                        _ => lorum_ai_contract::StopReason::Stop,
                    };
                }
                if let Some(u) = event.get("usage") {
                    self.usage.output_tokens =
                        u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
            }
            "message_stop" => {
                self.had_frames = true;
            }
            "error" => {
                let error = event.get("error").unwrap_or(event);
                let msg = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("anthropic error")
                    .to_string();
                let kind = error
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid_response");
                return Err(match kind {
                    "authentication_error" => ProviderError::Auth { message: msg },
                    "rate_limit_error" => ProviderError::RateLimited { message: msg },
                    _ => ProviderError::InvalidResponse { message: msg },
                });
            }
            _ => {
                // Non-streaming: accumulate for fallback
                if !self.had_frames {
                    self.non_streaming_events.push(event.clone());
                }
            }
        }

        Ok(())
    }

    fn finalize(&mut self) -> Result<(), ProviderError> {
        if !self.had_frames && !self.non_streaming_events.is_empty() {
            return self.emit_non_streaming_fallback();
        }

        if self.had_frames {
            self.sink.push_frame(AnthropicFrame::MessageDone {
                stop_reason: self.stop_reason,
                usage: self.usage.clone(),
            })?;
        }

        Ok(())
    }

    fn emit_non_streaming_fallback(&mut self) -> Result<(), ProviderError> {
        let response = self.non_streaming_events.first().ok_or_else(|| {
            ProviderError::InvalidResponse {
                message: "empty anthropic response".to_string(),
            }
        })?;

        if let Some(error) = response.get("error") {
            let msg = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("anthropic error");
            let kind = error
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("invalid_response");
            return Err(match kind {
                "authentication_error" => ProviderError::Auth {
                    message: msg.to_string(),
                },
                "rate_limit_error" => ProviderError::RateLimited {
                    message: msg.to_string(),
                },
                _ => ProviderError::InvalidResponse {
                    message: msg.to_string(),
                },
            });
        }

        let message_id = response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("anthropic-message")
            .to_string();

        self.sink.push_frame(AnthropicFrame::MessageStart {
            message_id,
        })?;

        let mut text = String::new();
        if let Some(content) = response.get("content").and_then(Value::as_array) {
            for block in content {
                if block.get("type").and_then(Value::as_str) == Some("text") {
                    if let Some(v) = block.get("text").and_then(Value::as_str) {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(v);
                    }
                }
            }
        }

        let usage_json = response.get("usage").cloned().unwrap_or(Value::Null);
        let usage = lorum_ai_contract::TokenUsage {
            input_tokens: usage_json
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            output_tokens: usage_json
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: None,
            cost_usd: None,
        };

        let stop_reason = match response
            .get("stop_reason")
            .and_then(Value::as_str)
            .unwrap_or("end_turn")
        {
            "max_tokens" => lorum_ai_contract::StopReason::Length,
            "tool_use" => lorum_ai_contract::StopReason::ToolUse,
            _ => lorum_ai_contract::StopReason::Stop,
        };

        if !text.is_empty() {
            let block_id = "text-0".to_string();
            self.sink
                .push_frame(AnthropicFrame::TextStart { block_id: block_id.clone() })?;
            self.sink.push_frame(AnthropicFrame::TextDelta {
                block_id: block_id.clone(),
                delta: text,
            })?;
            self.sink
                .push_frame(AnthropicFrame::TextEnd { block_id })?;
        }
        self.sink
            .push_frame(AnthropicFrame::MessageDone { stop_reason, usage })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// OpenAI codex header helpers
// ---------------------------------------------------------------------------

fn openai_codex_headers(
    access_token: &str,
    account_id: String,
    session_id: &str,
) -> Vec<(String, String)> {
    vec![
        (
            "Authorization".to_string(),
            format!("Bearer {access_token}"),
        ),
        (OPENAI_CODEX_ACCOUNT_HEADER.to_string(), account_id),
        (
            OPENAI_CODEX_BETA_HEADER.to_string(),
            OPENAI_CODEX_BETA_VALUE.to_string(),
        ),
        (
            OPENAI_CODEX_ORIGINATOR_HEADER.to_string(),
            OPENAI_CODEX_ORIGINATOR_VALUE.to_string(),
        ),
        (
            OPENAI_CODEX_SESSION_HEADER.to_string(),
            session_id.to_string(),
        ),
        (
            OPENAI_CODEX_CONVERSATION_HEADER.to_string(),
            session_id.to_string(),
        ),
    ]
}

fn chatgpt_account_id_from_access_token(access_token: &str) -> Result<String, ProviderError> {
    let payload_segment = access_token
        .split('.')
        .nth(1)
        .ok_or_else(|| ProviderError::Auth {
            message: "oauth access token had invalid jwt shape".to_string(),
        })?;
    let payload_bytes =
        URL_SAFE_NO_PAD
            .decode(payload_segment)
            .map_err(|err| ProviderError::Auth {
                message: format!("oauth access token payload decode failed: {err}"),
            })?;
    let payload: Value =
        serde_json::from_slice(&payload_bytes).map_err(|err| ProviderError::Auth {
            message: format!("oauth access token payload parse failed: {err}"),
        })?;

    payload
        .get(OPENAI_AUTH_CLAIM_PATH)
        .and_then(Value::as_object)
        .and_then(|auth| auth.get("chatgpt_account_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(ToString::to_string)
        .ok_or_else(|| ProviderError::Auth {
            message: "oauth access token missing chatgpt_account_id claim".to_string(),
        })
}

// ---------------------------------------------------------------------------
// OpenAI error handling
// ---------------------------------------------------------------------------

fn map_openai_error(error: &Value) -> ProviderError {
    if let Some(detail) = error.get("detail") {
        let message = detail
            .as_str()
            .map(ToString::to_string)
            .or_else(|| {
                detail
                    .get("message")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            })
            .unwrap_or_else(|| detail.to_string());
        let lower = message.to_lowercase();
        if lower.contains("unauthorized")
            || lower.contains("auth")
            || lower.contains("forbidden")
        {
            return ProviderError::Auth { message };
        }
        if lower.contains("rate") {
            return ProviderError::RateLimited { message };
        }
        return ProviderError::InvalidResponse { message };
    }

    let code = error
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("openai request failed")
        .to_string();
    let lower = message.to_lowercase();

    if code.contains("rate") {
        return ProviderError::RateLimited { message };
    }

    if code.contains("auth")
        || code.contains("api_key")
        || lower.contains("insufficient permissions")
        || lower.contains("missing scopes")
    {
        return ProviderError::Auth { message };
    }

    ProviderError::InvalidResponse {
        message: format!("{code}: {message}"),
    }
}

fn openai_error_payload_from_event(event: &Value) -> Option<&Value> {
    if event.get("type").and_then(Value::as_str) == Some("error") {
        return event.get("error").or(Some(event));
    }
    if event.get("error").is_some() {
        return event.get("error");
    }
    if event.get("detail").is_some() {
        return Some(event);
    }
    None
}

// ---------------------------------------------------------------------------
// OpenAI response/frame parsing helpers
// ---------------------------------------------------------------------------

fn openai_response_id_from_event(event: &Value) -> Option<String> {
    event
        .get("response")
        .and_then(|response| response.get("id"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            event
                .get("id")
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
}

fn parse_openai_usage(usage_json: &Value) -> lorum_ai_contract::TokenUsage {
    let input_tokens = usage_json
        .get("input_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cache_read_tokens = usage_json
        .get("cache_read_tokens")
        .and_then(Value::as_u64)
        .or_else(|| {
            usage_json
                .get("input_tokens_details")
                .and_then(Value::as_object)
                .and_then(|details| details.get("cached_tokens"))
                .and_then(Value::as_u64)
        })
        .unwrap_or(0);

    lorum_ai_contract::TokenUsage {
        input_tokens: input_tokens.saturating_sub(cache_read_tokens),
        output_tokens: usage_json
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_tokens,
        cache_write_tokens: usage_json
            .get("cache_write_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage_json.get("total_tokens").and_then(Value::as_u64),
        cost_usd: usage_json.get("cost_usd").and_then(Value::as_f64),
    }
}

fn map_openai_stop_reason(label: Option<&str>) -> lorum_ai_contract::StopReason {
    match label.unwrap_or("stop") {
        "stop" | "completed" => lorum_ai_contract::StopReason::Stop,
        "length" | "incomplete" => lorum_ai_contract::StopReason::Length,
        "tool_use" | "tool_calls" => lorum_ai_contract::StopReason::ToolUse,
        "aborted" | "cancelled" | "canceled" => lorum_ai_contract::StopReason::Aborted,
        "error" | "failed" => lorum_ai_contract::StopReason::Error,
        _ => lorum_ai_contract::StopReason::Stop,
    }
}

// Batch version kept for tests
#[allow(dead_code)]
fn openai_codex_frames_from_events(
    events: &[Value],
) -> Result<(Vec<OpenAiResponsesFrame>, Option<String>), ProviderError> {
    let mut sink = crate::CollectingFrameSink::default();
    let mut emitter = OpenAiSseToFrameEmitter::new(&mut sink);
    for event in events {
        emitter.process_event(event)?;
    }
    let response_id = emitter.finalize()?;
    Ok((sink.frames, response_id))
}

// ---------------------------------------------------------------------------
// Text extraction helpers
// ---------------------------------------------------------------------------

fn extract_openai_response_text(payload: &Value) -> String {
    let mut chunks = Vec::new();

    if let Some(output_items) = payload.get("output").and_then(Value::as_array) {
        for item in output_items {
            collect_openai_output_text_chunks(item, &mut chunks);
        }
    }

    if chunks.is_empty() {
        if let Some(content_items) = payload.get("content").and_then(Value::as_array) {
            for item in content_items {
                collect_openai_output_text_chunks(item, &mut chunks);
            }
        }
    }

    if chunks.is_empty() {
        if let Some(text) = extract_openai_string_like(payload.get("output_text")) {
            if !text.trim().is_empty() {
                chunks.push(text);
            }
        }
    }

    if chunks.is_empty() {
        collect_openai_text_fallback(payload, &mut chunks);
    }
    chunks.join("\n")
}

fn collect_openai_output_text_chunks(item: &Value, chunks: &mut Vec<String>) {
    if let Some(wrapped_item) = item.get("item") {
        collect_openai_output_text_chunks(wrapped_item, chunks);
    }

    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for block in content {
            collect_openai_text_block(block, chunks);
        }
        return;
    }

    if let Some(output_text) = extract_openai_string_like(item.get("output_text")) {
        if !output_text.trim().is_empty() {
            chunks.push(output_text);
        }
        return;
    }

    collect_openai_text_block(item, chunks);
}

fn collect_openai_text_block(block: &Value, chunks: &mut Vec<String>) {
    if let Some(value) = openai_text_from_block(block) {
        if !value.trim().is_empty() {
            chunks.push(value);
        }
    }
}

fn openai_text_from_block(block: &Value) -> Option<String> {
    let kind = block.get("type").and_then(Value::as_str);
    match kind {
        Some("output_text") | Some("text") => extract_openai_string_like(block.get("text"))
            .or_else(|| extract_openai_string_like(block.get("output_text"))),
        Some("refusal") => extract_openai_string_like(block.get("refusal"))
            .or_else(|| extract_openai_string_like(block.get("text"))),
        _ => extract_openai_string_like(block.get("text"))
            .or_else(|| extract_openai_string_like(block.get("output_text"))),
    }
}

fn extract_openai_string_like(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.to_string()),
        Some(Value::Object(map)) => map
            .get("text")
            .and_then(Value::as_str)
            .or_else(|| map.get("value").and_then(Value::as_str))
            .map(ToString::to_string),
        _ => None,
    }
}

fn collect_openai_text_fallback(value: &Value, chunks: &mut Vec<String>) {
    match value {
        Value::Array(values) => {
            for child in values {
                collect_openai_text_fallback(child, chunks);
            }
        }
        Value::Object(map) => {
            if map.get("type").and_then(Value::as_str) == Some("input_text") {
                return;
            }

            for key in ["output_text", "text", "refusal"] {
                if let Some(text) = extract_openai_string_like(map.get(key)) {
                    if !text.trim().is_empty() {
                        chunks.push(text);
                    }
                }
            }

            for (key, child) in map {
                if key == "output_text" || key == "text" || key == "refusal" {
                    continue;
                }
                collect_openai_text_fallback(child, chunks);
            }
        }
        _ => {}
    }
}

// Batch version kept for tests
#[allow(dead_code)]
fn openai_frames_from_response(
    response: &Value,
) -> Result<Vec<OpenAiResponsesFrame>, ProviderError> {
    let payload = response.get("response").unwrap_or(response);

    if let Some(err_payload) = openai_error_payload_from_event(payload) {
        return Err(map_openai_error(err_payload));
    }

    let message_id = payload
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("response")
        .to_string();

    let text = extract_openai_response_text(payload);

    if text.is_empty() && payload.get("output").is_none() && payload.get("status").is_none() {
        return Err(ProviderError::InvalidResponse {
            message: format!(
                "unexpected response format: {}",
                serde_json::to_string_pretty(payload)
                    .unwrap_or_else(|_| payload.to_string())
            ),
        });
    }

    let usage_json = payload.get("usage").cloned().unwrap_or(Value::Null);
    let usage = parse_openai_usage(&usage_json);

    let stop_reason = map_openai_stop_reason(
        payload
            .get("stop_reason")
            .and_then(Value::as_str)
            .or_else(|| payload.get("status").and_then(Value::as_str)),
    );

    let mut frames = Vec::new();
    frames.push(OpenAiResponsesFrame::ResponseStart { message_id });
    if !text.is_empty() {
        let block_id = "text-0".to_string();
        frames.push(OpenAiResponsesFrame::TextStart {
            block_id: block_id.clone(),
        });
        frames.push(OpenAiResponsesFrame::TextDelta {
            block_id: block_id.clone(),
            delta: text,
        });
        frames.push(OpenAiResponsesFrame::TextEnd { block_id });
    }
    frames.push(OpenAiResponsesFrame::Completed { stop_reason, usage });
    Ok(frames)
}

// ---------------------------------------------------------------------------
// Tool definition formatting helpers
// ---------------------------------------------------------------------------

fn openai_tool_definitions(tools: &[ToolDefinition]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                })
            })
            .collect(),
    )
}

fn anthropic_tool_definitions(tools: &[ToolDefinition]) -> Value {
    Value::Array(
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect(),
    )
}

// ---------------------------------------------------------------------------
// Prompt formatting helpers
// ---------------------------------------------------------------------------

fn openai_codex_input(request: &ProviderRequest) -> Value {
    let mut items = Vec::new();

    for msg in &request.input {
        match msg {
            ProviderInputMessage::User { content } => items.push(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": content}],
            })),
            ProviderInputMessage::Assistant { message } => {
                let mut content_blocks = Vec::new();
                for block in &message.content {
                    match block {
                        AssistantContent::Text(text) => {
                            content_blocks.push(serde_json::json!({
                                "type": "output_text",
                                "text": text.text,
                            }));
                        }
                        AssistantContent::ToolCall(tc) => {
                            items.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": tc.id,
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }));
                        }
                        _ => {}
                    }
                }
                if !content_blocks.is_empty() {
                    items.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                }
            }
            ProviderInputMessage::ToolResult {
                tool_call_id,
                result,
                ..
            } => items.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": tool_call_id,
                "output": result.to_string(),
            })),
        }
    }

    if items.is_empty() {
        items.push(serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}],
        }));
    }

    Value::Array(items)
}

fn openai_prompt_input(request: &ProviderRequest) -> Value {
    // Use structured input (same shape as codex) for tool calling support.
    // The OpenAI Responses API accepts either a string or an array for `input`.
    let mut items = Vec::new();

    for msg in &request.input {
        match msg {
            ProviderInputMessage::User { content } => items.push(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": content}],
            })),
            ProviderInputMessage::Assistant { message } => {
                let mut content_blocks = Vec::new();
                for block in &message.content {
                    match block {
                        AssistantContent::Text(text) => {
                            content_blocks.push(serde_json::json!({
                                "type": "output_text",
                                "text": text.text,
                            }));
                        }
                        AssistantContent::ToolCall(tc) => {
                            items.push(serde_json::json!({
                                "type": "function_call",
                                "call_id": tc.id,
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            }));
                        }
                        _ => {}
                    }
                }
                if !content_blocks.is_empty() {
                    items.push(serde_json::json!({
                        "type": "message",
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                }
            }
            ProviderInputMessage::ToolResult {
                tool_call_id,
                result,
                ..
            } => items.push(serde_json::json!({
                "type": "function_call_output",
                "call_id": tool_call_id,
                "output": result.to_string(),
            })),
        }
    }

    if items.is_empty() {
        items.push(serde_json::json!({
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}],
        }));
    }

    Value::Array(items)
}

fn anthropic_prompt_parts(request: &ProviderRequest) -> (Option<String>, Vec<Value>) {
    let mut messages = Vec::new();

    for msg in &request.input {
        match msg {
            ProviderInputMessage::User { content } => messages.push(serde_json::json!({
                "role": "user",
                "content": content,
            })),
            ProviderInputMessage::Assistant { message } => {
                let mut content_blocks = Vec::new();
                for block in &message.content {
                    match block {
                        AssistantContent::Text(text) => {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": text.text,
                            }));
                        }
                        AssistantContent::ToolCall(tc) => {
                            content_blocks.push(serde_json::json!({
                                "type": "tool_use",
                                "id": tc.id,
                                "name": tc.name,
                                "input": tc.arguments,
                            }));
                        }
                        _ => {}
                    }
                }
                if !content_blocks.is_empty() {
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                }
            }
            ProviderInputMessage::ToolResult {
                tool_call_id,
                is_error,
                result,
            } => {
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_call_id,
                        "is_error": is_error,
                        "content": result.to_string(),
                    }],
                }));
            }
        }
    }

    if messages.is_empty() {
        messages.push(serde_json::json!({
            "role": "user",
            "content": "hello",
        }));
    }

    (request.system_prompt.clone(), messages)
}

// ---------------------------------------------------------------------------
// Model defaults
// ---------------------------------------------------------------------------

fn default_openai_model() -> ModelRef {
    ModelRef {
        provider: "openai".to_string(),
        api: ApiKind::OpenAiResponses,
        model: env_first_non_empty(&["OMP_DEFAULT_OPENAI_MODEL", "OPENAI_MODEL"])
            .unwrap_or_else(|| DEFAULT_OPENAI_MODEL.to_string()),
    }
}

fn default_codex_model() -> ModelRef {
    ModelRef {
        provider: "openai".to_string(),
        api: ApiKind::OpenAiCodexResponses,
        model: env_first_non_empty(&["OMP_DEFAULT_CODEX_MODEL", "OPENAI_CODEX_MODEL"])
            .unwrap_or_else(|| DEFAULT_CODEX_MODEL.to_string()),
    }
}

fn default_anthropic_model() -> ModelRef {
    ModelRef {
        provider: "anthropic".to_string(),
        api: ApiKind::AnthropicMessages,
        model: env_first_non_empty(&["OMP_DEFAULT_ANTHROPIC_MODEL", "ANTHROPIC_MODEL"])
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_MODEL.to_string()),
    }
}

fn default_minimax_model() -> ModelRef {
    ModelRef {
        provider: "minimax".to_string(),
        api: ApiKind::MiniMaxMessages,
        model: env_first_non_empty(&["OMP_DEFAULT_MINIMAX_MODEL", "MINIMAX_MODEL"])
            .unwrap_or_else(|| DEFAULT_MINIMAX_MODEL.to_string()),
    }
}

fn env_first_non_empty(keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Ok(value) = env::var(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// SSE parsing helper (kept for tests)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
fn parse_sse_json_events(body: &str) -> Result<Vec<Value>, ProviderError> {
    let mut events = Vec::new();
    let mut current_data = Vec::new();

    for raw_line in body.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            if current_data.is_empty() {
                continue;
            }
            let payload = current_data.join("\n");
            current_data.clear();
            if payload.trim() == "[DONE]" {
                continue;
            }
            let event = serde_json::from_str::<Value>(&payload).map_err(|err| {
                ProviderError::InvalidResponse {
                    message: format!("sse event json parse failed: {err}; event={payload}"),
                }
            })?;
            events.push(event);
            continue;
        }

        if let Some(data_line) = line.strip_prefix("data:") {
            current_data.push(data_line.trim_start().to_string());
        }
    }

    if !current_data.is_empty() {
        let payload = current_data.join("\n");
        if payload.trim() != "[DONE]" {
            let event = serde_json::from_str::<Value>(&payload).map_err(|err| {
                ProviderError::InvalidResponse {
                    message: format!("sse event json parse failed: {err}; event={payload}"),
                }
            })?;
            events.push(event);
        }
    }

    if events.is_empty() {
        if let Ok(single) = serde_json::from_str::<Value>(body) {
            return Ok(vec![single]);
        }
        return Err(ProviderError::InvalidResponse {
            message: format!("sse stream contained no json events; body={body}"),
        });
    }

    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openai_scope_error_maps_to_auth_error() {
        let error = serde_json::json!({
            "code": "unknown",
            "message": "Missing scopes: api.responses.write",
        });

        let mapped = map_openai_error(&error);
        assert!(matches!(mapped, ProviderError::Auth { .. }));
    }

    #[test]
    fn codex_account_id_claim_is_extracted_from_oauth_token() {
        let token =
            "x.eyJodHRwczovL2FwaS5vcGVuYWkuY29tL2F1dGgiOnsiY2hhdGdwdF9hY2NvdW50X2lkIjoiYWNjdF8xMjMifX0.y";
        let account_id =
            chatgpt_account_id_from_access_token(token).expect("extract account id from jwt");
        assert_eq!(account_id, "acct_123");
    }
    #[test]
    fn sse_event_parser_reads_json_events_and_done_marker() {
        let body = concat!(
            "data: {\"type\":\"response.created\",\"response\":{\"id\":\"resp_1\"}}\n\n",
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hello\"}\n\n",
            "data: [DONE]\n\n"
        );

        let events = parse_sse_json_events(body).expect("parse sse events");
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0]
                .get("response")
                .and_then(|response| response.get("id"))
                .and_then(Value::as_str),
            Some("resp_1")
        );
        assert_eq!(
            events[1].get("delta").and_then(Value::as_str),
            Some("hello")
        );
    }

    #[test]
    fn codex_stream_events_map_to_text_and_completed_frames() {
        let events = vec![
            serde_json::json!({
                "type": "response.created",
                "response": {"id": "resp_sse"}
            }),
            serde_json::json!({
                "type": "response.output_text.delta",
                "delta": "hello"
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_sse",
                    "status": "completed",
                    "usage": {
                        "input_tokens": 8,
                        "output_tokens": 2,
                        "total_tokens": 10,
                        "input_tokens_details": {"cached_tokens": 3}
                    }
                }
            }),
        ];

        let (frames, response_id) =
            openai_codex_frames_from_events(&events).expect("map codex events to frames");
        assert_eq!(response_id.as_deref(), Some("resp_sse"));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::TextDelta { delta, .. } if delta == "hello"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::Completed { usage, .. }
                if usage.input_tokens == 5 && usage.cache_read_tokens == 3 && usage.output_tokens == 2
        )));
    }

    #[test]
    fn codex_reasoning_events_produce_reasoning_frames() {
        let events = vec![
            serde_json::json!({
                "type": "response.created",
                "response": {"id": "resp_think"}
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "delta": "thinking about "
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.delta",
                "delta": "the answer"
            }),
            serde_json::json!({
                "type": "response.reasoning_summary_text.done"
            }),
            serde_json::json!({
                "type": "response.output_text.delta",
                "delta": "Hello!"
            }),
            serde_json::json!({
                "type": "response.completed",
                "response": {
                    "id": "resp_think",
                    "status": "completed",
                    "usage": {"input_tokens": 5, "output_tokens": 2}
                }
            }),
        ];

        let (frames, _) =
            openai_codex_frames_from_events(&events).expect("map reasoning events");

        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::ReasoningStart { .. }
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::ReasoningDelta { delta, .. } if delta == "thinking about "
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::ReasoningEnd { .. }
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::TextDelta { delta, .. } if delta == "Hello!"
        )));
    }

    #[test]
    fn frame_parser_extracts_text_from_text_and_refusal_blocks() {
        let response = serde_json::json!({
            "id": "resp_text",
            "output": [
                {
                    "type": "message",
                    "content": [
                        {"type": "text", "text": "hello"},
                        {"type": "refusal", "refusal": "cannot comply"}
                    ]
                }
            ],
            "usage": {
                "input_tokens": 7,
                "output_tokens": 5
            }
        });

        let frames = openai_frames_from_response(&response).unwrap();
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::TextDelta { delta, .. } if delta == "hello\ncannot comply"
        )));
    }

    #[test]
    fn frame_parser_reads_nested_response_wrapper_and_output_text() {
        let response = serde_json::json!({
            "response": {
                "id": "resp_wrapped",
                "output_text": "wrapped hello",
                "status": "completed",
                "usage": {"input_tokens": 1, "output_tokens": 1}
            }
        });

        let frames = openai_frames_from_response(&response).unwrap();
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::ResponseStart { message_id } if message_id == "resp_wrapped"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::TextDelta { delta, .. } if delta == "wrapped hello"
        )));
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::Completed { stop_reason, .. } if *stop_reason == lorum_ai_contract::StopReason::Stop
        )));
    }
    #[test]
    fn frame_parser_extracts_text_from_item_wrapper_with_object_text() {
        let response = serde_json::json!({
            "id": "resp_item",
            "output": [
                {
                    "type": "response.output_item.done",
                    "item": {
                        "type": "message",
                        "content": [
                            {"type": "output_text", "text": {"value": "wrapped text"}}
                        ]
                    }
                }
            ],
            "status": "completed"
        });

        let frames = openai_frames_from_response(&response).unwrap();
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::TextDelta { delta, .. } if delta == "wrapped text"
        )));
    }

    #[test]
    fn codex_input_uses_message_array_shape() {
        let request = ProviderRequest {
            session_id: "session-1".to_string(),
            model: default_codex_model(),
            system_prompt: None,
            input: vec![ProviderInputMessage::User {
                content: "hello codex".to_string(),
            }],
            tools: vec![],
        };

        let input = openai_codex_input(&request);
        assert!(matches!(input, Value::Array(_)));
        let first = input
            .as_array()
            .and_then(|items| items.first())
            .expect("first input item exists");
        assert_eq!(first.get("type").and_then(Value::as_str), Some("message"));
        assert_eq!(first.get("role").and_then(Value::as_str), Some("user"));
        assert_eq!(
            first
                .get("content")
                .and_then(Value::as_array)
                .and_then(|content| content.first())
                .and_then(|part| part.get("type"))
                .and_then(Value::as_str),
            Some("input_text")
        );
    }
    #[test]
    fn frame_parser_fallback_recovers_text_from_untyped_nested_fields() {
        let response = serde_json::json!({
            "id": "resp_fallback",
            "output": [
                {
                    "type": "message",
                    "content": [
                        {"note": {"text": "fallback text"}}
                    ]
                }
            ],
            "status": "completed"
        });

        let frames = openai_frames_from_response(&response).unwrap();
        assert!(frames.iter().any(|frame| matches!(
            frame,
            OpenAiResponsesFrame::TextDelta { delta, .. } if delta == "fallback text"
        )));
    }
    #[test]
    fn catalog_exposes_default_and_named_presets() {
        let catalog = build_curl_provider_catalog();
        assert!(catalog.default_model().is_some());
        assert!(catalog.preset_model("openai").is_some());
        assert!(catalog.preset_model("codex").is_some());
        assert!(catalog.preset_model("anthropic").is_some());
        assert!(catalog.preset_model("minimax").is_some());
    }
}
