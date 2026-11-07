#![cfg(not(target_os = "macos"))]
use serde_json::{Value, json};
use solar::{
    engine::{Engine, ProviderDraft},
    model::Memory,
};
use solar_providers::{ProviderKind, ProviderProfile};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn fixture(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0u8; 4096];
            let (header_end, length) = loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|s| s == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length = header
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|s| s.trim().parse::<usize>().unwrap())
                        })
                        .unwrap();
                    break (end + 4, length);
                }
            };
            while bytes.len() < header_end + length {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            requests.push(serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap());
            let wire = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response
            );
            socket.write_all(wire.as_bytes()).await.unwrap();
        }
        requests
    });
    (format!("http://{address}/v1"), server)
}

fn response(delta: Value, reason: &str) -> String {
    let chunk = json!({"id":"fixture","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":delta,"finish_reason":null}]});
    let end = json!({"id":"fixture","object":"chat.completion.chunk","created":1,"model":"test-model","choices":[{"index":0,"delta":{},"finish_reason":reason}]});
    format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")
}

async fn configure(engine: &Arc<Engine>, base_url: String, ask: bool) {
    engine
        .save_provider(ProviderDraft {
            profile: ProviderProfile {
                id: "fixture".into(),
                name: "Local fixture".into(),
                kind: ProviderKind::OpenAi,
                base_url,
                models: vec!["test-model".into()],
                experimental: false,
            },
            api_key: Some("fixture-key".into()),
        })
        .unwrap();
    let mut orbit = engine.orbit("assistant").unwrap();
    orbit.provider_id = "fixture".into();
    orbit.model = "test-model".into();
    orbit.autonomy = if ask { "ask" } else { "workspace" }.into();
    engine.save_orbit(orbit).unwrap();
}

