//! Exercise Rig's final HTTP body, where its typed system message becomes JSON.

use std::time::Duration;

use futures::StreamExt;
use serde_json::{Value, json};
use solar_agent_core::{
    AgentEvent, ContentPart, ConversationMessage, MessageContent, MessageRole, StopReason,
    ToolDefinition,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::*;

async fn read_request(socket: &mut TcpStream) -> (String, Value) {
    let mut bytes = Vec::new();
    let header_end = loop {
        let mut chunk = [0; 4096];
        let read = socket.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "request ended before its HTTP headers");
        bytes.extend_from_slice(&chunk[..read]);
        assert!(bytes.len() < 512_000, "unexpectedly large fixture request");
        if let Some(position) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            break position + 4;
        }
    };
    let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().unwrap())
        })
        .expect("JSON request has a content length");
    assert!(length < 512_000);
    while bytes.len() < header_end + length {
        let mut chunk = [0; 4096];
        let read = socket.read(&mut chunk).await.unwrap();
        assert_ne!(read, 0, "request ended before its JSON body");
        bytes.extend_from_slice(&chunk[..read]);
    }
    (
        headers,
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
    )
}

#[tokio::test]
async fn chatgpt_gateway_receives_plain_system_text_without_changing_tools_or_images() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let (headers, body) =
            tokio::time::timeout(Duration::from_secs(5), read_request(&mut socket))
                .await
                .unwrap();
        // Reproduce the gateway boundary: array-valued system content is not
        // extracted as instructions and reaches ChatGPT as a forbidden role.
        let plain_system = body["messages"][0]["content"].is_string();
        let (status, content_type, response) = if plain_system {
            (
                "200 OK",
                "text/event-stream",
                concat!(
                    "data: {\"id\":\"completion-fixture\",\"model\":\"codex-gpt-6-astra\",\"choices\":[{\"delta\":{\"content\":\"Done\",\"tool_calls\":[]},\"finish_reason\":null}]}\n\n",
                    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                    "data: [DONE]\n\n",
                ),
            )
        } else {
            (
                "400 Bad Request",
                "application/json",
                r#"{"error":{"message":"ChatgptException - System messages are not allowed"}}"#,
            )
        };
        socket
            .write_all(
                format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len(),
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        (headers, body)
    });

    let system =
        "You are a careful Solar Orbit.\nRespect tool permissions and preserve this instruction.";
    let parameters = json!({
        "type": "object",
        "properties": { "query": { "type": "string" } },
        "required": ["query"],
        "additionalProperties": false,
    });
    let mut request = TurnRequest::new(
        "codex-gpt-6-astra",
        vec![
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("Find the release notes".into()),
            },
            ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::ToolUse {
                    tool_use_id: "call-fixture-1".into(),
                    name: "lookup".into(),
                    input: json!({"query": "release notes"}),
                },
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::ToolResult {
                    tool_use_id: "call-fixture-1".into(),
                    content: "Found the release notes".into(),
                    is_error: false,
                },
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::MultiPart(vec![
                    ContentPart::Text("Inspect this image too".into()),
                    ContentPart::Image {
                        data: vec![137, 80, 78, 71],
                        mime_type: "image/png".into(),
                    },
                ]),
            },
        ],
    );
    request.system_prompt = Some(system.into());
    request.max_output_tokens = Some(123);
    request.tools.push(ToolDefinition {
        name: "lookup".into(),
        description: "Look up release notes".into(),
        input_schema: parameters.clone(),
    });
    let runtime = OpenAICompatibleRuntime::new(OpenAICompatibleRuntimeConfig {
        base_url,
        api_key: Some("fixture-key".into()),
        model: "codex-gpt-6-astra".into(),
        max_output_tokens: Some(999),
        supports_system_messages: true,
    })
    .with_reasoning_effort(Some("high".into()));
    let (_sender, control) = solar_agent_core::turn_control();
    let started =
        tokio::time::timeout(Duration::from_secs(5), runtime.start_turn(request, control))
            .await
            .unwrap()
            .expect("plain system instructions should pass the gateway");
    // Rig opens its HTTP stream lazily. Poll it while the mock server accepts
    // and responds, rather than waiting for a request that has not started yet.
    let received_events = tokio::time::timeout(Duration::from_secs(5), started.collect::<Vec<_>>())
        .await
        .unwrap();
    let (headers, body) = server.await.unwrap();

    assert!(headers.starts_with("POST /v1/chat/completions HTTP/1.1\r\n"));
    assert_eq!(body["model"], "codex-gpt-6-astra");
    assert_eq!(body["max_tokens"], 123);
    assert_eq!(body["reasoning_effort"], "high");
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"]["include_usage"], true);
    let messages = body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 5);
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[0]["content"], system);
    assert_eq!(messages[1]["role"], "user");
    assert_eq!(messages[1]["content"], "Find the release notes");
    assert_eq!(messages[2]["role"], "assistant");
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call-fixture-1");
    assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "lookup");
    assert_eq!(
        serde_json::from_str::<Value>(
            messages[2]["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
        json!({"query": "release notes"}),
    );
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call-fixture-1");
    assert_eq!(messages[3]["content"], "Found the release notes");
    assert_eq!(messages[4]["role"], "user");
    assert_eq!(messages[4]["content"][0]["text"], "Inspect this image too");
    assert_eq!(
        messages[4]["content"][1]["image_url"]["url"],
        "data:image/png;base64,iVBORw==",
    );
    assert_eq!(body["tools"][0]["type"], "function");
    assert_eq!(body["tools"][0]["function"]["name"], "lookup");
    assert_eq!(body["tools"][0]["function"]["parameters"], parameters);

    let events = received_events
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(events.iter().any(|event| matches!(
        event, AgentEvent::TextDelta { text } if text == "Done"
    )));
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::TurnStopped {
            reason: StopReason::Completed
        }
    )));
}
