#![cfg(not(target_os = "macos"))]
mod common;

use serde_json::{Value, json};
use solar::{
    engine::{Engine, ProviderDraft},
    model::{Memory, Project},
};
use solar_providers::{ProviderKind, ProviderProfile};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

async fn fixture(responses: Vec<String>) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    fixture_with_status(
        responses
            .into_iter()
            .map(|response| (200, response))
            .collect(),
    )
    .await
}

async fn fixture_with_status(
    responses: Vec<(u16, String)>,
) -> (String, tokio::task::JoinHandle<Vec<Value>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (status, response) in responses {
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
                "HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
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
async fn shell_activity_streams_then_finishes_with_incremental_git_changes() {
    let directory = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let initialized = tokio::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(project.path())
        .status()
        .await
        .unwrap();
    assert!(initialized.success());
    std::fs::write(project.path().join("note.txt"), "existing user edit\n").unwrap();
    let call = json!({"role":"assistant","tool_calls":[{"index":0,"id":"live-shell","type":"function","function":{"name":"shell_run","arguments":json!({
        "command":"echo 'starting work'; sleep 1; echo 'updated by tool' > note.txt; echo 'done'",
        "workingDirectory":project.path(),
    }).to_string()}}]});
    let (url, server) = fixture(vec![
        response(call, "tool_calls"),
        response(json!({"content":"Updated the note."}), "stop"),
    ])
    .await;
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
    configure(&engine, url, false).await;
    engine
        .save_project(Project {
            id: "live-project".into(),
            name: "Live project".into(),
            directory: project.path().to_string_lossy().into(),
            description: String::new(),
        })
        .unwrap();
    let mut orbit = engine.orbit("assistant").unwrap();
    orbit.autonomy = "trusted".into();
    orbit.project_ids.push("live-project".into());
    engine.save_orbit(orbit).unwrap();
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Live tool".into(),
            None,
        )
        .unwrap();
    let task = engine
        .send_message(&chat.id, "Update the note.".into())
        .unwrap();
    let running_id = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let messages = engine.messages(&chat.id).unwrap();
            if let Some(message) = messages.iter().find(|message| {
                message.tool_activity.as_ref().is_some_and(|activity| {
                    activity.status == "running" && activity.output.contains("starting work")
                })
            }) {
                break message.id.clone();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    finished(&engine, &task.id).await;
    let messages = engine.messages(&chat.id).unwrap();
    let activities: Vec<_> = messages
        .iter()
        .filter(|message| message.tool_activity.is_some())
        .collect();
    assert_eq!(activities.len(), 1);
    assert_eq!(activities[0].id, running_id);
    let activity = activities[0].tool_activity.as_ref().unwrap();
    assert_eq!(activity.status, "completed");
    assert_eq!(activity.exit_code, Some(0));
    assert!(activity.output.contains("starting work\ndone"));
    assert_eq!(activity.diffs.len(), 1);
    assert!(activity.diffs[0].path.ends_with("note.txt"));
    assert!(activity.diffs[0].patch.contains("-existing user edit"));
    assert!(activity.diffs[0].patch.contains("+updated by tool"));
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    let tool_result = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    assert!(tool_result.to_string().contains("exitCode"));
    assert!(!tool_result.to_string().contains("toolActivity"));
    engine.shutdown().await;
}

#[tokio::test]
async fn selected_model_expands_raw_profile_on_create_and_edit_before_chat() {
    let initial_instructions = "GENERATION_ONE_OBSOLETE_PROMPT";
    let updated_instructions = "GENERATION_TWO_LIVE_PROMPT";
    let (url, server) = fixture(vec![
        response(
            json!({"content":json!({
                "role":"Research companion",
                "instructions":initial_instructions,
            }).to_string()}),
            "stop",
        ),
        response(
            json!({"content":json!({
                "role":"Suggested science specialty",
                "instructions":updated_instructions,
            }).to_string()}),
            "stop",
        ),
        response(json!({"content":"Ready to review your evidence."}), "stop"),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
    configure(&engine, url, false).await;
    let mut provider = engine.provider("fixture").unwrap();
    provider.models.push("edited-model".into());
    engine
        .save_provider(ProviderDraft {
            profile: provider,
            api_key: None,
        })
        .unwrap();

    let mut draft = engine.orbit("assistant").unwrap();
    draft.id = "profile-expansion-fixture".into();
    draft.name = "Nova".into();
    draft.gender = "they/them".into();
    draft.personality = "Quiet and practical".into();
    draft.tasks = "Compare public observatory research.".into();
    draft.instructions = "CLIENT_SUPPLIED_INITIAL_INSTRUCTIONS".into();
    let created = engine
        .create_orbit(draft.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(created.tasks, draft.tasks);
    assert_eq!(created.instructions, initial_instructions);

    let mut edit = created;
    edit.name = "Aster".into();
    edit.gender = "he/him".into();
    edit.role = "Science editor".into();
    edit.personality = "Warm, skeptical, and precise".into();
    edit.tasks = "Review laboratory evidence and cite reproducible findings.".into();
    edit.model = "edited-model".into();
    edit.instructions = "CLIENT_SUPPLIED_STALE_INSTRUCTIONS".into();
    let updated = engine
        .update_orbit_profile(edit.clone(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(updated.name, edit.name);
    assert_eq!(updated.gender, edit.gender);
    assert_eq!(updated.personality, edit.personality);
    assert_eq!(updated.role, edit.role);
    assert_eq!(updated.tasks, edit.tasks);
    assert_eq!(updated.model, edit.model);
    assert_eq!(updated.instructions, updated_instructions);
    let stored = engine.orbit(&edit.id).unwrap();
    assert_eq!(stored.instructions, updated_instructions);
    assert_eq!(stored.tasks, edit.tasks);

    let chat = engine
        .create_conversation(
            vec![edit.id.clone()],
            edit.id.clone(),
            "Edited profile".into(),
            None,
        )
        .unwrap();
    let task = engine
        .send_message(
            &chat.id,
            "Describe how you will review the evidence.".into(),
        )
        .unwrap();
    finished(&engine, &task.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0]["model"], "test-model");
    assert_eq!(requests[1]["model"], "edited-model");
    assert_eq!(requests[2]["model"], "edited-model");
    for request in &requests[..2] {
        assert!(
            request.get("tools").is_none_or(|tools| {
                tools.is_null() || tools.as_array().is_some_and(Vec::is_empty)
            }),
            "profile expansion must not advertise executable tools"
        );
    }
    let creation_profile = requests[0]["messages"].to_string();
    assert!(creation_profile.contains(&draft.name));
    assert!(creation_profile.contains(&draft.gender));
    assert!(creation_profile.contains(&draft.personality));
    assert!(creation_profile.contains(&draft.tasks));
    assert!(!creation_profile.contains(&draft.instructions));
    let edit_profile = requests[1]["messages"].to_string();
    for value in [
        &edit.name,
        &edit.gender,
        &edit.role,
        &edit.personality,
        &edit.tasks,
    ] {
        assert!(edit_profile.contains(value));
    }
    for obsolete in [initial_instructions, &draft.tasks, &edit.instructions] {
        assert!(!edit_profile.contains(obsolete));
    }
    let chat_prompt = requests[2]["messages"].to_string();
    assert!(chat_prompt.contains(updated_instructions));
    assert!(chat_prompt.contains(&edit.tasks));
    assert!(chat_prompt.contains(&edit.personality));
    assert!(!chat_prompt.contains(initial_instructions));
    assert!(!chat_prompt.contains(&draft.tasks));
    assert!(!chat_prompt.contains(&edit.instructions));
    assert!(requests[2]["tools"].as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|tool| tool["function"]["name"] == "shell_run")
    }));
    engine.shutdown().await;
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
    common::add_assistant(&engine);
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
    common::add_assistant(&engine);
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
async fn denial_or_revoked_project_never_executes_in_a_rig_run() {
    for revoke in [false, true] {
        let call = json!({"role":"assistant","tool_calls":[{"index":0,"id":"denied-memory","type":"function","function":{"name":"memory_save","arguments":"{\"content\":\"Must not be saved\"}"}}]});
        let (url, server) = fixture(vec![
            response(call, "tool_calls"),
            response(json!({"content":"The action was not allowed."}), "stop"),
        ])
        .await;
        let directory = tempfile::tempdir().unwrap();
        let engine = Engine::open(directory.path().into()).unwrap();
        common::add_assistant(&engine);
        configure(&engine, url, true).await;
        let project_folder = tempfile::tempdir().unwrap();
        if revoke {
            engine
                .save_project(Project {
                    id: "revoked-project".into(),
                    name: "Project".into(),
                    directory: project_folder.path().to_string_lossy().into(),
                    description: String::new(),
                })
                .unwrap();
            let mut orbit = engine.orbit("assistant").unwrap();
            orbit.project_ids = vec!["revoked-project".into()];
            engine.save_orbit(orbit).unwrap();
        }
        let chat = engine
            .create_conversation(
                vec!["assistant".into()],
                "assistant".into(),
                "Denied".into(),
                None,
            )
            .unwrap();
        let task = engine.send_message(&chat.id, "Save a fact".into()).unwrap();
        let approval = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(approval) = engine.bootstrap().unwrap().approvals.first().cloned() {
                    break approval;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        if revoke {
            let mut orbit = engine.orbit("assistant").unwrap();
            orbit.project_ids.clear();
            engine.save_orbit(orbit).unwrap();
        }
        if revoke {
            // Saving a less-permissive profile deliberately cancels active work.
            // A later request must see the interrupted call/result pair, not replay it.
            tokio::time::timeout(Duration::from_secs(10), async {
                loop {
                    if engine
                        .bootstrap()
                        .unwrap()
                        .tasks
                        .iter()
                        .any(|item| item.id == task.id && item.status == "cancelled")
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let followup = engine
                .send_message(&chat.id, "Continue after the permission change".into())
                .unwrap();
            finished(&engine, &followup.id).await;
        } else {
            engine.resolve_approval(&approval.id, false).unwrap();
            finished(&engine, &task.id).await;
        }
        let requests = tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
        let results: Vec<_> = requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "tool")
            .collect();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0]["tool_call_id"], "denied-memory");
        let content = results[0]["content"].as_str().unwrap();
        assert!(if revoke {
            content.contains("not replayed")
        } else {
            content.contains("Permission")
        });
        assert!(engine.memories(Some("assistant"), None).unwrap().is_empty());
        assert!(engine.bootstrap().unwrap().approvals.is_empty());
        engine.shutdown().await;
    }
}

#[tokio::test]
async fn rig_retries_a_transient_model_failure_without_duplicating_the_request() {
    let (url, server) = fixture_with_status(vec![
        (503, "temporary upstream failure".into()),
        (
            200,
            response(json!({"content":"The request succeeded."}), "stop"),
        ),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
    configure(&engine, url, false).await;
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Retry".into(),
            None,
        )
        .unwrap();
    let task = engine
        .send_message(&chat.id, "Do this once".into())
        .unwrap();
    finished(&engine, &task.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0], requests[1],
        "a transport retry must reuse the pending model request"
    );
    assert_eq!(
        requests[1]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|message| message["role"] == "user" && message["content"] == "Do this once")
            .count(),
        1
    );
    let messages = engine.messages(&chat.id).unwrap();
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == "user")
            .count(),
        1
    );
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.role == "assistant"
                && message.content == "The request succeeded.")
            .count(),
        1
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn rig_group_delegation_keeps_private_history_and_releases_the_model_slot() {
    let delegate = json!({"role":"assistant","tool_calls":[{"index":0,"id":"delegate-call","type":"function","function":{"name":"orbit_delegate","arguments":"{\"orbitId\":\"researcher\",\"task\":\"Remember a private finding and report it\"}"}}]});
    let memory = json!({"role":"assistant","tool_calls":[{"index":0,"id":"child-memory","type":"function","function":{"name":"memory_save","arguments":"{\"content\":\"Private researcher finding\"}"}}]});
    let (url, server) = fixture(vec![
        response(delegate, "tool_calls"),
        response(memory, "tool_calls"),
        response(json!({"content":"The research is complete."}), "stop"),
        response(json!({"content":"I have the researcher's result."}), "stop"),
        response(
            json!({"content":"Continuing the lead conversation."}),
            "stop",
        ),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
    configure(&engine, url, false).await;
    let mut child = engine.orbit("assistant").unwrap();
    child.id = "researcher".into();
    child.name = "Lyra".into();
    engine.save_orbit(child).unwrap();
    let mut settings = engine.settings().unwrap();
    settings.max_concurrent = 1;
    engine.save_settings(settings).unwrap();
    let chat = engine
        .create_conversation(
            vec!["assistant".into(), "researcher".into()],
            "assistant".into(),
            "Research group".into(),
            None,
        )
        .unwrap();
    let first = engine
        .send_message(&chat.id, "Ask Lyra to research this".into())
        .unwrap();
    finished(&engine, &first.id).await;
    assert!(engine.memories(Some("assistant"), None).unwrap().is_empty());
    assert_eq!(engine.memories(Some("researcher"), None).unwrap().len(), 1);
    let tasks = engine.bootstrap().unwrap().tasks;
    assert_eq!(tasks.len(), 2);
    assert!(tasks.iter().all(|task| task.status == "completed"));
    let second = engine.send_message(&chat.id, "Continue".into()).unwrap();
    finished(&engine, &second.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requests.len(), 5);
    let followup = requests[4]["messages"].to_string();
    assert!(followup.contains("delegate-call"));
    assert!(followup.contains("The research is complete."));
    assert!(
        !followup.contains("child-memory"),
        "child transcript must not overwrite the lead's history"
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn all_tools_and_assigned_project_folders_work_in_an_individual_chat() {
    let project_folder = tempfile::tempdir().unwrap();
    std::fs::write(
        project_folder.path().join("notes.txt"),
        "Assigned project content",
    )
    .unwrap();
    let calls = json!({"role":"assistant","tool_calls":[
        {"index":0,"id":"list-projects","type":"function","function":{"name":"project_list","arguments":"{}"}},
        {"index":1,"id":"read-project","type":"function","function":{"name":"files_read","arguments":json!({"path":project_folder.path().join("notes.txt")}).to_string()}},
        {"index":2,"id":"shell-project","type":"function","function":{"name":"shell_run","arguments":json!({"command":"pwd","workingDirectory":project_folder.path()}).to_string()}}
    ]});
    let (url, server) = fixture(vec![
        response(calls, "tool_calls"),
        response(json!({"content":"The project is accessible."}), "stop"),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
    configure(&engine, url, false).await;
    engine
        .save_project(Project {
            id: "assigned-project".into(),
            name: "Assigned project".into(),
            directory: project_folder.path().to_string_lossy().into(),
            description: "Fixture project".into(),
        })
        .unwrap();
    let mut orbit = engine.orbit("assistant").unwrap();
    orbit.tool_grants.clear(); // Legacy profiles must not hide any tool categories.
    orbit.project_ids = vec!["assigned-project".into()];
    orbit.autonomy = "trusted".into();
    engine.save_orbit(orbit).unwrap();
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Project access".into(),
            None,
        )
        .unwrap();
    let task = engine
        .send_message(&chat.id, "Read my project and run pwd there".into())
        .unwrap();
    finished(&engine, &task.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    let tool_names: Vec<_> = requests[0]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect();
    for name in [
        "shell_run",
        "files_read",
        "project_list",
        "screen_capture",
        "computer_click",
        "web_fetch",
        "orbit_delegate",
    ] {
        assert!(tool_names.contains(&name), "missing advertised tool {name}");
    }
    let messages = requests[0]["messages"].to_string();
    assert!(messages.contains("assigned-project"));
    assert!(messages.contains(&project_folder.path().to_string_lossy().to_string()));
    let results = requests[1]["messages"].as_array().unwrap();
    let result = |id: &str| {
        results
            .iter()
            .find(|message| message["role"] == "tool" && message["tool_call_id"] == id)
            .unwrap()["content"]
            .as_str()
            .unwrap()
    };
    assert!(result("list-projects").contains("assigned-project"));
    assert!(result("read-project").contains("Assigned project content"));
    assert!(
        result("shell-project").contains(
            &project_folder
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .to_string()
        )
    );
    assert!(engine.bootstrap().unwrap().approvals.is_empty());
    engine.shutdown().await;
}

#[tokio::test]
async fn rig_rejects_invalid_tool_batches_then_allows_a_corrected_retry() {
    let calls = json!({"role":"assistant","tool_calls":[
        {"index":0,"id":"valid-memory","type":"function","function":{"name":"memory_save","arguments":"{\"content\":\"Do not execute invalid batch\"}"}},
        {"index":1,"id":"invented-call","type":"function","function":{"name":"invented_tool","arguments":"{}"}}
    ]});
    let (url, server) = fixture(vec![
        response(calls, "tool_calls"),
        response(json!({"role":"assistant","tool_calls":[{"index":0,"id":"retry-memory","type":"function","function":{"name":"memory_save","arguments":"{\"content\":\"A valid fact\"}"}}]}), "tool_calls"),
        response(json!({"content":"I used the available tool."}), "stop"),
    ]).await;
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
    configure(&engine, url, false).await;
    let chat = engine
        .create_conversation(
            vec!["assistant".into()],
            "assistant".into(),
            "Invalid tool".into(),
            None,
        )
        .unwrap();
    let task = engine
        .send_message(&chat.id, "Save the fact".into())
        .unwrap();
    finished(&engine, &task.id).await;
    let requests = tokio::time::timeout(Duration::from_secs(2), server)
        .await
        .unwrap()
        .unwrap();
    let results: Vec<_> = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .collect();
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|message| {
        message["tool_call_id"] == "valid-memory"
            && message["content"]
                .as_str()
                .unwrap()
                .contains("not executed")
    }));
    assert!(
        results
            .iter()
            .any(|message| message["tool_call_id"] == "invented-call"
                && message["content"].as_str().unwrap().contains("[ERROR]"))
    );
    assert!(
        requests[2]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|message| message["role"] == "tool" && message["tool_call_id"] == "retry-memory")
    );
    let memories = engine.memories(Some("assistant"), None).unwrap();
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].content, "A valid fact");
    engine.shutdown().await;
}

#[tokio::test]
async fn disabled_and_deleted_orbits_do_not_leave_failure_alerts() {
    let directory = tempfile::tempdir().unwrap();
    let engine = Engine::open(directory.path().into()).unwrap();
    common::add_assistant(&engine);
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
    common::add_assistant(&engine);
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
