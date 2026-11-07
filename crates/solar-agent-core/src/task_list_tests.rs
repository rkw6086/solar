use serde_json::json;

use super::*;

fn task(id: &str, status: TaskListStatus) -> TaskListItem {
    TaskListItem {
        id: id.to_string(),
        title: format!("Task {id}"),
        status,
    }
}

#[test]
fn snapshot_round_trips_through_a_recognizable_canonical_result() {
    let arguments = json!({"tasks": [
        {"id": "inspect", "title": "Inspect the failure", "status": "completed"},
        {"id": "fix", "title": "Fix the failure", "status": "in_progress"},
        {"id": "review", "title": "Review the fix", "status": "pending"},
    ]});
    let snapshot = TaskListSnapshot::from_arguments(&arguments).unwrap();
    let content = snapshot.to_tool_result_content();

    assert_eq!(task_list_from_tool_result(&content), Some(snapshot.clone()));
    assert_eq!(snapshot.to_tool_result_content(), content);
    assert_eq!(
        serde_json::from_str::<JsonValue>(&content).unwrap(),
        json!({"type": "task_list_snapshot", "tasks": arguments["tasks"]}),
    );
    assert_eq!(snapshot.tasks[1].id, "fix");
    assert_eq!(snapshot.tasks[1].status, TaskListStatus::InProgress);
}

#[test]
fn snapshot_rejects_invented_statuses_partial_arguments_and_unknown_fields() {
    for arguments in [
        json!({"tasks": []}),
        json!({"tasks": [{"id": "one", "title": "Inspect", "status": "started"}]}),
        json!({"tasks": [{"id": "one", "title": "Inspect"}]}),
        json!({"tasks": [{"id": "one", "title": "Inspect", "status": "pending", "done": false}]}),
        json!({"tasks": [{"id": "one", "title": "Inspect", "status": "pending"}], "append": true}),
        json!([{"id": "one", "title": "Inspect", "status": "pending"}]),
    ] {
        assert!(TaskListSnapshot::from_arguments(&arguments).is_err());
    }
}

#[test]
fn argument_parser_accepts_only_the_reserved_tool_title_outside_the_snapshot() {
    let tasks = json!([{"id": "one", "title": "Inspect", "status": "in_progress"}]);
    let plain = TaskListSnapshot::from_arguments(&json!({"tasks": tasks})).unwrap();
    assert_eq!(
        TaskListSnapshot::from_arguments(&json!({
            "tasks": tasks,
            "_solar_tool_title": "Update progress",
        })),
        Ok(plain),
    );
    assert!(
        TaskListSnapshot::from_arguments(&json!({
            "tasks": tasks,
            "_solar_tool_title": false,
        }))
        .is_err()
    );
    assert!(
        TaskListSnapshot::from_arguments(&json!({
            "tasks": tasks,
            "_solar_tool_title": "Update progress",
            "other": "unsupported",
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<TaskListSnapshot>(json!({
            "tasks": tasks,
            "_solar_tool_title": "Update progress",
        }))
        .is_err()
    );
}

#[test]
fn snapshot_enforces_stable_unique_nonempty_ids_and_nonempty_titles() {
    for tasks in [
        vec![task(" ", TaskListStatus::Pending)],
        vec![task("", TaskListStatus::Pending)],
        vec![
            task("one", TaskListStatus::Completed),
            task("one", TaskListStatus::Pending),
        ],
        vec![TaskListItem {
            id: "one".to_string(),
            title: "\t\n".to_string(),
            status: TaskListStatus::Pending,
        }],
    ] {
        assert!(TaskListSnapshot { tasks }.validate().is_err());
    }
}

#[test]
fn snapshot_bounds_count_and_unicode_character_lengths() {
    let mut snapshot = TaskListSnapshot {
        tasks: (0..MAX_TASK_LIST_ITEMS)
            .map(|index| task(&index.to_string(), TaskListStatus::Pending))
            .collect(),
    };
    assert!(snapshot.validate().is_ok());
    snapshot.tasks.push(task("extra", TaskListStatus::Pending));
    assert!(snapshot.validate().is_err());

    let mut snapshot = TaskListSnapshot {
        tasks: vec![TaskListItem {
            id: "🦀".repeat(MAX_TASK_ID_CHARS),
            title: "界".repeat(MAX_TASK_TITLE_CHARS),
            status: TaskListStatus::Pending,
        }],
    };
    assert!(snapshot.validate().is_ok());
    snapshot.tasks[0].id.push('🦀');
    assert!(snapshot.validate().is_err());
    snapshot.tasks[0].id.pop();
    snapshot.tasks[0].title.push('界');
    assert!(snapshot.validate().is_err());
}

#[test]
fn current_work_precedes_pending_tasks_while_completed_tasks_may_stay_in_place() {
    let snapshot = TaskListSnapshot {
        tasks: vec![
            task("done", TaskListStatus::Completed),
            task("working", TaskListStatus::InProgress),
            task("also-done", TaskListStatus::Completed),
            task("next", TaskListStatus::Pending),
        ],
    };
    assert!(snapshot.validate().is_ok());

    for tasks in [
        vec![
            task("later", TaskListStatus::Pending),
            task("done", TaskListStatus::Completed),
            task("working", TaskListStatus::InProgress),
        ],
        vec![
            task("one", TaskListStatus::InProgress),
            task("two", TaskListStatus::InProgress),
        ],
    ] {
        assert!(TaskListSnapshot { tasks }.validate().is_err());
    }
}

#[test]
fn result_parser_requires_marker_and_validates_snapshot_instead_of_trusting_content() {
    let tasks = json!([{"id": "one", "title": "Inspect", "status": "pending"}]);
    for content in [
        json!({"tasks": tasks}).to_string(),
        json!({"type": "something_else", "tasks": tasks}).to_string(),
        json!({"type": "task_list_snapshot", "tasks": []}).to_string(),
        json!({"type": "task_list_snapshot", "tasks": tasks, "unrelated": true}).to_string(),
        "not JSON".to_string(),
    ] {
        assert_eq!(task_list_from_tool_result(&content), None);
    }
}
