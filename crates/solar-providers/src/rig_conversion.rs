//! Boundaries between Solar's UI/tool types and Rig's canonical conversation.
//!
//! Rig should retain the authoritative run history. Solar has a smaller message
//! vocabulary: a tool's provider item ID, extra provider metadata, and encrypted
//! reasoning cannot all be represented by `ConversationMessage`.

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use rig_core::OneOrMany;
use rig_core::completion::Usage as RigUsage;
use rig_core::message::{
    AssistantContent, DocumentSourceKind, Image, ImageMediaType, Message, MimeType,
    ReasoningContent, ToolCall as RigToolCall, ToolFunction, ToolResult as RigToolResult,
    ToolResultContent, UserContent,
};
use solar_agent_core::{
    AgentError, AgentErrorKind, ContentPart, ConversationMessage, MessageContent, MessageRole,
    ToolCall, ToolResult, ToolResultStatus, Usage,
};

/// Use the same provider-tested conversion as the existing subscription paths.
/// Raw image bytes are retained here; provider adapters choose their wire format.
pub fn solar_messages_to_rig(
    messages: Vec<ConversationMessage>,
) -> Result<Vec<Message>, AgentError> {
    messages
        .into_iter()
        .map(|message| crate::request::convert_message(message, false))
        .collect()
}

/// Convert representable Rig history for Solar's UI/persistence boundary.
///
/// Tool call correlation uses `call_id` when the provider separates it from its
/// item ID, matching Solar's established Responses API adapter. A Rig tool
/// result has no error flag: its model-visible text is preserved verbatim rather
/// than guessing that a literal `[ERROR]` prefix proves an execution failed.
pub fn rig_messages_to_solar(messages: &[Message]) -> Result<Vec<ConversationMessage>, AgentError> {
    messages
        .iter()
        .map(|message| match message {
            Message::System { .. } => Err(unsupported("system messages in conversation history")),
            Message::User { content } => {
                let mut parts = Vec::new();
                let mut images = Vec::new();
                for item in content.iter() {
                    match item {
                        UserContent::Text(text) => parts.push(ContentPart::Text(text.text.clone())),
                        UserContent::Image(image) => parts.push(image_part(image)?),
                        UserContent::ToolResult(result) => {
                            let call_id = correlation_id(&result.id, result.call_id.as_deref());
                            let mut text = Vec::new();
                            let mut result_images = Vec::new();
                            for output in result.content.iter() {
                                match output {
                                    ToolResultContent::Text(value) => text.push(value.text.clone()),
                                    ToolResultContent::Json { value } => {
                                        text.push(value.to_string())
                                    }
                                    ToolResultContent::Image(image) => {
                                        result_images.push(image_part(image)?)
                                    }
                                }
                            }
                            parts.push(ContentPart::ToolResult {
                                tool_use_id: call_id.to_owned(),
                                content: text.join("\n"),
                                is_error: false,
                            });
                            if !result_images.is_empty() {
                                images.push(ContentPart::Text(format!(
                                    "Images returned by tool call {call_id}:"
                                )));
                                images.extend(result_images);
                            }
                        }
                        UserContent::Audio(_) => return Err(unsupported("audio content")),
                        UserContent::Video(_) => return Err(unsupported("video content")),
                        UserContent::Document(_) => return Err(unsupported("document content")),
                    }
                }
                // Keep every tool result before attached user images, as required
                // by OpenAI's tool-result batch ordering.
                parts.extend(images);
                Ok(ConversationMessage {
                    role: MessageRole::User,
                    content: collapse(parts)?,
                })
            }
            Message::Assistant { content, .. } => {
                let mut parts = Vec::new();
                for item in content.iter() {
                    match item {
                        AssistantContent::Text(text) => {
                            parts.push(ContentPart::Text(text.text.clone()))
                        }
                        AssistantContent::ToolCall(call) => {
                            let call = rig_tool_call_to_solar(call);
                            parts.push(ContentPart::ToolUse {
                                tool_use_id: call.id,
                                name: call.name,
                                input: call.arguments,
                            });
                        }
                        AssistantContent::Image(image) => parts.push(image_part(image)?),
                        AssistantContent::Reasoning(reasoning) => {
                            for block in &reasoning.content {
                                match block {
                                    ReasoningContent::Text { text, signature } => {
                                        parts.push(ContentPart::Reasoning {
                                            text: text.clone(),
                                            signature: signature.clone(),
                                        });
                                    }
                                    ReasoningContent::Summary(text) => {
                                        parts.push(ContentPart::Reasoning {
                                            text: text.clone(),
                                            signature: None,
                                        })
                                    }
                                    // These are opaque provider payloads. Flattening
                                    // them to visible text would corrupt replay.
                                    _ => return Err(unsupported("opaque reasoning content")),
                                }
                            }
                        }
                    }
                }
                Ok(ConversationMessage {
                    role: MessageRole::Assistant,
                    content: collapse(parts)?,
                })
            }
        })
        .collect()
}

