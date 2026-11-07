use super::*;

#[test]
fn truncates_large_tool_results_for_provider_request() {
    let prefix = "start:";
    let suffix = ":end";
    let middle = "x".repeat(MAX_TOOL_RESULT_CHARS_FOR_PROVIDER_REQUEST + 1_000);
    let mut message = ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::ToolResult {
            tool_use_id: "toolu_1".to_string(),
            content: format!("{prefix}{middle}{suffix}"),
            is_error: false,
        },
    };

    message.truncate_tool_results_for_provider_request();

    let MessageContent::ToolResult { content, .. } = message.content else {
        panic!("expected tool result");
    };
    assert!(content.chars().count() <= MAX_TOOL_RESULT_CHARS_FOR_PROVIDER_REQUEST);
    assert!(content.starts_with(prefix));
    assert!(content.ends_with(suffix));
    assert!(content.contains("tool result truncated"));
}

#[test]
fn context_aware_tool_result_limit_preserves_the_original_message() {
    let output = format!("start:{}:end", "x".repeat(20_000));
    let mut message = ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::ToolResult {
            tool_use_id: "toolu_1".to_string(),
            content: output.clone(),
            is_error: false,
        },
    };

    message.truncate_tool_results_for_provider_request_with_context(Some(8_000));

    let MessageContent::ToolResult { content, .. } = message.content else {
        panic!("expected tool result");
    };
    assert!(content.chars().count() <= provider_output_char_limit(Some(8_000)));
    assert!(content.contains("tool result truncated"));
    assert!(content.starts_with("start:"));
    assert!(content.ends_with(":end"));
    assert_eq!(output.chars().count(), 20_010);
}

#[test]
fn terminal_output_bound_discloses_that_full_output_remains_in_terminal() {
    let output = format!("first-line\n{}\nlast-line", "x".repeat(20_000));

    let bounded = bounded_terminal_output_for_provider_request(&output, Some(8_000));

    assert!(bounded.chars().count() <= provider_output_char_limit(Some(8_000)));
    assert!(bounded.contains("terminal output truncated for model context"));
    assert!(bounded.contains("full output remains available in the terminal"));
    assert!(bounded.starts_with("first-line"));
    assert!(bounded.ends_with("last-line"));
    assert_eq!(output.chars().count(), 20_021);
}

#[test]
fn terminal_output_bound_respects_character_budget_for_multibyte_text() {
    let output = format!("first:{}:last", "界".repeat(20_000));

    let bounded = bounded_terminal_output_for_provider_request(&output, Some(8_000));

    assert!(bounded.chars().count() <= provider_output_char_limit(Some(8_000)));
    assert!(bounded.starts_with("first:"));
    assert!(bounded.ends_with(":last"));
    assert!(bounded.contains("terminal output truncated for model context"));
}

#[test]
fn tagged_terminal_output_is_bounded_without_mutating_surrounding_text() {
    let output = format!("first:{}:last", "x".repeat(20_000));
    let mut message = ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::Text(format!(
            "[Running command: verbose]\n[Terminal output:\n{output}\n]\nContinue the task"
        )),
    };

    message.truncate_tagged_terminal_output_for_provider_request_with_context(Some(8_000));
    let once = message.clone();
    message.truncate_tagged_terminal_output_for_provider_request_with_context(Some(8_000));

    assert_eq!(message, once);
    let MessageContent::Text(content) = message.content else {
        panic!("expected text message");
    };
    assert!(content.starts_with("[Running command: verbose]"));
    assert!(content.ends_with("Continue the task"));
    assert!(content.contains("terminal output truncated for model context"));
    assert!(content.contains("first:"));
    assert!(content.contains(":last"));
    assert!(!content.contains(&output));
}

#[test]
fn ambiguous_tagged_terminal_output_bounds_the_whole_provider_message() {
    let output = format!("first\n]\n{}\n]\nlast", "x".repeat(20_000));
    let mut message = ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::Text(format!("[Terminal output:\n{output}\n]\nContinue the task")),
    };

    message.truncate_tagged_terminal_output_for_provider_request_with_context(Some(8_000));

    let MessageContent::Text(content) = message.content else {
        panic!("expected text message");
    };
    assert!(content.chars().count() <= provider_output_char_limit(Some(8_000)));
    assert!(content.contains("message text truncated for model context"));
    assert!(content.contains("full content remains in conversation history"));
    assert!(!content.contains(&output));
}

#[test]
fn terminal_output_bound_keeps_a_disclosure_for_tiny_budgets() {
    let bounded = bounded_terminal_output_for_provider_request(&"x".repeat(200), Some(4));

    assert_eq!(bounded, "[terminal output truncated]");
    assert_eq!(bounded.chars().count(), provider_output_char_limit(Some(4)));
    assert!(!bounded.contains(&"x".repeat(200)));
}

#[test]
fn usage_total_combines_disjoint_cache_and_miss_buckets() {
    let usage = Usage {
        input_tokens: 10,
        output_tokens: 25,
        cached_input_tokens: 80,
        cache_creation_input_tokens: 10,
    };

    assert_eq!(usage.total_tokens(), 125);
}

#[test]
fn tool_events_keep_one_call_id_across_permission_and_execution() {
    let events = [
        ToolEvent::Proposed {
            call: ToolCall {
                id: "call-1".to_string(),
                name: "run_shell_command".to_string(),
                arguments: serde_json::json!({"command": "cargo test"}),
            },
        },
        ToolEvent::PermissionRequested {
            request: PermissionRequest {
                id: "permission:call-1".to_string(),
                call_id: "call-1".to_string(),
                kind: PermissionKind::Execute,
                reason: Some("Run a command".to_string()),
            },
        },
        ToolEvent::PermissionResolved {
            request_id: "permission:call-1".to_string(),
            call_id: "call-1".to_string(),
            decision: PermissionDecision::AllowOnce,
        },
        ToolEvent::Started {
            call_id: "call-1".to_string(),
        },
        ToolEvent::Completed {
            result: ToolResult {
                images: Vec::new(),
                call_id: "call-1".to_string(),
                content: "ok".to_string(),
                status: ToolResultStatus::Success,
            },
        },
    ];

    assert!(events.iter().all(|event| event.call_id() == "call-1"));
}

#[test]
fn denied_results_are_errors_but_cancelled_results_are_distinct() {
    let denied = ToolResult {
        images: Vec::new(),
        call_id: "denied".to_string(),
        content: "permission denied".to_string(),
        status: ToolResultStatus::Denied,
    };
    let cancelled = ToolResult {
        images: Vec::new(),
        call_id: "cancelled".to_string(),
        content: "cancelled".to_string(),
        status: ToolResultStatus::Cancelled,
    };

    assert!(denied.is_error());
    assert!(!cancelled.is_error());
}

#[test]
fn session_runtime_prompt_is_distinct_from_provider_history() {
    let request = TurnRequest::new("acp:codex", Vec::new())
        .with_prompt(MessageContent::Text("inspect the workspace".to_owned()));

    assert_eq!(
        request.prompt,
        Some(MessageContent::Text("inspect the workspace".to_owned()))
    );
    assert!(request.messages.is_empty());
}
