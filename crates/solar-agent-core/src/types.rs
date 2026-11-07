use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub const MAX_TOOL_RESULT_CHARS_FOR_PROVIDER_REQUEST: usize = 64_000;
const MIN_PROVIDER_OUTPUT_DISCLOSURE_CHARS: usize = 27;
const ESTIMATED_CHARS_PER_TOKEN: usize = 3;
const MAX_SINGLE_OUTPUT_CONTEXT_FRACTION: usize = 4;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: MessageRole,
    pub content: MessageContent,
}

impl ConversationMessage {
    pub fn truncate_tool_results_for_provider_request(&mut self) {
        self.truncate_tool_results_for_provider_request_with_context(None);
    }

    pub fn truncate_tool_results_for_provider_request_with_context(
        &mut self,
        max_context_tokens: Option<u32>,
    ) {
        truncate_tool_results_in_content(
            &mut self.content,
            provider_output_char_limit(max_context_tokens),
        );
    }

    pub fn truncate_tagged_terminal_output_for_provider_request_with_context(
        &mut self,
        max_context_tokens: Option<u32>,
    ) {
        truncate_tagged_terminal_output_in_content(&mut self.content, max_context_tokens);
    }
}

pub fn provider_output_char_limit(max_context_tokens: Option<u32>) -> usize {
    let context_aware_limit = max_context_tokens
        .and_then(|tokens| usize::try_from(tokens).ok())
        .map(|tokens| {
            tokens
                .saturating_mul(ESTIMATED_CHARS_PER_TOKEN)
                .saturating_div(MAX_SINGLE_OUTPUT_CONTEXT_FRACTION)
        })
        .unwrap_or(MAX_TOOL_RESULT_CHARS_FOR_PROVIDER_REQUEST);
    context_aware_limit.clamp(
        MIN_PROVIDER_OUTPUT_DISCLOSURE_CHARS,
        MAX_TOOL_RESULT_CHARS_FOR_PROVIDER_REQUEST,
    )
}

