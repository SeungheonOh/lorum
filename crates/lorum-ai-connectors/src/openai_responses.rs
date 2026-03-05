use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use lorum_ai_contract::{
    AssistantContent, AssistantEventSink, AssistantMessage, AssistantMessageEvent, ModelRef,
    ProviderAdapter, ProviderError, ProviderFinal, ProviderTransportDetails, StopReason,
    StreamBoundaryEvent, StreamDoneEvent, StreamErrorEvent, StreamStartEvent, StreamTextDelta,
    StreamThinkingDelta, StreamToolCallDelta, TextContent, ThinkingContent, TokenUsage, ToolCall,
};
use serde_json::Value;

use crate::interfaces::{FrameSink, OpenAiResponsesFrame, OpenAiResponsesTransport, RetryPolicy};
use crate::shared::{
    normalize_provider_error, sink_to_provider_error, CollectingSink, ToolCallJsonAccumulator,
};

pub struct OpenAiResponsesAdapter {
    transport: Arc<dyn OpenAiResponsesTransport>,
    retry_policy: RetryPolicy,
    provider_id: String,
}

impl OpenAiResponsesAdapter {
    pub fn new(transport: Arc<dyn OpenAiResponsesTransport>) -> Self {
        Self {
            transport,
            retry_policy: RetryPolicy::default(),
            provider_id: "openai".to_string(),
        }
    }

    pub fn with_retry_policy(mut self, retry_policy: RetryPolicy) -> Self {
        self.retry_policy = retry_policy;
        self
    }
}

#[async_trait]
impl ProviderAdapter for OpenAiResponsesAdapter {
    fn provider_id(&self) -> &str {
        &self.provider_id
    }

    fn api_kind(&self) -> lorum_ai_contract::ApiKind {
        lorum_ai_contract::ApiKind::OpenAiResponses
    }

    async fn stream(
        &self,
        request: lorum_ai_contract::ProviderRequest,
        context: lorum_ai_contract::ProviderContext,
        sink: &mut dyn AssistantEventSink,
    ) -> Result<ProviderFinal, ProviderError> {
        let mut last_error = None;

        for attempt in 1..=self.retry_policy.max_attempts {
            let mut processor = OpenAiFrameProcessor::new(sink, &request.model);
            match self
                .transport
                .stream_frames(&request, &context, &mut processor)
                .await
            {
                Ok(()) => {
                    return processor.finalize("sse", false);
                }
                Err(err) => {
                    if processor.has_received_frames()
                        || !self.retry_policy.should_retry(attempt, &err)
                    {
                        return Err(err);
                    }
                    last_error = Some(err);
                    continue;
                }
            }
        }

        Err(last_error.unwrap_or_else(|| ProviderError::Transport {
            message: "unknown transport failure".to_string(),
        }))
    }

    async fn complete(
        &self,
        request: lorum_ai_contract::ProviderRequest,
        context: lorum_ai_contract::ProviderContext,
    ) -> Result<AssistantMessage, ProviderError> {
        let mut collector = CollectingSink::default();
        let final_msg = self.stream(request, context, &mut collector).await?;
        Ok(final_msg.message)
    }
}

// ---------------------------------------------------------------------------
// Incremental frame processor: converts OpenAiResponsesFrame → events
// ---------------------------------------------------------------------------

pub(crate) struct OpenAiFrameProcessor<'a> {
    sink: &'a mut dyn AssistantEventSink,
    model: ModelRef,
    sequence_no: u64,
    message_id: String,
    has_frames: bool,

    text_blocks: HashMap<String, String>,
    reasoning_blocks: HashMap<String, String>,
    call_accumulators: HashMap<String, ToolCallJsonAccumulator>,
    call_metadata: HashMap<String, (String, String)>,

    content: Vec<AssistantContent>,
    stop_reason: StopReason,
    usage: TokenUsage,
}

impl<'a> OpenAiFrameProcessor<'a> {
    pub fn new(sink: &'a mut dyn AssistantEventSink, model: &ModelRef) -> Self {
        Self {
            sink,
            model: model.clone(),
            sequence_no: 0,
            message_id: String::new(),
            has_frames: false,
            text_blocks: HashMap::new(),
            reasoning_blocks: HashMap::new(),
            call_accumulators: HashMap::new(),
            call_metadata: HashMap::new(),
            content: Vec::new(),
            stop_reason: StopReason::Error,
            usage: TokenUsage::default(),
        }
    }

    pub fn has_received_frames(&self) -> bool {
        self.has_frames
    }

