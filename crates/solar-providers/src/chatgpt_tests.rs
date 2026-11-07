use rig_core::client::CompletionClient;
use rig_core::completion::{AssistantContent, CompletionModel, Message};
use rig_core::message::{ToolResultContent, UserContent};
use rig_core::providers::chatgpt::ChatGPTAuth;
use rig_core::test_utils::{RecordingHttpClient, SequencedStreamingHttpClient};
use solar_agent_core::StopReason;

use super::*;

const COMPLETED_RESPONSE: &str = r#"data: {"type":"response.output_text.delta","delta":"ok"}
data: {"type":"response.completed","sequence_number":54,"response":{"id":"resp_test","object":"response","created_at":1,"status":"completed","error":null,"incomplete_details":null,"instructions":null,"max_output_tokens":null,"model":"gpt-5.3-codex","usage":{"input_tokens":1,"input_tokens_details":{"cached_tokens":0},"output_tokens":1,"output_tokens_details":{"reasoning_tokens":0},"total_tokens":2},"output":[{"type":"message","id":"msg_test","status":"completed","role":"assistant","content":[{"type":"output_text","annotations":[],"text":"ok"}]}],"tools":[]}}
data: [DONE]"#;

#[test]
fn runtime_descriptor_identifies_chatgpt_subscription() {
    let runtime = ChatGPTSubscriptionRuntime::new(ChatGPTSubscriptionRuntimeConfig {
        model: "gpt-5.3-codex".to_string(),
        reasoning_effort: Some("high".to_string()),
        max_output_tokens: None,
    });

    assert_eq!(
        runtime.descriptor().id,
        "rig-chatgpt-subscription:gpt-5.3-codex"
    );
}

#[tokio::test]
async fn runtime_resolves_current_credentials_without_rig_oauth() {
    let runtime = ChatGPTSubscriptionRuntime::new(ChatGPTSubscriptionRuntimeConfig {
        model: "test-model".to_owned(),
        reasoning_effort: None,
        max_output_tokens: None,
    })
    .with_credential_resolver(std::sync::Arc::new(|| {
        Box::pin(async {
            Ok(ChatGPTSubscriptionCredentials {
                access_token: "resolved-token".to_owned(),
                account_id: Some("resolved-account".to_owned()),
            })
        })
    }));
    match runtime.resolve_auth().await.unwrap() {
        ChatGPTAuth::AccessToken {
            access_token,
            account_id,
        } => {
            assert_eq!(access_token, "resolved-token");
            assert_eq!(account_id.as_deref(), Some("resolved-account"));
        }
        ChatGPTAuth::OAuth => panic!("The runtime must not own OAuth refresh"),
    }
}

#[tokio::test]
async fn missing_or_failed_resolver_requires_sign_in_without_device_flow() {
    let runtime = ChatGPTSubscriptionRuntime::new(ChatGPTSubscriptionRuntimeConfig {
        model: "test-model".to_owned(),
        reasoning_effort: None,
        max_output_tokens: None,
    });
    assert_eq!(
        runtime.resolve_auth().await.unwrap_err().kind,
        AgentErrorKind::Authentication
    );
    let runtime = runtime.with_credential_resolver(std::sync::Arc::new(|| {
        Box::pin(async { Err("Sign in again".to_owned()) })
    }));
    assert_eq!(
        runtime.resolve_auth().await.unwrap_err().kind,
        AgentErrorKind::Authentication
    );
}

#[test]
fn reasoning_effort_is_encoded_for_the_responses_request() {
    let request = TurnRequest::new(
        "gpt-5.4".to_string(),
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("hello".to_string()),
        }],
    );
    let request = build_completion_request(
        request,
        None,
        true,
        false,
        reasoning_additional_params(Some("xhigh")),
    )
    .expect("request should convert");

    assert_eq!(
        request.additional_params,
        reasoning_additional_params(Some("xhigh"))
    );

    assert_eq!(
        reasoning_additional_params(Some("ultra")),
        Some(serde_json::json!({
            "reasoning": { "effort": "max" }
        }))
    );
}

