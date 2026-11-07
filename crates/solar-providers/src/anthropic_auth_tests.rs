use solar_agent_core::{
    ConversationMessage, MessageContent, MessageRole, ToolDefinition, TurnRequest,
};

use super::*;
use crate::request::build_completion_request;

#[test]
fn subscription_auth_is_bearer_only_and_redacted() {
    let token = "sk-ant-oat01-test-subscription-token";
    let client = anthropic_client(token).expect("subscription client");
    assert_eq!(client.headers()["authorization"], format!("Bearer {token}"));
    assert!(client.headers()["authorization"].is_sensitive());
    assert!(!client.headers().contains_key("x-api-key"));
    assert_eq!(
        client.headers()["anthropic-beta"],
        "claude-code-20250219,oauth-2025-04-20"
    );
    assert_eq!(client.headers()["user-agent"], "claude-cli/2.1.280");
    assert_eq!(
        client.headers()["anthropic-dangerous-direct-browser-access"],
        "true"
    );
    assert_eq!(client.headers()["x-app"], "cli");
    assert_eq!(client.headers()["accept"], "application/json");
    assert!(!format!("{client:?}").contains(token));
    let config = crate::AnthropicRuntimeConfig {
        api_key: token.to_string(),
        model: "claude-test".to_string(),
        max_output_tokens: None,
    };
    assert!(!format!("{config:?}").contains(token));
}

#[test]
fn api_key_keeps_normal_authentication() {
    let client = anthropic_client("sk-ant-api03-test-key").expect("API client");
    assert_eq!(client.headers()["x-api-key"], "sk-ant-api03-test-key");
    assert!(!client.headers().contains_key("authorization"));
}

#[test]
fn subscription_request_keeps_solar_prompt_and_tool_names() {
    let mut turn = TurnRequest::new(
        "claude-test".to_string(),
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("Fix the bug".to_string()),
        }],
    );
    turn.system_prompt = Some("Use Solar discovery and permissions.".to_string());
    turn.tools = vec![ToolDefinition {
        name: "search_codebase".to_string(),
        description: "Search Solar's index".to_string(),
        input_schema: serde_json::json!({"type":"object"}),
    }];
    let mut request = build_completion_request(turn, Some(4096), true, true, None).unwrap();
    prepare_subscription_request(&mut request).unwrap();
    let history = request.chat_history.iter().collect::<Vec<_>>();
    let parameters = request.additional_params.as_ref().unwrap();
    assert_eq!(parameters["system"][0]["text"], CLAUDE_CODE_IDENTITY);
    assert_eq!(
        parameters["system"][1]["text"],
        "Use Solar discovery and permissions."
    );
    assert_eq!(parameters["tools"][0]["name"], "search_codebase");
    assert_eq!(parameters["tools"][0]["eager_input_streaming"], true);
    assert!(request.tools.is_empty());
    assert_eq!(request.max_tokens, Some(4096));
    assert_eq!(history.len(), 1);
}

#[test]
fn pi_tool_casing_round_trips_without_changing_solar_execution_identity() {
    let mut turn = TurnRequest::new(
        "claude-test".to_string(),
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("Inspect".to_string()),
        }],
    );
    turn.tools = vec![ToolDefinition {
        name: "grep".to_string(),
        description: "Search".to_string(),
        input_schema: serde_json::json!({"type":"object","properties":{"query":{"type":"string"}},"required":["query"]}),
    }];
    let mut request = build_completion_request(turn, Some(4096), true, true, None).unwrap();
    let aliases = prepare_subscription_request(&mut request).unwrap();
    assert_eq!(
        request.additional_params.as_ref().unwrap()["tools"][0]["name"],
        "Grep"
    );
    let mut event = AgentEvent::Tool {
        event: ToolEvent::Proposed {
            call: solar_agent_core::ToolCall {
                id: "toolu_123".to_string(),
                name: "Grep".to_string(),
                arguments: json!({"query":"foo"}),
            },
        },
    };
    restore_tool_names(&mut event, &aliases);
    assert!(
        matches!(event, AgentEvent::Tool { event: ToolEvent::Proposed { call } }
        if call.name == "grep" && call.id == "toolu_123")
    );
    let mut progress = AgentEvent::ToolCallProgress {
        call_id: "toolu_123".to_string(),
        name: Some("GREP".to_string()),
        arguments_bytes: 42,
    };
    restore_tool_names(&mut progress, &aliases);
    assert!(
        matches!(progress, AgentEvent::ToolCallProgress { name: Some(name), .. } if name == "grep")
    );
}

#[test]
fn pi_call_id_normalization_preserves_tool_result_pairing() {
    let mut turn = TurnRequest::new(
        "claude-test".to_string(),
        vec![
            ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::ToolUse {
                    tool_use_id: "tool.call/1".to_string(),
                    name: "grep".to_string(),
                    input: json!({}),
                },
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::ToolResult {
                    tool_use_id: "tool.call/1".to_string(),
                    content: "found".to_string(),
                    is_error: false,
                },
            },
        ],
    );
    turn.system_prompt = Some("Solar".to_string());
    let mut request = build_completion_request(turn, Some(4096), true, true, None).unwrap();
    prepare_subscription_request(&mut request).unwrap();
    for message in request.chat_history.iter() {
        match message {
            Message::Assistant { content, .. } => {
                let AssistantContent::ToolCall(call) = content.first() else {
                    panic!("tool call");
                };
                assert_eq!(call.function.name, "Grep");
                assert_eq!(call.call_id.as_deref(), Some("tool_call_1"));
            }
            Message::User { content } => {
                let UserContent::ToolResult(result) = content.first() else {
                    panic!("tool result");
                };
                assert_eq!(result.call_id.as_deref(), Some("tool_call_1"));
            }
            _ => panic!("unexpected system message on wire history"),
        }
    }
}

