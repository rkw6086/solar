use crate::shell::ProcessGroup;
use crate::{
    ToolContext, ToolDefinition, ToolImage, ToolOutput, http_url, managed_environment,
    managed_subdirectory,
};
use base64::Engine;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

const MODERN_PROTOCOL: &str = "2026-07-28";
const LEGACY_PROTOCOL: &str = "2025-11-25";
const MAX_MESSAGE: usize = 2_097_152;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpConfig {
    pub id: String,
    pub name: String,
    pub transport: McpTransport,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum McpTransport {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

impl std::fmt::Debug for McpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpConfig")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("transport", &"<configuration redacted>")
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInfo {
    pub id: String,
    pub name: String,
    pub connected: bool,
    pub tools: usize,
}

pub struct McpManager {
    http: reqwest::Client,
    directory: PathBuf,
    clients: Mutex<HashMap<String, Arc<Mutex<McpClient>>>>,
}

impl McpManager {
    pub async fn shutdown(&self) {
        let clients = self.client_list().await;
        self.clients.lock().await.clear();
        for client in clients {
            client.lock().await.close().await;
        }
    }

    pub fn new(http: reqwest::Client, directory: PathBuf) -> Self {
        Self {
            http,
            directory,
            clients: Mutex::new(HashMap::new()),
        }
    }

    pub async fn connect(
        &self,
        config: McpConfig,
        allow_host_execution: bool,
    ) -> Result<Vec<ToolDefinition>, String> {
        if config.id.is_empty()
            || config.id.len() > 48
            || !config
                .id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err("MCP id must contain 1–48 letters, digits, underscores or hyphens".into());
        }
        if self
            .clients
            .lock()
            .await
            .keys()
            .any(|id| id != &config.id && id.replace('-', "_") == config.id.replace('-', "_"))
        {
            return Err("This MCP id collides with an existing server's tool namespace".into());
        }
        let mut client = McpClient::connect(
            config.clone(),
            self.http.clone(),
            allow_host_execution,
            &self.directory,
        )
        .await?;
        client.load_tools().await?;
        let definitions = client.definitions();
        self.clients
            .lock()
            .await
            .insert(config.id, Arc::new(Mutex::new(client)));
        Ok(definitions)
    }

    pub async fn disconnect(&self, id: &str) -> Result<(), String> {
        let client = self.clients.lock().await.remove(id);
        if let Some(client) = client {
            client.lock().await.close().await;
        }
        Ok(())
    }

    async fn client_list(&self) -> Vec<Arc<Mutex<McpClient>>> {
        self.clients.lock().await.values().cloned().collect()
    }

    pub async fn definitions(&self) -> Vec<ToolDefinition> {
        let mut result = Vec::new();
        for client in self.client_list().await {
            result.extend(client.lock().await.definitions());
        }
        result
    }

    pub async fn servers(&self) -> Vec<McpServerInfo> {
        let mut result = Vec::new();
        for client in self.client_list().await {
            let client = client.lock().await;
            result.push(McpServerInfo {
                id: client.config.id.clone(),
                name: client.config.name.clone(),
                connected: client.connected,
                tools: client.tools.len(),
            });
        }
        result.sort_by(|a, b| a.name.cmp(&b.name));
        result
    }

    pub async fn call(
        &self,
        alias: &str,
        args: Value,
        context: &ToolContext,
    ) -> Result<ToolOutput, String> {
        for client in self.client_list().await {
            let mut client = tokio::select! {
                _ = context.cancellation.cancelled() => return Err("MCP tool cancelled".into()),
                client = client.lock() => client,
            };
            let Some(tool) = client.tools.get(alias).cloned() else {
                continue;
            };
            if !client.connected {
                return Err("MCP server disconnected; reconnect it in settings".into());
            }
            if client.is_stdio() && !context.allow_host_execution {
                return Err(
                    "Local MCP executables require an explicit host-execution grant".into(),
                );
            }
            let call = client.rpc(
                "tools/call",
                json!({"name":tool.name,"arguments":args}),
                Some(&tool.schema),
            );
            let result = tokio::select! {
                result = call => result,
                _ = context.cancellation.cancelled() => {
                    // Closing a stdio transport kills its process group. Dropping the
                    // HTTP response stream signals request-scoped cancellation.
                    client.close().await;
                    return Err("MCP tool cancelled; reconnect local server before reuse".into());
                }
            }
            .map_err(|error| client.redact_credentials(error))?;
            let mut result =
                parse_tool_result(result).map_err(|error| client.redact_credentials(error))?;
            result.content = client.redact_credentials(result.content);
            return Ok(result);
        }
        Err("MCP tool is no longer available; reconnect its server".into())
    }
}

#[derive(Clone)]
struct RemoteTool {
    name: String,
    description: String,
    schema: Value,
}

struct McpClient {
    config: McpConfig,
    transport: Transport,
    protocol: String,
    modern: bool,
    next_id: u64,
    tools: BTreeMap<String, RemoteTool>,
    connected: bool,
}

enum Transport {
    Stdio {
        child: tokio::process::Child,
        input: tokio::process::ChildStdin,
        output: BufReader<tokio::process::ChildStdout>,
        line: Vec<u8>,
        _group: ProcessGroup,
    },
    Http {
        client: reqwest::Client,
        url: url::Url,
        headers: reqwest::header::HeaderMap,
        session: Option<String>,
    },
}

impl McpClient {
    fn redact_credentials(&self, mut message: String) -> String {
        let secrets = match &self.config.transport {
            McpTransport::Stdio { env, .. } => env,
            McpTransport::Http { headers, .. } => headers,
        };
        for value in secrets.values().filter(|value| value.len() >= 6) {
            message = message.replace(value, "<redacted>");
        }
        message
    }

    async fn connect(
        config: McpConfig,
        http: reqwest::Client,
        allow_host: bool,
        directory: &Path,
    ) -> Result<Self, String> {
        let transport = match &config.transport {
            McpTransport::Stdio { command, args, env } => {
                if !allow_host {
                    return Err(
                        "Starting a local MCP executable requires explicit host-execution approval"
                            .into(),
                    );
                }
                let home = managed_subdirectory(directory, &config.id)?;
                let environment = managed_environment(&home)?;
                let mut command_builder = tokio::process::Command::new(command);
                command_builder
                    .args(args)
                    .current_dir(&home)
                    .env_clear()
                    .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                    .envs(env)
                    // Storage roots cannot be redirected by a server's secret environment.
                    .envs(environment)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .kill_on_drop(true);
                #[cfg(unix)]
                command_builder.process_group(0);
                let mut child = command_builder
                    .spawn()
                    .map_err(|e| format!("Could not start MCP process: {e}"))?;
                let group = ProcessGroup(child.id());
                let input = child.stdin.take().ok_or("MCP stdin unavailable")?;
                let output = BufReader::new(child.stdout.take().ok_or("MCP stdout unavailable")?);
                Transport::Stdio {
                    child,
                    input,
                    output,
                    line: Vec::new(),
                    _group: group,
                }
            }
            McpTransport::Http { url, headers } => {
                let url = http_url(url)?;
                let mut parsed = reqwest::header::HeaderMap::new();
                for (name, value) in headers {
                    let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                        .map_err(|_| "Invalid MCP HTTP header name")?;
                    let value = reqwest::header::HeaderValue::from_str(value)
                        .map_err(|_| "Invalid MCP HTTP header value")?;
                    parsed.insert(name, value);
                }
                Transport::Http {
                    client: http,
                    url,
                    headers: parsed,
                    session: None,
                }
            }
        };
        let modern = matches!(transport, Transport::Http { .. });
        let mut client = Self {
            config,
            transport,
            protocol: if modern {
                MODERN_PROTOCOL
            } else {
                LEGACY_PROTOCOL
            }
            .into(),
            modern,
            next_id: 0,
            tools: BTreeMap::new(),
            connected: true,
        };
        if !modern {
            client.initialize().await?;
        }
        Ok(client)
    }

    fn is_stdio(&self) -> bool {
        matches!(self.transport, Transport::Stdio { .. })
    }

    async fn initialize(&mut self) -> Result<(), String> {
        let result = self.rpc("initialize", json!({"protocolVersion":LEGACY_PROTOCOL,"capabilities":{},"clientInfo":{"name":"Solar","version":env!("CARGO_PKG_VERSION")}}), None).await?;
        let protocol = result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(LEGACY_PROTOCOL);
        if !["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"].contains(&protocol) {
            return Err(format!("Unsupported MCP protocol {protocol}"));
        }
        self.protocol = protocol.into();
        self.notify("notifications/initialized", json!({})).await
    }

    async fn load_tools(&mut self) -> Result<(), String> {
        let mut cursor = None;
        let mut aliases = BTreeMap::new();
        for page in 0..100 {
            let params = cursor
                .as_ref()
                .map(|c| json!({"cursor":c}))
                .unwrap_or(json!({}));
            let result = match self.rpc("tools/list", params, None).await {
                Ok(result) => result,
                Err(error)
                    if self.modern
                        && page == 0
                        && (error.contains("400")
                            || error.contains("initialize")
                            || error.contains("protocol")
                            || error.contains("-32600")) =>
                {
                    self.modern = false;
                    self.protocol = LEGACY_PROTOCOL.into();
                    self.initialize().await?;
                    self.rpc("tools/list", json!({}), None).await?
                }
                Err(error) => return Err(error),
            };
            let tools = result
                .get("tools")
                .and_then(Value::as_array)
                .ok_or("MCP tools/list returned no tools array")?;
            for tool in tools {
                let name = tool
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or("MCP tool is missing its name")?;
                let schema = tool
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or(json!({"type":"object"}));
                if self.modern && parameter_headers(&schema, &json!({})).is_err() {
                    continue;
                }
                let alias = format!(
                    "mcp__{}__{}",
                    self.config.id.replace('-', "_"),
                    safe_name(name)
                );
                if aliases.contains_key(&alias) {
                    return Err("MCP tool names collide after normalization".into());
                }
                aliases.insert(
                    alias,
                    RemoteTool {
                        name: name.into(),
                        description: tool
                            .get("description")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .chars()
                            .take(2_000)
                            .collect(),
                        schema,
                    },
                );
                if aliases.len() > 1_000 {
                    return Err("MCP server advertises too many tools".into());
                }
            }
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if cursor.is_none() {
                self.tools = aliases;
                return Ok(());
            }
        }
        Err("MCP tool pagination did not terminate".into())
    }

    fn definitions(&self) -> Vec<ToolDefinition> {
        if !self.connected {
            return Vec::new();
        }
        self.tools
            .iter()
            .map(|(alias, tool)| ToolDefinition {
                name: alias.clone(),
                description: format!("{} / {}. {}", self.config.name, tool.name, tool.description),
                input_schema: tool.schema.clone(),
            })
            .collect()
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let message = json!({"jsonrpc":"2.0","method":method,"params":params});
        match &mut self.transport {
            Transport::Stdio { input, .. } => {
                input
                    .write_all(format!("{message}\n").as_bytes())
                    .await
                    .map_err(|e| e.to_string())?;
                input.flush().await.map_err(|e| e.to_string())
            }
            Transport::Http {
                client,
                url,
                headers,
                session,
            } => {
                let mut request = client
                    .post(url.clone())
                    .headers(headers.clone())
                    .header("Accept", "application/json, text/event-stream")
                    .header("MCP-Protocol-Version", &self.protocol)
                    .json(&message);
                if let Some(session) = session {
                    request = request.header("Mcp-Session-Id", session.as_str());
                }
                let response = request.send().await.map_err(|e| e.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("MCP HTTP {}", response.status()));
                }
                Ok(())
            }
        }
    }

    async fn rpc(
        &mut self,
        method: &str,
        mut params: Value,
        schema: Option<&Value>,
    ) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        if self.modern {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":self.protocol,"io.modelcontextprotocol/clientInfo":{"name":"Solar","version":env!("CARGO_PKG_VERSION")},"io.modelcontextprotocol/clientCapabilities":{}});
        }
        let message = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let request = async {
            match &mut self.transport {
                Transport::Stdio {
                    input,
                    output,
                    line,
                    ..
                } => {
                    input
                        .write_all(format!("{message}\n").as_bytes())
                        .await
                        .map_err(|e| e.to_string())?;
                    input.flush().await.map_err(|e| e.to_string())?;
                    loop {
                        read_bounded_line(output, line).await?;
                        let value: Value =
                            serde_json::from_slice(line).map_err(|_| "MCP emitted invalid JSON")?;
                        line.clear();
                        if value.get("id").and_then(Value::as_u64) == Some(id)
                            && value.get("method").is_none()
                        {
                            return response_result(value);
                        }
                        if let Some(server_id) =
                            value.get("id").filter(|_| value.get("method").is_some())
                        {
                            let reply = json!({"jsonrpc":"2.0","id":server_id,"error":{"code":-32601,"message":"Solar has not enabled this server-requested client capability"}});
                            input
                                .write_all(format!("{reply}\n").as_bytes())
                                .await
                                .map_err(|e| e.to_string())?;
                            input.flush().await.map_err(|e| e.to_string())?;
                        }
                    }
                }
                Transport::Http {
                    client,
                    url,
                    headers,
                    session,
                } => {
                    let mut request = client
                        .post(url.clone())
                        .headers(headers.clone())
                        .header("Accept", "application/json, text/event-stream")
                        .header("MCP-Protocol-Version", &self.protocol)
                        .json(&message);
                    if self.modern {
                        request = request.header("Mcp-Method", method);
                        if let Some(name) = params.get("name").and_then(Value::as_str) {
                            request = request.header("Mcp-Name", encode_header(name));
                        }
                        if let Some(schema) = schema {
                            for (name, value) in parameter_headers(schema, &params["arguments"])? {
                                request = request.header(name, value);
                            }
                        }
                    }
                    if let Some(session) = session.as_ref() {
                        request = request.header("Mcp-Session-Id", session);
                    }
                    let response = request.send().await.map_err(|e| e.to_string())?;
                    let status = response.status();
                    if let Some(value) = response
                        .headers()
                        .get("Mcp-Session-Id")
                        .and_then(|v| v.to_str().ok())
                    {
                        *session = Some(value.into());
                    }
                    if !status.is_success() {
                        let (body, _) = crate::bounded_response(response, 8192).await?;
                        let message = serde_json::from_slice::<Value>(&body)
                            .ok()
                            .and_then(|v| {
                                v.get("error")?.get("message")?.as_str().map(str::to_owned)
                            })
                            .unwrap_or_else(|| "Request rejected".into());
                        return Err(format!("MCP HTTP {status}: {message}"));
                    }
                    read_http_reply(response, id).await
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(60), request)
            .await
            .map_err(|_| {
                "MCP request timed out; inspect the remote state before retrying an action"
                    .to_owned()
            })?
    }

    async fn close(&mut self) {
        self.connected = false;
        match &mut self.transport {
            Transport::Stdio { child, _group, .. } => {
                _group.terminate();
                let _ = child.kill().await;
            }
            Transport::Http {
                client,
                url,
                headers,
                session: Some(session),
            } => {
                let _ = client
                    .delete(url.clone())
                    .headers(headers.clone())
                    .header("Mcp-Session-Id", session.as_str())
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await;
            }
            Transport::Http { .. } => {}
        }
    }
}

async fn read_bounded_line(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    line: &mut Vec<u8>,
) -> Result<(), String> {
    loop {
        let available = reader.fill_buf().await.map_err(|e| e.to_string())?;
        if available.is_empty() {
            return Err("MCP process closed its output".into());
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if line.len().saturating_add(count) > MAX_MESSAGE {
            return Err("MCP response exceeds size limit".into());
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            return Ok(());
        }
    }
}

fn response_result(value: Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        return Err(format!(
            "MCP {}: {}",
            error["code"],
            error["message"].as_str().unwrap_or("request failed")
        ));
    }
    value
        .get("result")
        .cloned()
        .ok_or_else(|| "MCP response has no result".into())
}

async fn read_http_reply(response: reqwest::Response, id: u64) -> Result<Value, String> {
    let event_stream = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.contains("text/event-stream"));
    if !event_stream {
        let (body, truncated) = crate::bounded_response(response, MAX_MESSAGE).await?;
        if truncated {
            return Err("MCP response exceeds size limit".into());
        }
        let value: Value =
            serde_json::from_slice(&body).map_err(|_| "MCP returned invalid JSON")?;
        if value.get("id").and_then(Value::as_u64) != Some(id) {
            return Err("MCP response id does not match request".into());
        }
        return response_result(value);
    }
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::new();
    let mut data = String::new();
    let mut total = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        total += chunk.len();
        if total > MAX_MESSAGE {
            return Err("MCP response exceeds size limit".into());
        }
        buffer.extend_from_slice(&chunk);
        while let Some(index) = buffer.iter().position(|byte| *byte == b'\n') {
            let line = std::str::from_utf8(&buffer[..index])
                .map_err(|_| "MCP SSE contains invalid UTF-8")?
                .trim_end_matches('\r')
                .to_owned();
            buffer.drain(..=index);
            if let Some(value) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
            } else if line.is_empty() && !data.is_empty() {
                let value: Value =
                    serde_json::from_str(&data).map_err(|_| "MCP SSE returned invalid JSON")?;
                data.clear();
                if value.get("id").and_then(Value::as_u64) == Some(id) {
                    return response_result(value);
                }
            }
        }
    }
    Err("MCP response stream closed without a matching result".into())
}

