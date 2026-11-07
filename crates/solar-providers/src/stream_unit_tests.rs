use rig_core::completion::CompletionError;
use solar_agent_core::{AgentErrorKind, StopReason};

use super::{
    completion_error_stop_reason, domain_tool_call, is_keepalive_event, map_completion_error,
};

#[test]
fn domain_tool_call_prefers_responses_call_id() {
    let tool_call = rig_core::message::ToolCall::new(
        "fc_item_123".to_string(),
        rig_core::message::ToolFunction {
            name: "read_files".to_string(),
            arguments: serde_json::json!({"files": ["Cargo.toml"]}),
        },
    )
    .with_call_id("call_123".to_string());

    let call = domain_tool_call(tool_call);

    assert_eq!(call.id, "call_123");
    assert_eq!(call.name, "read_files");
}

#[test]
fn domain_tool_call_falls_back_to_wire_id_without_responses_call_id() {
    let tool_call = rig_core::message::ToolCall::new(
        "fc_item_123".to_string(),
        rig_core::message::ToolFunction {
            name: "read_files".to_string(),
            arguments: serde_json::json!({"files": ["Cargo.toml"]}),
        },
    );

    let call = domain_tool_call(tool_call);

    assert_eq!(call.id, "fc_item_123");
    assert_eq!(call.name, "read_files");
}

#[test]
fn provider_context_window_error_maps_to_semantic_stop_reason_and_kind() {
    let status = rig_core::http_client::Response::builder()
        .status(400)
        .body(())
        .unwrap()
        .status();
    let error = CompletionError::from_http_response(
        status,
        r#"{"error":{"message":"Your input exceeds the context window of this model. Please adjust your input and try again.","code":"400"}}"#,
    );

    assert_eq!(
        completion_error_stop_reason(&error),
        Some(StopReason::ContextWindowExceeded)
    );
    let mapped = map_completion_error(error);
    assert_eq!(mapped.kind, AgentErrorKind::ContextWindowExceeded);
    assert!(!mapped.recoverable);
}

#[test]
fn provider_context_length_string_maps_to_semantic_stop_reason() {
    let error = CompletionError::ProviderError(
        "context_length_exceeded: maximum context length is 128000 tokens".to_string(),
    );

    assert_eq!(
        completion_error_stop_reason(&error),
        Some(StopReason::ContextWindowExceeded)
    );
}

#[test]
fn provider_keepalive_event_is_a_transport_heartbeat() {
    assert!(is_keepalive_event(&serde_json::json!({
        "type": "keepalive",
        "sequence_number": 3,
    })));
    assert!(!is_keepalive_event(&serde_json::json!({
        "type": "unsupported",
    })));
}

#[test]
fn provider_server_error_is_recoverable() {
    let status = rig_core::http_client::Response::builder()
        .status(503)
        .body(())
        .unwrap()
        .status();
    let error = CompletionError::from_http_response(
        status,
        r#"{"error":{"message":"Service temporarily unavailable"}}"#,
    );

    let mapped = map_completion_error(error);
    assert_eq!(mapped.kind, AgentErrorKind::Provider);
    assert!(mapped.recoverable);
}

#[test]
fn flattened_bedrock_credential_errors_are_authentication_failures() {
    for message in [
        r#"{"__type":"ExpiredTokenException","message":"The security token included in the request is expired"}"#,
        "UnrecognizedClientException: The security token included in the request is invalid",
        "AccessDeniedException: not authorized to perform bedrock:ConverseStream",
        "UnauthorizedException: Session token not found or invalid",
        "SSO session has expired or is otherwise invalid",
        "Failed to load AWS credentials: credentials were not loaded",
        "Bedrock credentials not configured",
    ] {
        let mapped = map_completion_error(CompletionError::ProviderError(message.to_string()));
        assert_eq!(mapped.kind, AgentErrorKind::Authentication);
        assert!(!mapped.recoverable);
        assert!(mapped.user_message.is_some());
    }
}

#[test]
fn non_aws_authentication_errors_do_not_recommend_an_aws_login() {
    for error in [
        CompletionError::from_http_response(http::StatusCode::UNAUTHORIZED, "Invalid API key"),
        CompletionError::ProviderError("AccessDenied: model access is not permitted".to_owned()),
    ] {
        let mapped = map_completion_error(error);
        assert_eq!(mapped.kind, AgentErrorKind::Authentication);
        assert!(!mapped.recoverable);
        let message = mapped.user_message.unwrap();
        assert!(message.contains("Provider authentication failed"));
        assert!(!message.contains("AWS"));
        assert!(!message.contains("Bedrock"));
    }
}

#[test]
fn unrelated_provider_failures_do_not_become_authentication_failures() {
    for message in [
        "HTTP client error: connection reset",
        "ProviderError: temporarily unavailable",
        "ValidationException: invalid model",
    ] {
        let mapped = map_completion_error(CompletionError::ProviderError(message.to_owned()));
        assert_ne!(mapped.kind, AgentErrorKind::Authentication);
    }
}

#[test]
fn provider_errors_redact_echoed_credentials() {
    let mut error =
        solar_agent_core::AgentError::new(AgentErrorKind::Provider, "server echoed token-secret");
    error.user_message = Some("token-secret expired".into());
    let error = super::redact_provider_error(error, "token-secret");
    assert!(!error.message.contains("token-secret"));
    assert!(!error.user_message.unwrap().contains("token-secret"));
}