pub fn bounded_terminal_output_for_provider_request(
    output: &str,
    max_context_tokens: Option<u32>,
) -> String {
    truncate_provider_text(
        output,
        provider_output_char_limit(max_context_tokens),
        "terminal output truncated for model context; full output remains available in the terminal",
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
    ToolUse {
        tool_use_id: String,
        name: String,
        input: JsonValue,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    MultiPart(Vec<ContentPart>),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ContentPart {
    Text(String),
    Reasoning {
        text: String,
        signature: Option<String>,
    },
    Image {
        data: Vec<u8>,
        mime_type: String,
    },
    ToolUse {
        tool_use_id: String,
        name: String,
        input: JsonValue,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModelId(String);

impl ModelId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ModelId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ModelId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TurnRequest {
    pub conversation_id: Option<String>,
    pub model: ModelId,
    pub system_prompt: Option<String>,
    /// Current runtime input when the runtime owns conversation history.
    ///
    /// Provider runtimes normally consume `messages`, while session-oriented
    /// runtimes such as ACP consume this single prompt and retain their own
    /// history. Keeping the distinction explicit prevents applications from
    /// serializing a prompt into provider history and translating it back.
    pub prompt: Option<MessageContent>,
    pub messages: Vec<ConversationMessage>,
    pub tools: Vec<ToolDefinition>,
    pub max_output_tokens: Option<u64>,
    pub metadata: BTreeMap<String, JsonValue>,
}

impl TurnRequest {
    pub fn new(model: impl Into<ModelId>, messages: Vec<ConversationMessage>) -> Self {
        Self {
            conversation_id: None,
            model: model.into(),
            system_prompt: None,
            prompt: None,
            messages,
            tools: Vec::new(),
            max_output_tokens: None,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_prompt(mut self, prompt: MessageContent) -> Self {
        self.prompt = Some(prompt);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: JsonValue,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub call_id: String,
    pub content: String,
    pub status: ToolResultStatus,
    /// Images returned by a tool, kept outside the text so provider adapters receive real images.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ToolResultImage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolResultImage {
    pub data: Vec<u8>,
    pub mime_type: String,
}

pub(crate) fn tool_image_group_call_id(part: &ContentPart) -> Option<&str> {
    let ContentPart::Text(text) = part else {
        return None;
    };
    text.strip_prefix("Images returned by tool call ")?
        .strip_suffix(':')
        .filter(|call_id| !call_id.is_empty())
}

impl ToolResult {
    pub fn into_batch_content_parts(results: Vec<Self>) -> Vec<ContentPart> {
        let mut parts = Vec::new();
        let mut image_parts = Vec::new();
        for result in results {
            let is_error = !matches!(result.status, ToolResultStatus::Success);
            if !result.images.is_empty() {
                image_parts.push(ContentPart::Text(format!(
                    "Images returned by tool call {}:",
                    result.call_id
                )));
                image_parts.extend(result.images.into_iter().map(|image| ContentPart::Image {
                    data: image.data,
                    mime_type: image.mime_type,
                }));
            }
            parts.push(ContentPart::ToolResult {
                tool_use_id: result.call_id,
                content: result.content,
                is_error,
            });
        }
        // OpenAI requires every tool result before the next user message. Its adapter splits
        // adjacent images into a user message, so never interleave them with a result batch.
        parts.extend(image_parts);
        parts
    }

    pub fn is_error(&self) -> bool {
        matches!(
            self.status,
            ToolResultStatus::Error | ToolResultStatus::Denied
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolResultStatus {
    Success,
    Error,
    Denied,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionKind {
    Read,
    Write,
    Execute,
    Network,
    ExternalTool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PermissionRequest {
    pub id: String,
    pub call_id: String,
    pub kind: PermissionKind,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PermissionDecision {
    AllowOnce,
    AlwaysAllow,
    Denied { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolEvent {
    Proposed {
        call: ToolCall,
    },
    PermissionRequested {
        request: PermissionRequest,
    },
    PermissionResolved {
        request_id: String,
        call_id: String,
        decision: PermissionDecision,
    },
    Started {
        call_id: String,
    },
    Completed {
        result: ToolResult,
    },
}

impl ToolEvent {
    pub fn call_id(&self) -> &str {
        match self {
            ToolEvent::Proposed { call } => &call.id,
            ToolEvent::PermissionRequested { request } => &request.call_id,
            ToolEvent::PermissionResolved { call_id, .. } | ToolEvent::Started { call_id } => {
                call_id
            }
            ToolEvent::Completed { result } => &result.call_id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    /// Input tokens not served by or written to a provider prompt cache.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Input tokens served from the provider prompt cache.
    pub cached_input_tokens: u64,
    /// Input tokens written to the provider prompt cache.
    pub cache_creation_input_tokens: u64,
}

impl Usage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.cached_input_tokens)
            .saturating_add(self.cache_creation_input_tokens)
            .saturating_add(self.output_tokens)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    Completed,
    Cancelled,
    MaxTokens,
    ContextWindowExceeded,
    Refusal,
    ToolLoopLimit,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeActivityStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
    Other(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeActivity {
    pub id: String,
    pub title: String,
    pub status: Option<RuntimeActivityStatus>,
    pub output: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum AgentEvent {
    TurnStarted {
        runtime_request_id: String,
    },
    /// A transport heartbeat proving that the current model stream is still connected.
    /// Consumers should use this to refresh idle timeouts without rendering output.
    KeepAlive,
    /// Progress received while the provider is streaming a tool call's JSON arguments.
    /// The arguments remain buffered by the provider runtime and are not executed until the
    /// complete tool call has been parsed and emitted as [`ToolEvent::Proposed`].
    ToolCallProgress {
        call_id: String,
        name: Option<String>,
        arguments_bytes: u64,
    },
    TextDelta {
        text: String,
    },
    ReasoningDelta {
        text: String,
    },
    ReasoningCompleted {
        text: String,
        signature: Option<String>,
    },
    Tool {
        event: ToolEvent,
    },
    UsageUpdated {
        usage: Usage,
    },
    /// Progress owned by a session runtime rather than a Solar-executed tool.
    RuntimeActivityUpdated {
        activity: RuntimeActivity,
    },
    /// Current context occupancy reported by a session runtime.
    ContextUsageUpdated {
        used_tokens: u64,
        context_size: u64,
    },
    /// Visible user input accepted while a turn was already running.
    UserInputAccepted {
        text: String,
    },
    /// A displayable runtime lifecycle or permission notice.
    RuntimeNotice {
        message: String,
    },
    TurnStopped {
        reason: StopReason,
    },
}

fn truncate_tagged_terminal_output_in_content(
    content: &mut MessageContent,
    max_context_tokens: Option<u32>,
) {
    match content {
        MessageContent::Text(text) => {
            *text = truncate_tagged_terminal_output(text, max_context_tokens);
        }
        MessageContent::MultiPart(parts) => {
            for part in parts {
                if let ContentPart::Text(text) = part {
                    *text = truncate_tagged_terminal_output(text, max_context_tokens);
                }
            }
        }
        MessageContent::ToolUse { .. } | MessageContent::ToolResult { .. } => {}
    }
}

fn truncate_tagged_terminal_output(content: &str, max_context_tokens: Option<u32>) -> String {
    const TAGS: [&str; 2] = ["[Terminal output:\n", "[Final terminal output:\n"];
    const END_TAG: &str = "\n]\n";

    let mut result = String::with_capacity(content.len());
    let mut remaining = content;
    loop {
        let Some((tag_offset, tag)) = TAGS
            .iter()
            .filter_map(|tag| remaining.find(tag).map(|offset| (offset, *tag)))
            .min_by_key(|(offset, _)| *offset)
        else {
            result.push_str(remaining);
            break;
        };
        let output_start = tag_offset + tag.len();
        result.push_str(&remaining[..output_start]);
        let after_tag = &remaining[output_start..];
        let mut output_ends = after_tag.match_indices(END_TAG);
        let Some((output_end, _)) = output_ends.next() else {
            result.push_str(&bounded_terminal_output_for_provider_request(
                after_tag,
                max_context_tokens,
            ));
            break;
        };
        if output_ends.next().is_some() {
            return truncate_provider_text(
                content,
                provider_output_char_limit(max_context_tokens),
                "message text truncated for model context; full content remains in conversation history",
            );
        }
        result.push_str(&bounded_terminal_output_for_provider_request(
            &after_tag[..output_end],
            max_context_tokens,
        ));
        result.push_str(END_TAG);
        remaining = &after_tag[output_end + END_TAG.len()..];
    }
    result
}

fn truncate_tool_results_in_content(content: &mut MessageContent, max_chars: usize) {
    match content {
        MessageContent::Text(_) | MessageContent::ToolUse { .. } => {}
        MessageContent::ToolResult { content, .. } => truncate_tool_result_text(content, max_chars),
        MessageContent::MultiPart(parts) => {
            for part in parts {
                match part {
                    ContentPart::ToolResult { content, .. } => {
                        truncate_tool_result_text(content, max_chars);
                    }
                    ContentPart::Text(_)
                    | ContentPart::Reasoning { .. }
                    | ContentPart::Image { .. }
                    | ContentPart::ToolUse { .. } => {}
                }
            }
        }
    }
}

fn truncate_tool_result_text(content: &mut String, max_chars: usize) {
    *content = truncate_provider_text(content, max_chars, "tool result truncated");
}

fn truncate_provider_text(content: &str, max_chars: usize, reason: &str) -> String {
    let char_count = content.chars().count();
    if char_count <= max_chars {
        return content.to_string();
    }

    let omitted_chars = char_count.saturating_sub(max_chars);
    let marker = format!("\n... [{reason}; omitted {omitted_chars} chars] ...\n");
    let marker_chars = marker.chars().count();
    if max_chars <= marker_chars {
        let short_reason = if reason.starts_with("terminal output") {
            "terminal output truncated"
        } else if reason.starts_with("message text") {
            "message text truncated"
        } else {
            "tool result truncated"
        };
        let short_marker = format!("[{short_reason}]");
        return short_marker.chars().take(max_chars).collect();
    }
    let retained_chars = max_chars.saturating_sub(marker_chars);
    let head_chars = retained_chars / 2;
    let tail_chars = retained_chars.saturating_sub(head_chars);
    let head: String = content.chars().take(head_chars).collect();
    let tail: String = content
        .chars()
        .rev()
        .take(tail_chars)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("{head}{marker}{tail}")
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
