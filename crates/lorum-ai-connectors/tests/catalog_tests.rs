use lorum_ai_connectors::internals::{
    anthropic_prompt_parts, chatgpt_account_id_from_access_token, default_codex_model,
    map_openai_error, openai_codex_frames_from_events, openai_codex_input, openai_prompt_input,
    parse_sse_json_events,
};
use lorum_ai_connectors::{build_curl_provider_catalog, OpenAiResponsesFrame};
use lorum_ai_contract::{
    ApiKind, AssistantContent, AssistantMessage, ModelRef, ProviderError, ProviderInputMessage,
    ProviderRequest, StopReason, TokenUsage, ToolCall,
};
use serde_json::Value;

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
fn codex_input_uses_message_array_shape() {
    let request = ProviderRequest {
        session_id: "session-1".to_string(),
        model: default_codex_model(),
        system_prompt: None,
        input: vec![ProviderInputMessage::User {
            content: "hello codex".to_string(),
        }],
        tools: vec![],
        tool_choice: None,
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
fn catalog_exposes_default_and_named_presets() {
    let catalog = build_curl_provider_catalog();
    assert!(catalog.default_model().is_some());
    assert!(catalog.preset_model("openai").is_some());
    assert!(catalog.preset_model("codex").is_some());
    assert!(catalog.preset_model("anthropic").is_some());
    assert!(catalog.preset_model("minimax").is_some());
}

fn request_with_orphaned_tool_calls() -> ProviderRequest {
    ProviderRequest {
        session_id: "s-1".to_string(),
        model: ModelRef {
            provider: "openai".to_string(),
            api: ApiKind::OpenAiResponses,
            model: "gpt-5.2".to_string(),
        },
        system_prompt: None,
        input: vec![
            ProviderInputMessage::User {
                content: "hello".to_string(),
            },
            ProviderInputMessage::Assistant {
                message: AssistantMessage {
                    message_id: "m-1".to_string(),
                    model: ModelRef {
                        provider: "openai".to_string(),
                        api: ApiKind::OpenAiResponses,
                        model: "gpt-5.2".to_string(),
                    },
                    content: vec![AssistantContent::ToolCall(ToolCall {
                        id: "tc-orphan".to_string(),
                        name: "read_file".to_string(),
                        arguments: serde_json::json!({"path": "src/lib.rs"}),
                    })],
                    usage: TokenUsage::default(),
                    stop_reason: StopReason::ToolUse,
                },
            },
            // No ToolResult for tc-orphan -- this is the orphan
        ],
        tools: vec![],
        tool_choice: None,
    }
}

#[test]
fn openai_serializer_patches_orphaned_tool_calls() {
    let request = request_with_orphaned_tool_calls();
    let input = openai_prompt_input(&request);

    let items = input.as_array().expect("input is array");
    let has_function_call_output = items.iter().any(|item| {
        item.get("type").and_then(Value::as_str) == Some("function_call_output")
            && item.get("call_id").and_then(Value::as_str) == Some("tc-orphan")
    });
    assert!(
        has_function_call_output,
        "expected synthetic function_call_output for orphaned tool call"
    );
}

#[test]
fn anthropic_serializer_patches_orphaned_tool_calls() {
    let request = request_with_orphaned_tool_calls();
    let (_system, messages) = anthropic_prompt_parts(&request);

    let has_tool_result = messages.iter().any(|msg| {
        if let Some(content) = msg.get("content").and_then(Value::as_array) {
            content.iter().any(|block| {
                block.get("type").and_then(Value::as_str) == Some("tool_result")
                    && block.get("tool_use_id").and_then(Value::as_str) == Some("tc-orphan")
            })
        } else {
            false
        }
    });
    assert!(
        has_tool_result,
        "expected synthetic tool_result for orphaned tool call"
    );
}
