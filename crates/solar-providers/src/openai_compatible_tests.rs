use futures::StreamExt;
use rig_core::client::CompletionClient;
use rig_core::completion::{AssistantContent, Message};
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::providers::openai;
use rig_core::test_utils::MockStreamingClient;
use solar_agent_core::{
    AgentEvent, AgentRuntime, ConversationMessage, MessageContent, MessageRole, StopReason,
    ToolEvent, TurnCommand, Usage,
};

use super::*;

fn text_request() -> TurnRequest {
    TurnRequest::new(
        "test-model",
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("Hello".to_string()),
        }],
    )
}

fn sse(lines: &[&str]) -> bytes::Bytes {
    lines
        .iter()
        .map(|line| format!("data: {line}\n\n"))
        .collect::<String>()
        .into()
}

#[tokio::test]
async fn rig_stream_maps_reasoning_text_usage_and_stop() {
    let http_client = MockStreamingClient {
        sse_bytes: sse(&[
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"reasoning_content":"thinking ","tool_calls":[]},"finish_reason":null}],"usage":null}"#,
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"content":"Hello ","tool_calls":[]},"finish_reason":null}],"usage":null}"#,
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"content":"world","tool_calls":[]},"finish_reason":"stop"}],"usage":null}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":4,"completion_tokens":6,"total_tokens":10,"prompt_tokens_details":{"cached_tokens":2}}}"#,
            "[DONE]",
        ]),
    };
    let client = openai::CompletionsClient::builder()
        .api_key("test-key")
        .base_url("http://localhost/v1")
        .http_client(http_client)
        .build()
        .unwrap();
    let model = client.completion_model("test-model");
    let (_, control) = solar_agent_core::turn_control();

    let events = start_model_turn(model, text_request(), control, None, true, None)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(matches!(events[0], AgentEvent::TurnStarted { .. }));
    assert_eq!(
        events[1..],
        [
            AgentEvent::ReasoningDelta {
                text: "thinking ".to_string(),
            },
            AgentEvent::TextDelta {
                text: "Hello ".to_string(),
            },
            AgentEvent::TextDelta {
                text: "world".to_string(),
            },
            AgentEvent::UsageUpdated {
                usage: Usage {
                    input_tokens: 2,
                    output_tokens: 6,
                    cached_input_tokens: 2,
                    cache_creation_input_tokens: 0,
                },
            },
            AgentEvent::TurnStopped {
                reason: StopReason::Completed,
            },
        ]
    );
}

#[tokio::test]
async fn rig_stream_ignores_empty_reasoning_heartbeats() {
    let http_client = MockStreamingClient {
        sse_bytes: sse(&[
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"reasoning_content":"","tool_calls":[]},"finish_reason":null}],"usage":null}"#,
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"content":"done","tool_calls":[]},"finish_reason":"stop"}],"usage":null}"#,
            "[DONE]",
        ]),
    };
    let client = openai::CompletionsClient::builder()
        .api_key("test-key")
        .base_url("http://localhost/v1")
        .http_client(http_client)
        .build()
        .unwrap();
    let model = client.completion_model("test-model");
    let (_, control) = solar_agent_core::turn_control();

    let events = start_model_turn(model, text_request(), control, None, true, None)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(!events.iter().any(|event| matches!(
        event,
        AgentEvent::ReasoningCompleted { text, signature }
            if text.is_empty() && signature.is_none()
    )));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, AgentEvent::TextDelta { text } if text == "done"))
    );
}

#[tokio::test]
async fn cancellation_before_stream_start_is_a_normal_stop() {
    let runtime = OpenAICompatibleRuntime::new(OpenAICompatibleRuntimeConfig {
        base_url: "http://localhost/v1".to_string(),
        api_key: None,
        model: "test-model".to_string(),
        max_output_tokens: None,
        supports_system_messages: true,
    });
    let (sender, control) = solar_agent_core::turn_control();
    sender.send(TurnCommand::Cancel).await.unwrap();

    let events = runtime
        .start_turn(text_request(), control)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(matches!(events[0], AgentEvent::TurnStarted { .. }));
    assert_eq!(
        events[1],
        AgentEvent::TurnStopped {
            reason: StopReason::Cancelled,
        }
    );
}