fn parse_tool_result(value: Value) -> Result<ToolOutput, String> {
    if value.get("inputRequests").is_some() {
        return Err("This MCP tool requests client input; configure a tool that does not require sampling or elicitation".into());
    }
    let mut text = Vec::new();
    let mut images = Vec::new();
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        for block in content {
            match block.get("type").and_then(Value::as_str) {
                Some("text") => {
                    if let Some(content) = block.get("text").and_then(Value::as_str) {
                        text.push(content.to_owned());
                    }
                }
                Some("image") => {
                    let encoded = block
                        .get("data")
                        .and_then(Value::as_str)
                        .ok_or("MCP image has no data")?;
                    if encoded.len() > MAX_MESSAGE {
                        return Err("MCP image exceeds size limit".into());
                    }
                    let data = base64::engine::general_purpose::STANDARD
                        .decode(encoded)
                        .map_err(|_| "MCP image is invalid base64")?;
                    images.push(ToolImage {
                        data,
                        mime_type: block
                            .get("mimeType")
                            .and_then(Value::as_str)
                            .unwrap_or("image/png")
                            .into(),
                    });
                }
                Some("resource") => text.push(
                    block
                        .get("resource")
                        .map(Value::to_string)
                        .unwrap_or_default(),
                ),
                _ => {}
            }
        }
    }
    if let Some(structured) = value.get("structuredContent") {
        text.push(structured.to_string());
    }
    let content = text.join("\n").chars().take(64_000).collect::<String>();
    if value
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return Err(content);
    }
    Ok(ToolOutput { content, images })
}

