//! Keep useful transport causes before Rig reduces SSE errors to their Display text.
use std::error::Error;
use std::fmt;

use bytes::Bytes;
use futures::StreamExt;
use rig_core::http_client::{
    self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
};
use rig_core::wasm_compat::WasmCompatSend;

#[derive(Clone, Debug, Default)]
pub(crate) struct DiagnosticHttpClient<H = http_client::ReqwestClient> {
    inner: H,
    plain_system_content: bool,
}

impl DiagnosticHttpClient {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

impl<H> DiagnosticHttpClient<H> {
    pub(crate) fn with_plain_system_content(mut self) -> Self {
        self.plain_system_content = true;
        self
    }

    fn prepare<T: Into<Bytes>>(&self, request: Request<T>) -> Request<Bytes> {
        let mut request = request.map(Into::into);
        if !self.plain_system_content {
            return request;
        }
        let Ok(mut body) = serde_json::from_slice::<serde_json::Value>(request.body()) else {
            return request;
        };
        if normalize_system_content(&mut body)
            && let Ok(body) = serde_json::to_vec(&body)
        {
            *request.body_mut() = body.into();
            request.headers_mut().remove(http::header::CONTENT_LENGTH);
        }
        request
    }
}

// LiteLLM's ChatGPT chat-to-Responses bridge extracts only string-valued system
// messages into `instructions`. Rig emits text-part arrays, which otherwise
// reach ChatGPT as unsupported system input. Keep the role and text intact.
fn normalize_system_content(body: &mut serde_json::Value) -> bool {
    let Some(messages) = body
        .get_mut("messages")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };
    let mut changed = false;
    for message in messages {
        if message.get("role").and_then(serde_json::Value::as_str) != Some("system") {
            continue;
        }
        let Some(parts) = message.get("content").and_then(serde_json::Value::as_array) else {
            continue;
        };
        let Some(text) = parts
            .iter()
            .map(|part| {
                (part.get("type").and_then(serde_json::Value::as_str) == Some("text"))
                    .then(|| part.get("text").and_then(serde_json::Value::as_str))
                    .flatten()
            })
            .collect::<Option<Vec<_>>>()
        else {
            // Leave future non-text system content untouched; never drop data.
            continue;
        };
        message["content"] = text.join("").into();
        changed = true;
    }
    changed
}

impl<H: HttpClientExt + Clone + 'static> HttpClientExt for DiagnosticHttpClient<H> {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let client = self.inner.clone();
        let request = self.prepare(request);
        async move {
            let response = client
                .send(request)
                .await
                .map_err(diagnose_transport_error)?;
            Ok(response.map(|body| {
                Box::pin(async move { body.await.map_err(diagnose_transport_error) }) as LazyBody<U>
            }))
        }
    }

    fn send_multipart<U>(
        &self,
        request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let client = self.inner.clone();
        async move {
            let response = client
                .send_multipart(request)
                .await
                .map_err(diagnose_transport_error)?;
            Ok(response.map(|body| {
                Box::pin(async move { body.await.map_err(diagnose_transport_error) }) as LazyBody<U>
            }))
        }
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let client = self.inner.clone();
        let request = self.prepare(request);
        async move {
            let response = client
                .send_streaming(request)
                .await
                .map_err(diagnose_transport_error)?;
            Ok(response.map(|body| {
                Box::pin(body.map(|chunk| chunk.map_err(diagnose_transport_error)))
                    as http_client::sse::BoxedStream
            }))
        }
    }
}

// Only static categories and numeric OS codes leave this module. Transport errors
// can contain proxy passwords, URL userinfo/query keys, or arbitrary source text;
// neither Display nor Debug of the original error is safe to forward verbatim.
// HTTP status/body errors pass through unchanged so Rig's provider classification
// and its body-inspection helpers retain their original semantics.
pub(crate) fn diagnose_transport_error(error: http_client::Error) -> http_client::Error {
    match error {
        http_client::Error::Instance(source) => http_client::Error::Instance(Box::new(
            TransportDiagnostic::from_source(source.as_ref()),
        )),
        other => other,
    }
}

