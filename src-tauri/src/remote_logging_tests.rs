use super::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn logger(capacity: usize) -> (Logger, mpsc::Receiver<Command>) {
    let (sender, receiver) = mpsc::channel(capacity);
    (
        Logger {
            sender,
            sanitizer: Sanitizer::new("test-ingestion-key"),
            session_id: "test-session".into(),
            dropped: AtomicU64::new(0),
        },
        receiver,
    )
}

#[test]
fn environment_overrides_bundle_and_blank_or_invalid_values_disable() {
    let fallback = Config::from_key(None, "bundled-test-value", ENDPOINT).unwrap();
    assert_eq!(fallback.api_key, "bundled-test-value");
    let override_value = Config::from_key(Some(" env-test-value "), "bundle", ENDPOINT).unwrap();
    assert_eq!(override_value.api_key, "env-test-value");
    assert!(override_value.api_key.is_sensitive());
    assert!(Config::from_key(Some("  "), "bundle", ENDPOINT).is_none());
    assert!(Config::from_key(Some("invalid\r\nheader"), "bundle", ENDPOINT).is_none());
    assert!(Config::from_key(None, "", ENDPOINT).is_none());
}

#[test]
fn records_redact_credentials_and_user_content_but_keep_operational_metadata() {
    let (logger, _) = logger(1);
    let payload = logger.payload("error", "provider_failure", "failed with test-ingestion-key", json!({
        "task_id": "task-123",
        "conversation_id": "conversation-456",
        "tool": { "name": "shell", "arguments": { "command": "private command" }, "result": "private output" },
        "prompt": "private prompt",
        "messages": ["private history"],
        "content": "private content",
        "nested": { "api_key": "private key", "accessToken": "private token", "password": "private password",
            "aws_secret_access_key": "private cloud secret", "aws_access_key_id": "private cloud key",
            "RYSERVE_LOGGING_KEY": "private logging override" },
        "error": "request to https://user:password@host.test/path?key=private-url-key failed; Bearer private-bearer",
        "provider_id": "provider-789",
        "tokens": 17,
    }));
    let serialized = serde_json::to_string(&payload).unwrap();
    for secret in [
        "test-ingestion-key",
        "private command",
        "private output",
        "private prompt",
        "private history",
        "private content",
        "private key",
        "private token",
        "private password",
        "private cloud secret",
        "private cloud key",
        "private logging override",
        "private-url-key",
        "private-bearer",
        "user:password",
    ] {
        assert!(!serialized.contains(secret), "leaked {secret}");
    }
    assert_eq!(payload.context["task_id"], "task-123");
    assert_eq!(payload.context["conversation_id"], "conversation-456");
    assert_eq!(payload.context["tool"]["name"], "shell");
    assert_eq!(payload.context["tokens"], 17);
    assert_eq!(payload.context["app"]["session_id"], "test-session");
    assert_eq!(payload.context["event"], "provider_failure");
    assert_eq!(payload.service, "solar");
    assert_eq!(payload.level, "error");
}

#[test]
fn credential_strings_with_quotes_or_escaped_json_cannot_leak_suffixes() {
    let sanitizer = Sanitizer::new("unused");
    for text in [
        r#"password="two words secret" operation failed"#,
        r#"{"error":"api_key=\"two words secret\" failed"}"#,
        "authorization: Bearer two words secret\nnext line",
        "refresh_token={\"nested\":\"two words secret\"} failed",
        "provider failed using sk-live-private-token and log_sk_private-token",
        "token eyJheader.eyJpayload.signature",
        r#"upstream failed: {"prompt":"two words secret","messages":[{"content":"two words secret"}]}"#,
    ] {
        let safe = sanitizer.text(text, 1_000);
        assert!(!safe.contains("two words secret"), "{safe}");
        assert!(!safe.contains("private-token"), "{safe}");
        assert!(!safe.contains("eyJheader"), "{safe}");
    }
}

#[test]
fn oversized_and_deep_records_stay_bounded_without_partial_credential_tails() {
    let (logger, _) = logger(1);
    let text = "🙂 ".repeat(40_000);
    let object: Map<String, Value> = (0..500)
        .map(|index| (format!("field_{index}"), json!(text)))
        .collect();
    let payload = logger.payload("unknown", &text, &text, Value::Object(object));
    let body = serde_json::to_vec(&payload).unwrap();
    assert!(body.len() <= MAX_PAYLOAD_BYTES, "{} bytes", body.len());
    assert_eq!(payload.level, "info");
    assert_eq!(payload.context["_truncated"], true);
    let huge = format!("{} {}", "x".repeat(MAX_INPUT_CHARS - 5), "sk-private-tail");
    assert!(
        !logger
            .sanitizer
            .text(&huge, MAX_INPUT_CHARS)
            .contains("sk-")
    );
    let mut deep = json!("private bottom");
    for _ in 0..100 {
        deep = json!({"child": deep});
    }
    assert!(
        !serde_json::to_string(&logger.payload("info", "bounded", "bounded", deep))
            .unwrap()
            .contains("private bottom")
    );
}