pub fn solar_tool_call_to_rig(call: &ToolCall) -> RigToolCall {
    RigToolCall::new(
        call.id.clone(),
        ToolFunction::new(call.name.clone(), call.arguments.clone()),
    )
    .with_call_id(call.id.clone())
}

pub fn rig_tool_call_to_solar(call: &RigToolCall) -> ToolCall {
    ToolCall {
        id: correlation_id(&call.id, call.call_id.as_deref()).to_owned(),
        name: call.function.name.clone(),
        arguments: call.function.arguments.clone(),
    }
}

/// A tool result sent back to Rig keeps image data attached to its call.
/// Error status is carried in model-visible feedback because Rig's message
/// representation has no separate execution-status field.
pub fn solar_tool_result_to_rig(result: ToolResult) -> Result<UserContent, AgentError> {
    let error = !matches!(result.status, ToolResultStatus::Success);
    let text = if error {
        format!("[ERROR] {}", result.content)
    } else {
        result.content
    };
    let mut content = vec![ToolResultContent::text(text)];
    for image in result.images {
        content.push(ToolResultContent::Image(Image {
            data: DocumentSourceKind::Raw(image.data),
            media_type: ImageMediaType::from_mime_type(&image.mime_type),
            detail: None,
            additional_params: None,
        }));
    }
    Ok(UserContent::ToolResult(RigToolResult {
        id: result.call_id.clone(),
        call_id: Some(result.call_id),
        content: OneOrMany::many(content).map_err(|_| unsupported("empty tool result"))?,
    }))
}

pub fn rig_usage_to_solar(usage: RigUsage) -> Usage {
    crate::stream::map_usage(usage)
}

pub fn solar_usage_to_rig(usage: &Usage) -> RigUsage {
    RigUsage {
        input_tokens: usage
            .input_tokens
            .saturating_add(usage.cached_input_tokens)
            .saturating_add(usage.cache_creation_input_tokens),
        output_tokens: usage.output_tokens,
        total_tokens: usage.total_tokens(),
        cached_input_tokens: usage.cached_input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
        tool_use_prompt_tokens: 0,
        reasoning_tokens: 0,
    }
}

fn correlation_id<'a>(id: &'a str, call_id: Option<&'a str>) -> &'a str {
    call_id.filter(|value| !value.is_empty()).unwrap_or(id)
}

fn image_part(image: &Image) -> Result<ContentPart, AgentError> {
    let data = match &image.data {
        DocumentSourceKind::Raw(bytes) => bytes.clone(),
        DocumentSourceKind::Base64(encoded) => BASE64_STANDARD
            .decode(encoded)
            .map_err(|_| unsupported("invalid base64 image data"))?,
        _ => return Err(unsupported("non-inline image source")),
    };
    let mime_type = image
        .media_type
        .as_ref()
        .map(MimeType::to_mime_type)
        .ok_or_else(|| unsupported("image without a media type"))?
        .to_owned();
    Ok(ContentPart::Image { data, mime_type })
}

fn collapse(mut parts: Vec<ContentPart>) -> Result<MessageContent, AgentError> {
    if parts.is_empty() {
        return Err(unsupported("empty message"));
    }
    if parts.len() != 1 {
        return Ok(MessageContent::MultiPart(parts));
    }
    Ok(match parts.pop().expect("one content part") {
        ContentPart::Text(text) => MessageContent::Text(text),
        ContentPart::ToolUse {
            tool_use_id,
            name,
            input,
        } => MessageContent::ToolUse {
            tool_use_id,
            name,
            input,
        },
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => MessageContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        },
        part => MessageContent::MultiPart(vec![part]),
    })
}

