use serde_json::json;

use super::*;
use crate::{AgentErrorKind, PermissionKind, TaskListItem, TaskListStatus, ToolResultImage};

fn initial_messages() -> Vec<ConversationMessage> {
    vec![ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::Text("Finish the task".to_string()),
    }]
}

fn run_with_limits(limits: ProviderRunLimits) -> ProviderRun {
    ProviderRun::new("run-1", initial_messages(), "base", limits)
}

fn run() -> ProviderRun {
    run_with_limits(ProviderRunLimits::default())
}

#[test]
fn ready_profile_can_change_without_replaying_or_adding_history() {
    let mut run = run();
    let original_transcript = run.transcript().to_vec();
    run.set_ready_profile("computer-use").unwrap();

    assert_eq!(run.profile().as_str(), "computer-use");
    assert_eq!(run.transcript(), original_transcript);
    assert_eq!(next_model_call(&mut run).profile.as_str(), "computer-use");
    assert!(run.set_ready_profile("base").is_err());
}

fn next_model_call(run: &mut ProviderRun) -> ProviderModelCall {
    let Some(ProviderRunStep::CallModel(call)) = run.next_step().unwrap() else {
        panic!("expected model call");
    };
    call
}

fn text_turn(text: &str) -> CompletedModelTurn {
    CompletedModelTurn {
        assistant_content: vec![ContentPart::Text(text.to_string())],
        tool_calls: Vec::new(),
        usage: Usage {
            input_tokens: 10,
            output_tokens: 3,
            ..Usage::default()
        },
        stop_reason: StopReason::Completed,
        advertised_tools: BTreeSet::new(),
    }
}

fn tool_call(id: &str, name: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: json!({"id": id}),
    }
}

fn tool_turn(calls: Vec<ToolCall>, advertised_tools: &[&str]) -> CompletedModelTurn {
    CompletedModelTurn {
        assistant_content: vec![ContentPart::Text("I will use tools.".to_string())],
        tool_calls: calls,
        usage: Usage {
            input_tokens: 20,
            output_tokens: 5,
            cached_input_tokens: 4,
            ..Usage::default()
        },
        stop_reason: StopReason::Completed,
        advertised_tools: advertised_tools
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    }
}

fn accept_tool_turn(run: &mut ProviderRun, turn: CompletedModelTurn) -> PendingToolBatch {
    let model_call = next_model_call(run);
    run.accept_model_turn(&model_call.work_id, turn).unwrap();
    let Some(ProviderRunStep::DispatchTools(batch)) = run.next_step().unwrap() else {
        panic!("expected tool dispatch");
    };
    batch
}

fn successful_result(call_id: &str, content: &str) -> ToolResult {
    ToolResult {
        images: Vec::new(),
        call_id: call_id.to_string(),
        content: content.to_string(),
        status: ToolResultStatus::Success,
    }
}

fn assert_serialization_round_trip(run: &ProviderRun) {
    let json = serde_json::to_string(run).unwrap();
    let restored: ProviderRun = serde_json::from_str(&json).unwrap();
    assert_eq!(&restored, run);
}

fn mutate_run_json(run: &ProviderRun, mutate: impl FnOnce(&mut serde_json::Value)) -> ProviderRun {
    let mut value = serde_json::to_value(run).unwrap();
    mutate(&mut value);
    serde_json::from_value(value).unwrap()
}

fn restored_state_error(run: &ProviderRun) -> String {
    let ProviderRunProtocolError::InvalidRestoredState { message } =
        run.validate_restored_state().unwrap_err()
    else {
        panic!("expected restored-state validation error");
    };
    message
}

fn awaiting_tool_run() -> ProviderRun {
    let mut run = run();
    accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("first", "read_files"),
                tool_call("second", "grep"),
            ],
            &["read_files", "grep"],
        ),
    );
    run
}

#[test]
fn next_step_reemits_identical_pending_model_work() {
    let mut run = run();

    let first = run.next_step().unwrap();
    let second = run.next_step().unwrap();

    assert_eq!(first, second);
    assert_eq!(run.epoch(), RunEpoch::new(0));
    assert_serialization_round_trip(&run);
}

#[test]
fn stale_model_completion_is_rejected_without_mutation() {
    let mut run = run();
    let call = next_model_call(&mut run);
    let stale = ExternalWorkId {
        run_id: call.work_id.run_id.clone(),
        epoch: RunEpoch::new(call.work_id.epoch.get() + 1),
    };
    let before = run.clone();

    let error = run
        .accept_model_turn(&stale, text_turn("done"))
        .unwrap_err();

    assert!(matches!(
        error,
        ProviderRunProtocolError::WorkMismatch { .. }
    ));
    assert_eq!(run, before);
}

#[test]
fn model_retries_reuse_work_identity_and_stop_at_the_budget() {
    let mut run = run_with_limits(ProviderRunLimits {
        max_model_turns: Some(5),
        max_model_retries_per_turn: 1,
    });
    let call = next_model_call(&mut run);
    let mut recoverable = AgentError::new(AgentErrorKind::Transport, "network failed");
    recoverable.recoverable = true;

    assert_eq!(
        run.register_model_failure(&call.work_id, recoverable.clone())
            .unwrap(),
        ModelFailureDisposition::RetryScheduled
    );
    let retry = next_model_call(&mut run);
    assert_eq!(retry.work_id, call.work_id);
    assert_eq!(retry.retry_attempt, 1);
    assert_eq!(retry.messages, call.messages);
    assert_eq!(run.model_retries(), 1);

    assert_eq!(
        run.register_model_failure(&retry.work_id, recoverable)
            .unwrap(),
        ModelFailureDisposition::RunFailed
    );
    let Some(ProviderRunStep::Done(ProviderRunOutcome::Failed(failure))) = run.next_step().unwrap()
    else {
        panic!("expected failed run");
    };
    assert_eq!(failure.kind, ProviderRunFailureKind::RetryLimitExceeded);
}

#[test]
fn text_only_turn_requires_an_explicit_driver_decision() {
    let mut run = run();
    let call = next_model_call(&mut run);
    run.accept_model_turn(&call.work_id, text_turn("finished"))
        .unwrap();

    assert_eq!(run.next_step().unwrap(), None);
    let driver_work = run.active_work_id().unwrap().clone();
    assert_eq!(driver_work.epoch, RunEpoch::new(1));
    assert!(!run.is_terminal());

    run.complete(&driver_work).unwrap();
    assert_eq!(
        run.next_step().unwrap(),
        Some(ProviderRunStep::Done(ProviderRunOutcome::Completed(
            ProviderRunCompletion {
                stop_reason: StopReason::Completed,
            }
        )))
    );
}

#[test]
fn driver_continuation_appends_observation_switches_profile_and_advances_epoch() {
    let mut run = run();
    let call = next_model_call(&mut run);
    run.accept_model_turn(&call.work_id, text_turn("command is still running"))
        .unwrap();
    assert_eq!(run.next_step().unwrap(), None);
    let driver_work = run.active_work_id().unwrap().clone();

    run.continue_with_observation(
        &driver_work,
        MessageContent::Text("command exited with code 1".to_string()),
        "cli-monitor",
    )
    .unwrap();

    assert_eq!(run.epoch(), RunEpoch::new(2));
    assert_eq!(run.profile().as_str(), "cli-monitor");
    let next = next_model_call(&mut run);
    assert_eq!(next.work_id.epoch, RunEpoch::new(2));
    assert_eq!(
        next.messages.last(),
        Some(&ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("command exited with code 1".to_string()),
        })
    );
}

