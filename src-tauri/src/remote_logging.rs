//! Bounded, best-effort operational diagnostics for the RyServe log service.
//! Credentials and user/model/tool content never belong in these records.

use std::{
    cell::Cell,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use regex::Regex;
use reqwest::{Client, header::HeaderValue};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio::sync::{mpsc, oneshot};

const ENDPOINT: &str = "https://logging.ryserve.net/api/logs";
const QUEUE_CAPACITY: usize = 128;
const MAX_ATTEMPTS: usize = 3;
const MAX_CONTEXT_BYTES: usize = 12_000;
const MAX_PAYLOAD_BYTES: usize = 20_000;
const MAX_INPUT_CHARS: usize = 16_384;
const PANIC_DELIVERY_TIMEOUT: Duration = Duration::from_millis(700);
const PANIC_WAIT_TIMEOUT: Duration = Duration::from_millis(900);
static LOGGER: OnceLock<Option<Logger>> = OnceLock::new();
thread_local! {
    static IN_LOGGER_WORKER: Cell<bool> = const { Cell::new(false) };
    static IN_PANIC_HOOK: Cell<bool> = const { Cell::new(false) };
}

struct Config {
    endpoint: String,
    api_key: HeaderValue,
    sanitizer: Sanitizer,
}

impl Config {
    fn from_key(environment: Option<&str>, endpoint: &str) -> Option<Self> {
        // Diagnostics are opt-in. Published source and binaries carry no key.
        let key = environment?.trim();
        if key.is_empty() || key.len() > 8_192 {
            return None;
        }
        let mut api_key = HeaderValue::from_str(key).ok()?;
        api_key.set_sensitive(true);
        Some(Self {
            endpoint: endpoint.into(),
            api_key,
            sanitizer: Sanitizer::new(key),
        })
    }
}

struct Logger {
    sender: mpsc::Sender<Command>,
    panic_sender: mpsc::Sender<PanicRecord>,
    sanitizer: Sanitizer,
    session_id: String,
    dropped: AtomicU64,
}

enum Command {
    Record(Payload),
    Flush(oneshot::Sender<()>),
}

struct PanicRecord {
    payload: Payload,
    completed: std::sync::mpsc::Sender<bool>,
}

#[derive(Serialize)]
struct Payload {
    level: String,
    message: String,
    service: &'static str,
    context: Value,
}

/// Initialize once before the desktop is created. Nothing connects until a
/// record is submitted, and tests/headless library users never initialize it.
pub(crate) fn init() {
    LOGGER.get_or_init(|| {
        let environment = match std::env::var("RYSERVE_LOGGING_KEY") {
            Ok(value) => Some(value),
            Err(std::env::VarError::NotPresent) => None,
            Err(std::env::VarError::NotUnicode(_)) => return None,
        };
        let config = Config::from_key(environment.as_deref(), ENDPOINT)?;
        let sanitizer = config.sanitizer.clone();
        // Regex initialization must happen during normal startup, not while
        // unwinding a panic or holding the panic-hook recursion guard.
        let _ = sanitizer.text("", 0);
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        let (panic_sender, panic_receiver) = mpsc::channel(1);
        std::thread::Builder::new()
            .name("solar-remote-logging".into())
            .spawn(move || {
                IN_LOGGER_WORKER.set(true);
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    eprintln!("Solar remote logging worker could not start");
                    return;
                };
                runtime.block_on(worker(config, receiver, panic_receiver));
            })
            .ok()?;
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            let message = panic.payload().downcast_ref::<&str>().copied()
                .or_else(|| panic.payload().downcast_ref::<String>().map(String::as_str))
                .unwrap_or("Rust panic with a non-string payload");
            if !IN_LOGGER_WORKER.get() && !IN_PANIC_HOOK.replace(true) {
                if let Some(Some(logger)) = LOGGER.get() {
                    let _ = logger.panic_record(message, json!({
                        "location": panic.location().map(|location| json!({
                            "file": location.file(), "line": location.line(), "column": location.column(),
                        })),
                    }), PANIC_WAIT_TIMEOUT);
                }
                IN_PANIC_HOOK.set(false);
            }
            previous_hook(panic);
        }));
        Some(Logger {
            sender,
            panic_sender,
            sanitizer,
            session_id: uuid::Uuid::new_v4().to_string(),
            dropped: AtomicU64::new(0),
        })
    });
}

