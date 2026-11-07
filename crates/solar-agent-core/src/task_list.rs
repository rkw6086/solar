use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub const UPDATE_TASK_LIST_NAME: &str = "update_task_list";
pub const MAX_TASK_LIST_ITEMS: usize = 50;
pub const MAX_TASK_ID_CHARS: usize = 200;
pub const MAX_TASK_TITLE_CHARS: usize = 1_000;

/// A complete replacement of the task list. Callers retain task IDs across updates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskListSnapshot {
    pub tasks: Vec<TaskListItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskListItem {
    pub id: String,
    pub title: String,
    pub status: TaskListStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskListStatus {
    Pending,
    InProgress,
    Completed,
}

impl TaskListSnapshot {
    pub fn from_arguments(arguments: &JsonValue) -> Result<Self, String> {
        let mut arguments = arguments.clone();
        if let Some(fields) = arguments.as_object_mut()
            && let Some(title) = fields.remove("_solar_tool_title")
            && !title.is_string()
        {
            return Err("The reserved tool title must be a string.".to_string());
        }
        let snapshot: Self = serde_json::from_value(arguments)
            .map_err(|error| format!("Invalid task list arguments: {error}"))?;
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.tasks.is_empty() || self.tasks.len() > MAX_TASK_LIST_ITEMS {
            return Err(format!(
                "Task lists must contain between 1 and {MAX_TASK_LIST_ITEMS} tasks."
            ));
        }

        let mut ids = HashSet::new();
        let mut has_in_progress = false;
        let mut has_pending = false;
        for task in &self.tasks {
            if task.id.trim().is_empty() || task.id.chars().count() > MAX_TASK_ID_CHARS {
                return Err(format!(
                    "Task IDs must be nonempty and at most {MAX_TASK_ID_CHARS} characters."
                ));
            }
            if !ids.insert(task.id.as_str()) {
                return Err(format!("Duplicate task ID: '{}'.", task.id));
            }
            if task.title.trim().is_empty() || task.title.chars().count() > MAX_TASK_TITLE_CHARS {
                return Err(format!(
                    "Task titles must be nonempty and at most {MAX_TASK_TITLE_CHARS} characters."
                ));
            }
            match task.status {
                TaskListStatus::Pending => has_pending = true,
                TaskListStatus::InProgress => {
                    if has_in_progress {
                        return Err("Only one task may be in progress at a time.".to_string());
                    }
                    if has_pending {
                        return Err(
                            "The task in progress must precede all pending tasks; reorder the task list to reflect the current work."
                                .to_string(),
                        );
                    }
                    has_in_progress = true;
                }
                TaskListStatus::Completed => {}
            }
        }
        Ok(())
    }

    pub fn to_tool_result_content(&self) -> String {
        serde_json::to_string(&TaskListToolResult {
            kind: TaskListResultKind::Snapshot,
            tasks: self.tasks.clone(),
        })
        .expect("task list snapshots contain only serializable strings and statuses")
    }
}

/// Recognizes a canonical snapshot result. The caller must first verify the owning
/// tool name and successful result status when reading conversation history.
pub fn task_list_from_tool_result(content: &str) -> Option<TaskListSnapshot> {
    let result: TaskListToolResult = serde_json::from_str(content).ok()?;
    let snapshot = TaskListSnapshot {
        tasks: result.tasks,
    };
    snapshot.validate().ok()?;
    Some(snapshot)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskListToolResult {
    #[serde(rename = "type")]
    kind: TaskListResultKind,
    tasks: Vec<TaskListItem>,
}

#[derive(Serialize, Deserialize)]
enum TaskListResultKind {
    #[serde(rename = "task_list_snapshot")]
    Snapshot,
}

#[cfg(test)]
#[path = "task_list_tests.rs"]
mod tests;