#[test]
fn ready_continuation_appends_observation_switches_profile_and_rejects_reuse() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![tool_call("shell", "run_shell_command")],
            &["run_shell_command"],
        ),
    );
    run.complete_tool(
        &batch.work_id,
        successful_result("shell", "command is still running"),
    )
    .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();
    let ready_work = run.ready_work_id().expect("ready work identity");

    run.continue_ready_with_observation(
        &ready_work,
        MessageContent::Text("Monitor command block-1.".to_string()),
        "cli-monitor",
    )
    .unwrap();

    assert_eq!(run.epoch(), RunEpoch::new(3));
    assert_eq!(run.profile().as_str(), "cli-monitor");
    assert!(matches!(
        run.transcript().last(),
        Some(ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text(text),
        }) if text == "Monitor command block-1."
    ));
    let continued = run.clone();
    assert!(matches!(
        run.continue_ready_with_observation(
            &ready_work,
            MessageContent::Text("duplicate".to_string()),
            "cli-monitor",
        )
        .unwrap_err(),
        ProviderRunProtocolError::WorkMismatch { .. }
    ));
    assert_eq!(run, continued);
}

#[test]
fn unknown_malformed_and_inline_tools_are_pre_resolved_in_the_same_batch() {
    let mut run = ProviderRun::new(
        "run-1",
        vec![
            ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::ToolUse {
                    tool_use_id: "old-read".to_string(),
                    name: "read_files".to_string(),
                    input: json!({"path": "old.txt"}),
                },
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::ToolResult {
                    tool_use_id: "old-read".to_string(),
                    content: "old contents".to_string(),
                    is_error: false,
                },
            },
        ],
        "base",
        ProviderRunLimits::default(),
    );
    let mut malformed = tool_call("malformed", "read_files");
    malformed.arguments = json!("not an object");
    let recall = ToolCall {
        id: "recall".to_string(),
        name: crate::RECALL_TOOL_HISTORY_NAME.to_string(),
        arguments: json!({"tool_use_id": "old-read"}),
    };
    let turn = tool_turn(
        vec![
            tool_call("external", "read_files"),
            tool_call("unknown", "invented_tool"),
            malformed,
            recall,
        ],
        &["read_files", crate::RECALL_TOOL_HISTORY_NAME],
    );

    let batch = accept_tool_turn(&mut run, turn);

    assert!(matches!(
        batch.calls[0].state,
        PendingToolCallState::Proposed
    ));
    for index in [1, 2, 3] {
        assert!(matches!(
            batch.calls[index].state,
            PendingToolCallState::Resolved { .. }
        ));
    }
    let recall_result = batch.calls[3].state.result().unwrap();
    assert!(recall_result.content.contains("old contents"));
    assert_eq!(batch.unresolved_call_ids(), vec!["external"]);
}

fn task_snapshot(status: TaskListStatus) -> TaskListSnapshot {
    TaskListSnapshot {
        tasks: vec![TaskListItem {
            id: "fix".to_string(),
            title: "Fix the failure".to_string(),
            status,
        }],
    }
}

fn update_task_call(id: &str, snapshot: &TaskListSnapshot) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: UPDATE_TASK_LIST_NAME.to_string(),
        arguments: serde_json::to_value(snapshot).unwrap(),
    }
}

#[test]
fn inline_task_updates_commit_atomically_in_call_order_with_other_tools() {
    let mut run = run();
    let initial = task_snapshot(TaskListStatus::Pending);
    let working = task_snapshot(TaskListStatus::InProgress);
    let completed = task_snapshot(TaskListStatus::Completed);
    run.set_task_list(Some(initial.clone())).unwrap();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                update_task_call("tasks-start", &working),
                tool_call("external", "read_files"),
                update_task_call("tasks-finish", &completed),
            ],
            &[UPDATE_TASK_LIST_NAME, "read_files"],
        ),
    );

    assert_eq!(batch.unresolved_call_ids(), vec!["external"]);
    assert_eq!(run.task_list(), Some(&initial));
    assert!(matches!(
        run.commit_tool_batch(&batch.work_id),
        Err(ProviderRunProtocolError::IncompleteToolBatch { .. }),
    ));
    assert_eq!(run.task_list(), Some(&initial));

    run.complete_tool(&batch.work_id, successful_result("external", "contents"))
        .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();

    assert_eq!(run.task_list(), Some(&completed));
    assert_eq!(run.state().phase(), ProviderRunPhase::ReadyToCallModel);
    assert_serialization_round_trip(&run);
    let committed = run.clone();
    assert!(run.commit_tool_batch(&batch.work_id).is_err());
    assert_eq!(run, committed);
}

#[test]
fn rejected_task_updates_do_not_replace_durable_progress() {
    let mut run = run();
    let initial = task_snapshot(TaskListStatus::InProgress);
    run.set_task_list(Some(initial.clone())).unwrap();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![ToolCall {
                id: "invalid-task-update".to_string(),
                name: UPDATE_TASK_LIST_NAME.to_string(),
                arguments: json!({"tasks": []}),
            }],
            &[UPDATE_TASK_LIST_NAME],
        ),
    );
    assert_eq!(
        batch.calls[0].state.result().unwrap().status,
        ToolResultStatus::Error,
    );
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert_eq!(run.task_list(), Some(&initial));

    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![update_task_call(
                "unadvertised-update",
                &task_snapshot(TaskListStatus::Completed),
            )],
            &[],
        ),
    );
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert_eq!(run.task_list(), Some(&initial));
}

#[test]
fn task_progress_survives_compaction_archive_eviction_and_restore() {
    let mut run = run();
    let snapshot = task_snapshot(TaskListStatus::InProgress);
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![update_task_call("tasks", &snapshot)],
            &[UPDATE_TASK_LIST_NAME],
        ),
    );
    assert!(batch.is_complete());
    run.commit_tool_batch(&batch.work_id).unwrap();
    run.compact_transcript_at_model_boundary(
        run.transcript().len(),
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::Text("Earlier conversation summary".to_string()),
        }],
    )
    .unwrap();

    let mut archive = run.tool_result_archive().to_vec();
    for index in 0..450 {
        archive.extend([
            ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::ToolUse {
                    tool_use_id: format!("read-{index}"),
                    name: "read_files".to_string(),
                    input: json!({}),
                },
            },
            ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::ToolResult {
                    tool_use_id: format!("read-{index}"),
                    content: "contents".to_string(),
                    is_error: false,
                },
            },
        ]);
    }
    run.replace_tool_result_archive(archive);
    assert!(
        !serde_json::to_string(run.tool_result_archive())
            .unwrap()
            .contains("task_list_snapshot")
    );
    assert_eq!(run.task_list(), Some(&snapshot));
    assert_serialization_round_trip(&run);
    run.validate_restored_state().unwrap();
}

