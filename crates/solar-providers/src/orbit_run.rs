//! Solar's durable effect receipts around Rig's agent state machine.
//!
//! Rig owns turn advancement, tool-call validation, canonical conversation
//! history, usage, and completion. Solar records only the host-side execution
//! facts needed to avoid replaying an interrupted action.

use std::collections::BTreeSet;

use rig_agent::agent::hook::InvalidToolCallAction;
use rig_agent::agent::run::{AgentRun, AgentRunStep, ModelTurn, ModelTurnOutcome};
use rig_core::OneOrMany;
use rig_core::message::{AssistantContent, Message, UserContent};
use serde::{Deserialize, Serialize};
use solar_agent_core::{
    CompletedModelTurn, ContentPart, ConversationMessage, MessageContent, MessageRole, ToolCall,
    ToolResult, ToolResultStatus, Usage,
};

use crate::rig_conversion::{
    rig_messages_to_solar, rig_tool_call_to_solar, rig_usage_to_solar, solar_messages_to_rig,
    solar_tool_result_to_rig, solar_usage_to_rig,
};

const FORMAT_VERSION: u8 = 1;

#[derive(Clone, Debug)]
pub enum OrbitStep {
    Model {
        messages: Vec<ConversationMessage>,
        turn: usize,
    },
    Tools {
        calls: Vec<ToolCall>,
    },
    Done {
        text: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct EffectReceipt {
    call: ToolCall,
    started: bool,
    result: Option<ToolResult>,
    /// Rig may settle an invalid tool call without sending it to Solar.
    /// Keep its exact protocol result while recording the denied status.
    preresolved: Option<UserContent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RigOrbitRun {
    format_version: u8,
    id: String,
    rig_run: AgentRun,
    #[serde(default)]
    effects: Vec<EffectReceipt>,
    #[serde(default)]
    awaiting_model: bool,
    #[serde(default)]
    advance_pending: bool,
    #[serde(default)]
    interruption: Option<String>,
}

impl RigOrbitRun {
    pub fn new(
        id: String,
        history: Vec<ConversationMessage>,
        max_turns: usize,
    ) -> Result<Self, String> {
        let mut messages = solar_messages_to_rig(history).map_err(|e| e.to_string())?;
        let prompt = messages.pop().ok_or("A Sprite run needs a user message")?;
        if !matches!(prompt, Message::User { .. }) {
            return Err("A Sprite run must begin with a user message".into());
        }
        Ok(Self {
            format_version: FORMAT_VERSION,
            id,
            rig_run: AgentRun::new(prompt)
                .with_history(messages)
                .max_turns(max_turns),
            effects: Vec::new(),
            awaiting_model: false,
            advance_pending: false,
            interruption: None,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn turn(&self) -> usize {
        self.rig_run.turn()
    }

    pub fn usage(&self) -> Usage {
        rig_usage_to_solar(self.rig_run.usage())
    }

    pub fn is_terminal(&self) -> bool {
        self.interruption.is_some() || self.rig_run.is_done()
    }

    pub fn interruption(&self) -> Option<&str> {
        self.interruption.as_deref()
    }

    pub fn history(&self) -> Result<Vec<ConversationMessage>, String> {
        self.ensure_format()?;
        rig_messages_to_solar(&self.rig_run.full_history()).map_err(|e| e.to_string())
    }

    pub fn next_step(&mut self) -> Result<OrbitStep, String> {
        self.ensure_active()?;
        if !self.effects.is_empty() {
            return Ok(OrbitStep::Tools {
                calls: self
                    .effects
                    .iter()
                    .filter(|effect| effect.result.is_none())
                    .map(|effect| effect.call.clone())
                    .collect(),
            });
        }
        let step = self.rig_run.next_step().map_err(|e| e.to_string())?;
        self.advance_pending = false;
        match step {
            AgentRunStep::CallModel {
                prompt,
                mut history,
                turn,
            } => {
                history.push(prompt);
                self.awaiting_model = true;
                Ok(OrbitStep::Model {
                    messages: rig_messages_to_solar(&history).map_err(|e| e.to_string())?,
                    turn,
                })
            }
            AgentRunStep::CallTools { calls } => {
                self.effects = calls
                    .into_iter()
                    .map(|pending| {
                        let call = rig_tool_call_to_solar(&pending.tool_call);
                        let result = if pending.preresolved_result.is_some() {
                            Some(denied_result(&call.id, "Rig rejected this tool call"))
                        } else if !call.arguments.is_object() {
                            Some(denied_result(
                                &call.id,
                                "Tool arguments must be a JSON object; no action was executed",
                            ))
                        } else {
                            None
                        };
                        EffectReceipt {
                            call,
                            started: false,
                            result,
                            preresolved: pending.preresolved_result,
                        }
                    })
                    .collect();
                Ok(OrbitStep::Tools {
                    calls: self
                        .effects
                        .iter()
                        .filter(|effect| effect.result.is_none())
                        .map(|effect| effect.call.clone())
                        .collect(),
                })
            }
            AgentRunStep::Done(response) => Ok(OrbitStep::Done {
                text: response.output,
            }),
        }
    }

    pub fn accept_model_turn(&mut self, turn: CompletedModelTurn) -> Result<(), String> {
        self.ensure_active()?;
        if !self.awaiting_model {
            return Err("No Sprite model call is awaiting a response".into());
        }
        // Reject ambiguous correlation atomically before changing Rig history.
        let mut ids = BTreeSet::new();
        for call in &turn.tool_calls {
            if call.id.trim().is_empty() || !ids.insert(call.id.clone()) {
                return Err("The model returned an empty or duplicate tool-call ID".into());
            }
        }
        if turn.assistant_content.iter().any(|part| {
            matches!(
                part,
                ContentPart::ToolUse { .. } | ContentPart::ToolResult { .. }
            )
        }) {
            return Err("Tool calls must be supplied separately from assistant content".into());
        }
        let mut parts = turn.assistant_content;
        parts.extend(
            turn.tool_calls
                .into_iter()
                .map(|call| ContentPart::ToolUse {
                    tool_use_id: call.id,
                    name: call.name,
                    input: call.arguments,
                }),
        );
        let choice = if parts.is_empty() {
            OneOrMany::one(AssistantContent::text(""))
        } else {
            let converted = solar_messages_to_rig(vec![ConversationMessage {
                role: MessageRole::Assistant,
                content: MessageContent::MultiPart(parts),
            }])
            .map_err(|e| e.to_string())?;
            let Message::Assistant { content, .. } = converted.into_iter().next().unwrap() else {
                return Err("Expected an assistant response".into());
            };
            content
        };
        let mut candidate = self.rig_run.clone();
        let mut outcome = candidate
            .model_response(ModelTurn::new(
                None,
                choice,
                solar_usage_to_rig(&turn.usage),
                turn.advertised_tools.clone(),
                turn.advertised_tools,
            ))
            .map_err(|e| e.to_string())?;
        while let ModelTurnOutcome::NeedsResolution(context) = outcome {
            outcome = candidate
                .resolve_invalid_tool_call(InvalidToolCallAction::skip(format!(
                    "[ERROR] Tool '{}' was not advertised for this request; no action was executed",
                    context.tool_name
                )))
                .map_err(|e| e.to_string())?;
        }
        self.rig_run = candidate;
        self.awaiting_model = false;
        self.advance_pending = true;
        Ok(())
    }

    pub fn record_tool_started(&mut self, call_id: &str) -> Result<(), String> {
        self.ensure_active()?;
        let effect = self.effect_mut(call_id)?;
        if effect.started || effect.result.is_some() {
            return Err("This tool call was already started or settled".into());
        }
        effect.started = true;
        Ok(())
    }

    pub fn record_tool_result(&mut self, result: ToolResult) -> Result<(), String> {
        self.ensure_active()?;
        let effect = self.effect_mut(&result.call_id)?;
        if effect.result.is_some() {
            return Err("This tool call already has a result".into());
        }
        if !effect.started
            && !matches!(
                result.status,
                ToolResultStatus::Denied | ToolResultStatus::Cancelled
            )
        {
            return Err(
                "A tool must have a start receipt before recording its execution result".into(),
            );
        }
        effect.result = Some(result);
        Ok(())
    }

    pub fn commit_tool_batch(&mut self) -> Result<(), String> {
        self.ensure_active()?;
        if self.effects.is_empty() {
            return Err("No Sprite tool batch is awaiting results".into());
        }
        let results = self
            .effects
            .iter()
            .map(|effect| {
                if let Some(result) = &effect.preresolved {
                    return Ok(result.clone());
                }
                let result = effect
                    .result
                    .clone()
                    .ok_or("A Sprite tool call is still awaiting its result")?;
                solar_tool_result_to_rig(result).map_err(|e| e.to_string())
            })
            .collect::<Result<Vec<_>, String>>()?;
        let mut candidate = self.rig_run.clone();
        candidate.tool_results(results).map_err(|e| e.to_string())?;
        self.rig_run = candidate;
        self.effects.clear();
        Ok(())
    }

    /// Stop without replaying host effects. Completed results remain exact;
    /// started effects without a receipt have an explicitly unknown outcome.
    pub fn interrupt(&mut self, reason: impl Into<String>) -> Result<(), String> {
        self.ensure_format()?;
        if self.interruption.is_some() {
            return Ok(());
        }
        if self.advance_pending {
            // Materialize the accepted assistant response and its pending calls.
            // This advances only Rig state; no provider or tool is invoked.
            let _ = self.next_step()?;
        }
        for effect in &mut self.effects {
            if effect.result.is_none() {
                effect.result = Some(ToolResult {
                    call_id: effect.call.id.clone(),
                    content: if effect.started {
                        "The task stopped after this action started. Its outcome is unknown; it was not replayed."
                    } else {
                        "The task stopped before this action was executed. It was not replayed."
                    }
                    .into(),
                    status: if effect.started {
                        ToolResultStatus::Error
                    } else {
                        ToolResultStatus::Cancelled
                    },
                    images: Vec::new(),
                });
            }
        }
        if !self.effects.is_empty() {
            self.commit_tool_batch()?;
        }
        self.interruption = Some(reason.into());
        Ok(())
    }

    fn effect_mut(&mut self, call_id: &str) -> Result<&mut EffectReceipt, String> {
        self.effects
            .iter_mut()
            .find(|effect| effect.call.id == call_id)
            .ok_or_else(|| "Tool result does not match a pending Sprite call".into())
    }

    fn ensure_format(&self) -> Result<(), String> {
        if self.format_version != FORMAT_VERSION {
            return Err("Unsupported Solar Rig run checkpoint version".into());
        }
        Ok(())
    }

    fn ensure_active(&self) -> Result<(), String> {
        self.ensure_format()?;
        if let Some(reason) = &self.interruption {
            return Err(reason.clone());
        }
        Ok(())
    }
}

fn denied_result(call_id: &str, message: &str) -> ToolResult {
    ToolResult {
        call_id: call_id.into(),
        content: message.into(),
        status: ToolResultStatus::Denied,
        images: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use solar_agent_core::StopReason;

    fn run(max_turns: usize) -> RigOrbitRun {
        RigOrbitRun::new(
            "run-1".into(),
            vec![ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text("Read my notes".into()),
            }],
            max_turns,
        )
        .unwrap()
    }

    fn call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            name: name.into(),
            arguments: json!({"path":"notes.txt"}),
        }
    }

    fn turn(calls: Vec<ToolCall>, text: &str) -> CompletedModelTurn {
        CompletedModelTurn {
            assistant_content: if text.is_empty() {
                vec![]
            } else {
                vec![ContentPart::Text(text.into())]
            },
            tool_calls: calls,
            usage: Usage {
                input_tokens: 10,
                output_tokens: 2,
                ..Default::default()
            },
            stop_reason: StopReason::Completed,
            advertised_tools: ["files_read".into()].into_iter().collect(),
        }
    }

    fn result(id: &str, text: &str) -> ToolResult {
        ToolResult {
            call_id: id.into(),
            content: text.into(),
            status: ToolResultStatus::Success,
            images: vec![],
        }
    }

    fn history_json(run: &RigOrbitRun) -> String {
        serde_json::to_string(&run.history().unwrap()).unwrap()
    }

    #[test]
    fn rig_drives_tool_result_followup_and_final_response() {
        let mut run = run(3);
        assert!(matches!(
            run.next_step().unwrap(),
            OrbitStep::Model { turn: 1, .. }
        ));
        run.accept_model_turn(turn(vec![call("read-1", "files_read")], "Checking"))
            .unwrap();
        let OrbitStep::Tools { calls } = run.next_step().unwrap() else {
            panic!("expected tool step")
        };
        assert_eq!(calls.len(), 1);
        run.record_tool_started("read-1").unwrap();
        run.record_tool_result(result("read-1", "notes contents"))
            .unwrap();
        run.commit_tool_batch().unwrap();
        let OrbitStep::Model { messages, turn: 2 } = run.next_step().unwrap() else {
            panic!("expected followup")
        };
        assert!(
            serde_json::to_string(&messages)
                .unwrap()
                .contains("notes contents")
        );
        run.accept_model_turn(turn(vec![], "Here are your notes"))
            .unwrap();
        let OrbitStep::Done { text } = run.next_step().unwrap() else {
            panic!("expected final response")
        };
        assert_eq!(text, "Here are your notes");
        assert!(run.is_terminal());
        assert_eq!(run.usage().input_tokens, 20);
        assert_eq!(run.usage().output_tokens, 4);
    }

    #[test]
    fn unknown_tools_and_their_peers_are_settled_without_execution() {
        let mut run = run(3);
        run.next_step().unwrap();
        run.accept_model_turn(turn(
            vec![call("known", "files_read"), call("unknown", "shell_delete")],
            "",
        ))
        .unwrap();
        let OrbitStep::Tools { calls } = run.next_step().unwrap() else {
            panic!("expected tool step")
        };
        assert!(calls.is_empty());
        assert!(run.record_tool_started("unknown").is_err());
        run.commit_tool_batch().unwrap();
        let history = history_json(&run);
        assert!(history.contains("unknown"));
        assert!(history.contains("not advertised"));
        assert!(history.contains("known"));
        assert!(history.contains("not executed"));
        assert!(matches!(
            run.next_step().unwrap(),
            OrbitStep::Model { turn: 2, .. }
        ));
    }

    #[test]
    fn malformed_arguments_are_correlated_denials() {
        let mut run = run(3);
        run.next_step().unwrap();
        let mut malformed = call("bad-arguments", "files_read");
        malformed.arguments = json!(["notes.txt"]);
        run.accept_model_turn(turn(vec![malformed], "")).unwrap();
        assert!(matches!(run.next_step().unwrap(), OrbitStep::Tools { calls } if calls.is_empty()));
        run.commit_tool_batch().unwrap();
        let history = history_json(&run);
        assert!(history.contains("bad-arguments"));
        assert!(history.contains("[ERROR] Tool arguments must be a JSON object"));
    }

    #[test]
    fn duplicate_or_empty_ids_are_rejected_atomically() {
        for calls in [
            vec![
                call("duplicate", "files_read"),
                call("duplicate", "files_read"),
            ],
            vec![call("", "files_read")],
        ] {
            let mut run = run(3);
            run.next_step().unwrap();
            let before = serde_json::to_value(&run).unwrap();
            assert!(run.accept_model_turn(turn(calls, "")).is_err());
            assert_eq!(serde_json::to_value(&run).unwrap(), before);
        }
    }

    #[test]
    fn interruption_preserves_receipts_and_does_not_replay_uncertain_tools() {
        let mut run = run(4);
        run.next_step().unwrap();
        run.accept_model_turn(turn(
            vec![
                call("finished", "files_read"),
                call("started", "files_read"),
                call("waiting", "files_read"),
            ],
            "",
        ))
        .unwrap();
        run.next_step().unwrap();
        run.record_tool_started("finished").unwrap();
        run.record_tool_result(result("finished", "saved receipt"))
            .unwrap();
        run.record_tool_started("started").unwrap();
        let bytes = serde_json::to_vec(&run).unwrap();
        let mut restored: RigOrbitRun = serde_json::from_slice(&bytes).unwrap();
        restored.interrupt("Solar restarted").unwrap();
        assert_eq!(restored.interruption(), Some("Solar restarted"));
        assert!(restored.next_step().is_err());
        let history = history_json(&restored);
        assert!(history.contains("saved receipt"));
        assert!(history.contains("outcome is unknown"));
        assert!(history.contains("before this action was executed"));
        let after = serde_json::to_value(&restored).unwrap();
        restored.interrupt("again").unwrap();
        assert_eq!(serde_json::to_value(restored).unwrap(), after);
    }

    #[test]
    fn interruption_materializes_accepted_partial_text_and_unstarted_calls() {
        let mut run = run(2);
        run.next_step().unwrap();
        run.accept_model_turn(turn(vec![call("pending", "files_read")], "Partial answer"))
            .unwrap();
        run.interrupt("Output limit").unwrap();
        let history = history_json(&run);
        assert!(history.contains("Partial answer"));
        assert!(history.contains("pending"));
        assert!(history.contains("before this action was executed"));
    }

    #[test]
    fn rig_enforces_turn_budget_and_results_must_match_pending_calls() {
        let mut run = run(1);
        run.next_step().unwrap();
        run.accept_model_turn(turn(vec![call("read-1", "files_read")], ""))
            .unwrap();
        run.next_step().unwrap();
        assert!(run.record_tool_result(result("other", "wrong")).is_err());
        assert!(run.commit_tool_batch().is_err());
        run.record_tool_started("read-1").unwrap();
        run.record_tool_result(result("read-1", "contents"))
            .unwrap();
        assert!(run.record_tool_result(result("read-1", "again")).is_err());
        run.commit_tool_batch().unwrap();
        assert!(run.next_step().is_err());
        run.interrupt("Turn budget reached").unwrap();
        assert!(run.is_terminal());
        assert!(history_json(&run).contains("contents"));
    }
}