#[tokio::test]
async fn usage_at_the_requested_limit_maps_to_max_tokens() {
    let http_client = MockStreamingClient {
        sse_bytes: sse(&[
            r#"{"choices":[{"delta":{"content":"cut off","tool_calls":[]},"finish_reason":"length"}],"usage":null}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":2,"completion_tokens":6,"total_tokens":8}}"#,
            "[DONE]",
        ]),
    };
    let client = openai::CompletionsClient::builder()
        .api_key("test-key")
        .base_url("http://localhost/v1")
        .http_client(http_client)
        .build()
        .unwrap();
    let model = client.completion_model("test-model");
    let (sender, control) = solar_agent_core::turn_control();
    let mut request = text_request();
    request.max_output_tokens = Some(6);

    let events = start_model_turn(model, request, control, None, true, None)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    drop(sender);

    assert_eq!(
        events.last(),
        Some(&AgentEvent::TurnStopped {
            reason: StopReason::MaxTokens,
        })
    );
}

#[tokio::test]
async fn rig_stream_maps_complete_tool_call_without_executing_it() {
    let http_client = MockStreamingClient {
        sse_bytes: sse(&[
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"tool_calls":[{"index":0,"id":"call-1","type":"function","function":{"name":"read_files","arguments":"{\"files\":[\"Cargo.toml\"]}"}}]},"finish_reason":null}],"usage":null}"#,
            r#"{"id":"cmpl-1","model":"test-model","choices":[{"delta":{"tool_calls":[]},"finish_reason":"tool_calls"}],"usage":null}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":8,"completion_tokens":4,"total_tokens":12}}"#,
            "[DONE]",
        ]),
    };
    let client = openai::CompletionsClient::builder()
        .api_key("test-key")
        .base_url("http://localhost/v1")
        .http_client(http_client)
        .build()
        .unwrap();
    let model = client.completion_model("test-model");
    let (_, control) = solar_agent_core::turn_control();
    let mut request = text_request();
    request.tools.push(solar_agent_core::ToolDefinition {
        name: "read_files".to_string(),
        description: "Read files".to_string(),
        input_schema: serde_json::json!({"type": "object"}),
    });

    let events = start_model_turn(model, request, control, None, true, None)
        .await
        .unwrap()
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(events.iter().any(|event| {
        matches!(
            event,
            AgentEvent::Tool {
                event: ToolEvent::Proposed { call },
            }
                if call.id == "call-1"
                    && call.name == "read_files"
                    && call.arguments == serde_json::json!({"files": ["Cargo.toml"]})
        )
    }));
    assert_eq!(
        events.last(),
        Some(&AgentEvent::TurnStopped {
            reason: StopReason::Completed,
        })
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                event,
                AgentEvent::Tool {
                    event: ToolEvent::Started { .. } | ToolEvent::Completed { .. },
                }
            ))
            .count(),
        0
    );
}

#[test]
fn request_conversion_preserves_history_tools_and_limits() {
    let mut request = text_request();
    request.system_prompt = Some("Be useful".to_string());
    request.max_output_tokens = Some(123);
    request.tools.push(solar_agent_core::ToolDefinition {
        name: "create_plan".to_string(),
        description: "Create a plan document".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"documents": {"type": "array"}},
            "required": ["documents"]
        }),
    });

    let converted = build_completion_request(request, Some(999), true, None).unwrap();

    assert_eq!(converted.max_tokens, Some(123));
    assert_eq!(converted.tools.len(), 1);
    assert_eq!(converted.tools[0].name, "create_plan");
    assert_eq!(
        converted.tools[0].parameters,
        serde_json::json!({
            "type": "object",
            "properties": {"documents": {"type": "array"}},
            "required": ["documents"]
        })
    );
    assert_eq!(converted.chat_history.len(), 2);
    assert!(matches!(
        converted.chat_history.iter().next(),
        Some(Message::System { content }) if content == "Be useful"
    ));
}

