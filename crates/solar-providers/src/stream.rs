use std::collections::HashMap;

use futures::{FutureExt, StreamExt};
use rig_core::completion::{CompletionError, CompletionModel, CompletionRequest};
use rig_core::streaming::{StreamedAssistantContent, ToolCallDeltaContent};
use solar_agent_core::{
    AgentError, AgentErrorKind, AgentEvent, AgentEventStream, StopReason, ToolCall, TurnCommand,
    TurnControl, Usage,
};
use uuid::Uuid;

pub(crate) async fn start_model_turn<M>(
    model: M,
    completion_request: CompletionRequest,
    control: TurnControl,
    max_output_tokens: Option<u64>,
) -> Result<AgentEventStream, AgentError>
where
    M: CompletionModel + Send + Sync + 'static,
{
    let runtime_request_id = Uuid::new_v4().to_string();
    let stream_future = model.stream(completion_request).fuse();
    let initial_control = control.clone();
    let control_future = initial_control.receive().fuse();
    futures::pin_mut!(stream_future, control_future);

    let stream_result = futures::select_biased! {
        command = control_future => match command {
            Ok(TurnCommand::Cancel) => {
                return Ok(stopped_before_stream(runtime_request_id));
            }
            Ok(TurnCommand::Steer { .. }) | Err(_) => stream_future.await,
        },
        result = stream_future => result,
    };
    let mut rig_stream = match stream_result {
        Ok(stream) => stream,
        Err(error) => {
            if let Some(reason) = completion_error_stop_reason(&error) {
                return Ok(stopped_with_reason(runtime_request_id, reason));
            }
            return Err(map_completion_error(error));
        }
    };

    let events = async_stream::stream! {
        yield Ok(AgentEvent::TurnStarted {
            runtime_request_id,
        });

        let mut control_open = true;
        let mut last_output_tokens = 0;
        let mut saw_final_response = false;
        let mut tool_call_progress = HashMap::<String, ToolCallProgressState>::new();
        loop {
            let next_item = rig_stream.next().fuse();
            let next_command = if control_open {
                futures::future::Either::Left(control.receive())
            } else {
                futures::future::Either::Right(futures::future::pending())
            }
            .fuse();
            futures::pin_mut!(next_item, next_command);

            futures::select_biased! {
                command = next_command => {
                    match command {
                        Ok(TurnCommand::Cancel) => {
                            rig_stream.cancel();
                            yield Ok(AgentEvent::TurnStopped {
                                reason: StopReason::Cancelled,
                            });
                            return;
                        }
                        Ok(TurnCommand::Steer { .. }) => {
                            // Steering is not advertised by provider runtimes yet.
                        }
                        Err(_) => control_open = false,
                    }
                }
                item = next_item => {
                    let Some(item) = item else {
                        if !saw_final_response {
                            let mut error = AgentError::new(
                                AgentErrorKind::Transport,
                                "Provider stream ended before a final response. The turn is incomplete.",
                            );
                            error.recoverable = true;
                            yield Err(error);
                            return;
                        }
                        yield Ok(AgentEvent::TurnStopped {
                            reason: if max_output_tokens.is_some_and(|max| {
                                last_output_tokens >= max
                            }) {
                                StopReason::MaxTokens
                            } else {
                                StopReason::Completed
                            },
                        });
                        return;
                    };

                    match item {
                        Ok(StreamedAssistantContent::Text(text)) => {
                            if !text.text.is_empty() {
                                yield Ok(AgentEvent::TextDelta { text: text.text });
                            }
                        }
                        Ok(StreamedAssistantContent::Reasoning(reasoning)) => {
                            let text = reasoning.display_text();
                            let signature = reasoning.first_signature().map(str::to_string);
                            // Some Responses providers send empty reasoning blocks as heartbeats.
                            // Do not expose them as model progress: doing so continually resets the
                            // coordinator's idle timeout and can leave a turn stuck forever.
                            if !text.is_empty() || signature.is_some() {
                                yield Ok(AgentEvent::ReasoningCompleted { text, signature });
                            }
                        }
                        Ok(StreamedAssistantContent::ReasoningDelta { reasoning, .. }) => {
                            if !reasoning.is_empty() {
                                yield Ok(AgentEvent::ReasoningDelta { text: reasoning });
                            }
                        }
                        Ok(StreamedAssistantContent::ToolCall {
                            tool_call,
                            internal_call_id,
                        }) => {
                            tool_call_progress.remove(&internal_call_id);
                            yield Ok(AgentEvent::Tool {
                                event: solar_agent_core::ToolEvent::Proposed {
                                    call: domain_tool_call(tool_call),
                                },
                            });
                        }
                        Ok(StreamedAssistantContent::ToolCallDelta {
                            id,
                            internal_call_id,
                            content,
                        }) => {
                            let progress = tool_call_progress
                                .entry(internal_call_id.clone())
                                .or_insert_with(|| ToolCallProgressState {
                                    call_id: if id.is_empty() {
                                        internal_call_id
                                    } else {
                                        id
                                    },
                                    ..ToolCallProgressState::default()
                                });
                            match content {
                                ToolCallDeltaContent::Name(name) => progress.name = Some(name),
                                ToolCallDeltaContent::Delta(arguments) => {
                                    progress.arguments_bytes = progress
                                        .arguments_bytes
                                        .saturating_add(arguments.len() as u64);
                                }
                            }
                            yield Ok(AgentEvent::ToolCallProgress {
                                call_id: progress.call_id.clone(),
                                name: progress.name.clone(),
                                arguments_bytes: progress.arguments_bytes,
                            });
                        }
                        Ok(StreamedAssistantContent::Final(response)) => {
                            saw_final_response = true;
                            let mapped_usage = map_usage(response.usage);
                            last_output_tokens = mapped_usage.output_tokens;
                            yield Ok(AgentEvent::UsageUpdated {
                                usage: mapped_usage,
                            });
                        }
                        Ok(StreamedAssistantContent::Unknown(value)) => {
                            // ChatGPT subscription streams include informational moderation
                            // metadata that Rig does not map to assistant content.
                            if value.get("type").and_then(serde_json::Value::as_str)
                                == Some("response.metadata")
                            {
                                continue;
                            }
                            if is_keepalive_event(&value) {
                                yield Ok(AgentEvent::KeepAlive);
                            } else {
                                yield Err(AgentError::new(
                                    AgentErrorKind::Protocol,
                                    format!("Rig returned an unsupported provider event: {value}"),
                                ));
                                return;
                            }
                        }
                        Err(error) => {
                            if let Some(reason) = completion_error_stop_reason(&error) {
                                yield Ok(AgentEvent::TurnStopped { reason });
                                return;
                            }
                            yield Err(map_completion_error(error));
                            return;
                        }
                    }
                }
            }
        }
    };

    Ok(Box::pin(events))
}