#[test]
fn task_list_setter_and_restored_snapshots_validate_before_mutation() {
    let mut run = run();
    let initial = task_snapshot(TaskListStatus::InProgress);
    run.set_task_list(Some(initial.clone())).unwrap();
    assert!(matches!(
        run.set_task_list(Some(TaskListSnapshot { tasks: Vec::new() })),
        Err(ProviderRunProtocolError::InvalidTaskList { .. }),
    ));
    assert_eq!(run.task_list(), Some(&initial));

    let invalid = mutate_run_json(&run, |value| value["task_list"] = json!({"tasks": []}));
    assert!(restored_state_error(&invalid).contains("invalid task list"));

    let legacy = mutate_run_json(&run, |value| {
        value.as_object_mut().unwrap().remove("task_list");
    });
    assert_eq!(legacy.task_list(), None);
    legacy.validate_restored_state().unwrap();
    run.set_task_list(None).unwrap();
    assert_eq!(run.task_list(), None);
}

#[test]
fn restoring_a_completed_inline_task_batch_commits_once_without_external_work() {
    let mut run = run();
    let snapshot = task_snapshot(TaskListStatus::InProgress);
    accept_tool_turn(
        &mut run,
        tool_turn(
            vec![update_task_call("tasks", &snapshot)],
            &[UPDATE_TASK_LIST_NAME],
        ),
    );
    let mut restored: ProviderRun =
        serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    restored.validate_restored_state().unwrap();
    assert_eq!(restored.task_list(), None);
    assert!(
        restored
            .normalize_after_restore()
            .unwrap()
            .committed_tool_batch
    );
    assert_eq!(restored.task_list(), Some(&snapshot));
    assert!(
        !restored
            .normalize_after_restore()
            .unwrap()
            .committed_tool_batch
    );
    assert_eq!(restored.task_list(), Some(&snapshot));

    let corrupted = mutate_run_json(&run, |value| {
        value["state"]["AwaitingTools"]["batch"]["calls"][0]["state"]["Resolved"]["result"]["content"] =
            json!(task_snapshot(TaskListStatus::Completed).to_tool_result_content());
    });
    assert!(restored_state_error(&corrupted).contains("does not match its arguments"));
}

#[test]
fn parallel_tool_results_commit_atomically_in_original_call_order() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("first", "read_files"),
                tool_call("second", "grep"),
            ],
            &["read_files", "grep"],
        ),
    );

    run.start_tool(&batch.work_id, "first").unwrap();
    run.start_tool(&batch.work_id, "second").unwrap();
    run.complete_tool(&batch.work_id, successful_result("second", "second result"))
        .unwrap();
    run.complete_tool(&batch.work_id, successful_result("first", "first result"))
        .unwrap();

    assert_eq!(run.transcript().len(), 2);
    let Some(ProviderRunStep::DispatchTools(completed)) = run.next_step().unwrap() else {
        panic!("completed batch must remain recoverable until commit");
    };
    assert!(completed.is_complete());
    assert_eq!(run.epoch(), RunEpoch::new(1));
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert_eq!(run.epoch(), RunEpoch::new(2));

    let MessageContent::MultiPart(parts) = &run.transcript().last().unwrap().content else {
        panic!("expected atomic multi-part result message");
    };
    let ids = parts
        .iter()
        .map(|part| match part {
            ContentPart::ToolResult { tool_use_id, .. } => tool_use_id.as_str(),
            ContentPart::Text(_)
            | ContentPart::Reasoning { .. }
            | ContentPart::Image { .. }
            | ContentPart::ToolUse { .. } => panic!("expected only tool results"),
        })
        .collect::<Vec<_>>();
    assert_eq!(ids, vec!["first", "second"]);
}

#[test]
fn duplicate_tool_start_is_idempotent() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
    );

    run.start_tool(&batch.work_id, "read").unwrap();
    let started = run.clone();
    run.start_tool(&batch.work_id, "read").unwrap();

    assert_eq!(run, started);
}

#[test]
fn identical_tool_completion_is_idempotent() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
    );
    let result = successful_result("read", "contents");

    run.complete_tool(&batch.work_id, result.clone()).unwrap();
    let completed = run.clone();
    run.complete_tool(&batch.work_id, result).unwrap();

    assert_eq!(run, completed);
}

#[test]
fn conflicting_tool_completion_is_rejected_without_mutation() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
    );
    run.complete_tool(&batch.work_id, successful_result("read", "contents"))
        .unwrap();
    let completed = run.clone();

    assert_eq!(
        run.complete_tool(&batch.work_id, successful_result("read", "different"))
            .unwrap_err(),
        ProviderRunProtocolError::DuplicateToolUpdate {
            call_id: "read".to_string(),
        }
    );
    assert_eq!(run, completed);
}

#[test]
fn duplicate_tool_callbacks_do_not_prevent_eventual_batch_completion() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("first", "read_files"),
                tool_call("second", "grep"),
            ],
            &["read_files", "grep"],
        ),
    );
    let first_result = successful_result("first", "one");

    run.start_tool(&batch.work_id, "first").unwrap();
    run.start_tool(&batch.work_id, "first").unwrap();
    run.complete_tool(&batch.work_id, first_result.clone())
        .unwrap();
    run.complete_tool(&batch.work_id, first_result).unwrap();
    run.complete_tool(&batch.work_id, successful_result("second", "two"))
        .unwrap();

    let ProviderRunState::AwaitingTools { batch: completed } = run.state() else {
        panic!("expected completed tool batch");
    };
    assert!(completed.is_complete());
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert_eq!(run.state().phase(), ProviderRunPhase::ReadyToCallModel);
}

#[test]
fn exact_batch_submission_rejects_missing_duplicate_and_unknown_results_without_mutation() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("first", "read_files"),
                tool_call("second", "grep"),
            ],
            &["read_files", "grep"],
        ),
    );
    let before = run.clone();

    let missing = run
        .complete_tool_batch(&batch.work_id, vec![successful_result("first", "one")])
        .unwrap_err();
    assert!(matches!(
        missing,
        ProviderRunProtocolError::ToolResultSetMismatch { .. }
    ));
    assert_eq!(run, before);

    let duplicate = run
        .complete_tool_batch(
            &batch.work_id,
            vec![
                successful_result("first", "one"),
                successful_result("first", "again"),
            ],
        )
        .unwrap_err();
    assert_eq!(
        duplicate,
        ProviderRunProtocolError::DuplicateToolResult {
            call_id: "first".to_string(),
        }
    );
    assert_eq!(run, before);

    let unknown = run
        .complete_tool_batch(
            &batch.work_id,
            vec![
                successful_result("first", "one"),
                successful_result("unknown", "bad"),
            ],
        )
        .unwrap_err();
    assert!(matches!(
        unknown,
        ProviderRunProtocolError::ToolResultSetMismatch { .. }
    ));
    assert_eq!(run, before);
}

