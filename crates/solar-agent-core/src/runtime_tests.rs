use std::sync::Mutex;

use futures::{StreamExt, stream};

use super::*;
use crate::{
    AgentEvent, ConversationMessage, MessageContent, MessageRole, ModelId, StopReason, Usage,
};

struct FakeRuntime {
    descriptor: RuntimeDescriptor,
    requests: Mutex<Vec<TurnRequest>>,
    events: Vec<AgentEvent>,
}

impl FakeRuntime {
    fn new(events: Vec<AgentEvent>) -> Self {
        Self {
            descriptor: RuntimeDescriptor {
                id: "fake".to_string(),
                display_name: "Deterministic fake".to_string(),
                kind: RuntimeKind::Provider,
                capabilities: RuntimeCapabilities {
                    model_selection: true,
                    ..RuntimeCapabilities::default()
                },
            },
            requests: Mutex::new(Vec::new()),
            events,
        }
    }
}

#[async_trait]
impl AgentRuntime for FakeRuntime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        &self.descriptor
    }

    async fn start_turn(
        &self,
        request: TurnRequest,
        _control: TurnControl,
    ) -> Result<AgentEventStream, AgentError> {
        self.requests.lock().unwrap().push(request);
        let events = self.events.clone().into_iter().map(Ok);
        Ok(Box::pin(stream::iter(events)))
    }
}

#[test]
fn fake_runtime_preserves_request_and_event_contract() {
    futures::executor::block_on(async {
        let expected_events = vec![
            AgentEvent::TurnStarted {
                runtime_request_id: "request-1".to_string(),
            },
            AgentEvent::TextDelta {
                text: "hello".to_string(),
            },
            AgentEvent::UsageUpdated {
                usage: Usage {
                    input_tokens: 4,
                    output_tokens: 1,
                    ..Usage::default()
                },
            },
            AgentEvent::TurnStopped {
                reason: StopReason::Completed,
            },
        ];
        let fake_runtime = FakeRuntime::new(expected_events.clone());
        let runtime: &dyn AgentRuntime = &fake_runtime;
        let request = TurnRequest::new(
            ModelId::new("fake-model"),
            vec![ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("Say hello".to_string()),
            }],
        );
        let (_sender, control) = turn_control();

        let actual_events = runtime
            .start_turn(request.clone(), control)
            .await
            .unwrap()
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(runtime.descriptor().id, "fake");
        assert_eq!(*fake_runtime.requests.lock().unwrap(), vec![request]);
        assert_eq!(actual_events, expected_events);
    });
}

#[test]
fn turn_control_delivers_cancel_and_steering_in_order() {
    futures::executor::block_on(async {
        let (sender, control) = turn_control();

        sender
            .send(TurnCommand::Steer {
                display_text: "focus on tests".to_string(),
                model_text: "focus on tests".to_string(),
            })
            .await
            .unwrap();
        sender.send(TurnCommand::Cancel).await.unwrap();

        assert_eq!(
            control.receive().await.unwrap(),
            TurnCommand::Steer {
                display_text: "focus on tests".to_string(),
                model_text: "focus on tests".to_string(),
            }
        );
        assert_eq!(control.receive().await.unwrap(), TurnCommand::Cancel);
    });
}

#[test]
fn runtime_shapes_publish_controller_policy_as_capabilities() {
    let provider = RuntimeCapabilities::provider();
    assert!(provider.host_managed_history);
    assert!(provider.host_tool_execution);
    assert!(provider.request_retries);
    assert!(provider.corrective_retries);
    assert!(provider.shared_session_sync);
    assert!(!provider.steering);

    let session = RuntimeCapabilities::session_runtime();
    assert!(!session.host_managed_history);
    assert!(!session.host_tool_execution);
    assert!(!session.request_retries);
    assert!(!session.corrective_retries);
    assert!(!session.shared_session_sync);
    assert!(session.steering);
}
