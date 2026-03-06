use lorum_ai_contract::{
    patch_orphaned_tool_calls, ApiKind, AssistantContent, AssistantMessage,
    AssistantMessageEvent, ModelRef, ProviderContext, ProviderError, ProviderFinal,
    ProviderInputMessage, ProviderRequest, ProviderTransportDetails, StopReason,
    StreamBoundaryEvent, StreamDoneEvent, StreamErrorEvent, StreamStartEvent, StreamTextDelta,
    StreamThinkingDelta, StreamToolCallDelta, TextContent, ThinkingContent, TokenUsage, ToolCall,
};
use serde_json::Value;

fn sample_model_ref() -> ModelRef {
    ModelRef {
        provider: "openai".to_string(),
        api: ApiKind::OpenAiResponses,
        model: "gpt-5.2".to_string(),
    }
}

fn sample_message(stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        message_id: "m-1".to_string(),
        model: sample_model_ref(),
        content: vec![AssistantContent::Text(TextContent {
            text: "hello".to_string(),
        })],
        usage: TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            total_tokens: None,
            cost_usd: None,
        },
        stop_reason,
    }
}

fn sample_assistant_with_tool_calls(tool_call_ids: &[&str]) -> ProviderInputMessage {
    let content = tool_call_ids
        .iter()
        .map(|id| {
            AssistantContent::ToolCall(ToolCall {
                id: id.to_string(),
                name: "test_tool".to_string(),
                arguments: serde_json::json!({}),
            })
        })
        .collect();
    ProviderInputMessage::Assistant {
        message: AssistantMessage {
            message_id: "m-1".to_string(),
            model: sample_model_ref(),
            content,
            usage: TokenUsage::default(),
            stop_reason: StopReason::ToolUse,
        },
    }
}

#[test]
fn api_kind_roundtrip_strings() {
    let all = [
        ApiKind::OpenAiCompletions,
        ApiKind::OpenAiResponses,
        ApiKind::OpenAiCodexResponses,
        ApiKind::AzureOpenAiResponses,
        ApiKind::AnthropicMessages,
        ApiKind::BedrockConverseStream,
        ApiKind::GoogleGenerativeAi,
        ApiKind::GoogleGeminiCli,
        ApiKind::GoogleVertex,
        ApiKind::CursorAgent,
        ApiKind::MiniMaxMessages,
    ];

    for kind in all {
        let as_string = kind.to_string();
        let parsed: ApiKind = as_string.parse().expect("must parse");
        assert_eq!(parsed, kind);
    }
}

#[test]
fn api_kind_parse_unknown_fails() {
    let err = "not-real".parse::<ApiKind>().expect_err("must fail");
    assert_eq!(err.value, "not-real");
    assert_eq!(err.to_string(), "unknown api kind: not-real");
}

#[test]
fn stop_reason_json_roundtrip() {
    let all = [
        StopReason::Stop,
        StopReason::Length,
        StopReason::ToolUse,
        StopReason::Error,
        StopReason::Aborted,
    ];

    for reason in all {
        let json = serde_json::to_string(&reason).expect("serialize reason");
        let back: StopReason = serde_json::from_str(&json).expect("deserialize reason");
        assert_eq!(back, reason);
    }
}

#[test]
fn token_usage_computed_total_prefers_explicit_total() {
    let usage = TokenUsage {
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_write_tokens: 4,
        total_tokens: Some(99),
        cost_usd: None,
    };

    assert_eq!(usage.computed_total_tokens(), 99);
}

#[test]
fn token_usage_computed_total_falls_back_to_sum() {
    let usage = TokenUsage {
        input_tokens: 1,
        output_tokens: 2,
        cache_read_tokens: 3,
        cache_write_tokens: 4,
        total_tokens: None,
        cost_usd: None,
    };

    assert_eq!(usage.computed_total_tokens(), 10);
}

#[test]
fn token_usage_has_any_usage_detects_cost() {
    let usage = TokenUsage {
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        total_tokens: None,
        cost_usd: Some(0.001),
    };

    assert!(usage.has_any_usage());
}

#[test]
fn token_usage_has_any_usage_detects_none() {
    let usage = TokenUsage::default();
    assert!(!usage.has_any_usage());
}

