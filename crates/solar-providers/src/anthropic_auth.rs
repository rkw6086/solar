//! Explicit Claude subscription tokens for the direct, Solar-owned runtime.
//! This is an unsupported authentication path; it never reads Claude Code's credentials.
use std::collections::HashMap;

use http::{HeaderMap, HeaderValue};
use rig_core::OneOrMany;
use rig_core::client::{ApiKey, Client, ClientBuilder, Provider, ProviderBuilder};
use rig_core::completion::{AssistantContent, CompletionRequest, Message};
use rig_core::http_client::{self, HttpClientExt};
use rig_core::message::UserContent;
use rig_core::providers::anthropic;
use rig_core::providers::anthropic::client::AnthropicExt;
use serde_json::json;
use solar_agent_core::{AgentError, AgentErrorKind, AgentEvent, ToolEvent};

use crate::http_client::DiagnosticHttpClient;

pub fn is_claude_subscription_token(credential: &str) -> bool {
    credential.trim().contains("sk-ant-oat")
}

// Pi OAuth compatibility profile, reviewed 2026-10-03:
// https://github.com/badlogic/pi-mono/blob/main/packages/ai/src/api/anthropic-messages.ts
// Keep this independent of the version of the separately installed ACP CLI.
const PI_CLAUDE_CODE_USER_AGENT: &str = "claude-cli/2.1.280";

pub(crate) const CLAUDE_CODE_IDENTITY: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

// Use Rig's native Messages translation and streaming, but replace its API-key
// header at client construction rather than sending both authentication methods.
#[derive(Clone, Default)]
struct SubscriptionBuilder;
struct SubscriptionProvider;
struct SubscriptionToken(String);

impl ApiKey for SubscriptionToken {
    fn into_header(self) -> Option<http_client::Result<(http::HeaderName, HeaderValue)>> {
        Some(
            http_client::make_auth_header(self.0).map(|(name, mut value)| {
                value.set_sensitive(true);
                (name, value)
            }),
        )
    }
}

impl Provider for SubscriptionProvider {
    type Builder = SubscriptionBuilder;
    const VERIFY_PATH: &'static str = "/v1/models";
}

impl ProviderBuilder for SubscriptionBuilder {
    type Extension<H>
        = AnthropicExt
    where
        H: HttpClientExt;
    type ApiKey = SubscriptionToken;
    const BASE_URL: &'static str = "https://api.anthropic.com";

    fn build<H>(_: &ClientBuilder<Self, Self::ApiKey, H>) -> http_client::Result<Self::Extension<H>>
    where
        H: HttpClientExt,
    {
        Ok(AnthropicExt)
    }
}

pub(crate) fn anthropic_client(
    credential: &str,
) -> Result<anthropic::Client<DiagnosticHttpClient>, String> {
    let credential = credential.trim();
    if !is_claude_subscription_token(credential) {
        return anthropic::Client::builder()
            .api_key(credential)
            .http_client(DiagnosticHttpClient::new())
            .build()
            .map_err(|error| error.to_string());
    }
    let headers = subscription_headers();
    Client::<SubscriptionProvider>::builder()
        .api_key(SubscriptionToken(credential.to_string()))
        .http_headers(headers)
        .http_client(DiagnosticHttpClient::new())
        .build()
        // Never include a credential or raw header-validation error in diagnostics.
        .map_err(|_| "Could not configure Claude subscription authentication.".to_string())
}

const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

fn wire_tool_name(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or(name)
        .to_string()
}

fn normalize_call_id(id: &mut String) {
    *id = id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
}

