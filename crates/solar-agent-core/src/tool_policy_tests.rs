use super::*;
use crate::MessageRole;

fn definition(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: String::new(),
        input_schema: serde_json::json!({"type": "object"}),
    }
}

fn tool_exchange(id: &str, name: &str, input: JsonValue, result: &str) -> Vec<ConversationMessage> {
    vec![
        ConversationMessage {
            role: MessageRole::Assistant,
            content: MessageContent::ToolUse {
                tool_use_id: id.to_string(),
                name: name.to_string(),
                input,
            },
        },
        ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::ToolResult {
                tool_use_id: id.to_string(),
                content: result.to_string(),
                is_error: false,
            },
        },
    ]
}

#[test]
fn executes_only_tools_advertised_for_this_turn() {
    let policy = ToolPolicy::new(&[definition("read_files")]);
    let read = ToolCall {
        id: "read-1".to_string(),
        name: "read_files".to_string(),
        arguments: serde_json::json!({"files": ["Cargo.toml"]}),
    };
    let shell = ToolCall {
        id: "shell-1".to_string(),
        name: "run_shell_command".to_string(),
        arguments: serde_json::json!({"command": "pwd"}),
    };

    assert_eq!(policy.decide(&read, &[], &[]), ToolCallDecision::Execute);
    let ToolCallDecision::Reject(result) = policy.decide(&shell, &[], &[]) else {
        panic!("unadvertised tool should be rejected");
    };
    assert_eq!(result.call_id, "shell-1");
    assert_eq!(result.status, ToolResultStatus::Error);
    assert!(result.content.contains("read_files"));
    assert!(
        !result
            .content
            .contains("available tools are: run_shell_command")
    );
}

#[test]
fn task_updates_are_inline_only_when_advertised_and_keep_the_call_identity() {
    let call = ToolCall {
        id: "task-update-1".to_string(),
        name: UPDATE_TASK_LIST_NAME.to_string(),
        arguments: serde_json::json!({"tasks": [
            {"id": "fix", "title": "Fix the failure", "status": "in_progress"},
            {"id": "review", "title": "Review the fix", "status": "pending"},
        ]}),
    };
    let unadvertised = ToolPolicy::new(&[definition("read_files")]);
    let ToolCallDecision::Reject(result) = unadvertised.decide(&call, &[], &[]) else {
        panic!("task updates cannot bypass advertised capabilities");
    };
    assert_eq!(result.call_id, call.id);
    assert_eq!(result.status, ToolResultStatus::Error);

    let policy = ToolPolicy::new(&[definition(UPDATE_TASK_LIST_NAME)]);
    let ToolCallDecision::Inline(result) = policy.decide(&call, &[], &[]) else {
        panic!("valid task updates must not dispatch external actions");
    };
    assert_eq!(result.call_id, call.id);
    assert_eq!(result.status, ToolResultStatus::Success);
    let snapshot = crate::task_list_from_tool_result(&result.content).unwrap();
    assert_eq!(snapshot.tasks[0].id, "fix");
    assert_eq!(snapshot.tasks[1].id, "review");
}

#[test]
fn malformed_task_updates_return_one_correlated_error_instead_of_executing() {
    let policy = ToolPolicy::new(&[definition(UPDATE_TASK_LIST_NAME)]);
    for arguments in [
        serde_json::json!({"tasks": []}),
        serde_json::json!({"tasks": [
            {"id": "same", "title": "First", "status": "completed"},
            {"id": "same", "title": "Second", "status": "pending"},
        ]}),
        serde_json::json!({"tasks": [
            {"id": "one", "title": "First", "status": "invented"},
        ]}),
    ] {
        let call = ToolCall {
            id: "invalid-task-update".to_string(),
            name: UPDATE_TASK_LIST_NAME.to_string(),
            arguments,
        };
        let ToolCallDecision::Reject(result) = policy.decide(&call, &[], &[]) else {
            panic!("invalid task updates must not execute or succeed");
        };
        assert_eq!(result.call_id, call.id);
        assert_eq!(result.status, ToolResultStatus::Error);
        assert!(result.content.starts_with("Error: "));
        assert_eq!(crate::task_list_from_tool_result(&result.content), None);
    }
}

