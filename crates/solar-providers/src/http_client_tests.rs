use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::executor::block_on;
use futures::{StreamExt, future, stream};
use http::StatusCode;

use super::*;

#[derive(Debug)]
struct SourceError {
    message: &'static str,
    source: Option<Box<SourceError>>,
}

impl fmt::Display for SourceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message)
    }
}

impl Error for SourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as &dyn Error)
    }
}

fn transport_failure() -> http_client::Error {
    http_client::Error::Instance(Box::new(SourceError {
        message: "error sending request for url (https://user:password@example.test/v1?api_key=secret-certificate)",
        source: Some(Box::new(SourceError {
            message: "failed to lookup address information: Name or service not known; Authorization: Bearer secret-header",
            source: None,
        })),
    }))
}

#[test]
fn nested_dns_cause_survives_without_url_credentials_queries_or_headers() {
    let message = diagnose_transport_error(transport_failure()).to_string();
    assert_eq!(
        message,
        "Http client error: provider request transport failed: DNS resolution failed"
    );
}

#[test]
fn tls_certificate_cause_is_specific_and_contains_only_static_diagnostics() {
    let error = http_client::Error::Instance(Box::new(SourceError {
        message: "error sending request for url (https://example.test/?token=private)",
        source: Some(Box::new(SourceError {
            message: "invalid peer certificate: UnknownIssuer (private-certificate-content)",
            source: None,
        })),
    }));
    assert_eq!(
        diagnose_transport_error(error).to_string(),
        "Http client error: provider request transport failed: TLS certificate issuer is not trusted"
    );
}

#[test]
fn unrecognized_causes_never_forward_arbitrary_source_text() {
    let error = http_client::Error::Instance(Box::new(SourceError {
        message: "Authorization: Bearer secret; api_key=private; password=hidden",
        source: None,
    }));
    assert_eq!(
        diagnose_transport_error(error).to_string(),
        "Http client error: provider request transport failed: underlying transport cause unavailable"
    );
}

#[test]
fn os_errors_retain_numeric_codes_without_the_original_message() {
    let message = diagnose_transport_error(http_client::Error::Instance(Box::new(
        std::io::Error::from_raw_os_error(111),
    )))
    .to_string();
    assert!(message.contains("OS error 111"));
    assert!(message.len() < 400);
}

#[test]
fn large_unicode_error_display_stops_before_formatting_later_chunks() {
    #[derive(Debug)]
    struct LargeError {
        payload: String,
        later_chunks: AtomicUsize,
    }
    impl fmt::Display for LargeError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str(&self.payload)?;
            self.later_chunks.fetch_add(1, Ordering::Relaxed);
            formatter.write_str("sensitive trailing data")
        }
    }
    impl Error for LargeError {}

    let error = LargeError {
        payload: "🌌".repeat(100_000),
        later_chunks: AtomicUsize::new(0),
    };
    assert_eq!(bounded_error_text(&error), "🌌".repeat(1024));
    assert_eq!(error.later_chunks.load(Ordering::Relaxed), 0);
}

#[test]
fn http_status_and_provider_body_remain_available_for_classification() {
    for status in [
        StatusCode::UNAUTHORIZED,
        StatusCode::TOO_MANY_REQUESTS,
        StatusCode::SERVICE_UNAVAILABLE,
    ] {
        let error = diagnose_transport_error(http_client::Error::InvalidStatusCodeWithMessage(
            status,
            "provider error body".to_owned(),
        ));
        match error {
            http_client::Error::InvalidStatusCodeWithMessage(actual_status, body) => {
                assert_eq!(actual_status, status);
                assert_eq!(body, "provider error body");
            }
            other => panic!("HTTP response changed error variant: {other:?}"),
        }
    }
}

#[derive(Clone, Debug)]
struct FakeHttpClient {
    calls: Arc<AtomicUsize>,
    fail_before_response: bool,
}

impl HttpClientExt for FakeHttpClient {
    fn send<T, U>(
        &self,
        _request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.calls.fetch_add(1, Ordering::Relaxed);
        future::ready(if self.fail_before_response {
            Err(transport_failure())
        } else {
            Ok(Response::new(
                Box::pin(async { Err(transport_failure()) }) as LazyBody<U>
            ))
        })
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        self.send(request.map(|_| Bytes::new()))
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        self.calls.fetch_add(1, Ordering::Relaxed);
        // The interceptor must forward the approved payload and credential header unchanged.
        assert_eq!(request.headers()["authorization"], "Bearer synthetic-key");
        assert_eq!(
            request.into_body().into(),
            Bytes::from_static(b"approved request")
        );
        future::ready(if self.fail_before_response {
            Err(transport_failure())
        } else {
            let body = stream::iter([
                Ok(Bytes::from_static(b"original SSE bytes")),
                Err(transport_failure()),
            ]);
            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("x-request-id", "original-request-id")
                .body(Box::pin(body) as http_client::sse::BoxedStream)
                .unwrap())
        })
    }
}

