use std::env;
use std::io::{self, Write};
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use futures::executor::block_on;
use lorum_ai_auth::{
    AuthError, OAuthBeginContext, OAuthCallbackFlow, OAuthHttpClient, OAuthProvider,
    OpenAiCodexOAuthProvider,
};
use lorum_ai_connectors::{
    FrameSink, OpenAiResponsesAdapter, OpenAiResponsesFrame, OpenAiResponsesTransport, RetryPolicy,
};
use lorum_ai_contract::{
    ApiKind, AssistantContent, ProviderAdapter, ProviderContext, ProviderError,
    ProviderInputMessage, ProviderRequest, StopReason, TokenUsage,
};
use serde_json::Value;

struct CurlOAuthHttpClient;

#[async_trait]
impl OAuthHttpClient for CurlOAuthHttpClient {
    async fn post_form(
        &self,
        url: &str,
        form: &[(String, String)],
        headers: &[(String, String)],
    ) -> Result<Value, AuthError> {
        let mut cmd = Command::new("curl");
        cmd.arg("-sS")
            .arg("-X")
            .arg("POST")
            .arg(url)
            .arg("-H")
            .arg("accept: application/json")
            .arg("-H")
            .arg("content-type: application/x-www-form-urlencoded");

        for (key, value) in headers {
            cmd.arg("-H").arg(format!("{key}: {value}"));
        }

        for (key, value) in form {
            cmd.arg("--data-urlencode").arg(format!("{key}={value}"));
        }

        let output = cmd.output().map_err(|err| {
            AuthError::InvalidCredential(format!("failed to execute curl for oauth request: {err}"))
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AuthError::InvalidCredential(format!(
                "oauth curl request failed with status {}: {}",
                output.status,
                stderr.trim()
            )));
        }

        let body = String::from_utf8(output.stdout).map_err(|err| {
            AuthError::Serialization(format!("oauth response was not valid utf8: {err}"))
        })?;

        serde_json::from_str(&body).map_err(|err| {
            AuthError::Serialization(format!("oauth json parse failed: {err}; body={body}"))
        })
    }
}

struct OpenAiResponsesHttpTransport;

#[async_trait]
impl OpenAiResponsesTransport for OpenAiResponsesHttpTransport {
    async fn stream_frames(
        &self,
        request: &ProviderRequest,
        context: &ProviderContext,
        sink: &mut dyn FrameSink<OpenAiResponsesFrame>,
    ) -> Result<(), ProviderError> {
        let api_key = context.api_key.clone().ok_or_else(|| ProviderError::Auth {
            message: "missing api key in provider context".to_string(),
        })?;

        let prompt = extract_prompt(request);
        let payload = serde_json::json!({
            "model": request.model.model,
            "input": prompt,
        });

        let output = Command::new("curl")
            .arg("-sS")
            .arg("-X")
            .arg("POST")
            .arg("https://api.openai.com/v1/responses")
            .arg("-H")
            .arg(format!("Authorization: Bearer {api_key}"))
            .arg("-H")
            .arg("Content-Type: application/json")
            .arg("-d")
            .arg(payload.to_string())
            .output()
            .map_err(|err| ProviderError::Transport {
                message: format!("failed to execute curl for responses request: {err}"),
            })?;

        let stdout =
            String::from_utf8(output.stdout).map_err(|err| ProviderError::InvalidResponse {
                message: format!("responses stdout was not utf8: {err}"),
            })?;
        let stderr =
            String::from_utf8(output.stderr).map_err(|err| ProviderError::InvalidResponse {
                message: format!("responses stderr was not utf8: {err}"),
            })?;

        if !output.status.success() {
            return Err(ProviderError::Transport {
                message: format!(
                    "responses request failed with status {}: stdout={} stderr={}",
                    output.status,
                    stdout.trim(),
                    stderr.trim()
                ),
            });
        }

        let response: Value =
            serde_json::from_str(&stdout).map_err(|err| ProviderError::InvalidResponse {
                message: format!("responses json parse failed: {err}; body={stdout}"),
            })?;

        if let Some(err) = response.get("error") {
            return Err(map_openai_error(err));
        }

        let message_id = response
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("response")
            .to_string();

        let text = extract_output_text(&response);
        let usage = parse_usage(&response);
        let stop_reason = parse_stop_reason(response.get("stop_reason").and_then(Value::as_str));

        sink.push_frame(OpenAiResponsesFrame::ResponseStart { message_id })?;
        if !text.is_empty() {
            let block_id = "text-0".to_string();
            sink.push_frame(OpenAiResponsesFrame::TextStart {
                block_id: block_id.clone(),
            })?;
            sink.push_frame(OpenAiResponsesFrame::TextDelta {
                block_id: block_id.clone(),
                delta: text,
            })?;
            sink.push_frame(OpenAiResponsesFrame::TextEnd { block_id })?;
        }
        sink.push_frame(OpenAiResponsesFrame::Completed { stop_reason, usage })?;
        Ok(())
    }
}