#[derive(Default)]
struct ToolCallProgressState {
    call_id: String,
    name: Option<String>,
    arguments_bytes: u64,
}

pub(crate) fn redact_provider_error(mut error: AgentError, credential: &str) -> AgentError {
    if !credential.is_empty() {
        error.message = error.message.replace(credential, "<redacted>");
        error.user_message = error
            .user_message
            .map(|message| message.replace(credential, "<redacted>"));
    }
    error
}

pub(crate) fn protect_stream(stream: AgentEventStream, credential: String) -> AgentEventStream {
    Box::pin(
        stream.map(move |result| result.map_err(|error| redact_provider_error(error, &credential))),
    )
}

fn domain_tool_call(tool_call: rig_core::message::ToolCall) -> ToolCall {
    ToolCall {
        // OpenAI Responses uses a separate `call_id` for function-call output
        // correlation. The domain model has one ID, so preserve that value when
        // it is available and fall back to the standard tool-call ID for other
        // OpenAI-compatible providers.
        id: tool_call.call_id.unwrap_or(tool_call.id),
        name: tool_call.function.name,
        arguments: tool_call.function.arguments,
    }
}

fn stopped_before_stream(runtime_request_id: String) -> AgentEventStream {
    stopped_with_reason(runtime_request_id, StopReason::Cancelled)
}