/// Submit without waiting for a socket, a retry, or room in the bounded queue.
pub(crate) fn event(level: &str, event: &str, message: &str, context: Value) {
    let Some(Some(logger)) = LOGGER.get() else {
        return;
    };
    let payload = logger.payload(level, event, message, context);
    if logger.sender.try_send(Command::Record(payload)).is_err() {
        let dropped = logger.dropped.fetch_add(1, Ordering::Relaxed) + 1;
        if dropped == 1 || dropped.is_multiple_of(64) {
            eprintln!("Solar remote logging queue unavailable; {dropped} records dropped");
        }
    }
}

/// Drain earlier records within a caller-supplied deadline; shutdown must not
/// depend on the service being reachable. A timeout leaves delivery best effort.
pub(crate) async fn flush(timeout: Duration) {
    let Some(Some(logger)) = LOGGER.get() else {
        return;
    };
    let (send, receive) = oneshot::channel();
    let operation = async {
        if logger.sender.send(Command::Flush(send)).await.is_ok() {
            let _ = receive.await;
        }
    };
    let _ = tokio::time::timeout(timeout, operation).await;
}

impl Logger {
    // This path uses only a nonblocking enqueue and an OS-thread wait. It is
    // safe to call from a native UI thread with no Tokio runtime entered.
    fn panic_record(&self, message: &str, context: Value, timeout: Duration) -> bool {
        if IN_LOGGER_WORKER.get() {
            return false;
        }
        let (completed, receive) = std::sync::mpsc::channel();
        let record = PanicRecord {
            payload: self.payload("error", "rust_panic", message, context),
            completed,
        };
        if self.panic_sender.try_send(record).is_err() {
            return false;
        }
        receive.recv_timeout(timeout).unwrap_or(false)
    }

    fn payload(&self, level: &str, event: &str, message: &str, context: Value) -> Payload {
        let mut budget = MAX_CONTEXT_BYTES;
        let context = self.sanitizer.value(context, 0, &mut budget);
        let mut context = match context {
            Value::Object(map) => map,
            other => Map::from_iter([("details".into(), other)]),
        };
        context.insert("event".into(), json!(self.sanitizer.text(event, 100)));
        context.insert("timestamp".into(), json!(chrono::Utc::now().to_rfc3339()));
        context.insert(
            "app".into(),
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "source_commit": option_env!("SOLAR_SOURCE_COMMIT").unwrap_or("unknown"),
            "source_modified": option_env!("SOLAR_SOURCE_MODIFIED").map(|value| value == "true").unwrap_or(false),
                "platform": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "session_id": self.session_id,
            }),
        );
        Payload {
            level: match level {
                "warn" => "warn",
                "error" => "error",
                _ => "info",
            }
            .into(),
            message: self.sanitizer.text(message, 1_000),
            service: "solar",
            context: Value::Object(context),
        }
    }
}

fn http_client() -> Result<Client, reqwest::Error> {
    Client::builder()
        .connect_timeout(Duration::from_secs(2))
        .timeout(Duration::from_secs(5))
        // A redirect must never carry the ingestion credential elsewhere.
        .redirect(reqwest::redirect::Policy::none())
        .build()
}

async fn worker(
    config: Config,
    mut receiver: mpsc::Receiver<Command>,
    mut panic_receiver: mpsc::Receiver<PanicRecord>,
) {
    let Ok(client) = http_client() else {
        eprintln!("Solar remote logging HTTP client could not start");
        return;
    };
    worker_with_client(config, &mut receiver, &mut panic_receiver, client).await;
}