#[test]
fn repeated_permission_notifications_preserve_pending_sibling_tools() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("first", "run_shell_command"),
                tool_call("second", "run_shell_command"),
            ],
            &["run_shell_command"],
        ),
    );
    let request = PermissionRequest {
        id: "permission-first".to_string(),
        call_id: "first".to_string(),
        kind: PermissionKind::Execute,
        reason: Some("run a command".to_string()),
    };
    run.request_tool_permission(&batch.work_id, request.clone())
        .unwrap();
    run.request_tool_permission(&batch.work_id, request.clone())
        .unwrap();
    let mut conflicting = request;
    conflicting.id = "different-permission".to_string();
    assert!(
        run.request_tool_permission(&batch.work_id, conflicting)
            .is_err()
    );
    for _ in 0..2 {
        run.resolve_tool_permission(
            &batch.work_id,
            "first",
            "permission-first",
            PermissionDecision::AllowOnce,
        )
        .unwrap();
    }
    assert!(
        run.resolve_tool_permission(
            &batch.work_id,
            "first",
            "permission-first",
            PermissionDecision::AlwaysAllow,
        )
        .is_err()
    );
    run.start_tool(&batch.work_id, "first").unwrap();
    let ProviderRunState::AwaitingTools { batch } = run.state() else {
        panic!("permission notifications must leave the batch pending");
    };
    assert_eq!(batch.calls[0].state, PendingToolCallState::Executing);
    assert_eq!(batch.calls[1].state, PendingToolCallState::Proposed);
}

#[test]
fn permission_denial_becomes_one_correlated_result() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![tool_call("shell", "run_shell_command")],
            &["run_shell_command"],
        ),
    );
    let request = PermissionRequest {
        id: "permission-shell".to_string(),
        call_id: "shell".to_string(),
        kind: PermissionKind::Execute,
        reason: Some("run a command".to_string()),
    };

    run.request_tool_permission(&batch.work_id, request)
        .unwrap();
    let wrong_request = run
        .resolve_tool_permission(
            &batch.work_id,
            "shell",
            "wrong",
            PermissionDecision::AllowOnce,
        )
        .unwrap_err();
    assert!(matches!(
        wrong_request,
        ProviderRunProtocolError::PermissionRequestMismatch { .. }
    ));
    run.resolve_tool_permission(
        &batch.work_id,
        "shell",
        "permission-shell",
        PermissionDecision::Denied {
            reason: Some("not allowed".to_string()),
        },
    )
    .unwrap();

    let Some(ProviderRunStep::DispatchTools(completed)) = run.next_step().unwrap() else {
        panic!("expected completed tool batch");
    };
    let result = completed.calls[0].state.result().unwrap();
    assert_eq!(result.status, ToolResultStatus::Denied);
    assert_eq!(result.content, "not allowed");
    assert!(completed.is_complete());
}

#[test]
fn stale_unknown_and_duplicate_tool_updates_do_not_mutate_the_batch() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
    );
    let stale = ExternalWorkId {
        run_id: batch.work_id.run_id.clone(),
        epoch: RunEpoch::new(batch.work_id.epoch.get() + 1),
    };
    let before = run.clone();
    assert!(matches!(
        run.start_tool(&stale, "read").unwrap_err(),
        ProviderRunProtocolError::WorkMismatch { .. }
    ));
    assert_eq!(run, before);
    assert_eq!(
        run.start_tool(&batch.work_id, "missing").unwrap_err(),
        ProviderRunProtocolError::UnknownToolCall {
            call_id: "missing".to_string(),
        }
    );
    assert_eq!(run, before);

    run.complete_tool(&batch.work_id, successful_result("read", "ok"))
        .unwrap();
    let completed = run.clone();
    assert_eq!(
        run.complete_tool(&batch.work_id, successful_result("read", "again"))
            .unwrap_err(),
        ProviderRunProtocolError::DuplicateToolUpdate {
            call_id: "read".to_string(),
        }
    );
    assert_eq!(run, completed);
}

#[test]
fn run_cancellation_preserves_completed_results_and_synthesizes_the_rest() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![tool_call("done", "read_files"), tool_call("active", "grep")],
            &["read_files", "grep"],
        ),
    );
    run.complete_tool(&batch.work_id, successful_result("done", "contents"))
        .unwrap();
    run.start_tool(&batch.work_id, "active").unwrap();

    run.cancel("user cancelled").unwrap();

    assert_eq!(run.epoch(), RunEpoch::new(2));
    let MessageContent::MultiPart(parts) = &run.transcript().last().unwrap().content else {
        panic!("expected tool results");
    };
    assert!(matches!(
        &parts[0],
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error: false,
        } if tool_use_id == "done" && content == "contents"
    ));
    assert!(matches!(
        &parts[1],
        ContentPart::ToolResult {
            tool_use_id,
            content,
            is_error: true,
        } if tool_use_id == "active" && content == "user cancelled"
    ));
    assert_eq!(
        run.next_step().unwrap(),
        Some(ProviderRunStep::Done(ProviderRunOutcome::Cancelled {
            reason: "user cancelled".to_string(),
        }))
    );
}

#[test]
fn failure_while_tools_are_pending_records_correlated_errors_before_terminal_state() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("project", "read_files")], &["read_files"]),
    );
    assert_eq!(batch.work_id.epoch, RunEpoch::new(1));

    run.fail(
        ProviderRunFailureKind::Projection,
        "proposal could not be projected",
    )
    .unwrap();

    assert_eq!(run.epoch(), RunEpoch::new(2));
    let MessageContent::MultiPart(parts) = &run.transcript().last().unwrap().content else {
        panic!("expected synthesized tool result");
    };
    assert!(matches!(
        &parts[0],
        ContentPart::ToolResult {
            tool_use_id,
            is_error: true,
            ..
        } if tool_use_id == "project"
    ));
    let Some(ProviderRunStep::Done(ProviderRunOutcome::Failed(failure))) = run.next_step().unwrap()
    else {
        panic!("expected failed run");
    };
    assert_eq!(failure.kind, ProviderRunFailureKind::Projection);
}

#[test]
fn turn_limit_cannot_finish_successfully_after_tools_require_another_model_turn() {
    let mut run = run_with_limits(ProviderRunLimits {
        max_model_turns: Some(1),
        max_model_retries_per_turn: 0,
    });
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
    );
    run.complete_tool(&batch.work_id, successful_result("read", "ok"))
        .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();

    let Some(ProviderRunStep::Done(ProviderRunOutcome::Failed(failure))) = run.next_step().unwrap()
    else {
        panic!("expected turn-limit failure");
    };
    assert_eq!(failure.kind, ProviderRunFailureKind::TurnLimitExceeded);
}

#[test]
fn invalid_model_turn_does_not_commit_partial_content_usage_or_epoch() {
    let mut run = run();
    let call = next_model_call(&mut run);
    let duplicate = tool_call("duplicate", "read_files");
    let turn = tool_turn(vec![duplicate.clone(), duplicate], &["read_files"]);
    let before = run.clone();

    assert!(matches!(
        run.accept_model_turn(&call.work_id, turn).unwrap_err(),
        ProviderRunProtocolError::InvalidModelTurn { .. }
    ));
    assert_eq!(run, before);
}

#[test]
fn every_nonterminal_phase_round_trips_through_json() {
    let mut ready = run();
    assert_serialization_round_trip(&ready);

    let call = next_model_call(&mut ready);
    assert_serialization_round_trip(&ready);

    let mut resolving_tools = ready.clone();
    resolving_tools
        .accept_model_turn(
            &call.work_id,
            tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
        )
        .unwrap();
    assert_eq!(
        resolving_tools.state().phase(),
        ProviderRunPhase::ResolvingModel
    );
    assert_serialization_round_trip(&resolving_tools);

    let Some(ProviderRunStep::DispatchTools(_)) = resolving_tools.next_step().unwrap() else {
        panic!("expected tool phase");
    };
    assert_serialization_round_trip(&resolving_tools);

    let mut resolving_text = ready;
    resolving_text
        .accept_model_turn(&call.work_id, text_turn("done"))
        .unwrap();
    assert_serialization_round_trip(&resolving_text);
    assert_eq!(resolving_text.next_step().unwrap(), None);
    assert_eq!(
        resolving_text.state().phase(),
        ProviderRunPhase::AwaitingDriver
    );
    assert_serialization_round_trip(&resolving_text);
}

