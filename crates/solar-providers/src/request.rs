use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use rig_core::OneOrMany;
use rig_core::completion::{AssistantContent, CompletionRequest, Message, ToolDefinition};
use rig_core::message::{
    DocumentSourceKind, Image, ImageMediaType, MimeType, Reasoning, ToolResultContent, UserContent,
};
use solar_agent_core::{
    AgentError, AgentErrorKind, ContentPart, ConversationMessage, MessageContent, MessageRole,
    TurnRequest,
};

pub(crate) fn build_completion_request(
    request: TurnRequest,
    configured_max_output_tokens: Option<u64>,
    supports_system_messages: bool,
    encode_images_as_base64: bool,
    additional_params: Option<serde_json::Value>,
) -> Result<CompletionRequest, AgentError> {
    let mut messages = Vec::new();
    if let Some(system_prompt) = request.system_prompt {
        if supports_system_messages {
            messages.push(Message::System {
                content: system_prompt,
            });
        } else {
            messages.push(Message::User {
                content: OneOrMany::one(UserContent::text(system_prompt)),
            });
        }
    }
    for message in request.messages {
        messages.push(convert_message(message, encode_images_as_base64)?);
    }

    let chat_history = OneOrMany::many(messages).map_err(|_| {
        AgentError::new(
            AgentErrorKind::InvalidRequest,
            "a Rig turn requires at least one conversation message",
        )
    })?;

    Ok(CompletionRequest {
        model: Some(request.model.as_str().to_string()),
        preamble: None,
        chat_history,
        documents: Vec::new(),
        tools: request
            .tools
            .into_iter()
            .map(|tool| ToolDefinition {
                name: tool.name,
                description: tool.description,
                parameters: tool.input_schema,
            })
            .collect(),
        temperature: None,
        max_tokens: request.max_output_tokens.or(configured_max_output_tokens),
        tool_choice: None,
        additional_params,
        output_schema: None,
        record_telemetry_content: false,
    })
}

fn convert_message(
    message: ConversationMessage,
    encode_images_as_base64: bool,
) -> Result<Message, AgentError> {
    match message.role {
        MessageRole::User => Ok(Message::User {
            content: user_content(message.content, encode_images_as_base64)?,
        }),
        MessageRole::Assistant => Ok(Message::Assistant {
            id: None,
            content: assistant_content(message.content, encode_images_as_base64)?,
        }),
    }
}

fn user_content(
    content: MessageContent,
    encode_images_as_base64: bool,
) -> Result<OneOrMany<UserContent>, AgentError> {
    let parts = match content {
        MessageContent::Text(text) => vec![UserContent::text(text)],
        MessageContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => vec![UserContent::tool_result_with_call_id(
            tool_use_id.clone(),
            tool_use_id,
            OneOrMany::one(ToolResultContent::text(tool_result_text(content, is_error))),
        )],
        MessageContent::MultiPart(parts) => parts
            .into_iter()
            .map(|part| convert_user_part(part, encode_images_as_base64))
            .collect::<Result<Vec<_>, _>>()?,
        MessageContent::ToolUse { .. } => {
            return Err(invalid_role("tool use", "user"));
        }
    };
    one_or_many(parts, "user")
}

fn assistant_content(
    content: MessageContent,
    encode_images_as_base64: bool,
) -> Result<OneOrMany<AssistantContent>, AgentError> {
    let parts = match content {
        MessageContent::Text(text) => vec![AssistantContent::text(text)],
        MessageContent::ToolUse {
            tool_use_id,
            name,
            input,
        } => vec![AssistantContent::tool_call_with_call_id(
            tool_use_id.clone(),
            tool_use_id,
            name,
            input,
        )],
        MessageContent::MultiPart(parts) => parts
            .into_iter()
            .map(|part| convert_assistant_part(part, encode_images_as_base64))
            .collect::<Result<Vec<_>, _>>()?,
        MessageContent::ToolResult { .. } => {
            return Err(invalid_role("tool result", "assistant"));
        }
    };
    one_or_many(parts, "assistant")
}

fn convert_user_part(
    part: ContentPart,
    encode_images_as_base64: bool,
) -> Result<UserContent, AgentError> {
    match part {
        ContentPart::Text(text) => Ok(UserContent::text(text)),
        ContentPart::Reasoning { .. } => Err(invalid_role("reasoning", "user")),
        ContentPart::Image { data, mime_type } => {
            let media_type = ImageMediaType::from_mime_type(&mime_type);
            if encode_images_as_base64 {
                Ok(UserContent::image_base64(
                    BASE64_STANDARD.encode(data),
                    media_type,
                    None,
                ))
            } else {
                Ok(UserContent::image_raw(data, media_type, None))
            }
        }
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Ok(UserContent::tool_result_with_call_id(
            tool_use_id.clone(),
            tool_use_id,
            OneOrMany::one(ToolResultContent::text(tool_result_text(content, is_error))),
        )),
        ContentPart::ToolUse { .. } => Err(invalid_role("tool use", "user")),
    }
}

fn convert_assistant_part(
    part: ContentPart,
    encode_images_as_base64: bool,
) -> Result<AssistantContent, AgentError> {
    match part {
        ContentPart::Text(text) => Ok(AssistantContent::text(text)),
        ContentPart::Reasoning { text, signature } => Ok(AssistantContent::Reasoning(
            Reasoning::new_with_signature(&text, signature),
        )),
        ContentPart::Image { data, mime_type } => Ok(AssistantContent::Image(Image {
            data: if encode_images_as_base64 {
                DocumentSourceKind::Base64(BASE64_STANDARD.encode(data))
            } else {
                DocumentSourceKind::Raw(data)
            },
            media_type: ImageMediaType::from_mime_type(&mime_type),
            detail: None,
            additional_params: None,
        })),
        ContentPart::ToolUse {
            tool_use_id,
            name,
            input,
        } => Ok(AssistantContent::tool_call_with_call_id(
            tool_use_id.clone(),
            tool_use_id,
            name,
            input,
        )),
        ContentPart::ToolResult { .. } => Err(invalid_role("tool result", "assistant")),
    }
}

fn tool_result_text(content: String, is_error: bool) -> String {
    // Rig core does not yet carry Bedrock's optional ToolResultStatus. Keep
    // Solar's structured error state in the domain model and make the error
    // semantic explicit in the provider-visible result text.
    if is_error {
        format!("[ERROR] {content}")
    } else {
        content
    }
}

fn one_or_many<T: Clone>(parts: Vec<T>, role: &str) -> Result<OneOrMany<T>, AgentError> {
    OneOrMany::many(parts).map_err(|_| {
        AgentError::new(
            AgentErrorKind::InvalidRequest,
            format!("{role} message has no content"),
        )
    })
}

fn invalid_role(content: &str, role: &str) -> AgentError {
    AgentError::new(
        AgentErrorKind::InvalidRequest,
        format!("{content} content cannot appear in a {role} message"),
    )
}
