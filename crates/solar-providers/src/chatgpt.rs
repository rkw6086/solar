use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use futures::future::BoxFuture;
use rig_core::client::CompletionClient;
use rig_core::providers::chatgpt::{self, ChatGPTAuth};
use solar_agent_core::{
    AgentError, AgentErrorKind, AgentEvent, AgentEventStream, AgentRuntime, ConversationMessage,
    MessageContent, MessageRole, RuntimeCapabilities, RuntimeDescriptor, RuntimeKind, TurnControl,
    TurnRequest, turn_control,
};

use crate::http_client::DiagnosticHttpClient;
use crate::request::build_completion_request;
use crate::stream::start_model_turn;

#[derive(Clone, PartialEq, Eq)]
pub struct ChatGPTSubscriptionCredentials {
    pub access_token: String,
    pub account_id: Option<String>,
}

impl std::fmt::Debug for ChatGPTSubscriptionCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatGPTSubscriptionCredentials")
            .field("access_token", &"<redacted>")
            .field(
                "account_id",
                &self.account_id.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}
pub type ChatGPTCredentialResolver = Arc<
    dyn Fn() -> BoxFuture<'static, Result<ChatGPTSubscriptionCredentials, String>> + Send + Sync,
>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChatGPTSubscriptionRuntimeConfig {
    pub model: String,
    pub reasoning_effort: Option<String>,
    pub max_output_tokens: Option<u64>,
}

#[derive(Clone)]
pub struct ChatGPTSubscriptionRuntime {
    config: ChatGPTSubscriptionRuntimeConfig,
    descriptor: RuntimeDescriptor,
    credential_resolver: Option<ChatGPTCredentialResolver>,
}

impl std::fmt::Debug for ChatGPTSubscriptionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatGPTSubscriptionRuntime")
            .field("config", &self.config)
            .field(
                "has_credential_resolver",
                &self.credential_resolver.is_some(),
            )
            .finish()
    }
}

fn reasoning_additional_params(reasoning_effort: Option<&str>) -> Option<serde_json::Value> {
    reasoning_effort.map(|effort| {
        // Codex exposes `ultra` as a client-side delegation mode, while the
        // Responses wire format carries its maximum reasoning as `max`.
        let wire_effort = if effort == "ultra" { "max" } else { effort };
        serde_json::json!({"reasoning": {"effort": wire_effort}})
    })
}

impl ChatGPTSubscriptionRuntime {
    pub fn new(config: ChatGPTSubscriptionRuntimeConfig) -> Self {
        let descriptor = RuntimeDescriptor {
            id: format!("rig-chatgpt-subscription:{}", config.model),
            display_name: format!("ChatGPT Subscription / {}", config.model),
            kind: RuntimeKind::Provider,
            capabilities: RuntimeCapabilities::provider(),
        };
        Self {
            config,
            descriptor,
            credential_resolver: None,
        }
    }

    pub fn with_credential_resolver(mut self, resolver: ChatGPTCredentialResolver) -> Self {
        self.credential_resolver = Some(resolver);
        self
    }

    async fn resolve_auth(&self) -> Result<ChatGPTAuth, AgentError> {
        let resolver = self.credential_resolver.as_ref().ok_or_else(|| {
            AgentError::new(
                AgentErrorKind::Authentication,
                "ChatGPT sign-in required. Reconnect ChatGPT in Providers.",
            )
        })?;
        let credentials = resolver()
            .await
            .map_err(|message| AgentError::new(AgentErrorKind::Authentication, message))?;
        Ok(ChatGPTAuth::AccessToken {
            access_token: credentials.access_token,
            account_id: credentials.account_id,
        })
    }

    /// Completes a text-only request for callers such as crosscheck reviewers.
    pub async fn complete_text(
        &self,
        system_prompt: String,
        user_prompt: String,
    ) -> Result<String, String> {
        let mut request = TurnRequest::new(
            self.config.model.clone(),
            vec![ConversationMessage {
                role: MessageRole::User,
                content: MessageContent::Text(user_prompt),
            }],
        );
        request.system_prompt = Some(system_prompt);
        request.max_output_tokens = self.config.max_output_tokens;

        let (_sender, control) = turn_control();
        let mut stream = self
            .start_turn(request, control)
            .await
            .map_err(|error| error.to_string())?;
        let mut output = String::new();
        while let Some(event) = stream.next().await {
            match event.map_err(|error| error.to_string())? {
                AgentEvent::TextDelta { text } => output.push_str(&text),
                AgentEvent::ReasoningDelta { .. }
                | AgentEvent::ReasoningCompleted { .. }
                | AgentEvent::TurnStarted { .. }
                | AgentEvent::KeepAlive
                | AgentEvent::ToolCallProgress { .. }
                | AgentEvent::UsageUpdated { .. }
                | AgentEvent::RuntimeActivityUpdated { .. }
                | AgentEvent::ContextUsageUpdated { .. }
                | AgentEvent::UserInputAccepted { .. }
                | AgentEvent::RuntimeNotice { .. }
                | AgentEvent::TurnStopped { .. }
                | AgentEvent::Tool { .. } => {}
            }
        }

        if output.is_empty() {
            return Err("ChatGPT returned an empty response.".to_string());
        }
        Ok(output)
    }
}

#[async_trait]
impl AgentRuntime for ChatGPTSubscriptionRuntime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        &self.descriptor
    }

    async fn start_turn(
        &self,
        request: TurnRequest,
        control: TurnControl,
    ) -> Result<AgentEventStream, AgentError> {
        let auth = self.resolve_auth().await?;
        let credential = match &auth {
            ChatGPTAuth::AccessToken { access_token, .. } => access_token.clone(),
            ChatGPTAuth::OAuth => String::new(),
        };
        let client = chatgpt::Client::builder()
            .api_key(auth)
            .allow_device_flow(false)
            .originator("solar")
            .http_client(DiagnosticHttpClient::new())
            .build()
            .map_err(|error| AgentError::new(AgentErrorKind::Configuration, error.to_string()))?;
        let model = client.completion_model(&self.config.model);
        let max_output_tokens = request.max_output_tokens.or(self.config.max_output_tokens);
        let additional_params =
            reasoning_additional_params(self.config.reasoning_effort.as_deref());
        let completion_request = build_completion_request(
            request,
            self.config.max_output_tokens,
            true,
            true,
            additional_params,
        )?;

        let stream = start_model_turn(model, completion_request, control, max_output_tokens)
            .await
            .map_err(|error| crate::stream::redact_provider_error(error, &credential))?;
        Ok(crate::stream::protect_stream(stream, credential))
    }
}

#[cfg(test)]
#[path = "chatgpt_tests.rs"]
mod tests;