#[test]
fn request_conversion_preserves_tool_call_and_denied_result_for_the_next_turn() {
    let request = TurnRequest::new(
        "test-model",
        vec![
            ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::ToolUse {
                    tool_use_id: "call-1".to_string(),
                    name: "run_shell_command".to_string(),
                    input: serde_json::json!({"command": "cargo test"}),
                },
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::ToolResult {
                    tool_use_id: "call-1".to_string(),
                    content: "Command not executed — permission denied.".to_string(),
                    is_error: true,
                },
            },
        ],
    );

    let converted = build_completion_request(request, None, true, None).unwrap();
    let messages = converted.chat_history.iter().collect::<Vec<_>>();
    let Message::Assistant { content, .. } = messages[0] else {
        panic!("expected assistant tool call");
    };
    let Some(AssistantContent::ToolCall(call)) = content.iter().next() else {
        panic!("expected assistant tool call content");
    };
    assert_eq!(call.id, "call-1");
    assert_eq!(call.call_id.as_deref(), Some("call-1"));
    assert_eq!(call.function.name, "run_shell_command");

    let Message::User { content } = messages[1] else {
        panic!("expected user tool result");
    };
    let Some(UserContent::ToolResult(result)) = content.iter().next() else {
        panic!("expected user tool result content");
    };
    assert_eq!(result.id, "call-1");
    assert_eq!(result.call_id.as_deref(), Some("call-1"));
    let Some(ToolResultContent::Text(text)) = result.content.iter().next() else {
        panic!("expected text tool result");
    };
    assert_eq!(
        text.text,
        "[ERROR] Command not executed — permission denied."
    );
}

#[test]
fn request_conversion_places_system_prompt_in_user_message_when_system_role_is_unsupported() {
    let mut request = text_request();
    request.system_prompt = Some("Be useful".to_string());

    let converted = build_completion_request(request, None, false, None).unwrap();
    let messages = converted.chat_history.iter().collect::<Vec<_>>();

    assert_eq!(messages.len(), 2);
    let Message::User { content } = messages[0] else {
        panic!("expected the system prompt to use the user role");
    };
    let Some(UserContent::Text(text)) = content.iter().next() else {
        panic!("expected text instructions");
    };
    assert_eq!(text.text, "Be useful");
    assert_eq!(
        messages
            .iter()
            .filter(|message| matches!(message, Message::System { .. }))
            .count(),
        0
    );
}

#[test]
fn selected_reasoning_effort_is_sent_without_changing_the_model() {
    let request = build_completion_request(text_request(), None, true, Some("high")).unwrap();
    assert_eq!(
        request.additional_params,
        Some(serde_json::json!({
            "stream_options": {"include_usage": true}, "reasoning_effort": "high"
        }))
    );
    let default = build_completion_request(text_request(), None, true, None).unwrap();
    assert!(
        default
            .additional_params
            .unwrap()
            .get("reasoning_effort")
            .is_none()
    );
}

#[test]
fn reasoning_effort_survives_openai_request_serialization() {
    let request = build_completion_request(text_request(), None, true, Some("low")).unwrap();
    let wire = rig_core::providers::openai::completion::CompletionRequest::try_from((
        "test-model".to_string(),
        request,
    ))
    .unwrap();
    let json = serde_json::to_value(wire).unwrap();
    assert_eq!(json["model"], "test-model");
    assert_eq!(json["reasoning_effort"], "low");
    assert_eq!(json["stream_options"]["include_usage"], true);
}