fn stopped_with_reason(runtime_request_id: String, reason: StopReason) -> AgentEventStream {
    Box::pin(futures::stream::iter([
        Ok(AgentEvent::TurnStarted { runtime_request_id }),
        Ok(AgentEvent::TurnStopped { reason }),
    ]))
}

pub(crate) fn map_usage(usage: rig_core::completion::Usage) -> Usage {
    // Rig preserves provider-native input semantics: OpenAI includes cached
    // tokens in `input_tokens`, while Anthropic reports cache reads/writes
    // separately. `total_tokens - output_tokens` gives the normalized prompt
    // size for both shapes, so store only the uncached portion in
    // `input_tokens` and keep cache reads/writes disjoint.
    let total_input_tokens = if usage.total_tokens > 0 && usage.total_tokens >= usage.output_tokens
    {
        usage.total_tokens - usage.output_tokens
    } else {
        usage
            .input_tokens
            .saturating_add(usage.cached_input_tokens)
            .saturating_add(usage.cache_creation_input_tokens)
    };
    Usage {
        input_tokens: total_input_tokens
            .saturating_sub(usage.cached_input_tokens)
            .saturating_sub(usage.cache_creation_input_tokens),
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cache_creation_input_tokens: usage.cache_creation_input_tokens,
    }
}

pub(crate) fn completion_error_stop_reason(error: &CompletionError) -> Option<StopReason> {
    if completion_error_indicates_context_window_exceeded(error) {
        return Some(StopReason::ContextWindowExceeded);
    }

    match error {
        // rig-bedrock 0.40 currently surfaces Bedrock's MaxTokens stop as a
        // provider error. Normalize it here so the UI sees the same semantic
        // stop reason as every other Rig-backed provider.
        CompletionError::ProviderError(message) if message == "Exceeded max tokens" => {
            Some(StopReason::MaxTokens)
        }
        _ => None,
    }
}

fn completion_error_indicates_context_window_exceeded(error: &CompletionError) -> bool {
    if let Ok(Some(value)) = error.provider_response_json()
        && json_value_indicates_context_window_exceeded(&value)
    {
        return true;
    }

    error
        .provider_response_body()
        .is_some_and(text_indicates_context_window_exceeded)
        || text_indicates_context_window_exceeded(&error.to_string())
}

fn json_value_indicates_context_window_exceeded(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => text_indicates_context_window_exceeded(text),
        serde_json::Value::Array(values) => values
            .iter()
            .any(json_value_indicates_context_window_exceeded),
        serde_json::Value::Object(map) => map
            .values()
            .any(json_value_indicates_context_window_exceeded),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            false
        }
    }
}

fn is_keepalive_event(value: &serde_json::Value) -> bool {
    value.get("type").and_then(serde_json::Value::as_str) == Some("keepalive")
}

fn text_indicates_context_window_exceeded(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("modelcontextwindowexceeded")
        || normalized.contains("context_length_exceeded")
        || normalized.contains("context window")
        || normalized.contains("context length")
        || normalized.contains("too many tokens")
        || normalized.contains("maximum context")
        || normalized.contains("input is too long")
        || (normalized.contains("input exceeds") && normalized.contains("context"))
        || normalized.contains("exceeds the context")
}

fn completion_error_indicates_recoverable_transport(error: &CompletionError) -> bool {
    match error {
        // Rig flattens non-status SSE transport failures into ProviderError.
        // Preserve their transport semantics so the durable provider run can
        // retry a truncated or otherwise interrupted response stream.
        CompletionError::ProviderError(message) => {
            let normalized = message.to_ascii_lowercase();
            normalized.starts_with("http client error:")
                || text_indicates_interrupted_stream(&normalized)
        }
        _ => text_indicates_interrupted_stream(&error.to_string().to_ascii_lowercase()),
    }
}

fn completion_error_indicates_transient_provider_failure(error: &CompletionError) -> bool {
    error
        .provider_response_body()
        .is_some_and(text_indicates_transient_provider_failure)
        || text_indicates_transient_provider_failure(&error.to_string())
}