#[tokio::test]
async fn chatgpt_follow_up_request_preserves_responses_call_ids() {
    let http_client = RecordingHttpClient::new(COMPLETED_RESPONSE);
    let client = chatgpt::Client::builder()
        .api_key(ChatGPTAuth::AccessToken {
            access_token: "test-token".to_string(),
            account_id: None,
        })
        .http_client(http_client.clone())
        .build()
        .expect("client should build");
    let model = client.completion_model("gpt-5.3-codex");
    let assistant_tool_call = AssistantContent::tool_call_with_call_id(
        "fc_native_1",
        "call_native_1".to_string(),
        "read_files",
        serde_json::json!({"files": ["Cargo.toml"]}),
    );
    let tool_result = UserContent::tool_result_with_call_id(
        "fc_native_1",
        "call_native_1".to_string(),
        rig_core::OneOrMany::one(ToolResultContent::text("contents")),
    );
    let chat_history = rig_core::OneOrMany::many(vec![
        Message::Assistant {
            id: None,
            content: rig_core::OneOrMany::one(assistant_tool_call),
        },
        Message::User {
            content: rig_core::OneOrMany::one(tool_result),
        },
        Message::user("Continue."),
    ])
    .expect("history should contain messages");

    model
        .completion(rig_core::completion::CompletionRequest {
            model: Some("gpt-5.3-codex".to_string()),
            preamble: None,
            chat_history,
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
            record_telemetry_content: false,
        })
        .await
        .expect("request should reach the mocked provider");

    let requests = http_client.requests();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    let input = body["input"].as_array().expect("input should be an array");
    assert_eq!(input[0]["call_id"], "call_native_1");
    assert_eq!(input[1]["call_id"], "call_native_1");
}

#[tokio::test]
async fn chatgpt_metadata_does_not_interrupt_text_or_completion() {
    let metadata = r#"data: {"type":"response.metadata","response_id":"resp_test","sequence_number":51,"metadata":{"openai_chatgpt_moderation_metadata":{"prompt":{},"generation":{},"tool_call":{},"tool_response":{}}}}"#;
    let delta = serde_json::json!({
        "type": "response.output_text.delta",
        "item_id": "msg_test",
        "output_index": 0,
        "content_index": 0,
        "sequence_number": 52,
        "delta": "ok",
    });
    let completed = COMPLETED_RESPONSE
        .lines()
        .find(|line| line.starts_with("data: {\"type\":\"response.completed\""))
        .expect("fixture should contain a completed response");
    let response =
        format!("{metadata}\n\ndata: {delta}\n\n{metadata}\n\n{completed}\n\ndata: [DONE]\n\n");
    let client = chatgpt::Client::builder()
        .api_key(ChatGPTAuth::AccessToken {
            access_token: "test-token".to_string(),
            account_id: None,
        })
        .http_client(SequencedStreamingHttpClient::new(vec![Ok(response.into())]))
        .build()
        .expect("client should build");
    let request = build_completion_request(
        TurnRequest::new(
            "gpt-5.3-codex".to_string(),
            vec![ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("hello".to_string()),
            }],
        ),
        None,
        true,
        true,
        None,
    )
    .expect("request should convert");
    let (_sender, control) = turn_control();
    let mut stream = start_model_turn(
        client.completion_model("gpt-5.3-codex"),
        request,
        control,
        None,
    )
    .await
    .expect("request should reach the mocked provider");
    let mut text = String::new();
    let mut usage = None;
    let mut stop_reason = None;
    while let Some(event) = stream.next().await {
        match event.expect("metadata should not fail the turn") {
            AgentEvent::TextDelta { text: delta } => text.push_str(&delta),
            AgentEvent::UsageUpdated { usage: updated } => usage = Some(updated),
            AgentEvent::TurnStopped { reason } => stop_reason = Some(reason),
            _ => {}
        }
    }

    assert_eq!(text, "ok");
    assert_eq!(
        usage.expect("completion should report usage").output_tokens,
        1
    );
    assert_eq!(stop_reason, Some(StopReason::Completed));
}

#[tokio::test]
async fn incomplete_stream_cannot_report_success() {
    let response = format!(
        "data: {}\n\n",
        serde_json::json!({
            "type":"response.output_text.delta", "item_id":"msg_test", "output_index":0,
            "content_index":0, "sequence_number":1, "delta":"partial"
        })
    );
    let client = chatgpt::Client::builder()
        .api_key(ChatGPTAuth::AccessToken {
            access_token: "test-token".into(),
            account_id: None,
        })
        .http_client(SequencedStreamingHttpClient::new(vec![Ok(response.into())]))
        .build()
        .unwrap();
    let request = build_completion_request(
        TurnRequest::new(
            "test-model",
            vec![ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("hello".into()),
            }],
        ),
        None,
        true,
        true,
        None,
    )
    .unwrap();
    let (_sender, control) = turn_control();
    let mut stream = start_model_turn(
        client.completion_model("test-model"),
        request,
        control,
        None,
    )
    .await
    .unwrap();
    let mut incomplete = false;
    while let Some(event) = stream.next().await {
        match event {
            Err(error) => {
                assert_eq!(error.kind, AgentErrorKind::Transport, "{}", error.message);
                incomplete = true;
            }
            Ok(AgentEvent::TurnStopped {
                reason: StopReason::Completed,
            }) => panic!("truncated stream reported success"),
            _ => {}
        }
    }
    assert!(incomplete);
}
