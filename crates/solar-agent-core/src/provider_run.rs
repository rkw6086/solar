use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::error::Error;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::types::tool_image_group_call_id;
use crate::{
    AgentError, ContentPart, ConversationMessage, MessageContent, MessageRole, PermissionDecision,
    PermissionRequest, StopReason, TaskListSnapshot, ToolCall, ToolCallDecision, ToolPolicy,
    ToolResult, ToolResultStatus, UPDATE_TASK_LIST_NAME, Usage, task_list_from_tool_result,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderRunId(String);

impl ProviderRunId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProviderRunId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ProviderRunId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct RunEpoch(u64);

impl RunEpoch {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }

    fn checked_next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalWorkId {
    pub run_id: ProviderRunId,
    pub epoch: RunEpoch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderRequestProfile(String);

impl ProviderRequestProfile {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ProviderRequestProfile {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for ProviderRequestProfile {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunLimits {
    /// Optional lifetime budget. Interactive runs continue until completion or cancellation.
    pub max_model_turns: Option<u32>,
    pub max_model_retries_per_turn: u32,
}

impl Default for ProviderRunLimits {
    fn default() -> Self {
        Self {
            max_model_turns: None,
            // Three retries means the initial call plus three delayed retries;
            // the fourth failure is surfaced to the user.
            max_model_retries_per_turn: 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderModelCall {
    pub work_id: ExternalWorkId,
    pub profile: ProviderRequestProfile,
    pub messages: Vec<ConversationMessage>,
    pub retry_attempt: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompletedModelTurn {
    /// Canonical non-tool assistant content. Tool calls are appended in `tool_calls` order.
    pub assistant_content: Vec<ContentPart>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Usage,
    pub stop_reason: StopReason,
    pub advertised_tools: BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingModelCall {
    pub work_id: ExternalWorkId,
    pub retry_attempt: u32,
    pub last_error: Option<AgentError>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PendingToolCallState {
    Proposed,
    PermissionPending {
        request: PermissionRequest,
    },
    Approved {
        request_id: String,
        decision: PermissionDecision,
    },
    Executing,
    /// External work survived a restart and must be reattached before it can complete.
    RecoveryPending,
    Resolved {
        result: ToolResult,
    },
}

impl PendingToolCallState {
    pub fn result(&self) -> Option<&ToolResult> {
        match self {
            Self::Resolved { result } => Some(result),
            Self::Proposed
            | Self::PermissionPending { .. }
            | Self::Approved { .. }
            | Self::Executing
            | Self::RecoveryPending => None,
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::PermissionPending { .. } => "permission_pending",
            Self::Approved { .. } => "approved",
            Self::Executing => "executing",
            Self::RecoveryPending => "recovery_pending",
            Self::Resolved { .. } => "resolved",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingToolCall {
    pub call: ToolCall,
    pub state: PendingToolCallState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingToolBatch {
    pub work_id: ExternalWorkId,
    pub calls: Vec<PendingToolCall>,
}

impl PendingToolBatch {
    pub fn is_complete(&self) -> bool {
        self.calls.iter().all(|call| call.state.result().is_some())
    }

    pub fn unresolved_call_ids(&self) -> Vec<String> {
        self.calls
            .iter()
            .filter(|call| call.state.result().is_none())
            .map(|call| call.call.id.clone())
            .collect()
    }

    fn ordered_results(&self) -> Option<Vec<ToolResult>> {
        self.calls
            .iter()
            .map(|call| call.state.result().cloned())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProviderRunState {
    ReadyToCallModel,
    AwaitingModel {
        call: PendingModelCall,
    },
    ResolvingModel {
        turn: CompletedModelTurn,
    },
    AwaitingTools {
        batch: PendingToolBatch,
    },
    AwaitingDriver {
        work_id: ExternalWorkId,
        stop_reason: StopReason,
    },
    Done {
        completion: ProviderRunCompletion,
    },
    Failed {
        failure: ProviderRunFailure,
    },
    Cancelled {
        reason: String,
    },
}

impl ProviderRunState {
    pub fn phase(&self) -> ProviderRunPhase {
        match self {
            Self::ReadyToCallModel => ProviderRunPhase::ReadyToCallModel,
            Self::AwaitingModel { .. } => ProviderRunPhase::AwaitingModel,
            Self::ResolvingModel { .. } => ProviderRunPhase::ResolvingModel,
            Self::AwaitingTools { .. } => ProviderRunPhase::AwaitingTools,
            Self::AwaitingDriver { .. } => ProviderRunPhase::AwaitingDriver,
            Self::Done { .. } => ProviderRunPhase::Done,
            Self::Failed { .. } => ProviderRunPhase::Failed,
            Self::Cancelled { .. } => ProviderRunPhase::Cancelled,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunPhase {
    ReadyToCallModel,
    AwaitingModel,
    ResolvingModel,
    AwaitingTools,
    AwaitingDriver,
    Done,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunCompletion {
    pub stop_reason: StopReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunFailureKind {
    ModelCall,
    RetryLimitExceeded,
    TurnLimitExceeded,
    Protocol,
    Projection,
    Restore,
    ExternalWork,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRunFailure {
    pub kind: ProviderRunFailureKind,
    pub message: String,
    pub source: Option<AgentError>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunOutcome {
    Completed(ProviderRunCompletion),
    Failed(ProviderRunFailure),
    Cancelled { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ProviderRunStep {
    CallModel(ProviderModelCall),
    DispatchTools(PendingToolBatch),
    Done(ProviderRunOutcome),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelFailureDisposition {
    RetryScheduled,
    RunFailed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderRunRestoreNormalization {
    pub interrupted_model_call: bool,
    pub permission_call_ids_reset: Vec<String>,
    pub interrupted_call_ids: Vec<String>,
    pub recovery_call_ids: Vec<String>,
    pub committed_tool_batch: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderRunProtocolError {
    WorkMismatch {
        expected: ExternalWorkId,
        received: ExternalWorkId,
    },
    UnexpectedState {
        expected: ProviderRunPhase,
        actual: ProviderRunPhase,
    },
    InvalidModelTurn {
        message: String,
    },
    UnknownToolCall {
        call_id: String,
    },
    DuplicateToolUpdate {
        call_id: String,
    },
    InvalidToolTransition {
        call_id: String,
        state: String,
        update: String,
    },
    PermissionRequestMismatch {
        call_id: String,
        expected_request_id: String,
        received_request_id: String,
    },
    IncompleteToolBatch {
        missing_call_ids: Vec<String>,
    },
    ToolResultSetMismatch {
        expected_call_ids: Vec<String>,
        received_call_ids: Vec<String>,
    },
    DuplicateToolResult {
        call_id: String,
    },
    InvalidDriverObservation {
        message: String,
    },
    InvalidTranscriptCompaction {
        message: String,
    },
    InvalidTaskList {
        message: String,
    },
    InvalidRestoredState {
        message: String,
    },
    EpochExhausted,
    Terminal,
}

impl fmt::Display for ProviderRunProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkMismatch { expected, received } => write!(
                f,
                "external work mismatch: expected {}:{}, received {}:{}",
                expected.run_id.as_str(),
                expected.epoch.get(),
                received.run_id.as_str(),
                received.epoch.get()
            ),
            Self::UnexpectedState { expected, actual } => {
                write!(f, "expected run phase {expected:?}, found {actual:?}")
            }
            Self::InvalidModelTurn { message } => write!(f, "invalid model turn: {message}"),
            Self::UnknownToolCall { call_id } => write!(f, "unknown tool call '{call_id}'"),
            Self::DuplicateToolUpdate { call_id } => {
                write!(f, "tool call '{call_id}' already has a result")
            }
            Self::InvalidToolTransition {
                call_id,
                state,
                update,
            } => write!(
                f,
                "cannot apply {update} to tool call '{call_id}' while it is {state}"
            ),
            Self::PermissionRequestMismatch {
                call_id,
                expected_request_id,
                received_request_id,
            } => write!(
                f,
                "permission request mismatch for '{call_id}': expected '{expected_request_id}', received '{received_request_id}'"
            ),
            Self::IncompleteToolBatch { missing_call_ids } => write!(
                f,
                "tool batch is incomplete; missing results for {}",
                missing_call_ids.join(", ")
            ),
            Self::ToolResultSetMismatch {
                expected_call_ids,
                received_call_ids,
            } => write!(
                f,
                "tool result set mismatch: expected [{}], received [{}]",
                expected_call_ids.join(", "),
                received_call_ids.join(", ")
            ),
            Self::DuplicateToolResult { call_id } => {
                write!(f, "duplicate tool result for '{call_id}'")
            }
            Self::InvalidDriverObservation { message } => {
                write!(f, "invalid driver observation: {message}")
            }
            Self::InvalidTranscriptCompaction { message } => {
                write!(f, "invalid transcript compaction: {message}")
            }
            Self::InvalidTaskList { message } => write!(f, "invalid task list: {message}"),
            Self::InvalidRestoredState { message } => {
                write!(f, "invalid restored provider run: {message}")
            }
            Self::EpochExhausted => f.write_str("provider run epoch is exhausted"),
            Self::Terminal => f.write_str("provider run is already terminal"),
        }
    }
}

impl Error for ProviderRunProtocolError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProviderRun {
    id: ProviderRunId,
    epoch: RunEpoch,
    transcript: Vec<ConversationMessage>,
    tool_result_archive: Vec<ConversationMessage>,
    #[serde(default)]
    task_list: Option<TaskListSnapshot>,
    usage: Usage,
    model_turns: u32,
    model_retries: u32,
    limits: ProviderRunLimits,
    profile: ProviderRequestProfile,
    state: ProviderRunState,
}

impl ProviderRun {
    pub fn new(
        id: impl Into<ProviderRunId>,
        transcript: Vec<ConversationMessage>,
        profile: impl Into<ProviderRequestProfile>,
        mut limits: ProviderRunLimits,
    ) -> Self {
        limits.max_model_turns = limits.max_model_turns.map(|limit| limit.max(1));
        Self {
            id: id.into(),
            epoch: RunEpoch::default(),
            transcript,
            tool_result_archive: Vec::new(),
            task_list: None,
            usage: Usage::default(),
            model_turns: 0,
            model_retries: 0,
            limits,
            profile: profile.into(),
            state: ProviderRunState::ReadyToCallModel,
        }
    }

    pub fn id(&self) -> &ProviderRunId {
        &self.id
    }

    pub fn epoch(&self) -> RunEpoch {
        self.epoch
    }

    pub fn profile(&self) -> &ProviderRequestProfile {
        &self.profile
    }

    pub fn state(&self) -> &ProviderRunState {
        &self.state
    }

    pub fn transcript(&self) -> &[ConversationMessage] {
        &self.transcript
    }

    pub fn task_list(&self) -> Option<&TaskListSnapshot> {
        self.task_list.as_ref()
    }

    pub fn set_task_list(
        &mut self,
        task_list: Option<TaskListSnapshot>,
    ) -> Result<(), ProviderRunProtocolError> {
        if let Some(snapshot) = &task_list {
            snapshot
                .validate()
                .map_err(|message| ProviderRunProtocolError::InvalidTaskList { message })?;
        }
        self.task_list = task_list;
        Ok(())
    }

    pub fn tool_result_archive(&self) -> &[ConversationMessage] {
        &self.tool_result_archive
    }

    pub fn replace_tool_result_archive(&mut self, archive: Vec<ConversationMessage>) {
        self.tool_result_archive = archive;
        archive_tool_results(&mut self.tool_result_archive, Vec::new());
    }

    /// Replaces a summarized prefix while the run is parked between model calls.
    ///
    /// Tool calls and results removed from the live transcript remain available to
    /// `recall_tool_history` through the run-owned archive.
    pub fn compact_transcript_at_model_boundary(
        &mut self,
        summarized_messages: usize,
        summary_prefix: Vec<ConversationMessage>,
    ) -> Result<(), ProviderRunProtocolError> {
        if !matches!(self.state, ProviderRunState::ReadyToCallModel) {
            return Err(self.unexpected_state(ProviderRunPhase::ReadyToCallModel));
        }
        if summarized_messages == 0 || summarized_messages > self.transcript.len() {
            return Err(ProviderRunProtocolError::InvalidTranscriptCompaction {
                message: format!(
                    "cannot summarize {summarized_messages} of {} messages",
                    self.transcript.len()
                ),
            });
        }

        let drained = self
            .transcript
            .drain(0..summarized_messages)
            .collect::<Vec<_>>();
        archive_tool_results(&mut self.tool_result_archive, drained);
        self.transcript.splice(0..0, summary_prefix);
        Ok(())
    }

    pub fn usage(&self) -> &Usage {
        &self.usage
    }

    pub fn model_turns(&self) -> u32 {
        self.model_turns
    }

    pub fn model_retries(&self) -> u32 {
        self.model_retries
    }

    pub fn max_model_retries_per_turn(&self) -> u32 {
        self.limits.max_model_retries_per_turn
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state,
            ProviderRunState::Done { .. }
                | ProviderRunState::Failed { .. }
                | ProviderRunState::Cancelled { .. }
        )
    }

    /// Validates persisted state before restore normalization can mutate it or external work can
    /// be reconstructed from it.
    pub fn validate_restored_state(&self) -> Result<(), ProviderRunProtocolError> {
        let invalid = |message: String| ProviderRunProtocolError::InvalidRestoredState { message };

        if self.id.as_str().is_empty() {
            return Err(invalid("run ID must not be empty".to_string()));
        }
        if self.profile.as_str().is_empty() {
            return Err(invalid("request profile must not be empty".to_string()));
        }
        if let Some(snapshot) = &self.task_list {
            snapshot
                .validate()
                .map_err(|error| invalid(format!("invalid task list: {error}")))?;
        }
        if self.limits.max_model_turns == Some(0) {
            return Err(invalid("model-turn limit must be at least one".to_string()));
        }
        if let Some(limit) = self.limits.max_model_turns
            && self.model_turns > limit
        {
            return Err(invalid(format!(
                "model-turn counter {} exceeds limit {limit}",
                self.model_turns
            )));
        }
        if self.epoch.get() < u64::from(self.model_turns) {
            return Err(invalid(format!(
                "epoch {} is behind model-turn counter {}",
                self.epoch.get(),
                self.model_turns
            )));
        }
        let retry_slots = u64::from(self.model_turns)
            .saturating_add(1)
            .saturating_mul(u64::from(self.limits.max_model_retries_per_turn));
        if u64::from(self.model_retries) > retry_slots {
            return Err(invalid(format!(
                "model-retry counter {} exceeds maximum possible {}",
                self.model_retries, retry_slots
            )));
        }

        if let Some(work_id) = self.active_work_id() {
            self.validate_restored_work_id(work_id)?;
        }

        match &self.state {
            ProviderRunState::ReadyToCallModel => {}
            ProviderRunState::AwaitingModel { call } => {
                if self.model_turn_limit_reached() {
                    return Err(invalid(
                        "awaiting a model call after reaching the model-turn limit".to_string(),
                    ));
                }
                if call.retry_attempt > self.limits.max_model_retries_per_turn {
                    return Err(invalid(format!(
                        "pending retry attempt {} exceeds per-turn limit {}",
                        call.retry_attempt, self.limits.max_model_retries_per_turn
                    )));
                }
                if call.retry_attempt > self.model_retries {
                    return Err(invalid(format!(
                        "pending retry attempt {} exceeds total retry counter {}",
                        call.retry_attempt, self.model_retries
                    )));
                }
                if (call.retry_attempt == 0) != call.last_error.is_none() {
                    return Err(invalid(
                        "pending retry error does not match its retry attempt".to_string(),
                    ));
                }
                if call
                    .last_error
                    .as_ref()
                    .is_some_and(|error| !error.recoverable)
                {
                    return Err(invalid(
                        "pending retry retains a non-recoverable model error".to_string(),
                    ));
                }
            }
            ProviderRunState::ResolvingModel { turn } => {
                self.validate_post_model_phase()?;
                validate_model_turn(turn).map_err(|error| invalid(error.to_string()))?;
                if self.transcript.last() != Some(&assistant_message(turn)) {
                    return Err(invalid(
                        "resolving model turn does not own the latest transcript message"
                            .to_string(),
                    ));
                }
                if !usage_contains(&self.usage, &turn.usage) {
                    return Err(invalid(
                        "aggregate usage does not include the resolving model turn".to_string(),
                    ));
                }
            }
            ProviderRunState::AwaitingTools { batch } => {
                if batch.calls.is_empty() {
                    return Err(invalid("pending tool batch is empty".to_string()));
                }
                self.validate_post_model_phase()?;
                let mut call_ids = HashSet::new();
                for pending in &batch.calls {
                    if pending.call.id.is_empty() {
                        return Err(invalid("pending tool call ID is empty".to_string()));
                    }
                    if !call_ids.insert(pending.call.id.as_str()) {
                        return Err(invalid(format!(
                            "duplicate pending tool call ID '{}'",
                            pending.call.id
                        )));
                    }
                    validate_pending_tool_state(pending).map_err(invalid)?;
                }
                let transcript_calls = self
                    .transcript
                    .last()
                    .map(tool_calls_from_message)
                    .unwrap_or_default();
                if transcript_calls
                    != batch
                        .calls
                        .iter()
                        .map(|pending| pending.call.clone())
                        .collect::<Vec<_>>()
                {
                    return Err(invalid(
                        "pending tool batch does not match the latest assistant message"
                            .to_string(),
                    ));
                }
            }
            ProviderRunState::AwaitingDriver { .. } => {
                self.validate_post_model_phase()?;
                let Some(message) = self.transcript.last() else {
                    return Err(invalid(
                        "driver wait is missing its assistant transcript message".to_string(),
                    ));
                };
                if message.role != MessageRole::Assistant {
                    return Err(invalid(
                        "driver wait does not follow an assistant transcript message".to_string(),
                    ));
                }
                if !tool_calls_from_message(message).is_empty() {
                    return Err(invalid(
                        "driver wait follows an uncommitted assistant tool call".to_string(),
                    ));
                }
            }
            ProviderRunState::Done { .. } => {
                if self.model_turns == 0 {
                    return Err(invalid(
                        "completed run has no completed model turn".to_string(),
                    ));
                }
            }
            ProviderRunState::Failed { failure } => match failure.kind {
                ProviderRunFailureKind::ModelCall
                    if failure
                        .source
                        .as_ref()
                        .is_none_or(|source| source.recoverable) =>
                {
                    return Err(invalid(
                        "model-call failure lacks a non-recoverable source".to_string(),
                    ));
                }
                ProviderRunFailureKind::RetryLimitExceeded
                    if failure
                        .source
                        .as_ref()
                        .is_none_or(|source| !source.recoverable) =>
                {
                    return Err(invalid(
                        "retry-limit failure lacks a recoverable source".to_string(),
                    ));
                }
                ProviderRunFailureKind::TurnLimitExceeded if !self.model_turn_limit_reached() => {
                    return Err(invalid(
                        "turn-limit failure occurred before reaching the limit".to_string(),
                    ));
                }
                ProviderRunFailureKind::ModelCall
                | ProviderRunFailureKind::RetryLimitExceeded
                | ProviderRunFailureKind::TurnLimitExceeded
                | ProviderRunFailureKind::Protocol
                | ProviderRunFailureKind::Projection
                | ProviderRunFailureKind::Restore
                | ProviderRunFailureKind::ExternalWork => {}
            },
            ProviderRunState::Cancelled { .. } => {}
        }
        Ok(())
    }

    fn validate_restored_work_id(
        &self,
        work_id: &ExternalWorkId,
    ) -> Result<(), ProviderRunProtocolError> {
        validate_work_id(&self.current_work_id(), work_id).map_err(|_| {
            ProviderRunProtocolError::InvalidRestoredState {
                message: format!(
                    "active work identity {}:{} does not match run {}:{}",
                    work_id.run_id.as_str(),
                    work_id.epoch.get(),
                    self.id.as_str(),
                    self.epoch.get()
                ),
            }
        })
    }

    fn validate_post_model_phase(&self) -> Result<(), ProviderRunProtocolError> {
        if self.model_turns == 0 {
            Err(ProviderRunProtocolError::InvalidRestoredState {
                message: format!("{:?} phase has no completed model turn", self.state.phase()),
            })
        } else {
            Ok(())
        }
    }

    pub fn normalize_after_restore(
        &mut self,
    ) -> Result<ProviderRunRestoreNormalization, ProviderRunProtocolError> {
        self.normalize_after_restore_with_recoverable_calls(&HashSet::new())
    }

    pub fn normalize_after_restore_with_recoverable_calls(
        &mut self,
        recoverable_call_ids: &HashSet<String>,
    ) -> Result<ProviderRunRestoreNormalization, ProviderRunProtocolError> {
        archive_tool_results(&mut self.tool_result_archive, Vec::new());
        if matches!(self.state, ProviderRunState::AwaitingModel { .. }) {
            self.state = ProviderRunState::Failed {
                failure: ProviderRunFailure {
                    kind: ProviderRunFailureKind::Restore,
                    message: "The model call was interrupted by application restart after dispatch may have begun. Its outcome is unknown, so it was not replayed to avoid duplicate billing or output."
                        .to_string(),
                    source: None,
                },
            };
            return Ok(ProviderRunRestoreNormalization {
                interrupted_model_call: true,
                ..ProviderRunRestoreNormalization::default()
            });
        }

        let ProviderRunState::AwaitingTools { batch } = &mut self.state else {
            return Ok(ProviderRunRestoreNormalization::default());
        };

        let mut normalization = ProviderRunRestoreNormalization::default();
        for pending in &mut batch.calls {
            match &pending.state {
                PendingToolCallState::PermissionPending { .. } => {
                    normalization
                        .permission_call_ids_reset
                        .push(pending.call.id.clone());
                    pending.state = PendingToolCallState::Proposed;
                }
                PendingToolCallState::Executing
                    if recoverable_call_ids.contains(&pending.call.id) =>
                {
                    normalization
                        .recovery_call_ids
                        .push(pending.call.id.clone());
                    pending.state = PendingToolCallState::RecoveryPending;
                }
                PendingToolCallState::RecoveryPending => {
                    normalization
                        .recovery_call_ids
                        .push(pending.call.id.clone());
                }
                PendingToolCallState::Approved { .. } | PendingToolCallState::Executing => {
                    normalization
                        .interrupted_call_ids
                        .push(pending.call.id.clone());
                    pending.state = PendingToolCallState::Resolved {
                        result: ToolResult {
                            images: Vec::new(),
                            call_id: pending.call.id.clone(),
                            content: "Tool execution was interrupted by application restart and was not replayed."
                                .to_string(),
                            status: ToolResultStatus::Error,
                        },
                    };
                }
                PendingToolCallState::Proposed | PendingToolCallState::Resolved { .. } => {}
            }
        }

        if batch.is_complete() {
            let work_id = batch.work_id.clone();
            self.commit_tool_batch(&work_id)?;
            normalization.committed_tool_batch = true;
        }
        Ok(normalization)
    }

    pub fn active_work_id(&self) -> Option<&ExternalWorkId> {
        match &self.state {
            ProviderRunState::AwaitingModel { call } => Some(&call.work_id),
            ProviderRunState::AwaitingTools { batch } => Some(&batch.work_id),
            ProviderRunState::AwaitingDriver { work_id, .. } => Some(work_id),
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => None,
        }
    }

    pub fn ready_work_id(&self) -> Option<ExternalWorkId> {
        matches!(self.state, ProviderRunState::ReadyToCallModel).then(|| self.current_work_id())
    }

    /// Selects the runtime for the next turn after a tool batch has been committed.
    pub fn set_ready_profile(
        &mut self,
        profile: impl Into<ProviderRequestProfile>,
    ) -> Result<(), ProviderRunProtocolError> {
        if !matches!(self.state, ProviderRunState::ReadyToCallModel) {
            return Err(self.unexpected_state(ProviderRunPhase::ReadyToCallModel));
        }
        self.profile = profile.into();
        Ok(())
    }

    fn model_turn_limit_reached(&self) -> bool {
        self.limits
            .max_model_turns
            .is_some_and(|limit| self.model_turns >= limit)
    }

    pub fn next_step(&mut self) -> Result<Option<ProviderRunStep>, ProviderRunProtocolError> {
        loop {
            match self.state.clone() {
                ProviderRunState::ReadyToCallModel => {
                    if let Some(limit) = self
                        .limits
                        .max_model_turns
                        .filter(|limit| self.model_turns >= *limit)
                    {
                        self.state = ProviderRunState::Failed {
                            failure: ProviderRunFailure {
                                kind: ProviderRunFailureKind::TurnLimitExceeded,
                                message: format!(
                                    "provider run reached its {limit} model-turn limit"
                                ),
                                source: None,
                            },
                        };
                        continue;
                    }
                    self.state = ProviderRunState::AwaitingModel {
                        call: PendingModelCall {
                            work_id: self.current_work_id(),
                            retry_attempt: 0,
                            last_error: None,
                        },
                    };
                }
                ProviderRunState::AwaitingModel { call } => {
                    return Ok(Some(ProviderRunStep::CallModel(ProviderModelCall {
                        work_id: call.work_id,
                        profile: self.profile.clone(),
                        messages: self.transcript.clone(),
                        retry_attempt: call.retry_attempt,
                    })));
                }
                ProviderRunState::ResolvingModel { turn } => {
                    if turn.tool_calls.is_empty() {
                        self.state = ProviderRunState::AwaitingDriver {
                            work_id: self.current_work_id(),
                            stop_reason: turn.stop_reason,
                        };
                    } else {
                        let batch = self.build_tool_batch(&turn);
                        self.state = ProviderRunState::AwaitingTools { batch };
                    }
                }
                ProviderRunState::AwaitingTools { batch } => {
                    return Ok(Some(ProviderRunStep::DispatchTools(batch)));
                }
                ProviderRunState::AwaitingDriver { .. } => return Ok(None),
                ProviderRunState::Done { completion } => {
                    return Ok(Some(ProviderRunStep::Done(ProviderRunOutcome::Completed(
                        completion,
                    ))));
                }
                ProviderRunState::Failed { failure } => {
                    return Ok(Some(ProviderRunStep::Done(ProviderRunOutcome::Failed(
                        failure,
                    ))));
                }
                ProviderRunState::Cancelled { reason } => {
                    return Ok(Some(ProviderRunStep::Done(ProviderRunOutcome::Cancelled {
                        reason,
                    })));
                }
            }
        }
    }

    pub fn accept_model_turn(
        &mut self,
        work_id: &ExternalWorkId,
        turn: CompletedModelTurn,
    ) -> Result<(), ProviderRunProtocolError> {
        let expected = match &self.state {
            ProviderRunState::AwaitingModel { call } => &call.work_id,
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingTools { .. }
            | ProviderRunState::AwaitingDriver { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(self.unexpected_state(ProviderRunPhase::AwaitingModel));
            }
        };
        validate_work_id(expected, work_id)?;
        validate_model_turn(&turn)?;
        let next_epoch = self.next_epoch()?;
        let assistant_message = assistant_message(&turn);

        self.transcript.push(assistant_message);
        add_usage(&mut self.usage, &turn.usage);
        self.model_turns = self.model_turns.saturating_add(1);
        self.epoch = next_epoch;
        self.state = ProviderRunState::ResolvingModel { turn };
        Ok(())
    }

    pub fn register_model_failure(
        &mut self,
        work_id: &ExternalWorkId,
        error: AgentError,
    ) -> Result<ModelFailureDisposition, ProviderRunProtocolError> {
        let call = match &mut self.state {
            ProviderRunState::AwaitingModel { call } => call,
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingTools { .. }
            | ProviderRunState::AwaitingDriver { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(self.unexpected_state(ProviderRunPhase::AwaitingModel));
            }
        };
        validate_work_id(&call.work_id, work_id)?;

        if error.recoverable && call.retry_attempt < self.limits.max_model_retries_per_turn {
            call.retry_attempt = call.retry_attempt.saturating_add(1);
            call.last_error = Some(error);
            self.model_retries = self.model_retries.saturating_add(1);
            return Ok(ModelFailureDisposition::RetryScheduled);
        }

        let kind = if error.recoverable {
            ProviderRunFailureKind::RetryLimitExceeded
        } else {
            ProviderRunFailureKind::ModelCall
        };
        let message = if error.recoverable {
            format!(
                "provider model call failed after {} retries: {}",
                call.retry_attempt, error.message
            )
        } else {
            error.message.clone()
        };
        self.state = ProviderRunState::Failed {
            failure: ProviderRunFailure {
                kind,
                message,
                source: Some(error),
            },
        };
        Ok(ModelFailureDisposition::RunFailed)
    }

    /// Parks a failed model call so its transcript can be compacted before retrying it.
    /// The run and current work identity are intentionally unchanged.
    pub fn prepare_context_compaction(
        &mut self,
        work_id: &ExternalWorkId,
    ) -> Result<(), ProviderRunProtocolError> {
        let call = match &self.state {
            ProviderRunState::AwaitingModel { call } => call,
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingTools { .. }
            | ProviderRunState::AwaitingDriver { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(self.unexpected_state(ProviderRunPhase::AwaitingModel));
            }
        };
        validate_work_id(&call.work_id, work_id)?;
        self.state = ProviderRunState::ReadyToCallModel;
        Ok(())
    }

    pub fn request_tool_permission(
        &mut self,
        work_id: &ExternalWorkId,
        request: PermissionRequest,
    ) -> Result<(), ProviderRunProtocolError> {
        let call = self.pending_tool_call_mut(work_id, &request.call_id)?;
        match &call.state {
            PendingToolCallState::Proposed => {
                call.state = PendingToolCallState::PermissionPending { request };
                Ok(())
            }
            PendingToolCallState::PermissionPending { request: pending } if pending == &request => {
                Ok(())
            }
            PendingToolCallState::Resolved { .. } => {
                Err(ProviderRunProtocolError::DuplicateToolUpdate {
                    call_id: call.call.id.clone(),
                })
            }
            PendingToolCallState::PermissionPending { .. }
            | PendingToolCallState::Approved { .. }
            | PendingToolCallState::Executing
            | PendingToolCallState::RecoveryPending => {
                Err(invalid_tool_transition(call, "permission request"))
            }
        }
    }

    pub fn resolve_tool_permission(
        &mut self,
        work_id: &ExternalWorkId,
        call_id: &str,
        request_id: &str,
        decision: PermissionDecision,
    ) -> Result<(), ProviderRunProtocolError> {
        let call = self.pending_tool_call_mut(work_id, call_id)?;
        let pending_request_id = match &call.state {
            PendingToolCallState::PermissionPending { request } => request.id.clone(),
            PendingToolCallState::Approved {
                request_id: approved_request_id,
                decision: approved_decision,
            } if approved_request_id == request_id && approved_decision == &decision => {
                return Ok(());
            }
            PendingToolCallState::Resolved { .. } => {
                return Err(ProviderRunProtocolError::DuplicateToolUpdate {
                    call_id: call.call.id.clone(),
                });
            }
            PendingToolCallState::Proposed
            | PendingToolCallState::Approved { .. }
            | PendingToolCallState::Executing
            | PendingToolCallState::RecoveryPending => {
                return Err(invalid_tool_transition(call, "permission resolution"));
            }
        };
        if pending_request_id != request_id {
            return Err(ProviderRunProtocolError::PermissionRequestMismatch {
                call_id: call_id.to_string(),
                expected_request_id: pending_request_id,
                received_request_id: request_id.to_string(),
            });
        }

        match decision {
            PermissionDecision::AllowOnce | PermissionDecision::AlwaysAllow => {
                call.state = PendingToolCallState::Approved {
                    request_id: request_id.to_string(),
                    decision,
                };
            }
            PermissionDecision::Denied { reason } => {
                let content = reason.unwrap_or_else(|| "Tool permission was denied.".to_string());
                call.state = PendingToolCallState::Resolved {
                    result: ToolResult {
                        images: Vec::new(),
                        call_id: call_id.to_string(),
                        content,
                        status: ToolResultStatus::Denied,
                    },
                };
            }
        }
        Ok(())
    }

    pub fn start_tool(
        &mut self,
        work_id: &ExternalWorkId,
        call_id: &str,
    ) -> Result<(), ProviderRunProtocolError> {
        let call = self.pending_tool_call_mut(work_id, call_id)?;
        match &call.state {
            PendingToolCallState::Proposed
            | PendingToolCallState::Approved { .. }
            | PendingToolCallState::RecoveryPending => {
                call.state = PendingToolCallState::Executing;
                Ok(())
            }
            PendingToolCallState::Resolved { .. } => {
                Err(ProviderRunProtocolError::DuplicateToolUpdate {
                    call_id: call.call.id.clone(),
                })
            }
            PendingToolCallState::Executing => Ok(()),
            PendingToolCallState::PermissionPending { .. } => {
                Err(invalid_tool_transition(call, "tool start"))
            }
        }
    }

    pub fn complete_tool(
        &mut self,
        work_id: &ExternalWorkId,
        result: ToolResult,
    ) -> Result<(), ProviderRunProtocolError> {
        let call = self.pending_tool_call_mut(work_id, &result.call_id)?;
        match &call.state {
            PendingToolCallState::Proposed
            | PendingToolCallState::Approved { .. }
            | PendingToolCallState::Executing
            | PendingToolCallState::RecoveryPending => {
                call.state = PendingToolCallState::Resolved { result };
                Ok(())
            }
            PendingToolCallState::Resolved {
                result: completed_result,
            } if completed_result == &result => Ok(()),
            PendingToolCallState::Resolved { .. } => {
                Err(ProviderRunProtocolError::DuplicateToolUpdate {
                    call_id: call.call.id.clone(),
                })
            }
            PendingToolCallState::PermissionPending { .. } => {
                Err(invalid_tool_transition(call, "tool completion"))
            }
        }
    }

    pub fn cancel_tool(
        &mut self,
        work_id: &ExternalWorkId,
        call_id: &str,
        reason: impl Into<String>,
    ) -> Result<(), ProviderRunProtocolError> {
        let call = self.pending_tool_call_mut(work_id, call_id)?;
        match &call.state {
            PendingToolCallState::Resolved { .. } => {
                Err(ProviderRunProtocolError::DuplicateToolUpdate {
                    call_id: call.call.id.clone(),
                })
            }
            PendingToolCallState::Proposed
            | PendingToolCallState::PermissionPending { .. }
            | PendingToolCallState::Approved { .. }
            | PendingToolCallState::Executing
            | PendingToolCallState::RecoveryPending => {
                call.state = PendingToolCallState::Resolved {
                    result: ToolResult {
                        images: Vec::new(),
                        call_id: call_id.to_string(),
                        content: reason.into(),
                        status: ToolResultStatus::Cancelled,
                    },
                };
                Ok(())
            }
        }
    }

    pub fn complete_tool_batch(
        &mut self,
        work_id: &ExternalWorkId,
        results: Vec<ToolResult>,
    ) -> Result<(), ProviderRunProtocolError> {
        let batch = self.pending_tool_batch(work_id)?;
        let expected = batch.unresolved_call_ids();
        let mut by_call_id = BTreeMap::new();
        for result in results {
            let call_id = result.call_id.clone();
            if by_call_id.insert(call_id.clone(), result).is_some() {
                return Err(ProviderRunProtocolError::DuplicateToolResult { call_id });
            }
        }
        let received = by_call_id.keys().cloned().collect::<Vec<_>>();
        let expected_set = expected.iter().cloned().collect::<BTreeSet<_>>();
        let received_set = received.iter().cloned().collect::<BTreeSet<_>>();
        if expected_set != received_set {
            return Err(ProviderRunProtocolError::ToolResultSetMismatch {
                expected_call_ids: expected,
                received_call_ids: received,
            });
        }

        let batch = self.pending_tool_batch_mut(work_id)?;
        for call in &mut batch.calls {
            if call.state.result().is_some() {
                continue;
            }
            let result = by_call_id
                .remove(&call.call.id)
                .expect("validated tool result set must contain every unresolved call");
            call.state = PendingToolCallState::Resolved { result };
        }
        Ok(())
    }

    pub fn commit_tool_batch(
        &mut self,
        work_id: &ExternalWorkId,
    ) -> Result<(), ProviderRunProtocolError> {
        let batch = self.pending_tool_batch(work_id)?.clone();
        let Some(results) = batch.ordered_results() else {
            return Err(ProviderRunProtocolError::IncompleteToolBatch {
                missing_call_ids: batch.unresolved_call_ids(),
            });
        };
        let next_epoch = self.next_epoch()?;
        let mut task_list = self.task_list.clone();
        for pending in &batch.calls {
            if pending.call.name != UPDATE_TASK_LIST_NAME {
                continue;
            }
            let result = pending
                .state
                .result()
                .expect("validated complete tool batch");
            if result.status == ToolResultStatus::Success {
                task_list = Some(task_list_from_tool_result(&result.content).ok_or_else(|| {
                    ProviderRunProtocolError::InvalidTaskList {
                        message: "successful task update has no valid snapshot result".to_string(),
                    }
                })?);
            }
        }
        let yielded_to_external_work = batch.calls.iter().any(|pending| {
            pending.state.result().is_some_and(|result| {
                result.status == ToolResultStatus::Success
                    && (is_local_agent_launch(&pending.call)
                        || is_background_job_launch(&pending.call, result))
            })
        });
        self.transcript.push(tool_result_message(results));
        self.task_list = task_list;
        self.epoch = next_epoch;
        // Launch acknowledgement is a yield, not an invitation to issue more tools.
        // The client owns the independent child/job lifecycle and supplies a new input once
        // that work finishes. Every result in this batch is committed before yielding.
        self.state = if yielded_to_external_work {
            ProviderRunState::Done {
                completion: ProviderRunCompletion {
                    stop_reason: StopReason::Completed,
                },
            }
        } else {
            ProviderRunState::ReadyToCallModel
        };
        Ok(())
    }

    pub fn complete(&mut self, work_id: &ExternalWorkId) -> Result<(), ProviderRunProtocolError> {
        let (expected, stop_reason) = match &self.state {
            ProviderRunState::AwaitingDriver {
                work_id,
                stop_reason,
            } => (work_id, stop_reason.clone()),
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::AwaitingModel { .. }
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingTools { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(self.unexpected_state(ProviderRunPhase::AwaitingDriver));
            }
        };
        validate_work_id(expected, work_id)?;
        self.state = ProviderRunState::Done {
            completion: ProviderRunCompletion { stop_reason },
        };
        Ok(())
    }

    pub fn continue_with_observation(
        &mut self,
        work_id: &ExternalWorkId,
        observation: MessageContent,
        next_profile: impl Into<ProviderRequestProfile>,
    ) -> Result<(), ProviderRunProtocolError> {
        let expected = match &self.state {
            ProviderRunState::AwaitingDriver { work_id, .. } => work_id,
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::AwaitingModel { .. }
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingTools { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(self.unexpected_state(ProviderRunPhase::AwaitingDriver));
            }
        };
        validate_work_id(expected, work_id)?;
        validate_driver_observation(&observation)?;
        let next_epoch = self.next_epoch()?;
        self.transcript.push(ConversationMessage {
            role: MessageRole::User,
            content: observation,
        });
        self.profile = next_profile.into();
        self.epoch = next_epoch;
        self.state = ProviderRunState::ReadyToCallModel;
        Ok(())
    }

    pub fn continue_ready_with_observation(
        &mut self,
        work_id: &ExternalWorkId,
        observation: MessageContent,
        next_profile: impl Into<ProviderRequestProfile>,
    ) -> Result<(), ProviderRunProtocolError> {
        if !matches!(self.state, ProviderRunState::ReadyToCallModel) {
            return Err(self.unexpected_state(ProviderRunPhase::ReadyToCallModel));
        }
        validate_work_id(&self.current_work_id(), work_id)?;
        validate_driver_observation(&observation)?;
        let next_epoch = self.next_epoch()?;
        self.transcript.push(ConversationMessage {
            role: MessageRole::User,
            content: observation,
        });
        self.profile = next_profile.into();
        self.epoch = next_epoch;
        Ok(())
    }

    pub fn cancel(&mut self, reason: impl Into<String>) -> Result<(), ProviderRunProtocolError> {
        let reason = reason.into();
        self.finish_run_with_pending_tools(
            ToolResultStatus::Cancelled,
            reason.clone(),
            ProviderRunState::Cancelled { reason },
        )
    }

    pub fn fail(
        &mut self,
        kind: ProviderRunFailureKind,
        message: impl Into<String>,
    ) -> Result<(), ProviderRunProtocolError> {
        let message = message.into();
        let terminal = ProviderRunState::Failed {
            failure: ProviderRunFailure {
                kind,
                message: message.clone(),
                source: None,
            },
        };
        self.finish_run_with_pending_tools(ToolResultStatus::Error, message, terminal)
    }

    fn current_work_id(&self) -> ExternalWorkId {
        ExternalWorkId {
            run_id: self.id.clone(),
            epoch: self.epoch,
        }
    }

    fn next_epoch(&self) -> Result<RunEpoch, ProviderRunProtocolError> {
        self.epoch
            .checked_next()
            .ok_or(ProviderRunProtocolError::EpochExhausted)
    }

    fn build_tool_batch(&self, turn: &CompletedModelTurn) -> PendingToolBatch {
        let policy = ToolPolicy::from_names(turn.advertised_tools.iter().cloned());
        let has_launch = turn.tool_calls.iter().any(|call| {
            is_local_agent_launch(call)
                && call.arguments.is_object()
                && turn.advertised_tools.contains(&call.name)
        });
        let calls = turn
            .tool_calls
            .iter()
            .cloned()
            .map(|call| {
                let state = if has_launch && !is_local_agent_launch(&call) {
                    PendingToolCallState::Resolved {
                        result: ToolResult {
                            images: Vec::new(),
                            call_id: call.id.clone(),
                            content: "Not executed: launching a child group yields this turn. Wait for the group's completion before scheduling dependent work.".to_string(),
                            status: ToolResultStatus::Error,
                        },
                    }
                } else if !call.arguments.is_object() {
                    PendingToolCallState::Resolved {
                        result: ToolResult {
                            images: Vec::new(),
                            call_id: call.id.clone(),
                            content: format!(
                                "Error: '{}' received malformed arguments; expected a JSON object.",
                                call.name
                            ),
                            status: ToolResultStatus::Error,
                        },
                    }
                } else {
                    match policy.decide(&call, &self.transcript, &self.tool_result_archive) {
                        ToolCallDecision::Execute => PendingToolCallState::Proposed,
                        ToolCallDecision::Inline(result) | ToolCallDecision::Reject(result) => {
                            PendingToolCallState::Resolved { result }
                        }
                    }
                };
                PendingToolCall { call, state }
            })
            .collect();
        PendingToolBatch {
            work_id: self.current_work_id(),
            calls,
        }
    }

    fn pending_tool_batch(
        &self,
        work_id: &ExternalWorkId,
    ) -> Result<&PendingToolBatch, ProviderRunProtocolError> {
        let batch = match &self.state {
            ProviderRunState::AwaitingTools { batch } => batch,
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::AwaitingModel { .. }
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingDriver { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(self.unexpected_state(ProviderRunPhase::AwaitingTools));
            }
        };
        validate_work_id(&batch.work_id, work_id)?;
        Ok(batch)
    }

    fn pending_tool_batch_mut(
        &mut self,
        work_id: &ExternalWorkId,
    ) -> Result<&mut PendingToolBatch, ProviderRunProtocolError> {
        let actual = self.state.phase();
        let batch = match &mut self.state {
            ProviderRunState::AwaitingTools { batch } => batch,
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::AwaitingModel { .. }
            | ProviderRunState::ResolvingModel { .. }
            | ProviderRunState::AwaitingDriver { .. }
            | ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => {
                return Err(ProviderRunProtocolError::UnexpectedState {
                    expected: ProviderRunPhase::AwaitingTools,
                    actual,
                });
            }
        };
        validate_work_id(&batch.work_id, work_id)?;
        Ok(batch)
    }

    fn pending_tool_call_mut(
        &mut self,
        work_id: &ExternalWorkId,
        call_id: &str,
    ) -> Result<&mut PendingToolCall, ProviderRunProtocolError> {
        let batch = self.pending_tool_batch_mut(work_id)?;
        batch
            .calls
            .iter_mut()
            .find(|call| call.call.id == call_id)
            .ok_or_else(|| ProviderRunProtocolError::UnknownToolCall {
                call_id: call_id.to_string(),
            })
    }

    fn finish_run_with_pending_tools(
        &mut self,
        status: ToolResultStatus,
        result_content: String,
        terminal: ProviderRunState,
    ) -> Result<(), ProviderRunProtocolError> {
        match self.state.clone() {
            ProviderRunState::Done { .. }
            | ProviderRunState::Failed { .. }
            | ProviderRunState::Cancelled { .. } => Err(ProviderRunProtocolError::Terminal),
            ProviderRunState::ResolvingModel { turn } => {
                if turn.tool_calls.is_empty() {
                    self.state = ProviderRunState::AwaitingDriver {
                        work_id: self.current_work_id(),
                        stop_reason: turn.stop_reason,
                    };
                } else {
                    let batch = self.build_tool_batch(&turn);
                    self.state = ProviderRunState::AwaitingTools { batch };
                }
                self.finish_run_with_pending_tools(status, result_content, terminal)
            }
            ProviderRunState::AwaitingTools { mut batch } => {
                for call in &mut batch.calls {
                    if call.state.result().is_none() {
                        call.state = PendingToolCallState::Resolved {
                            result: ToolResult {
                                images: Vec::new(),
                                call_id: call.call.id.clone(),
                                content: result_content.clone(),
                                status,
                            },
                        };
                    }
                }
                let results = batch
                    .ordered_results()
                    .expect("all unresolved tool calls were assigned a terminal result");
                let next_epoch = self.next_epoch()?;
                self.transcript.push(tool_result_message(results));
                self.epoch = next_epoch;
                self.state = terminal;
                Ok(())
            }
            ProviderRunState::ReadyToCallModel
            | ProviderRunState::AwaitingModel { .. }
            | ProviderRunState::AwaitingDriver { .. } => {
                self.state = terminal;
                Ok(())
            }
        }
    }

    fn unexpected_state(&self, expected: ProviderRunPhase) -> ProviderRunProtocolError {
        ProviderRunProtocolError::UnexpectedState {
            expected,
            actual: self.state.phase(),
        }
    }
}

fn validate_work_id(
    expected: &ExternalWorkId,
    received: &ExternalWorkId,
) -> Result<(), ProviderRunProtocolError> {
    if expected == received {
        Ok(())
    } else {
        Err(ProviderRunProtocolError::WorkMismatch {
            expected: expected.clone(),
            received: received.clone(),
        })
    }
}

fn validate_pending_tool_state(pending: &PendingToolCall) -> Result<(), String> {
    match &pending.state {
        PendingToolCallState::PermissionPending { request } => {
            if request.id.is_empty() {
                return Err(format!(
                    "permission request for '{}' has an empty request ID",
                    pending.call.id
                ));
            }
            if request.call_id != pending.call.id {
                return Err(format!(
                    "permission request for '{}' belongs to call '{}'",
                    pending.call.id, request.call_id
                ));
            }
        }
        PendingToolCallState::Approved {
            request_id,
            decision,
        } => {
            if request_id.is_empty() {
                return Err(format!(
                    "approved tool call '{}' has an empty request ID",
                    pending.call.id
                ));
            }
            if matches!(decision, PermissionDecision::Denied { .. }) {
                return Err(format!(
                    "approved tool call '{}' contains a denied decision",
                    pending.call.id
                ));
            }
        }
        PendingToolCallState::Resolved { result } if result.call_id != pending.call.id => {
            return Err(format!(
                "resolved result for '{}' belongs to call '{}'",
                pending.call.id, result.call_id
            ));
        }
        PendingToolCallState::Proposed
        | PendingToolCallState::Executing
        | PendingToolCallState::RecoveryPending
        | PendingToolCallState::Resolved { .. } => {}
    }
    if pending.call.name == UPDATE_TASK_LIST_NAME
        && let Some(result) = pending.state.result()
        && result.status == ToolResultStatus::Success
    {
        let snapshot = task_list_from_tool_result(&result.content)
            .ok_or_else(|| "successful task update has no valid snapshot result".to_string())?;
        let expected = TaskListSnapshot::from_arguments(&pending.call.arguments)?;
        if snapshot != expected {
            return Err("task update result does not match its arguments".to_string());
        }
    }
    Ok(())
}

fn tool_calls_from_message(message: &ConversationMessage) -> Vec<ToolCall> {
    let MessageContent::MultiPart(parts) = &message.content else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ToolUse {
                tool_use_id,
                name,
                input,
            } => Some(ToolCall {
                id: tool_use_id.clone(),
                name: name.clone(),
                arguments: input.clone(),
            }),
            ContentPart::Text(_)
            | ContentPart::Reasoning { .. }
            | ContentPart::ToolResult { .. }
            | ContentPart::Image { .. } => None,
        })
        .collect()
}

fn usage_contains(total: &Usage, part: &Usage) -> bool {
    total.input_tokens >= part.input_tokens
        && total.output_tokens >= part.output_tokens
        && total.cached_input_tokens >= part.cached_input_tokens
        && total.cache_creation_input_tokens >= part.cache_creation_input_tokens
}

fn is_local_agent_launch(call: &ToolCall) -> bool {
    // Solar normalizes direct-provider delegation to local child execution.
    matches!(call.name.as_str(), "run_agents" | "start_agent")
}

fn is_background_job_launch(call: &ToolCall, result: &ToolResult) -> bool {
    if call.name != "run_shell_command" {
        return false;
    }
    let Ok(content) = serde_json::from_str::<serde_json::Value>(&result.content) else {
        return false;
    };
    content.get("background_job_started").is_some_and(|job| {
        job.get("job_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| !id.is_empty())
            && job
                .get("command")
                .and_then(serde_json::Value::as_str)
                .is_some()
    })
}

fn validate_model_turn(turn: &CompletedModelTurn) -> Result<(), ProviderRunProtocolError> {
    for part in &turn.assistant_content {
        match part {
            ContentPart::Text(_) | ContentPart::Reasoning { .. } | ContentPart::Image { .. } => {}
            ContentPart::ToolUse { .. } | ContentPart::ToolResult { .. } => {
                return Err(ProviderRunProtocolError::InvalidModelTurn {
                    message: "assistant_content must not contain tool calls or results".to_string(),
                });
            }
        }
    }

    let mut call_ids = HashSet::new();
    for call in &turn.tool_calls {
        if call.id.is_empty() {
            return Err(ProviderRunProtocolError::InvalidModelTurn {
                message: "tool call IDs must not be empty".to_string(),
            });
        }
        if !call_ids.insert(call.id.as_str()) {
            return Err(ProviderRunProtocolError::InvalidModelTurn {
                message: format!("duplicate tool call ID '{}'", call.id),
            });
        }
    }
    Ok(())
}

fn assistant_message(turn: &CompletedModelTurn) -> ConversationMessage {
    let mut parts = turn.assistant_content.clone();
    parts.extend(turn.tool_calls.iter().map(|call| ContentPart::ToolUse {
        tool_use_id: call.id.clone(),
        name: call.name.clone(),
        input: call.arguments.clone(),
    }));
    let content = match parts.as_slice() {
        [ContentPart::Text(text)] => MessageContent::Text(text.clone()),
        [] => MessageContent::Text(String::new()),
        [ContentPart::Reasoning { .. }]
        | [ContentPart::Image { .. }]
        | [ContentPart::ToolUse { .. }]
        | [ContentPart::ToolResult { .. }]
        | [_, _, ..] => MessageContent::MultiPart(parts),
    };
    ConversationMessage {
        role: MessageRole::Assistant,
        content,
    }
}

fn tool_result_message(results: Vec<ToolResult>) -> ConversationMessage {
    ConversationMessage {
        role: MessageRole::User,
        content: MessageContent::MultiPart(ToolResult::into_batch_content_parts(results)),
    }
}

fn validate_driver_observation(
    observation: &MessageContent,
) -> Result<(), ProviderRunProtocolError> {
    match observation {
        MessageContent::Text(_) => Ok(()),
        MessageContent::MultiPart(parts) => {
            for part in parts {
                match part {
                    ContentPart::Text(_)
                    | ContentPart::Reasoning { .. }
                    | ContentPart::Image { .. } => {}
                    ContentPart::ToolUse { .. } | ContentPart::ToolResult { .. } => {
                        return Err(ProviderRunProtocolError::InvalidDriverObservation {
                            message:
                                "continuation observations cannot inject tool calls or results"
                                    .to_string(),
                        });
                    }
                }
            }
            Ok(())
        }
        MessageContent::ToolUse { .. } | MessageContent::ToolResult { .. } => {
            Err(ProviderRunProtocolError::InvalidDriverObservation {
                message: "continuation observations cannot inject tool calls or results"
                    .to_string(),
            })
        }
    }
}

fn invalid_tool_transition(call: &PendingToolCall, update: &str) -> ProviderRunProtocolError {
    ProviderRunProtocolError::InvalidToolTransition {
        call_id: call.call.id.clone(),
        state: call.state.name().to_string(),
        update: update.to_string(),
    }
}

fn add_usage(total: &mut Usage, turn: &Usage) {
    total.input_tokens = total.input_tokens.saturating_add(turn.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(turn.output_tokens);
    total.cached_input_tokens = total
        .cached_input_tokens
        .saturating_add(turn.cached_input_tokens);
    total.cache_creation_input_tokens = total
        .cache_creation_input_tokens
        .saturating_add(turn.cache_creation_input_tokens);
}

/// Retain tool history once per call/result ID, including when repairing older archives.
/// A multipart message is appended once, never once per tool contained in it.
pub fn archive_tool_results(
    archive: &mut Vec<ConversationMessage>,
    messages: Vec<ConversationMessage>,
) {
    const MAX_TOOL_RESULT_ARCHIVE_ENTRIES: usize = 400;
    let mut seen_uses = HashSet::new();
    let mut seen_results = HashSet::new();
    let previous = std::mem::take(archive);
    for mut message in previous.into_iter().chain(messages) {
        let keep = match &mut message.content {
            MessageContent::ToolUse { tool_use_id, .. } => seen_uses.insert(tool_use_id.clone()),
            MessageContent::ToolResult { tool_use_id, .. } => {
                seen_results.insert(tool_use_id.clone())
            }
            MessageContent::MultiPart(parts) => {
                let mut retained_results = HashSet::new();
                let mut keep_parts: Vec<_> = parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::ToolUse { tool_use_id, .. } => {
                            seen_uses.insert(tool_use_id.clone())
                        }
                        ContentPart::ToolResult { tool_use_id, .. } => {
                            let keep = seen_results.insert(tool_use_id.clone());
                            if keep {
                                retained_results.insert(tool_use_id.clone());
                            }
                            keep
                        }
                        ContentPart::Text(_)
                        | ContentPart::Reasoning { .. }
                        | ContentPart::Image { .. } => false,
                    })
                    .collect();
                let mut keep_image_group = false;
                for (index, part) in parts.iter().enumerate() {
                    match part {
                        ContentPart::Text(_) => {
                            keep_image_group = tool_image_group_call_id(part)
                                .is_some_and(|call_id| retained_results.contains(call_id))
                                && matches!(parts.get(index + 1), Some(ContentPart::Image { .. }));
                            keep_parts[index] = keep_image_group;
                        }
                        ContentPart::Image { .. } => keep_parts[index] = keep_image_group,
                        ContentPart::Reasoning { .. }
                        | ContentPart::ToolUse { .. }
                        | ContentPart::ToolResult { .. } => keep_image_group = false,
                    }
                }
                let mut retained = keep_parts.into_iter();
                parts.retain(|_| retained.next().unwrap_or(false));
                !parts.is_empty()
            }
            MessageContent::Text(_) => false,
        };
        if keep {
            archive.push(message);
        }
    }
    if archive.len() > MAX_TOOL_RESULT_ARCHIVE_ENTRIES {
        archive.drain(0..archive.len() - MAX_TOOL_RESULT_ARCHIVE_ENTRIES);
    }
}

#[cfg(test)]
#[path = "provider_run_tests.rs"]
mod tests;