#[test]
fn assistant_content_serializes_with_tagged_union() {
    let content = AssistantContent::ToolCall(ToolCall {
        id: "tc-1".to_string(),
        name: "grep".to_string(),
        arguments: serde_json::json!({"path":"src"}),
    });

    let json = serde_json::to_value(content).expect("serialize content");
    assert_eq!(json["type"], "tool_call");
}

#[test]
fn assistant_event_sequence_number_is_extracted_for_each_variant() {
    let cases = vec![
        AssistantMessageEvent::Start(StreamStartEvent {
            sequence_no: 1,
            message_id: "m".to_string(),
            model: sample_model_ref(),
        }),
        AssistantMessageEvent::TextStart(StreamBoundaryEvent {
            sequence_no: 2,
            block_id: "b".to_string(),
        }),
        AssistantMessageEvent::TextDelta(StreamTextDelta {
            sequence_no: 3,
            block_id: "b".to_string(),
            delta: "a".to_string(),
        }),
        AssistantMessageEvent::TextEnd(StreamBoundaryEvent {
            sequence_no: 4,
            block_id: "b".to_string(),
        }),
        AssistantMessageEvent::ThinkingStart(StreamBoundaryEvent {
            sequence_no: 5,
            block_id: "t".to_string(),
        }),
        AssistantMessageEvent::ThinkingDelta(StreamThinkingDelta {
            sequence_no: 6,
            block_id: "t".to_string(),
            delta: "b".to_string(),
        }),
        AssistantMessageEvent::ThinkingEnd(StreamBoundaryEvent {
            sequence_no: 7,
            block_id: "t".to_string(),
        }),
        AssistantMessageEvent::ToolCallStart(StreamBoundaryEvent {
            sequence_no: 8,
            block_id: "tc".to_string(),
        }),
        AssistantMessageEvent::ToolCallDelta(StreamToolCallDelta {
            sequence_no: 9,
            block_id: "tc".to_string(),
            delta: "{".to_string(),
        }),
        AssistantMessageEvent::ToolCallEnd(StreamBoundaryEvent {
            sequence_no: 10,
            block_id: "tc".to_string(),
        }),
        AssistantMessageEvent::Done(StreamDoneEvent {
            sequence_no: 11,
            message: sample_message(StopReason::Stop),
        }),
        AssistantMessageEvent::Error(StreamErrorEvent {
            sequence_no: 12,
            code: "transport".to_string(),
            message: "broken".to_string(),
            retryable: true,
        }),
    ];

    let observed: Vec<u64> = cases
        .iter()
        .map(AssistantMessageEvent::sequence_no)
        .collect();
    assert_eq!(observed, (1..=12).collect::<Vec<_>>());
}

#[test]
fn assistant_event_terminal_detection_is_precise() {
    let done = AssistantMessageEvent::Done(StreamDoneEvent {
        sequence_no: 1,
        message: sample_message(StopReason::Stop),
    });
    let err = AssistantMessageEvent::Error(StreamErrorEvent {
        sequence_no: 2,
        code: "x".to_string(),
        message: "y".to_string(),
        retryable: false,
    });
    let delta = AssistantMessageEvent::TextDelta(StreamTextDelta {
        sequence_no: 3,
        block_id: "b".to_string(),
        delta: "z".to_string(),
    });

    assert!(done.is_terminal());
    assert!(err.is_terminal());
    assert!(!delta.is_terminal());
}

#[test]
fn assistant_event_stop_reason_uses_done_message_reason() {
    let event = AssistantMessageEvent::Done(StreamDoneEvent {
        sequence_no: 1,
        message: sample_message(StopReason::ToolUse),
    });

    assert_eq!(event.stop_reason(), Some(StopReason::ToolUse));
}

#[test]
fn assistant_event_stop_reason_maps_error_to_error() {
    let event = AssistantMessageEvent::Error(StreamErrorEvent {
        sequence_no: 1,
        code: "auth".to_string(),
        message: "bad key".to_string(),
        retryable: false,
    });

    assert_eq!(event.stop_reason(), Some(StopReason::Error));
}