fn text_indicates_transient_provider_failure(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    normalized.contains("server_is_overloaded")
        || normalized.contains("service_unavailable_error")
        || normalized.contains("temporarily unavailable")
        || normalized.contains("service unavailable")
        || normalized.contains("server is overloaded")
        || normalized.contains("servers are currently overloaded")
        || normalized.contains("overload")
        || normalized.contains("bad gateway")
        || normalized.contains("gateway timeout")
        || normalized.contains("upstream connect error")
        || normalized.contains("upstream request timeout")
        || normalized.contains("upstream timed out")
        || normalized.contains("upstream unavailable")
}

fn text_indicates_interrupted_stream(text: &str) -> bool {
    text.contains("unexpected eof")
        || text.contains("unexpected end of file")
        || text.contains("stream terminated")
        || text.contains("stream ended")
        || text.contains("stream closed")
        || text.contains("connection reset")
        || text.contains("connection closed")
        || text.contains("incomplete message")
}

fn map_completion_error(error: CompletionError) -> AgentError {
    let is_context_window_exceeded = completion_error_indicates_context_window_exceeded(&error);
    let is_authentication_failure = completion_error_indicates_authentication_failure(&error);
    let is_recoverable_transport = completion_error_indicates_recoverable_transport(&error);
    let is_transient_provider_failure =
        completion_error_indicates_transient_provider_failure(&error);
    let status = error
        .provider_response_status()
        .map(|status| status.as_u16());
    let kind = if is_context_window_exceeded {
        AgentErrorKind::ContextWindowExceeded
    } else if is_authentication_failure {
        // Bedrock sometimes returns credential errors as a provider error with no
        // HTTP status (and Rig can flatten the response body). Classify by the
        // service error code as well as status so the host can run its existing
        // credential refresh/login flow.
        AgentErrorKind::Authentication
    } else if is_recoverable_transport || matches!(status, Some(408 | 425)) {
        AgentErrorKind::Transport
    } else {
        match status {
            Some(401 | 403) => AgentErrorKind::Authentication,
            Some(429) => AgentErrorKind::RateLimited,
            Some(400 | 404 | 413 | 422) => AgentErrorKind::InvalidRequest,
            Some(500..=599) => AgentErrorKind::Provider,
            Some(_) => AgentErrorKind::Provider,
            None => match &error {
                CompletionError::HttpError(_)
                | CompletionError::UrlError(_)
                | CompletionError::RequestError(_) => AgentErrorKind::Transport,
                CompletionError::JsonError(_) | CompletionError::ResponseError(_) => {
                    AgentErrorKind::Protocol
                }
                CompletionError::ProviderError(_) | CompletionError::ProviderResponse(_) => {
                    AgentErrorKind::Provider
                }
                _ => AgentErrorKind::Provider,
            },
        }
    };
    let mut mapped = AgentError::new(kind, error.to_string());
    if kind == AgentErrorKind::Authentication {
        mapped.user_message = Some(
            "Provider authentication failed. Check the credentials and access permissions for this connection, then try again."
                .to_string(),
        );
    }
    mapped.recoverable = matches!(
        kind,
        AgentErrorKind::RateLimited | AgentErrorKind::Transport
    ) || status.is_some_and(|status| {
        matches!(status, 408 | 425 | 502 | 503 | 504) || (500..=599).contains(&status)
    }) || is_transient_provider_failure;
    mapped
}

fn completion_error_indicates_authentication_failure(error: &CompletionError) -> bool {
    error
        .provider_response_body()
        .is_some_and(text_indicates_authentication_failure)
        || text_indicates_authentication_failure(&error.to_string())
}

fn text_indicates_authentication_failure(text: &str) -> bool {
    let normalized = text.to_ascii_lowercase();
    [
        "accessdenied",
        "access denied",
        "expiredtoken",
        "expired token",
        "invalidclienttokenid",
        "invalid client token",
        "unrecognizedclient",
        "unrecognized client",
        "security token included in the request is invalid",
        "unable to locate credentials",
        "unauthorizedexception",
        "session token not found or invalid",
        "sso session has expired",
        "failed to load aws credentials",
        "bedrock credentials not configured",
        "credential should be scoped to a valid region",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

#[cfg(test)]
#[path = "stream_unit_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "stream_tests.rs"]
mod regression_tests;
