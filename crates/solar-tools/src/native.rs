use crate::{ToolContext, ToolDefinition, ToolOutput, definition};
use serde_json::{Value, json};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        definition(
            "native_permissions",
            "Read macOS screen-sharing and Accessibility permission status.",
            json!({}),
            &[],
        ),
        definition(
            "screen_capture",
            "Capture the screen/window the user is currently sharing with this Orbit. The user must start sharing explicitly.",
            json!({}),
            &[],
        ),
        definition(
            "computer_inspect",
            "Read the accessibility hierarchy of the foreground macOS application during this Orbit's active shared-screen session.",
            json!({}),
            &[],
        ),
        definition(
            "computer_click",
            "Click a global desktop coordinate during this Orbit's user-started shared-screen session.",
            json!({"x":{"type":"number"},"y":{"type":"number"},"button":{"type":"string","enum":["left","right"]}}),
            &["x", "y"],
        ),
        definition(
            "computer_type",
            "Type text into the focused macOS control during this Orbit's user-started screen session.",
            json!({"text":{"type":"string"}}),
            &["text"],
        ),
        definition(
            "computer_key",
            "Press a keyboard shortcut during this Orbit's shared-screen session. Supported keys include Return, Tab, Escape, arrows, and letter shortcuts such as CMD+A.",
            json!({"key":{"type":"string"}}),
            &["key"],
        ),
    ]
}

#[cfg(target_os = "macos")]
pub struct NativeBridge {
    child: Option<tokio::process::Child>,
    input: Option<tokio::process::ChildStdin>,
    output: Option<tokio::io::BufReader<tokio::process::ChildStdout>>,
    owner: Option<String>,
    next_id: u64,
}

#[cfg(not(target_os = "macos"))]
pub struct NativeBridge;

impl NativeBridge {
    #[cfg(not(target_os = "macos"))]
    pub async fn shutdown(&mut self) {}

    #[cfg(target_os = "macos")]
    pub async fn shutdown(&mut self) {
        self.owner = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill().await;
        }
        self.input = None;
        self.output = None;
    }

    #[cfg(target_os = "macos")]
    pub fn new() -> Self {
        Self {
            child: None,
            input: None,
            output: None,
            owner: None,
            next_id: 0,
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn new() -> Self {
        Self
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn start_share(&mut self, _: &str) -> Result<ToolOutput, String> {
        Err("Screen sharing and native computer control require macOS 14 or newer".into())
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn stop_share(&mut self, _: &str) -> Result<(), String> {
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    pub async fn execute(
        &mut self,
        name: &str,
        _: &Value,
        _: &ToolContext,
    ) -> Result<ToolOutput, String> {
        if name == "native_permissions" {
            return Ok(ToolOutput::json(
                json!({"platform":std::env::consts::OS,"supported":false,"screenSharing":false,"accessibility":false}),
            ));
        }
        Err("Native screen and computer tools require macOS 14 or newer".into())
    }

    #[cfg(target_os = "macos")]
    pub async fn start_share(&mut self, orbit_id: &str) -> Result<ToolOutput, String> {
        if self.owner.as_deref().is_some_and(|owner| owner != orbit_id) {
            return Err("Another Orbit owns the active screen-sharing session. Stop it before sharing with this Orbit.".into());
        }
        let result = self.request("share_start", json!({})).await?;
        self.owner = Some(orbit_id.into());
        Ok(ToolOutput::json(result))
    }

    #[cfg(target_os = "macos")]
    pub async fn stop_share(&mut self, orbit_id: &str) -> Result<(), String> {
        if self.owner.as_deref().is_some_and(|owner| owner != orbit_id) {
            return Err("Screen sharing belongs to another Orbit".into());
        }
        self.owner = None;
        if self.child.is_some() {
            self.request("share_stop", json!({})).await?;
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    pub async fn execute(
        &mut self,
        name: &str,
        args: &Value,
        context: &ToolContext,
    ) -> Result<ToolOutput, String> {
        if name != "native_permissions"
            && (!context.screen_share_allowed || self.owner.as_deref() != Some(&context.orbit_id))
        {
            return Err("The user must explicitly share a screen with this Orbit before native observation or control".into());
        }
        let action = match name {
            "native_permissions" => "permissions",
            "screen_capture" => "capture",
            "computer_inspect" => "inspect",
            "computer_click" => "click",
            "computer_type" => "type",
            "computer_key" => "key",
            _ => return Err("Unknown native tool".into()),
        };
        let request = self.request(action, args.clone());
        let response = tokio::select! {
            result = request => result,
            _ = context.cancellation.cancelled() => {
                self.shutdown().await;
                return Err("Native operation cancelled; screen sharing stopped".into());
            }
        };
        let mut result = response?;
        if let Some(encoded) = result.get("image").and_then(Value::as_str) {
            use base64::Engine;
            let data = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(|_| "Invalid native image")?;
            result.as_object_mut().map(|object| object.remove("image"));
            return Ok(ToolOutput {
                content: result.to_string(),
                images: vec![crate::ToolImage {
                    data,
                    mime_type: "image/png".into(),
                }],
            });
        }
        Ok(ToolOutput::json(result))
    }

    #[cfg(target_os = "macos")]
    async fn request(&mut self, action: &str, args: Value) -> Result<Value, String> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        if self
            .child
            .as_mut()
            .is_some_and(|child| child.try_wait().ok().flatten().is_some())
        {
            self.child = None;
            self.input = None;
            self.output = None;
            self.owner = None;
        }
        if self.child.is_none() {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            let bundled = executable
                .parent()
                .ok_or("Application executable has no directory")?
                .join("solar-native-helper");
            let path = if bundled.is_file() {
                bundled
            } else {
                std::path::PathBuf::from(env!("SOLAR_NATIVE_HELPER_BUILT"))
            };
            let mut child = tokio::process::Command::new(path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| format!("Native helper unavailable: {e}"))?;
            self.input = child.stdin.take();
            self.output = child.stdout.take().map(BufReader::new);
            self.child = Some(child);
        }
        self.next_id += 1;
        let id = self.next_id;
        let input = self
            .input
            .as_mut()
            .ok_or("Native helper input unavailable")?;
        input
            .write_all(
                format!("{}\n", json!({"id":id,"action":action,"arguments":args})).as_bytes(),
            )
            .await
            .map_err(|e| e.to_string())?;
        input.flush().await.map_err(|e| e.to_string())?;
        let output = self
            .output
            .as_mut()
            .ok_or("Native helper output unavailable")?;
        let result = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            loop {
                let mut line = String::new();
                if output
                    .read_line(&mut line)
                    .await
                    .map_err(|e| e.to_string())?
                    == 0
                {
                    return Err("Native helper stopped".into());
                }
                if line.len() > 24_000_000 {
                    return Err("Native result exceeds size limit".into());
                }
                let value: Value = serde_json::from_str(&line)
                    .map_err(|_| "Native helper returned invalid JSON")?;
                if value.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = value.get("error").and_then(Value::as_str) {
                    return Err(error.to_owned());
                }
                return Ok(value["result"].clone());
            }
        })
        .await
        .map_err(|_| "Native helper timed out".to_owned())?;
        if result.is_err() && action == "capture" {
            self.shutdown().await;
        }
        result
    }
}