#[test]
fn assistant_event_stop_reason_non_terminal_is_none() {
    let event = AssistantMessageEvent::TextStart(StreamBoundaryEvent {
        sequence_no: 1,
        block_id: "b1".to_string(),
    });

    assert_eq!(event.stop_reason(), None);
}

#[test]
fn provider_error_display_is_stable() {
    let err = ProviderError::Transport {
        message: "timeout".to_string(),
    };
    assert_eq!(err.to_string(), "transport failure: timeout");
}

#[test]
fn provider_error_serialization_uses_kind_tag() {
    let err = ProviderError::RateLimited {
        message: "quota".to_string(),
    };
    let json = serde_json::to_value(err).expect("serialize provider error");
    assert_eq!(json["kind"], "rate_limited");
}

#[test]
fn stream_error_event_roundtrip_json() {
    let event = AssistantMessageEvent::Error(StreamErrorEvent {
        sequence_no: 44,
        code: "rate_limited".to_string(),
        message: "try later".to_string(),
        retryable: true,
    });

    let json = serde_json::to_string(&event).expect("serialize event");
    let back: AssistantMessageEvent = serde_json::from_str(&json).expect("deserialize event");
    assert_eq!(back, event);
}

#[test]
fn assistant_message_roundtrip_json() {
    let message = sample_message(StopReason::Stop);
    let json = serde_json::to_string(&message).expect("serialize message");
    let back: AssistantMessage = serde_json::from_str(&json).expect("deserialize message");
    assert_eq!(back, message);
}

#[test]
fn provider_context_supports_optional_api_key() {
    let ctx = ProviderContext {
        api_key: None,
        timeout_ms: 30_000,
    };

    let json = serde_json::to_string(&ctx).expect("serialize context");
    assert!(json.contains("timeout_ms"));
}

#[test]
fn model_ref_roundtrip_json() {
    let model = sample_model_ref();
    let json = serde_json::to_string(&model).expect("serialize model ref");
    let back: ModelRef = serde_json::from_str(&json).expect("deserialize model ref");
    assert_eq!(back, model);
}

#[test]
fn assistant_message_event_json_contains_type_discriminator() {
    let event = AssistantMessageEvent::TextDelta(StreamTextDelta {
        sequence_no: 3,
        block_id: "b".to_string(),
        delta: "abc".to_string(),
    });

    let json = serde_json::to_value(event).expect("serialize event");
    assert_eq!(json["type"], "text_delta");
}

#[test]
fn computed_total_tokens_handles_zero_explicit_total() {
    let usage = TokenUsage {
        input_tokens: 10,
        output_tokens: 5,
        cache_read_tokens: 1,
        cache_write_tokens: 1,
        total_tokens: Some(0),
        cost_usd: None,
    };

    assert_eq!(usage.computed_total_tokens(), 0);
}

#[test]
fn usage_has_any_usage_when_total_tokens_set() {
    let usage = TokenUsage {
        total_tokens: Some(123),
        ..TokenUsage::default()
    };

    assert!(usage.has_any_usage());
}

#[test]
fn assistant_message_can_hold_multiple_content_blocks() {
    let message = AssistantMessage {
        message_id: "m-2".to_string(),
        model: sample_model_ref(),
        content: vec![
            AssistantContent::Thinking(ThinkingContent {
                text: "hmm".to_string(),
            }),
            AssistantContent::ToolCall(ToolCall {
                id: "tc".to_string(),
                name: "read".to_string(),
                arguments: serde_json::json!({ "path": "src/lib.rs" }),
            }),
        ],
        usage: TokenUsage::default(),
        stop_reason: StopReason::ToolUse,
    };

    assert_eq!(message.content.len(), 2);
}

#[test]
fn provider_transport_details_roundtrip_json() {
    let details = ProviderTransportDetails {
        transport: "websocket".to_string(),
        reused_provider_session: true,
    };

    let json = serde_json::to_string(&details).expect("serialize details");
    let back: ProviderTransportDetails =
        serde_json::from_str(&json).expect("deserialize details");
    assert_eq!(back, details);
}