#[derive(Debug)]
struct TransportDiagnostic(String);

impl TransportDiagnostic {
    fn from_source(source: &(dyn Error + 'static)) -> Self {
        const MAX_DEPTH: usize = 12;
        const MAX_CAUSES: usize = 4;
        let mut causes = Vec::<String>::new();
        let mut source = Some(source);
        for _ in 0..MAX_DEPTH {
            let Some(error) = source else { break };
            let text = bounded_error_text(error)
                .split_whitespace()
                .filter(|word| !word.contains("://"))
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase();
            if let Some(category) = transport_category(&text) {
                let category = category.to_string();
                if !causes.contains(&category) && causes.len() < MAX_CAUSES {
                    causes.push(category);
                }
            }
            if let Some(error) = error.downcast_ref::<std::io::Error>() {
                let detail = match error.raw_os_error() {
                    Some(code) => format!("I/O {:?} (OS error {code})", error.kind()),
                    None => format!("I/O {:?}", error.kind()),
                };
                if !causes.contains(&detail) && causes.len() < MAX_CAUSES {
                    causes.push(detail);
                }
            }
            source = error.source();
        }
        if causes.is_empty() {
            causes.push("underlying transport cause unavailable".to_string());
        }
        Self(format!(
            "provider request transport failed: {}",
            causes.join("; caused by: ")
        ))
    }
}

impl fmt::Display for TransportDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for TransportDiagnostic {}

fn bounded_error_text(error: &(dyn Error + 'static)) -> String {
    struct BoundedText(String);
    impl fmt::Write for BoundedText {
        fn write_str(&mut self, value: &str) -> fmt::Result {
            let remaining = 1024usize.saturating_sub(self.0.chars().count());
            match value.char_indices().nth(remaining) {
                Some((boundary, _)) => {
                    self.0.push_str(&value[..boundary]);
                    Err(fmt::Error)
                }
                None => {
                    self.0.push_str(value);
                    Ok(())
                }
            }
        }
    }
    let mut text = BoundedText(String::new());
    let _ = fmt::write(&mut text, format_args!("{error}"));
    text.0
}

fn transport_category(text: &str) -> Option<&'static str> {
    if text.contains("unknownissuer")
        || text.contains("unknown issuer")
        || text.contains("unknown ca")
    {
        Some("TLS certificate issuer is not trusted")
    } else if text.contains("notvalidforname")
        || text.contains("certificate")
            && (text.contains("hostname") || text.contains("not valid for name"))
    {
        Some("TLS certificate hostname mismatch")
    } else if text.contains("certificate")
        && (text.contains("expired")
            || text.contains("notvalidyet")
            || text.contains("not valid yet"))
    {
        Some("TLS certificate is expired or not yet valid")
    } else if text.contains("certificate")
        && (text.contains("invalid") || text.contains("verify") || text.contains("validation"))
        || text.contains("cert verify")
    {
        Some("TLS certificate validation failed")
    } else if text.contains("dns")
        || text.contains("failed to lookup address")
        || text.contains("name or service not known")
        || text.contains("nodename nor servname")
    {
        Some("DNS resolution failed")
    } else if text.contains("proxy") && (text.contains("authentication") || text.contains("407")) {
        Some("proxy authentication failed")
    } else if text.contains("connection refused") {
        Some("connection refused")
    } else if text.contains("connection reset") {
        Some("connection reset")
    } else if text.contains("timed out") || text.contains("timeout") {
        Some("network operation timed out")
    } else if text.contains("unreachable") {
        Some("network or destination unreachable")
    } else if text.contains("tls") || text.contains("ssl") || text.contains("handshake") {
        Some("TLS handshake failed")
    } else if text.contains("unexpected eof")
        || text.contains("connection closed")
        || text.contains("incomplete message")
    {
        Some("connection closed before response completed")
    } else {
        None
    }
}

#[cfg(test)]
#[path = "http_client_tests.rs"]
mod tests;