    pub fn finalize(
        mut self,
        transport: &str,
        reused_provider_session: bool,
    ) -> Result<ProviderFinal, ProviderError> {
        self.sequence_no += 1;
        let message = AssistantMessage {
            message_id: self.message_id,
            model: self.model,
            content: self.content,
            usage: self.usage,
            stop_reason: self.stop_reason,
        };

        self.sink
            .push(AssistantMessageEvent::Done(StreamDoneEvent {
                sequence_no: self.sequence_no,
                message: message.clone(),
            }))
            .map_err(sink_to_provider_error)?;

        Ok(ProviderFinal {
            message,
            transport_details: Some(ProviderTransportDetails {
                transport: transport.to_string(),
                reused_provider_session,
            }),
        })
    }
}

impl<'a> FrameSink<OpenAiResponsesFrame> for OpenAiFrameProcessor<'a> {
    fn push_frame(&mut self, frame: OpenAiResponsesFrame) -> Result<(), ProviderError> {
        self.has_frames = true;
        self.sequence_no += 1;
        let seq = self.sequence_no;

        match frame {
            OpenAiResponsesFrame::ResponseStart { message_id } => {
                self.message_id = message_id.clone();
                self.sink
                    .push(AssistantMessageEvent::Start(StreamStartEvent {
                        sequence_no: seq,
                        message_id,
                        model: self.model.clone(),
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::TextStart { block_id } => {
                self.text_blocks.insert(block_id.clone(), String::new());
                self.sink
                    .push(AssistantMessageEvent::TextStart(StreamBoundaryEvent {
                        sequence_no: seq,
                        block_id,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::TextDelta { block_id, delta } => {
                self.text_blocks
                    .entry(block_id.clone())
                    .or_default()
                    .push_str(&delta);
                self.sink
                    .push(AssistantMessageEvent::TextDelta(StreamTextDelta {
                        sequence_no: seq,
                        block_id,
                        delta,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::TextEnd { block_id } => {
                if let Some(text) = self.text_blocks.remove(&block_id) {
                    self.content.push(AssistantContent::Text(TextContent { text }));
                }
                self.sink
                    .push(AssistantMessageEvent::TextEnd(StreamBoundaryEvent {
                        sequence_no: seq,
                        block_id,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::ReasoningStart { block_id } => {
                self.reasoning_blocks
                    .insert(block_id.clone(), String::new());
                self.sink
                    .push(AssistantMessageEvent::ThinkingStart(StreamBoundaryEvent {
                        sequence_no: seq,
                        block_id,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::ReasoningDelta { block_id, delta } => {
                self.reasoning_blocks
                    .entry(block_id.clone())
                    .or_default()
                    .push_str(&delta);
                self.sink
                    .push(AssistantMessageEvent::ThinkingDelta(StreamThinkingDelta {
                        sequence_no: seq,
                        block_id,
                        delta,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::ReasoningEnd { block_id } => {
                if let Some(text) = self.reasoning_blocks.remove(&block_id) {
                    self.content
                        .push(AssistantContent::Thinking(ThinkingContent { text }));
                }
                self.sink
                    .push(AssistantMessageEvent::ThinkingEnd(StreamBoundaryEvent {
                        sequence_no: seq,
                        block_id,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::FunctionCallStart {
                block_id,
                call_id,
                name,
            } => {
                self.call_accumulators
                    .insert(block_id.clone(), ToolCallJsonAccumulator::default());
                self.call_metadata
                    .insert(block_id.clone(), (call_id, name));
                self.sink
                    .push(AssistantMessageEvent::ToolCallStart(StreamBoundaryEvent {
                        sequence_no: seq,
                        block_id,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::FunctionCallDelta { block_id, delta } => {
                self.call_accumulators
                    .entry(block_id.clone())
                    .or_default()
                    .push_chunk(&delta);
                self.sink
                    .push(AssistantMessageEvent::ToolCallDelta(StreamToolCallDelta {
                        sequence_no: seq,
                        block_id,
                        delta,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::FunctionCallEnd { block_id } => {
                let args = self
                    .call_accumulators
                    .remove(&block_id)
                    .map(|acc| acc.finalize())
                    .unwrap_or(Value::Null);
                if let Some((call_id, name)) = self.call_metadata.remove(&block_id) {
                    self.content.push(AssistantContent::ToolCall(ToolCall {
                        id: call_id,
                        name,
                        arguments: args,
                    }));
                }
                self.sink
                    .push(AssistantMessageEvent::ToolCallEnd(StreamBoundaryEvent {
                        sequence_no: seq,
                        block_id,
                    }))
                    .map_err(sink_to_provider_error)?;
            }
            OpenAiResponsesFrame::Completed {
                stop_reason,
                usage,
            } => {
                self.stop_reason = stop_reason;
                self.usage = usage;
            }
            OpenAiResponsesFrame::Error {
                code,
                message,
                retryable,
            } => {
                let error = normalize_provider_error(&code, &message, retryable);
                self.sink
                    .push(AssistantMessageEvent::Error(StreamErrorEvent {
                        sequence_no: seq,
                        code,
                        message,
                        retryable,
                    }))
                    .map_err(sink_to_provider_error)?;
                return Err(error);
            }
        }

        Ok(())
    }
}