#[test]
fn restored_state_validation_accepts_valid_snapshots_in_every_phase() {
    let ready = run();
    let mut awaiting_model = ready.clone();
    let call = next_model_call(&mut awaiting_model);
    let mut resolving = awaiting_model.clone();
    resolving
        .accept_model_turn(&call.work_id, text_turn("done"))
        .unwrap();
    let mut awaiting_driver = resolving.clone();
    assert_eq!(awaiting_driver.next_step().unwrap(), None);
    let awaiting_tools = awaiting_tool_run();
    let mut done = awaiting_driver.clone();
    let work_id = awaiting_driver.active_work_id().unwrap().clone();
    done.complete(&work_id).unwrap();
    let mut failed = ready.clone();
    failed
        .fail(ProviderRunFailureKind::ExternalWork, "failed")
        .unwrap();
    let mut cancelled = ready;
    cancelled.cancel("cancelled").unwrap();

    for candidate in [
        awaiting_model,
        resolving,
        awaiting_tools,
        awaiting_driver,
        done,
        failed,
        cancelled,
    ] {
        candidate.validate_restored_state().unwrap();
    }
}

#[test]
fn restored_state_validation_rejects_active_work_run_and_epoch_mismatches() {
    let mut awaiting_model = run();
    next_model_call(&mut awaiting_model);
    let wrong_run = mutate_run_json(&awaiting_model, |value| {
        value["state"]["AwaitingModel"]["call"]["work_id"]["run_id"] = json!("other-run");
    });
    assert!(restored_state_error(&wrong_run).contains("active work identity other-run:0"));

    let wrong_epoch = mutate_run_json(&awaiting_model, |value| {
        value["state"]["AwaitingModel"]["call"]["work_id"]["epoch"] = json!(9);
    });
    assert!(restored_state_error(&wrong_epoch).contains("active work identity run-1:9"));
}

#[test]
fn restored_state_validation_rejects_duplicate_tool_call_ids() {
    let corrupted = mutate_run_json(&awaiting_tool_run(), |value| {
        value["state"]["AwaitingTools"]["batch"]["calls"][1]["call"]["id"] = json!("first");
    });
    assert_eq!(
        restored_state_error(&corrupted),
        "duplicate pending tool call ID 'first'"
    );
}

#[test]
fn restored_state_validation_rejects_result_and_permission_call_ownership() {
    let mut resolved = awaiting_tool_run();
    let work_id = resolved.active_work_id().unwrap().clone();
    resolved
        .complete_tool(&work_id, successful_result("first", "done"))
        .unwrap();
    let wrong_result = mutate_run_json(&resolved, |value| {
        value["state"]["AwaitingTools"]["batch"]["calls"][0]["state"]["Resolved"]["result"]["call_id"] =
            json!("second");
    });
    assert_eq!(
        restored_state_error(&wrong_result),
        "resolved result for 'first' belongs to call 'second'"
    );

    let mut permission = awaiting_tool_run();
    let work_id = permission.active_work_id().unwrap().clone();
    permission
        .request_tool_permission(
            &work_id,
            PermissionRequest {
                id: "request-1".to_string(),
                call_id: "first".to_string(),
                kind: PermissionKind::Read,
                reason: None,
            },
        )
        .unwrap();
    let wrong_permission = mutate_run_json(&permission, |value| {
        value["state"]["AwaitingTools"]["batch"]["calls"][0]["state"]["PermissionPending"]["request"]
            ["call_id"] = json!("second");
    });
    assert_eq!(
        restored_state_error(&wrong_permission),
        "permission request for 'first' belongs to call 'second'"
    );
}

#[test]
fn restored_state_validation_rejects_phase_specific_corruption() {
    let empty_batch = mutate_run_json(&awaiting_tool_run(), |value| {
        value["state"]["AwaitingTools"]["batch"]["calls"] = json!([]);
    });
    assert_eq!(
        restored_state_error(&empty_batch),
        "pending tool batch is empty"
    );

    let mismatched_batch = mutate_run_json(&awaiting_tool_run(), |value| {
        value["state"]["AwaitingTools"]["batch"]["calls"][0]["call"]["name"] =
            json!("different_tool");
    });
    assert!(restored_state_error(&mismatched_batch).contains("latest assistant message"));

    let resolving_without_turn = mutate_run_json(&awaiting_tool_run(), |value| {
        value["model_turns"] = json!(0);
    });
    assert!(restored_state_error(&resolving_without_turn).contains("has no completed model turn"));

    let mut resolving = run();
    let call = next_model_call(&mut resolving);
    resolving
        .accept_model_turn(&call.work_id, text_turn("done"))
        .unwrap();
    let invalid_turn = mutate_run_json(&resolving, |value| {
        value["state"]["ResolvingModel"]["turn"]["assistant_content"] = json!([{
            "ToolUse": {
                "tool_use_id": "injected",
                "name": "read_files",
                "input": {}
            }
        }]);
    });
    assert!(restored_state_error(&invalid_turn).contains("assistant_content"));

    let mut awaiting_driver = resolving;
    assert_eq!(awaiting_driver.next_step().unwrap(), None);
    let wrong_driver_owner = mutate_run_json(&awaiting_driver, |value| {
        let last = value["transcript"]
            .as_array_mut()
            .unwrap()
            .last_mut()
            .unwrap();
        last["role"] = json!("User");
    });
    assert!(restored_state_error(&wrong_driver_owner).contains("does not follow an assistant"));
}

#[test]
fn restored_state_validation_rejects_retry_counter_and_terminal_corruption() {
    let zero_limit = mutate_run_json(&run(), |value| {
        value["limits"]["max_model_turns"] = json!(0);
    });
    assert_eq!(
        restored_state_error(&zero_limit),
        "model-turn limit must be at least one"
    );

    let excessive_retries = mutate_run_json(&run(), |value| {
        value["model_retries"] = json!(4);
    });
    assert!(restored_state_error(&excessive_retries).contains("model-retry counter"));

    let mut awaiting_model = run();
    next_model_call(&mut awaiting_model);
    let inconsistent_retry = mutate_run_json(&awaiting_model, |value| {
        value["state"]["AwaitingModel"]["call"]["retry_attempt"] = json!(1);
    });
    assert!(restored_state_error(&inconsistent_retry).contains("total retry counter"));

    let completed_without_turn = mutate_run_json(&run(), |value| {
        value["state"] = json!({"Done": {"completion": {"stop_reason": "Completed"}}});
    });
    assert_eq!(
        restored_state_error(&completed_without_turn),
        "completed run has no completed model turn"
    );

    let early_turn_limit = mutate_run_json(&run(), |value| {
        value["state"] = json!({
            "Failed": {"failure": {
                "kind": "TurnLimitExceeded",
                "message": "bad",
                "source": null
            }}
        });
    });
    assert_eq!(
        restored_state_error(&early_turn_limit),
        "turn-limit failure occurred before reaching the limit"
    );

    let missing_retry_source = mutate_run_json(&run(), |value| {
        value["state"] = json!({
            "Failed": {"failure": {
                "kind": "RetryLimitExceeded",
                "message": "bad",
                "source": null
            }}
        });
    });
    assert_eq!(
        restored_state_error(&missing_retry_source),
        "retry-limit failure lacks a recoverable source"
    );
}