fn request() -> Request<Bytes> {
    Request::builder()
        .uri("https://example.test/v1?secret=private")
        .header("authorization", "Bearer synthetic-key")
        .body(Bytes::from_static(b"approved request"))
        .unwrap()
}

#[test]
fn plain_system_content_preserves_instruction_role_and_other_messages() {
    let payload = serde_json::json!({
        "model": "codex-gpt-6-astra",
        "messages": [
            {"role": "system", "content": [{"type": "text", "text": "Role: researcher.\n"}, {"type": "text", "text": "Use tools carefully. 🌞"}]},
            {"role": "user", "content": [{"type": "text", "text": "Inspect this."}, {"type": "image_url", "image_url": {"url": "data:image/png;base64,fixture"}}]},
            {"role": "assistant", "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "lookup", "arguments": "{}"}}]},
            {"role": "tool", "tool_call_id": "call-1", "content": "result"}
        ],
        "stream": true, "tools": [{"type": "function", "function": {"name": "lookup"}}]
    });
    let make_request = || {
        Request::builder()
            .uri("http://localhost/v1/chat/completions")
            .header("authorization", "Bearer synthetic-key")
            .header("content-length", "999")
            .body(serde_json::to_vec(&payload).unwrap())
            .unwrap()
    };
    let unmodified = DiagnosticHttpClient::new().prepare(make_request());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(unmodified.body()).unwrap(),
        payload
    );
    assert_eq!(unmodified.headers()["content-length"], "999");

    let normalized = DiagnosticHttpClient::new()
        .with_plain_system_content()
        .prepare(make_request());
    let body: serde_json::Value = serde_json::from_slice(normalized.body()).unwrap();
    let mut expected = payload;
    expected["messages"][0]["content"] = "Role: researcher.\nUse tools carefully. 🌞".into();
    assert_eq!(body, expected);
    assert_eq!(
        normalized.headers()["authorization"],
        "Bearer synthetic-key"
    );
    assert!(!normalized.headers().contains_key("content-length"));
}

#[test]
fn system_normalization_never_drops_unknown_content_or_rewrites_strings() {
    for content in [
        serde_json::json!("Already a string"),
        serde_json::json!([{"type": "text", "text": "Keep"}, {"type": "future", "data": "also keep"}]),
    ] {
        let original = serde_json::json!({"messages": [{"role": "system", "content": content}]});
        let mut body = original.clone();
        assert!(!normalize_system_content(&mut body));
        assert_eq!(body, original);
    }
}

#[test]
fn streaming_adapter_preserves_response_and_chunks_without_internal_retries() {
    block_on(async {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = DiagnosticHttpClient {
            inner: FakeHttpClient {
                calls: calls.clone(),
                fail_before_response: false,
            },
            plain_system_content: false,
        };
        let response = client.send_streaming(request()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-request-id"], "original-request-id");
        let mut body = response.into_body();
        assert_eq!(
            body.next().await.unwrap().unwrap(),
            Bytes::from_static(b"original SSE bytes")
        );
        assert!(
            body.next()
                .await
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("DNS resolution failed")
        );
        assert!(body.next().await.is_none());
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn adapter_preserves_causes_before_headers_and_during_lazy_body_reads() {
    block_on(async {
        let calls = Arc::new(AtomicUsize::new(0));
        let client = DiagnosticHttpClient {
            inner: FakeHttpClient {
                calls: calls.clone(),
                fail_before_response: true,
            },
            plain_system_content: false,
        };
        let error = client.send_streaming(request()).await.err().unwrap();
        assert!(error.to_string().contains("DNS resolution failed"));
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        let client = DiagnosticHttpClient {
            inner: FakeHttpClient {
                calls: calls.clone(),
                fail_before_response: false,
            },
            plain_system_content: false,
        };
        let response = client.send::<_, Bytes>(request()).await.unwrap();
        let error = response.into_body().await.unwrap_err();
        assert!(error.to_string().contains("DNS resolution failed"));
        assert_eq!(calls.load(Ordering::Relaxed), 2);
    });
}