pub(crate) fn prepare_subscription_request(
    request: &mut CompletionRequest,
) -> Result<HashMap<String, String>, AgentError> {
    let mut aliases = HashMap::new();
    let mut tools = Vec::new();
    let tool_count = request.tools.len();
    for (index, tool) in request.tools.drain(..).enumerate() {
        let name = wire_tool_name(&tool.name);
        if aliases.insert(name.clone(), tool.name.clone()).is_some() {
            return Err(AgentError::new(
                AgentErrorKind::InvalidRequest,
                "Claude subscription tool names collide after canonical casing.",
            ));
        }
        // Pi's default non-strict schema preserves properties and required.
        let mut definition = json!({
            "name": name,
            "description": tool.description,
            "input_schema": {
                "type": "object",
                "properties": tool.parameters.get("properties").cloned().unwrap_or(json!({})),
                "required": tool.parameters.get("required").cloned().unwrap_or(json!([])),
            },
            "eager_input_streaming": true,
        });
        if index + 1 == tool_count {
            definition["cache_control"] = json!({"type": "ephemeral"});
        }
        tools.push(definition);
    }
    let mut history = Vec::new();
    let mut solar_system = Vec::new();
    for mut message in request.chat_history.iter().cloned() {
        match &mut message {
            Message::System { content } => {
                solar_system.push(content.clone());
                continue;
            }
            Message::Assistant { content, .. } => {
                for part in content.iter_mut() {
                    if let AssistantContent::ToolCall(call) = part {
                        call.function.name = wire_tool_name(&call.function.name);
                        normalize_call_id(&mut call.id);
                        if let Some(id) = call.call_id.as_mut() {
                            normalize_call_id(id);
                        }
                    }
                }
            }
            Message::User { content } => {
                for part in content.iter_mut() {
                    if let UserContent::ToolResult(result) = part {
                        normalize_call_id(&mut result.id);
                        if let Some(id) = result.call_id.as_mut() {
                            normalize_call_id(id);
                        }
                    }
                }
            }
        }
        history.push(message);
    }
    request.chat_history = OneOrMany::many(history)
        .map_err(|error| AgentError::new(AgentErrorKind::InvalidRequest, error.to_string()))?;
    let mut system = vec![json!({
        "type": "text", "text": CLAUDE_CODE_IDENTITY,
        "cache_control": {"type": "ephemeral"},
    })];
    if !solar_system.is_empty() {
        system.push(json!({
            "type": "text", "text": solar_system.join("\n\n"),
            "cache_control": {"type": "ephemeral"},
        }));
    }
    let mut parameters = request.additional_params.take().unwrap_or(json!({}));
    let Some(parameters_map) = parameters.as_object_mut() else {
        return Err(AgentError::new(
            AgentErrorKind::InvalidRequest,
            "Claude subscription request parameters must be an object.",
        ));
    };
    parameters_map.insert("system".to_string(), system.into());
    if !tools.is_empty() {
        parameters_map.insert("tools".to_string(), tools.into());
    }
    request.additional_params = Some(parameters);
    Ok(aliases)
}

pub(crate) fn restore_tool_names(event: &mut AgentEvent, aliases: &HashMap<String, String>) {
    let name = match event {
        AgentEvent::Tool {
            event: ToolEvent::Proposed { call },
        } => Some(&mut call.name),
        AgentEvent::ToolCallProgress { name, .. } => name.as_mut(),
        _ => None,
    };
    if let Some(name) = name {
        // Pi matches against the advertised tools case-insensitively on return.
        if let Some(original) = aliases
            .iter()
            .find_map(|(wire, original)| wire.eq_ignore_ascii_case(name).then_some(original))
        {
            name.clone_from(original);
        }
    }
}

fn subscription_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
    headers.insert(
        "anthropic-beta",
        HeaderValue::from_static("claude-code-20250219,oauth-2025-04-20"),
    );
    headers.insert(
        "user-agent",
        HeaderValue::from_static(PI_CLAUDE_CODE_USER_AGENT),
    );
    headers.insert(
        "anthropic-dangerous-direct-browser-access",
        HeaderValue::from_static("true"),
    );
    headers.insert("x-app", HeaderValue::from_static("cli"));
    headers.insert("accept", HeaderValue::from_static("application/json"));
    headers
}

#[cfg(test)]
#[path = "anthropic_auth_tests.rs"]
mod tests;