fn safe_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn encode_header(value: &str) -> String {
    if value.trim() != value
        || value.bytes().any(|b| !(b == 9 || (32..=126).contains(&b)))
        || (value.starts_with("=?base64?") && value.ends_with("?="))
    {
        format!(
            "=?base64?{}?=",
            base64::engine::general_purpose::STANDARD.encode(value)
        )
    } else {
        value.into()
    }
}

fn parameter_headers(schema: &Value, args: &Value) -> Result<Vec<(String, String)>, String> {
    fn walk(
        schema: &Value,
        args: &Value,
        headers: &mut BTreeMap<String, String>,
    ) -> Result<(), String> {
        if let Some(name) = schema.get("x-mcp-header") {
            let name = name.as_str().ok_or("Invalid MCP parameter header")?;
            if name.is_empty()
                || !name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
            {
                return Err("Invalid MCP parameter header".into());
            }
            let key = format!("Mcp-Param-{name}");
            if headers.contains_key(&key.to_ascii_lowercase()) {
                return Err("Duplicate MCP parameter header".into());
            }
            if ![Some("string"), Some("integer"), Some("boolean")]
                .contains(&schema.get("type").and_then(Value::as_str))
            {
                return Err("MCP parameter header requires a primitive type".into());
            }
            let value = match args {
                Value::Null => String::new(),
                Value::String(s) => encode_header(s),
                Value::Bool(_) | Value::Number(_) => encode_header(&args.to_string()),
                _ => return Err("MCP header argument has invalid type".into()),
            };
            headers.insert(key.to_ascii_lowercase(), value);
        }
        if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
            for (name, child) in properties {
                walk(child, &args[name], headers)?;
            }
        }
        Ok(())
    }
    let mut headers = BTreeMap::new();
    walk(schema, args, &mut headers)?;
    Ok(headers.into_iter().filter(|(_, v)| !v.is_empty()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_images_and_headers() {
        let output = parse_tool_result(json!({"content":[{"type":"text","text":"done"},{"type":"image","mimeType":"image/png","data":"aGk="}]})).unwrap();
        assert_eq!(output.content, "done");
        assert_eq!(output.images[0].data, b"hi");
        assert!(encode_header("line\nvalue").starts_with("=?base64?"));
        let headers = parameter_headers(
            &json!({"properties":{"region":{"type":"string","x-mcp-header":"Region"}}}),
            &json!({"region":"us-west1"}),
        )
        .unwrap();
        assert_eq!(
            headers,
            vec![("mcp-param-region".into(), "us-west1".into())]
        );
    }

    #[tokio::test]
    async fn stdio_tool_round_trip_requires_host_grant() {
        let storage = tempfile::tempdir().unwrap();
        let manager = McpManager::new(reqwest::Client::new(), storage.path().join("mcp"));
        let script = r#"test "$PWD" = "$HOME" || exit 10
test -d "$XDG_CACHE_HOME" && test -d "$TMPDIR" || exit 11
case "$XDG_CACHE_HOME" in "$HOME"/*) ;; *) exit 12 ;; esac
while IFS= read -r line; do
case "$line" in
*'"method":"initialize"'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}}' ;;
*'"method":"tools/list"'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"fixture","inputSchema":{"type":"object"}}]}}' ;;
*'"method":"tools/call"'*) printf '%s\n' '{"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"fixture result"}]}}' ;;
esac
done"#;
        let config = McpConfig {
            id: "fixture".into(),
            name: "Fixture".into(),
            transport: McpTransport::Stdio {
                command: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: BTreeMap::from([
                    ("HOME".into(), "/unmanaged-home".into()),
                    ("XDG_CACHE_HOME".into(), "/unmanaged-cache".into()),
                ]),
            },
        };
        assert!(manager.connect(config.clone(), false).await.is_err());
        let definitions = manager.connect(config, true).await.unwrap();
        assert_eq!(definitions[0].name, "mcp__fixture__echo");
        let root = tempfile::tempdir().unwrap();
        let context = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![],
            allow_host_execution: true,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        let output = manager
            .call("mcp__fixture__echo", json!({}), &context)
            .await
            .unwrap();
        assert_eq!(output.content, "fixture result");
        manager.disconnect("fixture").await.unwrap();
        assert!(manager.definitions().await.is_empty());
    }

    #[tokio::test]
    async fn modern_http_uses_metadata_and_parameter_headers() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for index in 0..2 {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 4096];
                loop {
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    if let Some(split) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..split]).to_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if bytes.len() < split + 4 + length {
                            continue;
                        }
                        assert!(headers.contains("mcp-protocol-version: 2026-07-28"));
                        let message: Value =
                            serde_json::from_slice(&bytes[split + 4..split + 4 + length]).unwrap();
                        assert_eq!(
                            message["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
                            "2026-07-28"
                        );
                        let result = if index == 0 {
                            json!({"tools":[{"name":"weather","inputSchema":{"type":"object","properties":{"region":{"type":"string","x-mcp-header":"Region"}}}}]})
                        } else {
                            assert!(headers.contains("mcp-param-region: west"));
                            json!({"content":[{"type":"text","text":"sunny"}]})
                        };
                        let body =
                            json!({"jsonrpc":"2.0","id":message["id"],"result":result}).to_string();
                        stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
                        break;
                    }
                }
            }
        });
        let storage = tempfile::tempdir().unwrap();
        let manager = McpManager::new(reqwest::Client::new(), storage.path().join("mcp"));
        manager
            .connect(
                McpConfig {
                    id: "http".into(),
                    name: "HTTP fixture".into(),
                    transport: McpTransport::Http {
                        url: format!("http://{address}/mcp"),
                        headers: BTreeMap::new(),
                    },
                },
                false,
            )
            .await
            .unwrap();
        let root = tempfile::tempdir().unwrap();
        let context = ToolContext {
            orbit_id: "test".into(),
            workspace: root.path().into(),
            data_directory: root.path().into(),
            allowed_roots: vec![],
            allow_host_execution: false,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        };
        assert_eq!(
            manager
                .call("mcp__http__weather", json!({"region":"west"}), &context)
                .await
                .unwrap()
                .content,
            "sunny"
        );
        server.await.unwrap();
    }
}