#[test]
fn provider_final_roundtrip_json() {
    let final_msg = ProviderFinal {
        message: sample_message(StopReason::Stop),
        transport_details: Some(ProviderTransportDetails {
            transport: "sse".to_string(),
            reused_provider_session: false,
        }),
    };

    let json = serde_json::to_string(&final_msg).expect("serialize final");
    let back: ProviderFinal = serde_json::from_str(&json).expect("deserialize final");
    assert_eq!(back, final_msg);
}

#[test]
fn provider_request_roundtrip_json() {
    let req = ProviderRequest {
        session_id: "s-1".to_string(),
        model: sample_model_ref(),
        system_prompt: Some("You are a helpful assistant.".to_string()),
        input: vec![ProviderInputMessage::User {
            content: "hello".to_string(),
        }],
        tools: vec![],
        tool_choice: None,
    };

    let json = serde_json::to_string(&req).expect("serialize request");
    let back: ProviderRequest = serde_json::from_str(&json).expect("deserialize request");
    assert_eq!(back, req);
}

#[test]
fn patch_orphaned_injects_for_unmatched_ids() {
    let mut messages = vec![
        ProviderInputMessage::User {
            content: "hi".to_string(),
        },
        sample_assistant_with_tool_calls(&["tc-1"]),
    ];

    patch_orphaned_tool_calls(&mut messages, "not available");

    assert_eq!(messages.len(), 3);
    match &messages[2] {
        ProviderInputMessage::ToolResult {
            tool_call_id,
            is_error,
            result,
        } => {
            assert_eq!(tool_call_id, "tc-1");
            assert!(is_error);
            assert_eq!(result, &Value::String("not available".to_string()));
        }
        other => panic!("expected ToolResult, got {:?}", other),
    }
}

#[test]
fn patch_orphaned_no_op_when_all_matched() {
    let mut messages = vec![
        ProviderInputMessage::User {
            content: "hi".to_string(),
        },
        sample_assistant_with_tool_calls(&["tc-1"]),
        ProviderInputMessage::ToolResult {
            tool_call_id: "tc-1".to_string(),
            is_error: false,
            result: serde_json::json!("ok"),
        },
    ];

    let original_len = messages.len();
    patch_orphaned_tool_calls(&mut messages, "not available");
    assert_eq!(messages.len(), original_len);
}

#[test]
fn patch_orphaned_handles_multiple_tool_calls_per_message() {
    let mut messages = vec![
        ProviderInputMessage::User {
            content: "hi".to_string(),
        },
        sample_assistant_with_tool_calls(&["tc-1", "tc-2", "tc-3"]),
    ];

    patch_orphaned_tool_calls(&mut messages, "unavailable");

    assert_eq!(messages.len(), 5);
    for (offset, expected_id) in [(2, "tc-1"), (3, "tc-2"), (4, "tc-3")] {
        match &messages[offset] {
            ProviderInputMessage::ToolResult {
                tool_call_id,
                is_error,
                ..
            } => {
                assert_eq!(tool_call_id, expected_id);
                assert!(is_error);
            }
            other => panic!("expected ToolResult at {}, got {:?}", offset, other),
        }
    }
}

#[test]
fn patch_orphaned_preserves_existing_results() {
    let mut messages = vec![
        ProviderInputMessage::User {
            content: "hi".to_string(),
        },
        sample_assistant_with_tool_calls(&["tc-1", "tc-2"]),
        ProviderInputMessage::ToolResult {
            tool_call_id: "tc-1".to_string(),
            is_error: false,
            result: serde_json::json!("real result"),
        },
    ];

    patch_orphaned_tool_calls(&mut messages, "synthetic");

    // tc-1 already matched, only tc-2 should be injected
    assert_eq!(messages.len(), 4);
    match &messages[2] {
        ProviderInputMessage::ToolResult {
            tool_call_id,
            result,
            ..
        } => {
            assert_eq!(tool_call_id, "tc-2");
            assert_eq!(result, &Value::String("synthetic".to_string()));
        }
        other => panic!("expected synthetic ToolResult, got {:?}", other),
    }
    // Original result should still be there
    match &messages[3] {
        ProviderInputMessage::ToolResult {
            tool_call_id,
            result,
            ..
        } => {
            assert_eq!(tool_call_id, "tc-1");
            assert_eq!(result, &serde_json::json!("real result"));
        }
        other => panic!("expected original ToolResult, got {:?}", other),
    }
}