async fn worker_with_client(
    config: Config,
    receiver: &mut mpsc::Receiver<Command>,
    panic_receiver: &mut mpsc::Receiver<PanicRecord>,
    client: Client,
) {
    let mut last_warning: Option<Instant> = None;
    loop {
        let command = tokio::select! {
            biased;
            Some(panic) = panic_receiver.recv() => {
                deliver_panic(&client, &config, panic).await;
                continue;
            }
            command = receiver.recv() => command,
        };
        let Some(command) = command else { break };
        match command {
            Command::Record(payload) => {
                let delivered = tokio::select! {
                    biased;
                    Some(panic) = panic_receiver.recv() => {
                        // A fatal panic cannot wait behind an ordinary log's
                        // network timeout/retries. Drop that best-effort send.
                        deliver_panic(&client, &config, panic).await;
                        continue;
                    }
                    delivered = deliver(&client, &config, &payload, Duration::from_millis(200)) => delivered,
                };
                if !delivered
                    && last_warning.is_none_or(|last| last.elapsed() >= Duration::from_secs(30))
                {
                    // Never format reqwest errors or response bodies here:
                    // they can contain server-controlled text or credentials.
                    eprintln!("Solar remote logging delivery failed; continuing locally");
                    last_warning = Some(Instant::now());
                }
            }
            Command::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

async fn deliver_panic(client: &Client, config: &Config, record: PanicRecord) {
    // One attempt only. The outer deadline also covers response headers so a
    // server that accepts a socket but stalls cannot hold the panic hook.
    let delivered = tokio::time::timeout(PANIC_DELIVERY_TIMEOUT, async {
        let Ok(body) = serde_json::to_vec(&record.payload) else {
            return false;
        };
        if body.len() > MAX_PAYLOAD_BYTES {
            return false;
        }
        client
            .post(&config.endpoint)
            .header("x-api-key", config.api_key.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body)
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    })
    .await
    .unwrap_or(false);
    let _ = record.completed.send(delivered);
}

async fn deliver(client: &Client, config: &Config, payload: &Payload, backoff: Duration) -> bool {
    let Ok(body) = serde_json::to_vec(payload) else {
        return false;
    };
    if body.len() > MAX_PAYLOAD_BYTES {
        return false;
    }
    for attempt in 0..MAX_ATTEMPTS {
        let retry = match client
            .post(&config.endpoint)
            .header("x-api-key", config.api_key.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.clone())
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return true,
            Ok(response) => {
                let status = response.status();
                status.is_server_error() || matches!(status.as_u16(), 408 | 425 | 429)
            }
            Err(_) => true,
        };
        if !retry || attempt + 1 == MAX_ATTEMPTS {
            return false;
        }
        tokio::time::sleep(backoff * (attempt as u32 + 1)).await;
    }
    false
}

#[derive(Clone)]
struct Sanitizer {
    key: String,
}

impl Sanitizer {
    fn new(key: &str) -> Self {
        Self { key: key.into() }
    }

    fn text(&self, value: &str, max: usize) -> String {
        use std::sync::LazyLock;
        static URL: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)https?://[^\s\"'<>]+"#).unwrap());
        static CREDENTIAL: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r#"(?i)[\"'\\]*\b(?:authorization|proxy-authorization|x-api-key|api[_-]?key|access[_-]?token|refresh[_-]?token|id[_-]?token|password|secret|headers|cookies|messages|prompt|content|input|output|arguments|args|result|memory|instructions|body|payload)\b[\"'\\\s]*[:=]\s*"#
        ).unwrap()
        });
        static BEARER: LazyLock<Regex> =
            LazyLock::new(|| Regex::new(r#"(?i)\bbearer\s+[^\s,\"';}\]]+"#).unwrap());
        static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
            Regex::new(
            r#"\b(?:sk[-_]|log_sk_|gh[pousr]_|github_pat_|xox[baprs]-)[a-zA-Z0-9_-]+|\beyJ[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+\.[a-zA-Z0-9_-]+"#
        ).unwrap()
        });
        let mut prefix: String = value.chars().take(MAX_INPUT_CHARS).collect();
        if value.len() > prefix.len() {
            // A truncated token is still a credential. Drop that trailing token.
            prefix.truncate(prefix.rfind(char::is_whitespace).unwrap_or(0));
        }
        if !self.key.is_empty() {
            prefix = prefix.replace(&self.key, "[redacted]");
        }
        let urls = URL.replace_all(&prefix, "[redacted URL]");
        let mut assigned = String::new();
        let mut cursor = 0;
        for found in CREDENTIAL.find_iter(&urls) {
            if found.start() < cursor {
                continue;
            }
            assigned.push_str(&urls[cursor..found.end()]);
            assigned.push_str("[redacted]");
            cursor = found.end() + credential_end(&urls[found.end()..]);
        }
        assigned.push_str(&urls[cursor..]);
        let bearers = BEARER.replace_all(&assigned, "Bearer [redacted]");
        let tokens = TOKEN.replace_all(&bearers, "[redacted]");
        let compact = tokens.split_whitespace().collect::<Vec<_>>().join(" ");
        bounded(&compact, max)
    }

    fn value(&self, value: Value, depth: usize, budget: &mut usize) -> Value {
        if depth > 5 || *budget == 0 {
            return json!("[truncated]");
        }
        match value {
            Value::String(text) => {
                let safe = self.text(&text, 1_000.min(*budget / 4));
                *budget = budget.saturating_sub(safe.len());
                Value::String(safe)
            }
            Value::Object(values) => {
                let mut result = Map::new();
                for (index, (key, value)) in values.into_iter().enumerate() {
                    if index >= 32 || *budget < 64 {
                        result.insert("_truncated".into(), Value::Bool(true));
                        break;
                    }
                    let key = self.text(&key, 64);
                    *budget = budget.saturating_sub(key.len() + 16);
                    let value = if sensitive(&key) {
                        json!("[redacted]")
                    } else {
                        self.value(value, depth + 1, budget)
                    };
                    result.insert(key, value);
                }
                Value::Object(result)
            }
            Value::Array(values) => {
                let mut result = Vec::new();
                for (index, value) in values.into_iter().enumerate() {
                    if index >= 16 || *budget < 16 {
                        result.push(json!("[truncated]"));
                        break;
                    }
                    *budget = budget.saturating_sub(16);
                    result.push(self.value(value, depth + 1, budget));
                }
                Value::Array(result)
            }
            other => {
                *budget = budget.saturating_sub(32);
                other
            }
        }
    }
}