#[test]
fn recall_searches_archived_and_live_results_with_live_results_most_recent() {
    let policy = ToolPolicy::new(&[definition(RECALL_TOOL_HISTORY_NAME)]);
    let archive = tool_exchange(
        "archived-read",
        "read_files",
        serde_json::json!({"files": ["old.txt"]}),
        "old contents",
    );
    let messages = tool_exchange(
        "live-read",
        "read_files",
        serde_json::json!({"files": ["new.txt"]}),
        "new contents",
    );
    let latest = ToolCall {
        id: "recall-latest".to_string(),
        name: RECALL_TOOL_HISTORY_NAME.to_string(),
        arguments: serde_json::json!({"tool_name": "read_files"}),
    };
    let previous = ToolCall {
        id: "recall-previous".to_string(),
        name: RECALL_TOOL_HISTORY_NAME.to_string(),
        arguments: serde_json::json!({
            "tool_name": "read_files",
            "offset_from_end": 1,
        }),
    };

    let ToolCallDecision::Inline(latest_result) = policy.decide(&latest, &messages, &archive)
    else {
        panic!("recall should execute inline");
    };
    assert!(latest_result.content.contains("Tool Use ID: live-read"));
    assert!(latest_result.content.contains("new contents"));

    let ToolCallDecision::Inline(previous_result) = policy.decide(&previous, &messages, &archive)
    else {
        panic!("recall should execute inline");
    };
    assert!(
        previous_result
            .content
            .contains("Tool Use ID: archived-read")
    );
    assert!(previous_result.content.contains("old contents"));
}

#[test]
fn recall_supports_exact_call_id_and_case_insensitive_text_search() {
    let messages = tool_exchange(
        "shell-7",
        "run_shell_command",
        serde_json::json!({"command": "cargo test"}),
        "ALL TESTS PASSED",
    );

    let exact = recall_tool_history(
        &messages,
        &[],
        ToolHistoryQuery {
            tool_use_id: "shell-7",
            search_query: "all tests",
            ..Default::default()
        },
    );
    let missing = recall_tool_history(
        &messages,
        &[],
        ToolHistoryQuery {
            tool_use_id: "missing",
            ..Default::default()
        },
    );

    assert!(exact.contains("cargo test"));
    assert!(exact.contains("ALL TESTS PASSED"));
    assert_eq!(
        missing,
        "No matching tool calls found in conversation history."
    );
}

#[test]
fn recall_never_returns_a_prior_recall_call() {
    let messages = tool_exchange(
        "recall-1",
        RECALL_TOOL_HISTORY_NAME,
        serde_json::json!({}),
        "a prior synthetic recall result",
    );

    assert_eq!(
        recall_tool_history(&messages, &[], ToolHistoryQuery::default()),
        "No matching tool calls found in conversation history."
    );
}

fn screenshot_exchange(id: &str, bytes: Vec<u8>) -> Vec<ConversationMessage> {
    let mut messages = tool_exchange(id, "use_computer", serde_json::json!({}), "captured");
    messages[1].content =
        MessageContent::MultiPart(ToolResult::into_batch_content_parts(vec![ToolResult {
            call_id: id.to_string(),
            content: "captured".to_string(),
            status: ToolResultStatus::Success,
            images: vec![ToolResultImage {
                data: bytes,
                mime_type: "image/png".to_string(),
            }],
        }]));
    messages
}