#[tokio::test]
async fn serialized_subscription_request_matches_pi_auth_and_tool_profile() {
    use futures::StreamExt;
    use rig_core::client::CompletionClient;
    use rig_core::completion::CompletionModel;
    let http_client = RecordingStreamingClient::new(
        r#"event: message_start
data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","model":"claude-test","content":[],"stop_reason":null,"stop_sequence":null,"usage":{"input_tokens":1,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"ok"}}

event: content_block_stop
data: {"type":"content_block_stop","index":0}

event: message_delta
data: {"type":"message_delta","delta":{"stop_reason":"end_turn","stop_sequence":null},"usage":{"output_tokens":1}}

event: message_stop
data: {"type":"message_stop"}

"#,
    );
    let client = Client::<SubscriptionProvider>::builder()
        .api_key(SubscriptionToken("sk-ant-oat01-test".to_string()))
        .http_headers(subscription_headers())
        .http_client(http_client.clone())
        .build()
        .unwrap();
    let mut turn = TurnRequest::new(
        "claude-test".to_string(),
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("Inspect".to_string()),
        }],
    );
    turn.system_prompt = Some("Solar discovery rules".to_string());
    turn.tools = vec![ToolDefinition {
        name: "read".to_string(),
        description: "Read".to_string(),
        input_schema: json!({"type":"object","properties":{},"required":[]}),
    }];
    let mut request = build_completion_request(turn, Some(4096), true, true, None).unwrap();
    prepare_subscription_request(&mut request).unwrap();
    let mut stream = client
        .completion_model("claude-test")
        .with_prompt_caching()
        .stream(request)
        .await
        .unwrap();
    while let Some(event) = stream.next().await {
        event.unwrap();
    }
    let requests = http_client.recording.requests();
    assert_eq!(requests.len(), 1);
    let sent = &requests[0];
    assert_eq!(sent.headers["user-agent"], "claude-cli/2.1.280");
    assert_eq!(sent.headers["accept"], "application/json");
    assert_eq!(
        sent.headers["anthropic-dangerous-direct-browser-access"],
        "true"
    );
    assert_eq!(
        sent.headers["anthropic-beta"],
        "claude-code-20250219,oauth-2025-04-20"
    );
    assert!(!sent.headers.contains_key("x-api-key"));
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(body["system"].as_array().unwrap().len(), 2);
    assert_eq!(body["system"][0]["text"], CLAUDE_CODE_IDENTITY);
    assert_eq!(body["system"][1]["text"], "Solar discovery rules");
    assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    assert_eq!(body["tools"][0]["name"], "Read");
    assert_eq!(body["tools"][0]["eager_input_streaming"], true);
    assert_eq!(body["tools"][0]["cache_control"]["type"], "ephemeral");
}

// Rig's unary recording client does not implement streaming. Compose it with the
// SSE fixture client to capture the actual streaming payload and headers.
#[derive(Clone, Default)]
struct RecordingStreamingClient {
    recording: rig_core::test_utils::RecordingHttpClient,
    streaming: rig_core::test_utils::MockStreamingClient,
}

impl RecordingStreamingClient {
    fn new(sse: &'static str) -> Self {
        Self {
            recording: rig_core::test_utils::RecordingHttpClient::default(),
            streaming: rig_core::test_utils::MockStreamingClient {
                sse_bytes: bytes::Bytes::from_static(sse.as_bytes()),
            },
        }
    }
}

impl HttpClientExt for RecordingStreamingClient {
    fn send<T, U>(
        &self,
        request: http_client::Request<T>,
    ) -> impl std::future::Future<
        Output = http_client::Result<http_client::Response<http_client::LazyBody<U>>>,
    > + rig_core::wasm_compat::WasmCompatSend
    + 'static
    where
        T: Into<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend,
        U: From<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend + 'static,
    {
        self.recording.send(request)
    }

    fn send_multipart<U>(
        &self,
        request: http_client::Request<http_client::MultipartForm>,
    ) -> impl std::future::Future<
        Output = http_client::Result<http_client::Response<http_client::LazyBody<U>>>,
    > + rig_core::wasm_compat::WasmCompatSend
    + 'static
    where
        U: From<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend + 'static,
    {
        self.recording.send_multipart(request)
    }

    fn send_streaming<T>(
        &self,
        request: http_client::Request<T>,
    ) -> impl std::future::Future<Output = http_client::Result<http_client::StreamingResponse>>
    + rig_core::wasm_compat::WasmCompatSend
    where
        T: Into<bytes::Bytes> + rig_core::wasm_compat::WasmCompatSend,
    {
        let recorded = self
            .recording
            .send::<bytes::Bytes, bytes::Bytes>(request.map(Into::into));
        let streaming = self.streaming.clone();
        async move {
            recorded.await?;
            streaming
                .send_streaming(http_client::Request::new(bytes::Bytes::new()))
                .await
        }
    }
}