fn unsupported(content: &str) -> AgentError {
    AgentError::new(
        AgentErrorKind::InvalidRequest,
        format!("Solar cannot represent Rig {content}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use rig_core::message::Reasoning;
    use solar_agent_core::ToolResultImage;

    #[test]
    fn conversation_roundtrip_preserves_tool_correlation_reasoning_and_images() {
        let original = vec![
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("Inspect this".into()),
            },
            ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::MultiPart(vec![
                    ContentPart::Reasoning {
                        text: "Inspect the file".into(),
                        signature: Some("signature".into()),
                    },
                    ContentPart::Text("Checking".into()),
                    ContentPart::ToolUse {
                        tool_use_id: "call-123".into(),
                        name: "files_read".into(),
                        input: serde_json::json!({"path":"note.txt"}),
                    },
                ]),
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::MultiPart(vec![
                    ContentPart::ToolResult {
                        tool_use_id: "call-123".into(),
                        content: "File read".into(),
                        is_error: false,
                    },
                    ContentPart::Image {
                        data: vec![1, 2, 3],
                        mime_type: "image/png".into(),
                    },
                ]),
            },
        ];
        let rig = solar_messages_to_rig(original.clone()).unwrap();
        assert_eq!(rig_messages_to_solar(&rig).unwrap(), original);
    }

    #[test]
    fn responses_item_id_does_not_replace_the_tool_correlation_id() {
        let call = RigToolCall::new(
            "fc-provider-item".into(),
            ToolFunction::new("web_fetch".into(), serde_json::json!({})),
        )
        .with_call_id("call-correlation".into());
        let converted = rig_tool_call_to_solar(&call);
        assert_eq!(converted.id, "call-correlation");
        let restored = solar_tool_call_to_rig(&converted);
        assert_eq!(restored.call_id.as_deref(), Some("call-correlation"));
    }

    #[test]
    fn tool_images_and_error_feedback_survive_conversion() {
        let result = solar_tool_result_to_rig(ToolResult {
            call_id: "call-screen".into(),
            content: "Partial capture".into(),
            status: ToolResultStatus::Error,
            images: vec![ToolResultImage {
                data: vec![0, 1, 2],
                mime_type: "image/png".into(),
            }],
        })
        .unwrap();
        let messages = rig_messages_to_solar(&[Message::User {
            content: OneOrMany::one(result),
        }])
        .unwrap();
        assert_eq!(
            messages[0].content,
            MessageContent::MultiPart(vec![
                ContentPart::ToolResult {
                    tool_use_id: "call-screen".into(),
                    content: "[ERROR] Partial capture".into(),
                    is_error: false
                },
                ContentPart::Text("Images returned by tool call call-screen:".into()),
                ContentPart::Image {
                    data: vec![0, 1, 2],
                    mime_type: "image/png".into()
                },
            ])
        );
        let wire = solar_messages_to_rig(messages).unwrap();
        let Message::User { content } = &wire[0] else {
            panic!("expected user message")
        };
        let UserContent::ToolResult(result) = content.first() else {
            panic!("expected tool result")
        };
        assert_eq!(
            result.content.first().as_text(),
            Some("[ERROR] Partial capture")
        );
    }

    #[test]
    fn a_literal_error_prefix_is_never_interpreted_as_execution_status() {
        let message = Message::User {
            content: OneOrMany::one(UserContent::tool_result(
                "call",
                OneOrMany::one(ToolResultContent::text("[ERROR] quoted file contents")),
            )),
        };
        let converted = rig_messages_to_solar(&[message]).unwrap();
        assert_eq!(
            converted[0].content,
            MessageContent::ToolResult {
                tool_use_id: "call".into(),
                content: "[ERROR] quoted file contents".into(),
                is_error: false
            }
        );
    }

    #[test]
    fn tool_result_batches_keep_images_after_all_correlated_results() {
        let results: Vec<UserContent> = ["call-a", "call-b"]
            .into_iter()
            .map(|id| {
                solar_tool_result_to_rig(ToolResult {
                    call_id: id.into(),
                    content: id.into(),
                    status: ToolResultStatus::Success,
                    images: vec![ToolResultImage {
                        data: vec![42],
                        mime_type: "image/png".into(),
                    }],
                })
                .unwrap()
            })
            .collect();
        let converted = rig_messages_to_solar(&[Message::User {
            content: OneOrMany::many(results).unwrap(),
        }])
        .unwrap();
        let MessageContent::MultiPart(parts) = &converted[0].content else {
            panic!("expected result batch");
        };
        assert!(
            matches!(&parts[0], ContentPart::ToolResult { tool_use_id, .. } if tool_use_id == "call-a")
        );
        assert!(
            matches!(&parts[1], ContentPart::ToolResult { tool_use_id, .. } if tool_use_id == "call-b")
        );
        assert_eq!(parts.len(), 6);
        assert!(matches!(&parts[2], ContentPart::Text(text) if text.ends_with("call-a:")));
        assert!(matches!(&parts[4], ContentPart::Text(text) if text.ends_with("call-b:")));
    }

    #[test]
    fn base64_images_decode_without_changing_bytes_or_media_type() {
        let converted = rig_messages_to_solar(&[Message::User {
            content: OneOrMany::one(UserContent::image_base64(
                "AAECAw==",
                Some(ImageMediaType::PNG),
                None,
            )),
        }])
        .unwrap();
        assert_eq!(
            converted[0].content,
            MessageContent::MultiPart(vec![ContentPart::Image {
                data: vec![0, 1, 2, 3],
                mime_type: "image/png".into(),
            }])
        );
    }

    #[test]
    fn unsupported_roles_and_opaque_reasoning_fail_without_silent_data_loss() {
        assert!(
            rig_messages_to_solar(&[Message::System {
                content: "instructions".into()
            }])
            .is_err()
        );
        let reasoning = Message::Assistant {
            id: None,
            content: OneOrMany::one(AssistantContent::Reasoning(Reasoning::encrypted("opaque"))),
        };
        assert!(rig_messages_to_solar(&[reasoning]).is_err());
        let invalid = ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::ToolUse {
                tool_use_id: "call".into(),
                name: "files_read".into(),
                input: serde_json::json!({}),
            },
        };
        assert!(solar_messages_to_rig(vec![invalid]).is_err());
    }

    #[test]
    fn cached_and_uncached_usage_roundtrip_without_double_counting() {
        let usage = Usage {
            input_tokens: 11,
            output_tokens: 7,
            cached_input_tokens: 13,
            cache_creation_input_tokens: 17,
        };
        assert_eq!(rig_usage_to_solar(solar_usage_to_rig(&usage)), usage);
    }
}