async fn finished(engine: &Arc<Engine>, task_id: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let task = engine
                .bootstrap()
                .unwrap()
                .tasks
                .into_iter()
                .find(|t| t.id == task_id)
                .unwrap();
            if task.status == "completed" {
                break;
            }
            assert!(
                !["failed", "cancelled"].contains(&task.status.as_str()),
                "task failed: {:?}",
                task.error
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("task must complete");
}

#[tokio::test]
async fn normal_entrypoint_waits_for_approval_executes_tool_and_preserves_followup() {
    let call = json!({"role":"assistant","tool_calls":[{"index":0,"id":"call_memory","type":"function","function":{"name":"memory_save","arguments":"{\"content\":\"The demo color is blue\",\"shared\":false}"}}]});
    let (url, server) = fixture(vec![
        response(call, "tool_calls"),
        response(json!({"content":"Saved the color."}), "stop"),
        response(json!({"content":"The color is blue."}), "stop"),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    configure(&engine, url, true).await;
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Fixture".into(),
            None,
        )
        .unwrap();
    let first = engine
        .send_message(&chat.id, "Remember the demo color is blue.".into())
        .unwrap();
    let approval = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(approval) = engine.bootstrap().unwrap().approvals.first().cloned()
                && engine
                    .attention_items()
                    .unwrap()
                    .iter()
                    .any(|item| item.id == format!("approval:{}", approval.id))
            {
                break approval;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(engine.memories(Some("assistant"), None).unwrap().is_empty());
    assert_eq!(approval.tool, "memory_save");
    let alert = engine.attention_items().unwrap().remove(0);
    assert_eq!(alert.kind, "approval");
    assert_eq!(alert.task_id, first.id);
    assert_eq!(alert.conversation_id, chat.id);
    engine.dismiss_attention(&alert.id).unwrap();
    assert!(engine.attention_items().unwrap().is_empty());
    assert_eq!(engine.bootstrap().unwrap().approvals.len(), 1);
    assert!(engine.memories(Some("assistant"), None).unwrap().is_empty());
    engine.resolve_approval(&approval.id, true).unwrap();
    finished(&engine, &first.id).await;
    assert_eq!(engine.memories(Some("assistant"), None).unwrap().len(), 1);
    assert!(engine.memories(Some("qa"), None).unwrap().is_empty());
    let second = engine.send_message(&chat.id, "What color?".into()).unwrap();
    finished(&engine, &second.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "tool" && m["tool_call_id"] == "call_memory")
    );
    assert!(requests[2].to_string().contains("What color?"));
    assert!(requests[2].to_string().contains("Saved the color."));
    assert!(engine.bootstrap().unwrap().approvals.is_empty());
    engine.shutdown().await;
}

#[tokio::test]
async fn cancelling_an_approval_preserves_tool_pairs_for_queued_followup() {
    let call = json!({"role":"assistant","tool_calls":[{"index":0,"id":"cancelled_call","type":"function","function":{"name":"memory_save","arguments":"{\"content\":\"Do not save this\"}"}}]});
    let (url, server) = fixture(vec![
        response(call, "tool_calls"),
        response(json!({"content":"Continuing safely."}), "stop"),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    configure(&engine, url, true).await;
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Cancellation".into(),
            None,
        )
        .unwrap();
    let first = engine
        .send_message(&chat.id, "First request".into())
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if !engine.bootstrap().unwrap().approvals.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let second = engine
        .send_message(&chat.id, "Second request".into())
        .unwrap();
    engine.stop_task(&first.id).unwrap();
    finished(&engine, &second.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    let history = requests[1]["messages"].as_array().unwrap();
    assert!(
        history
            .iter()
            .any(|m| m["role"] == "tool" && m["tool_call_id"] == "cancelled_call")
    );
    assert_eq!(
        history
            .iter()
            .filter(|m| m["role"] == "user" && m["content"] == "Second request")
            .count(),
        1
    );
    assert!(engine.memories(Some("assistant"), None).unwrap().is_empty());
    assert!(engine.bootstrap().unwrap().approvals.is_empty());
    assert!(
        engine
            .attention_items()
            .unwrap()
            .iter()
            .all(|item| item.task_id != first.id),
        "cancelled tasks must not leave approval or completion alerts"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn disabled_and_deleted_orbits_do_not_leave_failure_alerts() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Missing connection".into(),
            None,
        )
        .unwrap();

    for remove in [false, true] {
        let task = engine
            .send_message(&chat.id, "A task without a model".into())
            .unwrap();
        let alert = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(alert) = engine
                    .attention_items()
                    .unwrap()
                    .into_iter()
                    .find(|item| item.task_id == task.id)
                {
                    break alert;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("a model configuration failure should request attention");
        assert_eq!(alert.kind, "failed");
        assert_eq!(alert.conversation_id, chat.id);
        if remove {
            engine.delete_orbit("assistant").unwrap();
        } else {
            let mut orbit = engine.orbit("assistant").unwrap();
            orbit.enabled = false;
            engine.save_orbit(orbit.clone()).unwrap();
            assert!(engine.attention_items().unwrap().is_empty());
            assert!(
                engine
                    .send_message(&chat.id, "Still disabled".into())
                    .is_err()
            );
            orbit.enabled = true;
            engine.save_orbit(orbit).unwrap();
        }
        assert!(engine.attention_items().unwrap().is_empty());
    }
    engine.shutdown().await;
}

#[test]
fn import_strips_authority_and_private_memory_stays_scoped() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    let mut exported: Value =
        serde_json::from_str(&engine.export_orbit("assistant").unwrap()).unwrap();
    exported["orbit"]["autonomy"] = json!("trusted");
    exported["orbit"]["providerId"] = json!("other-secret-account");
    let imported = engine.import_orbit(&exported.to_string()).unwrap();
    assert_eq!(imported.autonomy, "ask");
    assert!(imported.provider_id.is_empty());
    engine
        .save_memory(Memory {
            id: "private".into(),
            orbit_id: Some("assistant".into()),
            project_id: None,
            content: "Private note".into(),
            created_at: String::new(),
        })
        .unwrap();
    assert!(engine.memories(Some("qa"), None).unwrap().is_empty());
    assert!(engine.memories(None, None).unwrap().is_empty());
}