#[test]
fn restored_state_validation_rejects_empty_run_identity_and_profile() {
    let empty_run = mutate_run_json(&run(), |value| value["id"] = json!(""));
    assert_eq!(restored_state_error(&empty_run), "run ID must not be empty");

    let empty_profile = mutate_run_json(&run(), |value| value["profile"] = json!(""));
    assert_eq!(
        restored_state_error(&empty_profile),
        "request profile must not be empty"
    );
}

#[test]
fn restore_normalization_preserves_model_work_outside_the_uncertain_dispatch_boundary() {
    let ready = run();
    let mut awaiting_model = ready.clone();
    let call = next_model_call(&mut awaiting_model);

    let mut resolving = awaiting_model.clone();
    resolving
        .accept_model_turn(&call.work_id, text_turn("done"))
        .unwrap();
    let mut awaiting_driver = resolving.clone();
    assert_eq!(awaiting_driver.next_step().unwrap(), None);

    for mut candidate in [ready, resolving, awaiting_driver] {
        let before = candidate.clone();
        assert_eq!(
            candidate.normalize_after_restore().unwrap(),
            ProviderRunRestoreNormalization::default()
        );
        assert_eq!(candidate, before);
    }
}

#[test]
fn crash_after_model_acceptance_before_checkpoint_does_not_replay_the_persisted_call() {
    let mut live_run = run();
    let call = next_model_call(&mut live_run);
    let serialized_at_dispatch_boundary = serde_json::to_string(&live_run).unwrap();

    // Simulate remote acceptance followed by a crash before the accepted turn is checkpointed.
    live_run
        .accept_model_turn(&call.work_id, text_turn("accepted but not checkpointed"))
        .unwrap();
    assert_eq!(live_run.model_turns(), 1);

    let mut restored: ProviderRun = serde_json::from_str(&serialized_at_dispatch_boundary).unwrap();

    let normalization = restored.normalize_after_restore().unwrap();

    assert!(normalization.interrupted_model_call);
    let ProviderRunState::Failed { failure } = restored.state() else {
        panic!("uncertain model work must become terminal on restore");
    };
    assert_eq!(failure.kind, ProviderRunFailureKind::Restore);
    assert!(failure.message.contains("outcome is unknown"));
    assert!(failure.message.contains("not replayed"));
    assert_eq!(restored.active_work_id(), None);
    assert!(matches!(
        restored.next_step().unwrap(),
        Some(ProviderRunStep::Done(ProviderRunOutcome::Failed(_)))
    ));

    // The persisted dispatch identity remains useful for diagnostics but can never be called again.
    assert_eq!(call.work_id.epoch, RunEpoch::new(0));
}

#[test]
fn restore_after_model_acceptance_keeps_the_committed_turn_without_replaying() {
    let mut run = run();
    let call = next_model_call(&mut run);
    run.accept_model_turn(&call.work_id, text_turn("accepted output"))
        .unwrap();
    let serialized_after_acceptance = serde_json::to_string(&run).unwrap();
    let mut restored: ProviderRun = serde_json::from_str(&serialized_after_acceptance).unwrap();

    assert_eq!(
        restored.normalize_after_restore().unwrap(),
        ProviderRunRestoreNormalization::default()
    );
    assert_eq!(restored.state().phase(), ProviderRunPhase::ResolvingModel);
    assert_eq!(restored.model_turns(), 1);
    assert_eq!(restored.next_step().unwrap(), None);
    assert_eq!(restored.state().phase(), ProviderRunPhase::AwaitingDriver);
}

#[test]
fn restore_normalization_reproposes_permissions_and_interrupts_unsafe_tools() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("proposed", "read_files"),
                tool_call("permission", "grep"),
                tool_call("approved", "run_shell_command"),
                tool_call("executing", "read_files"),
                tool_call("resolved", "grep"),
            ],
            &["read_files", "grep", "run_shell_command"],
        ),
    );
    let permission = PermissionRequest {
        id: "permission-1".to_string(),
        call_id: "permission".to_string(),
        kind: PermissionKind::Read,
        reason: None,
    };
    run.request_tool_permission(&batch.work_id, permission)
        .unwrap();
    let approved = PermissionRequest {
        id: "permission-2".to_string(),
        call_id: "approved".to_string(),
        kind: PermissionKind::Execute,
        reason: None,
    };
    run.request_tool_permission(&batch.work_id, approved)
        .unwrap();
    run.resolve_tool_permission(
        &batch.work_id,
        "approved",
        "permission-2",
        PermissionDecision::AllowOnce,
    )
    .unwrap();
    run.start_tool(&batch.work_id, "executing").unwrap();
    run.complete_tool(
        &batch.work_id,
        successful_result("resolved", "already finished"),
    )
    .unwrap();

    let normalization = run.normalize_after_restore().unwrap();

    assert_eq!(
        normalization.permission_call_ids_reset,
        vec!["permission".to_string()]
    );
    assert_eq!(
        normalization.interrupted_call_ids,
        vec!["approved".to_string(), "executing".to_string()]
    );
    assert!(!normalization.committed_tool_batch);
    let ProviderRunState::AwaitingTools { batch } = run.state() else {
        panic!("partially resolved batch must remain pending");
    };
    assert!(matches!(
        batch.calls[0].state,
        PendingToolCallState::Proposed
    ));
    assert!(matches!(
        batch.calls[1].state,
        PendingToolCallState::Proposed
    ));
    for index in [2, 3] {
        let result = batch.calls[index].state.result().unwrap();
        assert_eq!(result.status, ToolResultStatus::Error);
        assert!(result.content.contains("was not replayed"));
    }
    assert_eq!(
        batch.calls[4].state.result(),
        Some(&successful_result("resolved", "already finished"))
    );
}

#[test]
fn restore_normalization_preserves_selected_executing_tools_for_recovery() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("command", "run_command"),
                tool_call("read", "read_files"),
            ],
            &["run_command", "read_files"],
        ),
    );
    run.start_tool(&batch.work_id, "command").unwrap();
    run.start_tool(&batch.work_id, "read").unwrap();

    let normalization = run
        .normalize_after_restore_with_recoverable_calls(&HashSet::from(["command".to_string()]))
        .unwrap();

    assert_eq!(normalization.recovery_call_ids, vec!["command"]);
    assert_eq!(normalization.interrupted_call_ids, vec!["read"]);
    let ProviderRunState::AwaitingTools { batch } = run.state() else {
        panic!("recovered tool batch should remain pending");
    };
    assert!(matches!(
        batch.calls[0].state,
        PendingToolCallState::RecoveryPending
    ));
    assert_eq!(
        batch.calls[1].state.result().unwrap().status,
        ToolResultStatus::Error
    );
}

