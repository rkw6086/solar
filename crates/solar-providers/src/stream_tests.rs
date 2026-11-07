use rig_core::completion::CompletionError;
use solar_agent_core::AgentErrorKind;

use super::map_completion_error;

#[test]
fn flattened_sse_http_client_error_is_recoverable_transport() {
    let error = CompletionError::ProviderError(
        "Http client error: error decoding response body".to_string(),
    );

    let mapped = map_completion_error(error);

    assert_eq!(mapped.kind, AgentErrorKind::Transport);
    assert!(mapped.recoverable);
}

#[test]
fn safe_transport_diagnostics_survive_rig_flattening_and_remain_retryable() {
    let error = crate::http_client::diagnose_transport_error(
        rig_core::http_client::Error::Instance(Box::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "private-token-not-for-diagnostics",
        ))),
    );
    // Rig uses Display text for SSE failures without a provider response status.
    let mapped = map_completion_error(CompletionError::ProviderError(error.to_string()));
    assert_eq!(mapped.kind, AgentErrorKind::Transport);
    assert!(mapped.recoverable);
    assert!(mapped.message.contains("ConnectionRefused"));
    assert!(!mapped.message.contains("private-token"));
}

#[test]
fn flattened_streamed_provider_overload_is_recoverable() {
    let error = CompletionError::ProviderError(
        r#"{"type":"error","error":{"type":"service_unavailable_error","code":"server_is_overloaded","message":"Our servers are currently overloaded. Please try again later."}}"#
            .to_string(),
    );

    let mapped = map_completion_error(error);

    assert_eq!(mapped.kind, AgentErrorKind::Provider);
    assert!(mapped.recoverable);
}

#[test]
fn transient_http_statuses_are_recoverable() {
    for status_code in [408, 425, 502, 503, 504] {
        let status = rig_core::http_client::Response::builder()
            .status(status_code)
            .body(())
            .unwrap()
            .status();
        let error = CompletionError::from_http_response(status, "transient failure");

        let mapped = map_completion_error(error);

        assert!(mapped.recoverable, "HTTP {status_code} should be retryable");
        assert_eq!(
            mapped.kind,
            if matches!(status_code, 408 | 425) {
                AgentErrorKind::Transport
            } else {
                AgentErrorKind::Provider
            },
            "unexpected classification for HTTP {status_code}"
        );
    }
}

#[test]
fn transient_gateway_upstream_and_overload_messages_are_recoverable() {
    for message in [
        "502 Bad Gateway",
        "upstream request timeout",
        "upstream timed out",
        "The server is overloaded; retry later",
    ] {
        let mapped = map_completion_error(CompletionError::ProviderError(message.to_string()));

        assert_eq!(mapped.kind, AgentErrorKind::Provider);
        assert!(mapped.recoverable, "message should be retryable: {message}");
    }
}

#[test]
fn interrupted_stream_messages_are_recoverable_transport() {
    for message in [
        "Http client error: unexpected EOF",
        "Http client error: stream terminated unexpectedly",
        "Http client error: connection reset by peer",
        "unexpected EOF",
    ] {
        let mapped = map_completion_error(CompletionError::ProviderError(message.to_string()));

        assert_eq!(mapped.kind, AgentErrorKind::Transport);
        assert!(mapped.recoverable, "message should be retryable: {message}");
    }
}
