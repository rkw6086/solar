use std::error::Error;
use std::fmt;
use std::pin::Pin;

use async_channel::{Receiver, Sender, TrySendError};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};

use crate::{AgentEvent, TurnRequest};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimeKind {
    Provider,
    Acp,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeCapabilities {
    pub model_selection: bool,
    pub session_resume: bool,
    pub steering: bool,
    pub tool_permissions: bool,
    /// Solar owns and persists the message history supplied to each turn.
    pub host_managed_history: bool,
    /// Tool proposals cross the runtime boundary for Solar to approve and execute.
    pub host_tool_execution: bool,
    /// A failed turn can be safely replayed from the same request payload.
    pub request_retries: bool,
    /// Solar can append corrective instructions and start a follow-up turn.
    pub corrective_retries: bool,
    /// Transcript events can be forwarded through Solar shared sessions.
    pub shared_session_sync: bool,
}

impl RuntimeCapabilities {
    #[must_use]
    pub const fn provider() -> Self {
        Self {
            model_selection: true,
            session_resume: false,
            steering: false,
            tool_permissions: false,
            host_managed_history: true,
            host_tool_execution: true,
            request_retries: true,
            corrective_retries: true,
            shared_session_sync: true,
        }
    }

    #[must_use]
    pub const fn session_runtime() -> Self {
        Self {
            model_selection: false,
            session_resume: true,
            steering: true,
            tool_permissions: true,
            host_managed_history: false,
            host_tool_execution: false,
            request_retries: false,
            corrective_retries: false,
            shared_session_sync: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeDescriptor {
    pub id: String,
    pub display_name: String,
    pub kind: RuntimeKind,
    pub capabilities: RuntimeCapabilities,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TurnCommand {
    Cancel,
    Steer {
        display_text: String,
        model_text: String,
    },
}

#[derive(Clone, Debug)]
pub struct TurnCommandSender(Sender<TurnCommand>);

impl TurnCommandSender {
    pub async fn send(&self, command: TurnCommand) -> Result<(), TurnControlClosed> {
        self.0.send(command).await.map_err(|_| TurnControlClosed)
    }

    pub fn try_send(&self, command: TurnCommand) -> Result<(), TrySendError<TurnCommand>> {
        self.0.try_send(command)
    }
}

#[derive(Clone, Debug)]
pub struct TurnControl(Receiver<TurnCommand>);

impl TurnControl {
    pub async fn receive(&self) -> Result<TurnCommand, TurnControlClosed> {
        self.0.recv().await.map_err(|_| TurnControlClosed)
    }

    pub fn try_receive(&self) -> Result<TurnCommand, async_channel::TryRecvError> {
        self.0.try_recv()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TurnControlClosed;

impl fmt::Display for TurnControlClosed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("turn control channel is closed")
    }
}

impl Error for TurnControlClosed {}

pub fn turn_control() -> (TurnCommandSender, TurnControl) {
    let (sender, receiver) = async_channel::unbounded();
    (TurnCommandSender(sender), TurnControl(receiver))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentErrorKind {
    Configuration,
    Authentication,
    RateLimited,
    ContextWindowExceeded,
    InvalidRequest,
    Transport,
    Provider,
    Protocol,
    Tool,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentError {
    pub kind: AgentErrorKind,
    pub message: String,
    pub user_message: Option<String>,
    pub recoverable: bool,
}

impl AgentError {
    pub fn new(kind: AgentErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            user_message: None,
            recoverable: false,
        }
    }
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for AgentError {}

#[cfg(not(target_family = "wasm"))]
pub type AgentEventStream =
    Pin<Box<dyn Stream<Item = Result<AgentEvent, AgentError>> + Send + 'static>>;

#[cfg(target_family = "wasm")]
pub type AgentEventStream = Pin<Box<dyn Stream<Item = Result<AgentEvent, AgentError>> + 'static>>;

#[cfg_attr(target_family = "wasm", async_trait(?Send))]
#[cfg_attr(not(target_family = "wasm"), async_trait)]
pub trait AgentRuntime: Send + Sync {
    fn descriptor(&self) -> &RuntimeDescriptor;

    async fn start_turn(
        &self,
        request: TurnRequest,
        control: TurnControl,
    ) -> Result<AgentEventStream, AgentError>;
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