#[test]
fn recovery_pending_tool_survives_another_restore_and_completes_once() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("run-agents", "run_agents")], &["run_agents"]),
    );
    run.start_tool(&batch.work_id, "run-agents").unwrap();
    run.normalize_after_restore_with_recoverable_calls(&HashSet::from(["run-agents".to_string()]))
        .unwrap();
    let serialized = serde_json::to_string(&run).unwrap();
    let mut restored: ProviderRun = serde_json::from_str(&serialized).unwrap();

    let normalization = restored.normalize_after_restore().unwrap();
    assert_eq!(normalization.recovery_call_ids, vec!["run-agents"]);
    restored
        .complete_tool(
            &batch.work_id,
            successful_result("run-agents", "children launched"),
        )
        .unwrap();
    assert!(matches!(
        restored.complete_tool(
            &batch.work_id,
            successful_result("run-agents", "duplicate completion"),
        ),
        Err(ProviderRunProtocolError::DuplicateToolUpdate { .. })
    ));
    restored.commit_tool_batch(&batch.work_id).unwrap();
    assert_eq!(restored.state().phase(), ProviderRunPhase::Done);
}

#[test]
fn transcript_compaction_at_model_boundary_archives_drained_tool_history() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("read", "read_files")], &["read_files"]),
    );
    run.complete_tool(
        &batch.work_id,
        successful_result("read", "important file contents"),
    )
    .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();
    let original_len = run.transcript().len();
    let summary_prefix = vec![ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::Text("summary".to_string()),
    }];

    run.compact_transcript_at_model_boundary(original_len, summary_prefix.clone())
        .unwrap();

    assert_eq!(run.transcript(), summary_prefix);
    assert_eq!(run.tool_result_archive().len(), 2);
    assert!(matches!(
        run.tool_result_archive()[0].content,
        MessageContent::MultiPart(_)
    ));
    assert!(matches!(
        run.tool_result_archive()[1].content,
        MessageContent::MultiPart(_)
    ));
}

#[test]
fn transcript_compaction_is_rejected_during_a_model_call() {
    let mut run = run();
    let _ = next_model_call(&mut run);

    assert!(matches!(
        run.compact_transcript_at_model_boundary(
            1,
            vec![ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("summary".to_string()),
            }],
        ),
        Err(ProviderRunProtocolError::UnexpectedState {
            expected: ProviderRunPhase::ReadyToCallModel,
            actual: ProviderRunPhase::AwaitingModel,
        })
    ));
}

#[test]
fn context_compaction_preserves_run_and_work_identity() {
    let mut run = run();
    let call = next_model_call(&mut run);
    let run_id = run.id().clone();

    run.prepare_context_compaction(&call.work_id).unwrap();

    assert_eq!(run.id(), &run_id);
    assert_eq!(run.ready_work_id(), Some(call.work_id));
    assert_eq!(run.state().phase(), ProviderRunPhase::ReadyToCallModel);
}

#[test]
fn restore_normalization_commits_a_fully_resolved_batch() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("executing", "read_files"),
                tool_call("resolved", "grep"),
            ],
            &["read_files", "grep"],
        ),
    );
    run.start_tool(&batch.work_id, "executing").unwrap();
    run.complete_tool(
        &batch.work_id,
        successful_result("resolved", "already finished"),
    )
    .unwrap();

    let normalization = run.normalize_after_restore().unwrap();

    assert!(normalization.committed_tool_batch);
    assert_eq!(
        normalization.interrupted_call_ids,
        vec!["executing".to_string()]
    );
    assert_eq!(run.state().phase(), ProviderRunPhase::ReadyToCallModel);
    assert_eq!(run.epoch(), RunEpoch::new(2));
    let next = next_model_call(&mut run);
    assert_eq!(next.work_id.epoch, RunEpoch::new(2));
    let MessageContent::MultiPart(parts) = &next.messages.last().unwrap().content else {
        panic!("expected committed tool results");
    };
    assert!(matches!(
        &parts[0],
        ContentPart::ToolResult {
            tool_use_id,
            is_error: true,
            ..
        } if tool_use_id == "executing"
    ));
    assert!(matches!(
        &parts[1],
        ContentPart::ToolResult {
            tool_use_id,
            is_error: false,
            ..
        } if tool_use_id == "resolved"
    ));
}

#[test]
fn default_run_continues_past_one_hundred_tool_turns_and_restores() {
    let mut run = run();
    for index in 0..150 {
        let id = format!("poll-{index}");
        let batch = accept_tool_turn(
            &mut run,
            tool_turn(
                vec![tool_call(&id, "read_shell_command_output")],
                &["read_shell_command_output"],
            ),
        );
        run.complete_tool(&batch.work_id, successful_result(&id, "still running"))
            .unwrap();
        run.commit_tool_batch(&batch.work_id).unwrap();
        if index == 99 {
            run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
            run.validate_restored_state().unwrap();
        }
    }
    assert_eq!(run.model_turns(), 150);
    let call = next_model_call(&mut run);
    run.accept_model_turn(&call.work_id, text_turn("Finished"))
        .unwrap();
    assert_eq!(run.next_step().unwrap(), None);
    assert!(matches!(
        run.state(),
        ProviderRunState::AwaitingDriver { .. }
    ));
}

#[test]
fn serialized_numeric_turn_budgets_remain_explicit_limits() {
    let limits: ProviderRunLimits = serde_json::from_value(json!({
        "max_model_turns": 100,
        "max_model_retries_per_turn": 3
    }))
    .unwrap();
    assert_eq!(limits.max_model_turns, Some(100));
    assert_eq!(ProviderRunLimits::default().max_model_turns, None);
}

#[test]
fn multipart_archive_is_not_multiplied_and_legacy_duplicates_are_repaired() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            (0..4)
                .map(|id| tool_call(&format!("search-{id}"), "search_codebase"))
                .collect(),
            &["search_codebase"],
        ),
    );
    for id in 0..4 {
        run.complete_tool(
            &batch.work_id,
            successful_result(&format!("search-{id}"), &format!("file contents {id}")),
        )
        .unwrap();
    }
    run.commit_tool_batch(&batch.work_id).unwrap();
    let transcript = run.transcript().to_vec();
    let mut archive = Vec::new();
    archive_tool_results(&mut archive, transcript.clone());
    assert_eq!(archive.len(), 2);
    for message in &archive {
        let MessageContent::MultiPart(parts) = &message.content else {
            panic!("expected multipart tools");
        };
        assert_eq!(parts.len(), 4);
    }
    let expected = archive.clone();
    let duplicated: Vec<_> = (0..4).flat_map(|_| expected.clone()).collect();
    run.replace_tool_result_archive(duplicated);
    assert_eq!(run.tool_result_archive(), expected);
    // A repeated compaction callback must not grow the archive either.
    archive_tool_results(&mut archive, transcript);
    assert_eq!(archive, expected);
}