async fn server(statuses: Vec<u16>) -> (String, tokio::task::JoinHandle<Vec<(String, Value)>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/api/logs", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for status in statuses {
            let Ok(accepted) =
                tokio::time::timeout(Duration::from_secs(1), listener.accept()).await
            else {
                break;
            };
            let (mut socket, _) = accepted.unwrap();
            let mut bytes = Vec::new();
            let header_end = loop {
                let mut buffer = [0; 1_024];
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
                assert!(bytes.len() < 64_000);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                        .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            while bytes.len() < header_end + length {
                let mut buffer = [0; 1_024];
                let count = socket.read(&mut buffer).await.unwrap();
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let body = serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
            requests.push((headers, body));
            let redirect = if status == 307 {
                "Location: /redirected\r\n"
            } else {
                ""
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 {status} Test\r\n{redirect}Content-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        }
        requests
    });
    (url, task)
}

fn test_client() -> Client {
    Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

#[tokio::test]
async fn wire_format_matches_galaxy_and_retries_transient_failures_only() {
    let (url, received) = server(vec![503, 204]).await;
    let config = Config::from_key(Some("test-ingestion-key"), "", &url).unwrap();
    let (logger, _) = logger(1);
    let payload = logger.payload(
        "info",
        "app_started",
        "Solar started",
        json!({"ready": true}),
    );
    assert!(deliver(&test_client(), &config, &payload, Duration::ZERO).await);
    let requests = received.await.unwrap();
    assert_eq!(requests.len(), 2);
    for (headers, body) in &requests {
        assert!(headers.starts_with("POST /api/logs HTTP/1.1\r\n"));
        assert!(
            headers
                .to_lowercase()
                .contains("x-api-key: test-ingestion-key\r\n")
        );
        assert!(
            headers
                .to_lowercase()
                .contains("content-type: application/json\r\n")
        );
        assert_eq!(body["service"], "solar");
        assert_eq!(body["context"]["event"], "app_started");
        assert!(
            !serde_json::to_string(body)
                .unwrap()
                .contains("test-ingestion-key")
        );
    }
    assert_eq!(requests[0].1, requests[1].1);
    let (url, received) = server(vec![401]).await;
    let config = Config::from_key(Some("test-ingestion-key"), "", &url).unwrap();
    assert!(!deliver(&test_client(), &config, &payload, Duration::ZERO).await);
    assert_eq!(received.await.unwrap().len(), 1);
}

#[tokio::test]
async fn retries_are_bounded_and_redirects_do_not_forward_credentials() {
    let (url, received) = server(vec![503; MAX_ATTEMPTS]).await;
    let config = Config::from_key(Some("test-ingestion-key"), "", &url).unwrap();
    let (logger, _) = logger(1);
    let payload = logger.payload("warn", "retry_test", "Retry test", json!({}));
    assert!(!deliver(&test_client(), &config, &payload, Duration::ZERO).await);
    assert_eq!(received.await.unwrap().len(), MAX_ATTEMPTS);
    let (url, received) = server(vec![307, 204]).await;
    let config = Config::from_key(Some("test-ingestion-key"), "", &url).unwrap();
    assert!(!deliver(&test_client(), &config, &payload, Duration::ZERO).await);
    assert_eq!(received.await.unwrap().len(), 1);
}

#[tokio::test]
async fn queue_is_bounded_and_flush_acknowledges_earlier_records() {
    let (url, received) = server(vec![204]).await;
    let config = Config::from_key(Some("test-ingestion-key"), "", &url).unwrap();
    let (logger, receiver) = logger(1);
    assert!(
        logger
            .sender
            .try_send(Command::Record(logger.payload(
                "info",
                "queued",
                "Queued",
                json!({}),
            )))
            .is_ok()
    );
    assert!(
        logger
            .sender
            .try_send(Command::Record(logger.payload(
                "info",
                "overflow",
                "Overflow",
                json!({})
            )))
            .is_err()
    );
    let running = tokio::spawn(async move {
        let mut receiver = receiver;
        worker_with_client(config, &mut receiver, test_client()).await;
    });
    let (send, acknowledged) = oneshot::channel();
    assert!(logger.sender.send(Command::Flush(send)).await.is_ok());
    tokio::time::timeout(Duration::from_secs(3), acknowledged)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(received.await.unwrap()[0].1["context"]["event"], "queued");
    drop(logger);
    running.await.unwrap();
}