#[test]
fn recall_attaches_only_the_exact_selected_screenshots_from_archived_or_live_results() {
    let archive = screenshot_exchange("capture", vec![1, 2]);
    let mut messages = screenshot_exchange("capture-more", vec![3, 4]);
    messages.push(ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::MultiPart(vec![
            ContentPart::Text("Images returned by tool call capture:".to_string()),
            ContentPart::Image {
                data: vec![99],
                mime_type: "image/png".to_string(),
            },
        ]),
    });
    let policy = ToolPolicy::new(&[definition(RECALL_TOOL_HISTORY_NAME)]);
    for (arguments, original_id, bytes) in [
        (
            serde_json::json!({"tool_use_id": "capture"}),
            "capture",
            vec![1, 2],
        ),
        (
            serde_json::json!({"tool_name": "use_computer"}),
            "capture-more",
            vec![3, 4],
        ),
        (
            serde_json::json!({"tool_name": "use_computer", "offset_from_end": 1}),
            "capture",
            vec![1, 2],
        ),
    ] {
        let call = ToolCall {
            id: "recall-image".to_string(),
            name: RECALL_TOOL_HISTORY_NAME.to_string(),
            arguments,
        };
        let ToolCallDecision::Inline(result) = policy.decide(&call, &messages, &archive) else {
            panic!("recall should execute inline");
        };
        assert_eq!(result.call_id, call.id);
        assert!(
            result
                .content
                .contains(&format!("Tool Use ID: {original_id}\n"))
        );
        assert_eq!(
            result.images,
            vec![ToolResultImage {
                data: bytes,
                mime_type: "image/png".to_string(),
            }]
        );
        let parts = ToolResult::into_batch_content_parts(vec![result]);
        assert!(matches!(
            &parts[0], ContentPart::ToolResult { tool_use_id, .. } if tool_use_id == "recall-image"
        ));
        assert!(matches!(
            &parts[1], ContentPart::Text(text) if text == "Images returned by tool call recall-image:"
        ));
        assert!(matches!(&parts[2], ContentPart::Image { .. }));
    }
    let call = ToolCall {
        id: "missing".to_string(),
        name: RECALL_TOOL_HISTORY_NAME.to_string(),
        arguments: serde_json::json!({"tool_use_id": "capture-missing"}),
    };
    let ToolCallDecision::Inline(result) = policy.decide(&call, &messages, &archive) else {
        panic!("missing recall should execute inline");
    };
    assert!(result.images.is_empty());
    assert_eq!(
        result.content,
        "No matching tool calls found in conversation history."
    );
}

#[test]
fn recall_does_not_reattach_images_from_an_earlier_recall_result() {
    let mut messages = screenshot_exchange("recall-previous", vec![99]);
    let MessageContent::ToolUse { name, .. } = &mut messages[0].content else {
        panic!("expected tool call");
    };
    *name = RECALL_TOOL_HISTORY_NAME.to_string();
    let policy = ToolPolicy::new(&[definition(RECALL_TOOL_HISTORY_NAME)]);
    let call = ToolCall {
        id: "recall-now".to_string(),
        name: RECALL_TOOL_HISTORY_NAME.to_string(),
        arguments: serde_json::json!({}),
    };
    let ToolCallDecision::Inline(result) = policy.decide(&call, &messages, &[]) else {
        panic!("recall should execute inline");
    };
    assert!(result.images.is_empty());
    assert_eq!(
        result.content,
        "No matching tool calls found in conversation history."
    );
}

#[test]
fn loop_guard_detects_repeated_failures_and_resets_after_detection() {
    let mut guard = ToolLoopGuard::new(5, 3);
    guard.record_failure(7, "cargo test failed");
    guard.record_failure(11, "another command failed");
    guard.record_failure(7, "cargo test failed again");
    assert_eq!(guard.detect_and_reset(), None);

    guard.record_failure(7, "cargo test failed a third time");
    assert_eq!(
        guard.detect_and_reset(),
        Some(ToolLoopDetected {
            description: "cargo test failed a third time".to_string(),
            threshold: 3,
        })
    );
    assert_eq!(guard.detect_and_reset(), None);
}

#[test]
fn loop_guard_clears_failures_when_a_tool_makes_progress() {
    let mut guard = ToolLoopGuard::new(5, 2);
    guard.record_failure(7, "first failure");
    guard.record_success();
    guard.record_failure(7, "failure after success");

    assert_eq!(guard.detect_and_reset(), None);
}
