use async_trait::async_trait;
use rig_core::client::CompletionClient;
use rig_core::completion::{CompletionModel, CompletionRequest};
use rig_core::providers::openai;
use solar_agent_core::{
    AgentError, AgentErrorKind, AgentEventStream, AgentRuntime, RuntimeCapabilities,
    RuntimeDescriptor, RuntimeKind, TurnControl, TurnRequest,
};

use crate::http_client::DiagnosticHttpClient;
use crate::request::build_completion_request as build_provider_completion_request;
use crate::stream::start_model_turn as start_provider_model_turn;

#[derive(Clone, PartialEq, Eq)]
pub struct OpenAICompatibleRuntimeConfig {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_output_tokens: Option<u64>,
    pub supports_system_messages: bool,
}

impl std::fmt::Debug for OpenAICompatibleRuntimeConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompatibleRuntimeConfig")
            .field("base_url", &self.base_url)
            .field("has_api_key", &self.api_key.is_some())
            .field("model", &self.model)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("supports_system_messages", &self.supports_system_messages)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct OpenAICompatibleRuntime {
    config: OpenAICompatibleRuntimeConfig,
    descriptor: RuntimeDescriptor,
    reasoning_effort: Option<String>,
}

impl OpenAICompatibleRuntime {
    pub fn with_reasoning_effort(mut self, effort: Option<String>) -> Self {
        self.reasoning_effort = effort;
        self
    }

    pub fn new(config: OpenAICompatibleRuntimeConfig) -> Self {
        let descriptor = RuntimeDescriptor {
            id: format!("rig-openai-compatible:{}", config.model),
            display_name: format!("Rig / {}", config.model),
            kind: RuntimeKind::Provider,
            capabilities: RuntimeCapabilities::provider(),
        };
        Self {
            config,
            descriptor,
            reasoning_effort: None,
        }
    }
}

#[async_trait]
impl AgentRuntime for OpenAICompatibleRuntime {
    fn descriptor(&self) -> &RuntimeDescriptor {
        &self.descriptor
    }

    async fn start_turn(
        &self,
        request: TurnRequest,
        control: TurnControl,
    ) -> Result<AgentEventStream, AgentError> {
        let client = openai::CompletionsClient::builder()
            // Rig 0.40 requires an API-key builder value. An empty key preserves
            // compatibility with unauthenticated local OpenAI-compatible servers.
            .api_key(self.config.api_key.as_deref().unwrap_or_default())
            .base_url(&self.config.base_url)
            .http_client(DiagnosticHttpClient::new())
            .build()
            .map_err(|error| AgentError::new(AgentErrorKind::Configuration, error.to_string()))?;
        let model = client.completion_model(&self.config.model);
        let credential = self.config.api_key.clone().unwrap_or_default();
        let stream = start_model_turn(
            model,
            request,
            control,
            self.config.max_output_tokens,
            self.config.supports_system_messages,
            self.reasoning_effort.as_deref(),
        )
        .await
        .map_err(|error| crate::stream::redact_provider_error(error, &credential))?;
        Ok(crate::stream::protect_stream(stream, credential))
    }
}

async fn start_model_turn<M>(
    model: M,
    request: TurnRequest,
    control: TurnControl,
    configured_max_output_tokens: Option<u64>,
    supports_system_messages: bool,
    reasoning_effort: Option<&str>,
) -> Result<AgentEventStream, AgentError>
where
    M: CompletionModel + Send + Sync + 'static,
{
    let max_output_tokens = request.max_output_tokens.or(configured_max_output_tokens);
    let completion_request = build_completion_request(
        request,
        configured_max_output_tokens,
        supports_system_messages,
        reasoning_effort,
    )?;
    start_provider_model_turn(model, completion_request, control, max_output_tokens).await
}

fn build_completion_request(
    request: TurnRequest,
    configured_max_output_tokens: Option<u64>,
    supports_system_messages: bool,
    reasoning_effort: Option<&str>,
) -> Result<CompletionRequest, AgentError> {
    let mut params = serde_json::json!({ "stream_options": { "include_usage": true } });
    if let Some(effort) = reasoning_effort {
        params["reasoning_effort"] = effort.into();
    }
    build_provider_completion_request(
        request,
        configured_max_output_tokens,
        supports_system_messages,
        true,
        Some(params),
    )
}

#[cfg(test)]
#[path = "openai_compatible_tests.rs"]
mod tests;
