//! Local capabilities owned by Solar's permission broker.
//!
//! Callers authorize every operation before dispatch. File operations use
//! directory capabilities. Arbitrary shell and MCP executables require a
//! separate host-execution grant and do not inherit a filesystem sandbox.

mod browser;
mod files;
mod mcp;
mod native;
mod shell;

pub use mcp::{McpConfig, McpServerInfo, McpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::PathBuf;
use tokio::sync::Mutex;
pub use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Clone, Debug)]
pub struct ToolContext {
    pub orbit_id: String,
    pub workspace: PathBuf,
    pub allowed_roots: Vec<PathBuf>,
    pub allow_host_execution: bool,
    pub screen_share_allowed: bool,
    pub cancellation: CancellationToken,
}

#[derive(Clone, Debug, Default)]
pub struct ToolOutput {
    pub content: String,
    pub images: Vec<ToolImage>,
}

#[derive(Clone, Debug)]
pub struct ToolImage {
    pub data: Vec<u8>,
    pub mime_type: String,
}

impl ToolOutput {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn json(value: Value) -> Self {
        Self::text(value.to_string())
    }
}

pub struct ToolBroker {
    http: reqwest::Client,
    browser: Mutex<browser::BrowserManager>,
    mcp: mcp::McpManager,
    native: Mutex<native::NativeBridge>,
}

impl ToolBroker {
    pub fn new(data_dir: PathBuf) -> Result<Self, String> {
        std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(45))
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            browser: Mutex::new(browser::BrowserManager::new(data_dir.join("browsers"))),
            mcp: mcp::McpManager::new(http.clone()),
            native: Mutex::new(native::NativeBridge::new()),
            http,
        })
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        let mut result = Vec::new();
        result.extend(files::definitions());
        result.extend(shell::definitions());
        result.extend(browser::definitions());
        result.extend(native::definitions());
        result.push(definition(
            "web_fetch",
            "Fetch an HTTP or HTTPS URL. Returns bounded text and the final source URL.",
            json!({"url":{"type":"string"}}),
            &["url"],
        ));
        result
    }

    pub async fn all_definitions(&self) -> Vec<ToolDefinition> {
        let mut result = self.definitions();
        result.extend(self.mcp.definitions().await);
        result
    }

    pub async fn execute(
        &self,
        name: &str,
        args: Value,
        context: &ToolContext,
    ) -> Result<ToolOutput, String> {
        if context.cancellation.is_cancelled() {
            return Err("Task cancelled".into());
        }
        // MCP owns cancellation so it can close the transport before returning.
        if name.starts_with("mcp__") {
            return self.mcp.call(name, args, context).await;
        }
        if matches!(
            name,
            "screen_capture"
                | "computer_inspect"
                | "computer_click"
                | "computer_type"
                | "computer_key"
                | "native_permissions"
        ) {
            let mut native = tokio::select! {
                _ = context.cancellation.cancelled() => return Err("Task cancelled".into()),
                native = self.native.lock() => native,
            };
            return native.execute(name, &args, context).await;
        }
        let operation = async {
            match name {
                "files_list" | "files_read" | "files_write" | "files_search" => {
                    files::execute(name, &args, context).await
                }
                "shell_run" => shell::execute(&args, context).await,
                "web_fetch" => self.fetch(&args).await,
                "browser_open" | "browser_search" | "browser_inspect" | "browser_click"
                | "browser_type" | "browser_screenshot" => {
                    self.browser
                        .lock()
                        .await
                        .execute(name, &args, context)
                        .await
                }
                _ => Err(format!("Unknown tool {name}")),
            }
        };
        tokio::select! {
            biased;
            _ = context.cancellation.cancelled() => Err("Task cancelled".into()),
            result = operation => result,
        }
    }

    async fn fetch(&self, args: &Value) -> Result<ToolOutput, String> {
        let url = http_url(required_str(args, "url")?)?;
        let response = self.http.get(url).send().await.map_err(|e| e.to_string())?;
        let status = response.status();
        let source = response.url().to_string();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let (body, truncated) = bounded_response(response, 1_048_576).await?;
        if !status.is_success() {
            return Err(format!("HTTP {status} from {source}"));
        }
        Ok(ToolOutput::json(
            json!({"url":source,"contentType":content_type,"text":String::from_utf8_lossy(&body),"truncated":truncated}),
        ))
    }

    pub async fn connect_mcp(
        &self,
        config: McpConfig,
        allow_host_execution: bool,
    ) -> Result<Vec<ToolDefinition>, String> {
        self.mcp.connect(config, allow_host_execution).await
    }
    pub async fn disconnect_mcp(&self, id: &str) -> Result<(), String> {
        self.mcp.disconnect(id).await
    }
    pub async fn mcp_servers(&self) -> Vec<McpServerInfo> {
        self.mcp.servers().await
    }
    pub async fn start_screen_share(&self, orbit_id: &str) -> Result<ToolOutput, String> {
        self.native.lock().await.start_share(orbit_id).await
    }
    pub async fn stop_screen_share(&self, orbit_id: &str) -> Result<(), String> {
        self.native.lock().await.stop_share(orbit_id).await
    }

    pub async fn shutdown(&self) {
        self.mcp.shutdown().await;
        self.browser.lock().await.shutdown();
        self.native.lock().await.shutdown().await;
    }
}

pub(crate) fn definition(
    name: &str,
    description: &str,
    properties: Value,
    required: &[&str],
) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        input_schema: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
    }
}

pub(crate) fn required_str<'a>(args: &'a Value, field: &str) -> Result<&'a str, String> {
    args.get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("{field} must be a nonempty string"))
}

pub(crate) fn http_url(value: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(value).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("A valid HTTP(S) URL without embedded credentials is required".into());
    }
    Ok(url)
}

pub(crate) async fn bounded_response(
    response: reqwest::Response,
    limit: usize,
) -> Result<(Vec<u8>, bool), String> {
    use futures::StreamExt;
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        let remaining = limit.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        if chunk.len() > remaining {
            return Ok((body, true));
        }
    }
    Ok((body, false))
}