fn bounded(value: &str, max: usize) -> String {
    match value.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &value[..end]),
        None => value.into(),
    }
}

fn sensitive(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect();
    matches!(
        normalized.as_str(),
        "authorization"
            | "proxyauthorization"
            | "xapikey"
            | "apikey"
            | "key"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "idtoken"
            | "secret"
            | "password"
            | "credentials"
            | "credential"
            | "headers"
            | "cookie"
            | "cookies"
            | "content"
            | "messages"
            | "prompt"
            | "prompts"
            | "reply"
            | "response"
            | "input"
            | "output"
            | "arguments"
            | "args"
            | "result"
            | "results"
            | "toolarguments"
            | "toolresult"
            | "toolresults"
            | "memory"
            | "memories"
            | "body"
            | "payload"
            | "rawpayload"
            | "systemprompt"
            | "instructions"
    ) || normalized.contains("secret")
        || normalized.ends_with("accesskey")
        || normalized.ends_with("accesskeyid")
        || normalized.ends_with("loggingkey")
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("apikey")
        || normalized.ends_with("token")
}

// A quoted credential can contain whitespace and escaped delimiters. An
// unquoted value is redacted through the line boundary rather than guessed at.
fn credential_end(value: &str) -> usize {
    let escapes = value.bytes().take_while(|byte| *byte == b'\\').count();
    if let Some(quote @ (b'"' | b'\'')) = value.as_bytes().get(escapes).copied() {
        let span = escapes + 1;
        if !span.is_power_of_two() {
            return value.len();
        }
        let mut slashes = 0;
        let start = escapes + 1;
        for (index, character) in value[start..].char_indices() {
            if character == '\\' {
                slashes += 1;
                continue;
            }
            if character as u32 == u32::from(quote) && slashes % (2 * span) == escapes {
                return start + index + 1;
            }
            slashes = 0;
        }
        return value.len();
    }
    if value.starts_with('{') || value.starts_with('[') {
        let mut values = serde_json::Deserializer::from_str(value).into_iter::<Value>();
        return if values.next().is_some_and(|result| result.is_ok()) {
            values.byte_offset()
        } else {
            value.len()
        };
    }
    value.find(['\r', '\n']).unwrap_or(value.len())
}

#[cfg(test)]
#[path = "remote_logging_tests.rs"]
mod tests;