fn extract_prompt(request: &ProviderRequest) -> String {
    let mut chunks: Vec<String> = Vec::new();

    if let Some(ref system) = request.system_prompt {
        chunks.push(system.clone());
    }

    for msg in &request.input {
        if let ProviderInputMessage::User { content } = msg {
            chunks.push(content.clone());
        }
    }

    if chunks.is_empty() {
        chunks.push("hello world".to_string());
    }

    chunks.join("\n")
}

fn parse_usage(response: &Value) -> TokenUsage {
    let usage = response.get("usage").cloned().unwrap_or(Value::Null);
    TokenUsage {
        input_tokens: usage
            .get("input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        output_tokens: usage
            .get("output_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_tokens: usage
            .get("cache_read_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_write_tokens: usage
            .get("cache_write_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        total_tokens: usage.get("total_tokens").and_then(Value::as_u64),
        cost_usd: usage.get("cost_usd").and_then(Value::as_f64),
    }
}

fn parse_stop_reason(stop_reason: Option<&str>) -> StopReason {
    match stop_reason.unwrap_or("stop") {
        "stop" => StopReason::Stop,
        "length" => StopReason::Length,
        "tool_use" => StopReason::ToolUse,
        "aborted" => StopReason::Aborted,
        "error" => StopReason::Error,
        _ => StopReason::Stop,
    }
}

fn map_openai_error(error_obj: &Value) -> ProviderError {
    let code = error_obj
        .get("code")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let message = error_obj
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown OpenAI error")
        .to_string();

    if code.contains("rate") {
        return ProviderError::RateLimited { message };
    }
    if code.contains("auth") || code.contains("api_key") || code.contains("invalid_api_key") {
        return ProviderError::Auth { message };
    }
    ProviderError::InvalidResponse {
        message: format!("{code}: {message}"),
    }
}

fn extract_output_text(response: &Value) -> String {
    let parts: Vec<String> = response
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|block| {
            if block.get("type").and_then(Value::as_str) == Some("output_text") {
                block
                    .get("text")
                    .and_then(Value::as_str)
                    .map(ToString::to_string)
            } else {
                None
            }
        })
        .collect();

    parts.join("\n")
}

fn required_env(name: &str) -> Result<String, Box<dyn std::error::Error>> {
    let value = env::var(name).map_err(|_| format!("missing required env var: {name}"))?;
    if value.trim().is_empty() {
        return Err(format!("required env var is empty: {name}").into());
    }
    Ok(value)
}

fn read_line(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn mask_token(token: &str) -> String {
    if token.len() <= 8 {
        return "***".to_string();
    }
    format!("{}...{}", &token[..4], &token[token.len() - 4..])
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client_id = required_env("OMP_SMOKE_OPENAI_CLIENT_ID")
        .or_else(|_| required_env("OPENAI_OAUTH_CLIENT_ID"))?;
    let redirect_uri = required_env("OMP_SMOKE_OPENAI_REDIRECT_URI")
        .or_else(|_| required_env("OPENAI_OAUTH_REDIRECT_URI"))?;

    let provider = OpenAiCodexOAuthProvider::new(
        Arc::new(CurlOAuthHttpClient),
        client_id,
        redirect_uri.clone(),
    );

    let begin = block_on(provider.begin_flow(OAuthBeginContext {
        redirect_uri,
        scopes: Vec::new(),
        state: None,
    }))?;

    println!(
        "OpenAI OAuth authorization URL:\n{}\n",
        begin.authorization_url
    );
    println!("Expected state: {}", begin.state);
    println!("Code verifier generated: {}", begin.code_verifier.is_some());
    println!("\nOpen the URL above, complete login/consent, then paste callback URL or raw code.");

    let callback_or_code = env::var("OPENAI_OAUTH_CALLBACK_INPUT")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .map(|v| v.trim().to_string())
        .or_else(|| {
            env::var("OPENAI_OAUTH_CODE")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .map(|v| v.trim().to_string())
        })
        .unwrap_or(read_line("callback URL or code: ")?);

    let callback_flow = OAuthCallbackFlow::new(0, 300);
    let code = callback_flow.parse_callback_or_manual_input(&callback_or_code, &begin.state)?;

    let token = block_on(provider.exchange_code(&code, begin.code_verifier.as_deref()))?;

    println!("\nExchange succeeded");
    println!(
        "access_token: {}",
        mask_token(&token.credential.access_token)
    );
    println!(
        "refresh_token: {}",
        token
            .credential
            .refresh_token
            .as_deref()
            .map(mask_token)
            .unwrap_or_else(|| "<none>".to_string())
    );
    println!("expires_at_unix: {:?}", token.credential.expires_at_unix);

    let mut active_credential = token.credential.clone();
    let refresh_now = read_line("refresh now? [y/N]: ")?;
    if refresh_now.eq_ignore_ascii_case("y") {
        let refreshed = block_on(provider.refresh(&active_credential))?;
        println!("\nRefresh succeeded");
        println!("access_token: {}", mask_token(&refreshed.access_token));
        println!(
            "refresh_token: {}",
            refreshed
                .refresh_token
                .as_deref()
                .map(mask_token)
                .unwrap_or_else(|| "<none>".to_string())
        );
        println!("expires_at_unix: {:?}", refreshed.expires_at_unix);
        active_credential = refreshed;
    }

    let model = env::var("OPENAI_OAUTH_TEST_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "gpt-4.1-mini".to_string());
    let prompt = env::var("OPENAI_OAUTH_TEST_PROMPT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "hello world".to_string());

    println!("\nRunning connector-backed hello-world request with model: {model}");
    let adapter = OpenAiResponsesAdapter::new(Arc::new(OpenAiResponsesHttpTransport))
        .with_retry_policy(RetryPolicy::new(1));
    let message = block_on(adapter.complete(
        ProviderRequest {
            session_id: "oauth-rig-session".to_string(),
            model: lorum_ai_contract::ModelRef {
                provider: "openai".to_string(),
                api: ApiKind::OpenAiResponses,
                model,
            },
            system_prompt: None,
            input: vec![ProviderInputMessage::User { content: prompt }],
            tools: vec![],
            tool_choice: None,
        },
        ProviderContext {
            api_key: Some(active_credential.access_token),
            timeout_ms: 30_000,
        },
    ))?;

    let text_blocks: Vec<&str> = message
        .content
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect();

    if text_blocks.is_empty() {
        println!(
            "model output (non-text): {}",
            serde_json::to_string_pretty(&serde_json::to_value(&message)?)?
        );
    } else {
        println!("model output: {}", text_blocks.join("\n"));
    }

    Ok(())
}
