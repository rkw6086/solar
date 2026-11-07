use std::collections::{BTreeSet, HashMap, VecDeque};

use serde_json::Value as JsonValue;

use crate::types::tool_image_group_call_id;
use crate::{
    ContentPart, ConversationMessage, MessageContent, TaskListSnapshot, ToolCall, ToolDefinition,
    ToolResult, ToolResultImage, ToolResultStatus, UPDATE_TASK_LIST_NAME,
};

pub const RECALL_TOOL_HISTORY_NAME: &str = "recall_tool_history";
const MAX_RECALLED_RESULT_CHARS: usize = 50_000;
const DEFAULT_LOOP_WINDOW: usize = 10;
const DEFAULT_LOOP_THRESHOLD: usize = 3;

#[derive(Clone, Debug, PartialEq)]
pub enum ToolCallDecision {
    Execute,
    Inline(ToolResult),
    Reject(ToolResult),
}

#[derive(Clone, Debug, Default)]
pub struct ToolPolicy {
    advertised_tools: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolFailureRecord {
    signature: u64,
    description: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolLoopDetected {
    pub description: String,
    pub threshold: usize,
}

#[derive(Clone, Debug)]
pub struct ToolLoopGuard {
    recent_failures: VecDeque<ToolFailureRecord>,
    window: usize,
    threshold: usize,
}

impl Default for ToolLoopGuard {
    fn default() -> Self {
        Self::new(DEFAULT_LOOP_WINDOW, DEFAULT_LOOP_THRESHOLD)
    }
}

impl ToolLoopGuard {
    pub fn new(window: usize, threshold: usize) -> Self {
        Self {
            recent_failures: VecDeque::new(),
            window: window.max(1),
            threshold: threshold.max(1),
        }
    }

    pub fn record_failure(&mut self, signature: u64, description: impl Into<String>) {
        self.recent_failures.push_back(ToolFailureRecord {
            signature,
            description: description.into(),
        });
        if self.recent_failures.len() > self.window {
            self.recent_failures.pop_front();
        }
    }

    pub fn record_success(&mut self) {
        self.recent_failures.clear();
    }

    pub fn detect_and_reset(&mut self) -> Option<ToolLoopDetected> {
        let mut counts = HashMap::new();
        for (index, failure) in self.recent_failures.iter().enumerate() {
            let count = counts.entry(failure.signature).or_insert((0usize, 0usize));
            count.0 += 1;
            count.1 = index;
        }
        let latest_index = counts
            .values()
            .filter(|(count, _)| *count >= self.threshold)
            .map(|(_, index)| *index)
            .max()?;
        let detected = ToolLoopDetected {
            description: self.recent_failures[latest_index].description.clone(),
            threshold: self.threshold,
        };
        self.recent_failures.clear();
        Some(detected)
    }
}

impl ToolPolicy {
    pub fn new(tools: &[ToolDefinition]) -> Self {
        Self::from_names(tools.iter().map(|tool| tool.name.clone()))
    }

    pub fn from_names(names: impl IntoIterator<Item = String>) -> Self {
        Self {
            advertised_tools: names.into_iter().collect(),
        }
    }

    pub fn decide(
        &self,
        call: &ToolCall,
        messages: &[ConversationMessage],
        archive: &[ConversationMessage],
    ) -> ToolCallDecision {
        if !self.advertised_tools.contains(&call.name) {
            let available = if self.advertised_tools.is_empty() {
                "no tools are available".to_string()
            } else {
                format!(
                    "available tools are: {}",
                    self.advertised_tools
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            return ToolCallDecision::Reject(ToolResult {
                images: Vec::new(),
                call_id: call.id.clone(),
                content: format!(
                    "Error: '{}' is not a valid tool for this request; {available}. Do not invent tool names.",
                    call.name
                ),
                status: ToolResultStatus::Error,
            });
        }

        if call.name == RECALL_TOOL_HISTORY_NAME {
            let (content, images) = recall_tool_history_with_images(
                messages,
                archive,
                ToolHistoryQuery::from_arguments(&call.arguments),
            );
            return ToolCallDecision::Inline(ToolResult {
                images,
                call_id: call.id.clone(),
                content,
                status: ToolResultStatus::Success,
            });
        }

        if call.name == UPDATE_TASK_LIST_NAME {
            return match TaskListSnapshot::from_arguments(&call.arguments) {
                Ok(snapshot) => ToolCallDecision::Inline(ToolResult {
                    images: Vec::new(),
                    call_id: call.id.clone(),
                    content: snapshot.to_tool_result_content(),
                    status: ToolResultStatus::Success,
                }),
                Err(error) => ToolCallDecision::Reject(ToolResult {
                    images: Vec::new(),
                    call_id: call.id.clone(),
                    content: format!("Error: {error}"),
                    status: ToolResultStatus::Error,
                }),
            };
        }

        ToolCallDecision::Execute
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ToolHistoryQuery<'a> {
    pub search_query: &'a str,
    pub tool_name: &'a str,
    pub tool_use_id: &'a str,
    pub offset_from_end: usize,
}

impl<'a> ToolHistoryQuery<'a> {
    fn from_arguments(arguments: &'a JsonValue) -> Self {
        Self {
            search_query: arguments
                .get("search_query")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            tool_name: arguments
                .get("tool_name")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            tool_use_id: arguments
                .get("tool_use_id")
                .and_then(JsonValue::as_str)
                .unwrap_or_default(),
            offset_from_end: arguments
                .get("offset_from_end")
                .and_then(JsonValue::as_u64)
                .and_then(|offset| usize::try_from(offset).ok())
                .unwrap_or_default(),
        }
    }
}

pub fn recall_tool_history(
    messages: &[ConversationMessage],
    archive: &[ConversationMessage],
    query: ToolHistoryQuery<'_>,
) -> String {
    recall_tool_history_with_images(messages, archive, query).0
}

fn recall_tool_history_with_images(
    messages: &[ConversationMessage],
    archive: &[ConversationMessage],
    query: ToolHistoryQuery<'_>,
) -> (String, Vec<ToolResultImage>) {
    let mut entries = Vec::new();
    collect_tool_entries(archive, &mut entries);
    collect_tool_entries(messages, &mut entries);

    let filtered = entries
        .iter()
        .filter(|entry| entry.name != RECALL_TOOL_HISTORY_NAME)
        .filter(|entry| {
            (query.tool_use_id.is_empty() || entry.tool_use_id == query.tool_use_id)
                && (query.tool_name.is_empty() || entry.name == query.tool_name)
                && (query.search_query.is_empty()
                    || format!("{} {} {}", entry.name, entry.input, entry.result)
                        .to_lowercase()
                        .contains(&query.search_query.to_lowercase()))
        })
        .collect::<Vec<_>>();

    let Some(index) = filtered
        .len()
        .checked_sub(1usize.saturating_add(query.offset_from_end))
    else {
        return (
            "No matching tool calls found in conversation history.".to_string(),
            Vec::new(),
        );
    };
    let entry = filtered[index];
    let result = truncate_recalled_result(&entry.result);

    (
        format!(
            "Tool: {}\nTool Use ID: {}\nInput: {}\nResult:\n{result}",
            entry.name, entry.tool_use_id, entry.input
        ),
        entry
            .images
            .iter()
            .map(|(data, mime_type)| ToolResultImage {
                data: data.to_vec(),
                mime_type: (*mime_type).to_string(),
            })
            .collect(),
    )
}

#[derive(Clone, Debug, PartialEq)]
struct ToolHistoryEntry<'a> {
    tool_use_id: String,
    name: String,
    input: String,
    result: String,
    images: Vec<(&'a [u8], &'a str)>,
}

fn collect_tool_entries<'a>(
    messages: &'a [ConversationMessage],
    entries: &mut Vec<ToolHistoryEntry<'a>>,
) {
    let mut pending = Vec::new();
    for message in messages {
        match &message.content {
            MessageContent::ToolUse {
                tool_use_id,
                name,
                input,
            } => pending.push((tool_use_id.clone(), name.clone(), input.to_string())),
            MessageContent::ToolResult {
                tool_use_id,
                content,
                ..
            } => pair_result(tool_use_id, content, &mut pending, entries),
            MessageContent::MultiPart(parts) => {
                let entries_start = entries.len();
                for part in parts {
                    match part {
                        ContentPart::ToolUse {
                            tool_use_id,
                            name,
                            input,
                        } => {
                            pending.push((tool_use_id.clone(), name.clone(), input.to_string()));
                        }
                        ContentPart::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => pair_result(tool_use_id, content, &mut pending, entries),
                        ContentPart::Text(_)
                        | ContentPart::Reasoning { .. }
                        | ContentPart::Image { .. } => {}
                    }
                }
                // ToolResult::into_batch_content_parts places labelled image groups
                // after every correlated result. Only attach images to results from
                // this same message, never a nearby user upload or another call.
                let result_ids: BTreeSet<_> = parts
                    .iter()
                    .filter_map(|part| match part {
                        ContentPart::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                        ContentPart::Text(_)
                        | ContentPart::Reasoning { .. }
                        | ContentPart::Image { .. }
                        | ContentPart::ToolUse { .. } => None,
                    })
                    .collect();
                let mut image_call_id = None;
                for part in parts {
                    match part {
                        ContentPart::Text(_) => {
                            image_call_id = tool_image_group_call_id(part)
                                .filter(|call_id| result_ids.contains(call_id));
                        }
                        ContentPart::Image { data, mime_type } => {
                            if let Some(entry) = image_call_id.and_then(|call_id| {
                                entries[entries_start..]
                                    .iter_mut()
                                    .rev()
                                    .find(|entry| entry.tool_use_id == call_id)
                            }) {
                                entry.images.push((data, mime_type));
                            }
                        }
                        ContentPart::Reasoning { .. }
                        | ContentPart::ToolUse { .. }
                        | ContentPart::ToolResult { .. } => image_call_id = None,
                    }
                }
            }
            MessageContent::Text(_) => {}
        }
    }
}

fn pair_result(
    tool_use_id: &str,
    content: &str,
    pending: &mut Vec<(String, String, String)>,
    entries: &mut Vec<ToolHistoryEntry<'_>>,
) {
    let Some(index) = pending.iter().position(|(id, _, _)| id == tool_use_id) else {
        return;
    };
    let (tool_use_id, name, input) = pending.remove(index);
    entries.push(ToolHistoryEntry {
        tool_use_id,
        name,
        input,
        result: content.to_string(),
        images: Vec::new(),
    });
}

fn truncate_recalled_result(result: &str) -> String {
    let char_count = result.chars().count();
    if char_count <= MAX_RECALLED_RESULT_CHARS {
        return result.to_string();
    }
    let truncated = result
        .chars()
        .take(MAX_RECALLED_RESULT_CHARS)
        .collect::<String>();
    format!("{truncated}... [truncated, {char_count} total chars]")
}

#[cfg(test)]
#[path = "tool_policy_tests.rs"]
mod tests;