#[test]
fn screenshot_results_survive_checkpoint_compaction_and_archive_deduplication() {
    let mut run = ProviderRun::new(
        "screenshots",
        vec![ConversationMessage {
            role: MessageRole::User,
            content: MessageContent::MultiPart(vec![ContentPart::Image {
                data: vec![99],
                mime_type: "image/png".to_string(),
            }]),
        }],
        "base",
        ProviderRunLimits::default(),
    );
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("capture", "use_computer"),
                tool_call("capture-more", "use_computer"),
            ],
            &["use_computer"],
        ),
    );
    for (call_id, bytes) in [("capture", vec![1, 2]), ("capture-more", vec![3, 4])] {
        let mut result = successful_result(call_id, "screenshot captured");
        result.images.push(ToolResultImage {
            data: bytes,
            mime_type: "image/png".to_string(),
        });
        run.complete_tool(&batch.work_id, result).unwrap();
    }
    // Pending typed results and their image payloads must survive a checkpoint.
    assert_serialization_round_trip(&run);
    run = serde_json::from_str(&serde_json::to_string(&run).unwrap()).unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();
    let transcript = run.transcript().to_vec();
    let summary = vec![ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::Text("Screenshots inspected.".to_string()),
    }];
    run.compact_transcript_at_model_boundary(transcript.len(), summary.clone())
        .unwrap();
    assert_eq!(run.transcript(), summary);
    assert_eq!(run.tool_result_archive().len(), 2);
    let MessageContent::MultiPart(parts) = &run.tool_result_archive()[1].content else {
        panic!("expected archived screenshot result batch");
    };
    assert_eq!(parts.len(), 6);
    assert!(matches!(
        &parts[2], ContentPart::Text(text) if text == "Images returned by tool call capture:"
    ));
    assert!(matches!(
        &parts[3], ContentPart::Image { data, .. } if data == &[1, 2]
    ));
    assert!(matches!(
        &parts[4], ContentPart::Text(text) if text == "Images returned by tool call capture-more:"
    ));
    assert!(matches!(
        &parts[5], ContentPart::Image { data, .. } if data == &[3, 4]
    ));
    let expected = run.tool_result_archive().to_vec();
    let duplicated = expected.iter().cloned().chain(transcript).collect();
    run.replace_tool_result_archive(duplicated);
    assert_eq!(run.tool_result_archive(), expected);
    assert_serialization_round_trip(&run);
}

#[test]
fn archives_only_images_labelled_for_new_results_in_the_same_message() {
    let result = successful_result("capture", "captured");
    let mut parts = ToolResult::into_batch_content_parts(vec![result]);
    parts.extend([
        ContentPart::Image {
            data: vec![99],
            mime_type: "image/png".to_string(),
        },
        ContentPart::Text("Images returned by tool call missing:".to_string()),
        ContentPart::Image {
            data: vec![88],
            mime_type: "image/png".to_string(),
        },
        ContentPart::Text("Images returned by tool call capture:".to_string()),
        ContentPart::Image {
            data: vec![1],
            mime_type: "image/png".to_string(),
        },
        ContentPart::Text("Unrelated user upload".to_string()),
        ContentPart::Image {
            data: vec![77],
            mime_type: "image/png".to_string(),
        },
    ]);
    let mut archive = Vec::new();
    let message = ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::MultiPart(parts),
    };
    archive_tool_results(&mut archive, vec![message.clone()]);
    let MessageContent::MultiPart(parts) = &archive[0].content else {
        panic!("expected correlated tool result with one image");
    };
    assert_eq!(parts.len(), 3);
    assert!(matches!(&parts[2], ContentPart::Image { data, .. } if data == &[1]));
    let expected = archive.clone();
    archive_tool_results(&mut archive, vec![message]);
    assert_eq!(archive, expected);
}

#[test]
fn legacy_tool_results_without_images_still_restore() {
    let result: ToolResult = serde_json::from_value(json!({
        "call_id": "legacy",
        "content": "old text result",
        "status": "Success"
    }))
    .unwrap();
    assert!(result.images.is_empty());
    assert_eq!(result.content, "old text result");
}

#[test]
fn local_child_launch_yields_and_does_not_execute_batched_follow_up_tools() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("children", "run_agents"),
                tool_call("edit", "apply_file_diffs"),
            ],
            &["run_agents", "apply_file_diffs"],
        ),
    );
    assert!(matches!(
        batch.calls[0].state,
        PendingToolCallState::Proposed
    ));
    assert!(
        matches!(&batch.calls[1].state, PendingToolCallState::Resolved { result }
        if result.status == ToolResultStatus::Error)
    );
    run.complete_tool(
        &batch.work_id,
        successful_result("children", "Children launched"),
    )
    .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert!(matches!(run.state(), ProviderRunState::Done { .. }));
    assert_serialization_round_trip(&run);
    assert!(matches!(
        run.next_step().unwrap(),
        Some(ProviderRunStep::Done(_))
    ));
}

#[test]
fn failed_child_launch_allows_a_corrective_model_turn() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(vec![tool_call("children", "run_agents")], &["run_agents"]),
    );
    run.complete_tool(
        &batch.work_id,
        ToolResult {
            images: Vec::new(),
            call_id: "children".to_string(),
            content: "Launch failed".to_string(),
            status: ToolResultStatus::Error,
        },
    )
    .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert!(matches!(run.state(), ProviderRunState::ReadyToCallModel));
}

#[test]
fn background_job_launch_yields_after_committing_the_complete_tool_batch() {
    let mut run = run();
    let batch = accept_tool_turn(
        &mut run,
        tool_turn(
            vec![
                tool_call("build", "run_shell_command"),
                tool_call("inspect", "read_files"),
            ],
            &["run_shell_command", "read_files"],
        ),
    );
    assert!(
        batch
            .calls
            .iter()
            .all(|pending| matches!(pending.state, PendingToolCallState::Proposed))
    );
    run.complete_tool(
        &batch.work_id,
        successful_result(
            "build",
            &json!({
                "background_job_started": { "job_id": "job-1", "command": "cargo build" },
            })
            .to_string(),
        ),
    )
    .unwrap();
    assert!(run.commit_tool_batch(&batch.work_id).is_err());
    run.complete_tool(
        &batch.work_id,
        successful_result("inspect", "file contents"),
    )
    .unwrap();
    run.commit_tool_batch(&batch.work_id).unwrap();
    assert!(matches!(run.state(), ProviderRunState::Done { .. }));
    assert_serialization_round_trip(&run);
    let MessageContent::MultiPart(parts) = &run.transcript().last().unwrap().content else {
        panic!("the batch should remain in provider history");
    };
    assert_eq!(
        parts
            .iter()
            .filter(|part| matches!(part, ContentPart::ToolResult { .. }))
            .count(),
        2
    );
}

#[test]
fn failed_or_legacy_shell_results_allow_the_normal_corrective_turn() {
    for (status, content) in [
        (
            ToolResultStatus::Error,
            json!({"background_job_started": {"job_id": "failed", "command": "build"}}).to_string(),
        ),
        (
            ToolResultStatus::Success,
            "legacy terminal snapshot".to_owned(),
        ),
        (
            ToolResultStatus::Success,
            json!({"output": "background_job_started", "exit_code": 0}).to_string(),
        ),
    ] {
        let mut run = run();
        let batch = accept_tool_turn(
            &mut run,
            tool_turn(
                vec![tool_call("shell", "run_shell_command")],
                &["run_shell_command"],
            ),
        );
        run.complete_tool(
            &batch.work_id,
            ToolResult {
                call_id: "shell".to_owned(),
                content,
                status,
                images: Vec::new(),
            },
        )
        .unwrap();
        run.commit_tool_batch(&batch.work_id).unwrap();
        assert!(matches!(run.state(), ProviderRunState::ReadyToCallModel));
    }
}
