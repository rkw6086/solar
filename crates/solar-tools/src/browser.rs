use crate::shell::ProcessGroup;
use crate::{
    ToolContext, ToolDefinition, ToolImage, ToolOutput, definition, http_url, managed_environment,
    managed_subdirectory, required_str,
};
use base64::Engine;
use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite::Message};

pub fn definitions() -> Vec<ToolDefinition> {
    vec![
        definition(
            "browser_open",
            "Open an HTTP(S) page in this Orbit's visible, separate Solar browser profile.",
            json!({"url":{"type":"string"}}),
            &["url"],
        ),
        definition(
            "browser_search",
            "Search the web in this Orbit's separate browser profile. Inspect the resulting page for sources.",
            json!({"query":{"type":"string"}}),
            &["query"],
        ),
        definition(
            "browser_inspect",
            "Read the current page title, URL, bounded visible text, and CSS selectors for interactive elements. Password values are excluded.",
            json!({}),
            &[],
        ),
        definition(
            "browser_click",
            "Click a CSS selector on the current page. Inspect the current page first.",
            json!({"selector":{"type":"string"}}),
            &["selector"],
        ),
        definition(
            "browser_type",
            "Replace the value of an editable element selected by CSS and dispatch input/change events.",
            json!({"selector":{"type":"string"},"text":{"type":"string"}}),
            &["selector", "text"],
        ),
        definition(
            "browser_screenshot",
            "Capture the current Solar browser page as an image.",
            json!({}),
            &[],
        ),
    ]
}

pub struct BrowserManager {
    sessions: HashMap<String, BrowserSession>,
}

impl BrowserManager {
    pub fn shutdown(&mut self) {
        self.sessions.clear();
    }

    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }

    pub async fn execute(
        &mut self,
        name: &str,
        args: &Value,
        context: &ToolContext,
    ) -> Result<ToolOutput, String> {
        if !context.allow_host_execution {
            return Err("Browser automation launches a host browser and requires this Orbit's explicit host-execution grant".into());
        }
        let directory = profile_directory(context)?;
        if self
            .sessions
            .get(&context.orbit_id)
            .is_some_and(|session| session.directory != directory)
        {
            self.sessions.remove(&context.orbit_id);
        }
        if !self.sessions.contains_key(&context.orbit_id) {
            let session = BrowserSession::launch(&directory, &context.data_directory).await?;
            self.sessions.insert(context.orbit_id.clone(), session);
        }
        let session = self
            .sessions
            .get_mut(&context.orbit_id)
            .ok_or("Browser session unavailable")?;
        match name {
            "browser_open" | "browser_search" => {
                let url = if name == "browser_search" {
                    let mut url = http_url("https://duckduckgo.com/")?;
                    url.query_pairs_mut()
                        .append_pair("q", required_str(args, "query")?);
                    url
                } else {
                    http_url(required_str(args, "url")?)?
                };
                let response = session
                    .call("Page.navigate", json!({"url":url.as_str()}))
                    .await?;
                if let Some(error) = response.get("errorText").and_then(Value::as_str) {
                    return Err(format!("Navigation failed: {error}"));
                }
                session
                    .wait_for_document(response.get("loaderId").and_then(Value::as_str))
                    .await?;
                session.inspect().await
            }
            "browser_inspect" => session.inspect().await,
            "browser_click" => {
                let selector = serde_json::to_string(required_str(args, "selector")?)
                    .map_err(|e| e.to_string())?;
                let expression = format!(
                    "(()=>{{const e=document.querySelector({selector});if(!e)throw Error('Element not found');e.scrollIntoView({{block:'center'}});e.click();return {{clicked:true}};}})()"
                );
                session.evaluate(&expression).await.map(ToolOutput::json)
            }
            "browser_type" => {
                let selector = serde_json::to_string(required_str(args, "selector")?)
                    .map_err(|e| e.to_string())?;
                let text = serde_json::to_string(
                    args.get("text")
                        .and_then(Value::as_str)
                        .ok_or("text must be a string")?,
                )
                .map_err(|e| e.to_string())?;
                let expression = format!(
                    "(()=>{{const e=document.querySelector({selector});if(!e)throw Error('Element not found');e.focus();if(e.isContentEditable)e.textContent={text};else {{const p=Object.getOwnPropertyDescriptor(Object.getPrototypeOf(e),'value');if(p?.set)p.set.call(e,{text});else e.value={text};}}e.dispatchEvent(new Event('input',{{bubbles:true}}));e.dispatchEvent(new Event('change',{{bubbles:true}}));return {{typed:true}};}})()"
                );
                session.evaluate(&expression).await.map(ToolOutput::json)
            }
            "browser_screenshot" => {
                let response = session
                    .call(
                        "Page.captureScreenshot",
                        json!({"format":"png","captureBeyondViewport":false}),
                    )
                    .await?;
                let data = response
                    .get("data")
                    .and_then(Value::as_str)
                    .ok_or("Browser returned no image")?;
                if data.len() > 24_000_000 {
                    return Err("Browser image exceeds the capture limit".into());
                }
                let image = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|e| e.to_string())?;
                Ok(ToolOutput {
                    content: "Current Solar browser viewport".into(),
                    images: vec![ToolImage {
                        data: image,
                        mime_type: "image/png".into(),
                    }],
                })
            }
            _ => Err("Unknown browser operation".into()),
        }
    }
}

fn profile_directory(context: &ToolContext) -> Result<PathBuf, String> {
    managed_subdirectory(&context.data_directory, "browser")
}

// Solar owns this profile. On macOS Chromium otherwise asks the OS Keychain
// for its Safe Storage key. Its documented automation mode avoids that lookup;
// profile files are private, but this is not secret-backed encryption.
fn browser_flags(without_keychain: bool) -> Vec<&'static str> {
    let mut flags = vec![
        "--remote-debugging-port=0",
        "--remote-debugging-address=127.0.0.1",
        "--no-first-run",
        "--no-default-browser-check",
        "--profile-directory=Default",
    ];
    if without_keychain {
        flags.push("--use-mock-keychain");
    }
    flags
}

fn prepare_browser_credentials(directory: &Path, without_keychain: bool) -> Result<(), String> {
    if !without_keychain {
        return Ok(());
    }
    use cap_std::fs::{Dir, OpenOptions};
    use std::io::{Read, Write};
    let profile = managed_subdirectory(directory, "Default")?;
    let profile = Dir::open_ambient_dir(profile, cap_std::ambient_authority())
        .map_err(|_| "Cannot open Solar's private browser profile")?;
    let mut settings = match profile.symlink_metadata("Preferences") {
        Ok(metadata) => {
            if !metadata.is_file()
                || metadata.file_type().is_symlink()
                || metadata.len() > 4_194_304
            {
                return Err(
                    "Solar browser preferences must be a regular file smaller than 4 MiB".into(),
                );
            }
            let mut options = OpenOptions::new();
            options.read(true);
            #[cfg(unix)]
            {
                use cap_std::fs::OpenOptionsExt;
                options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
            }
            let file = profile
                .open_with("Preferences", &options)
                .map_err(|_| "Cannot read Solar browser preferences")?;
            if !file
                .metadata()
                .map_err(|_| "Cannot inspect Solar browser preferences")?
                .is_file()
            {
                return Err("Solar browser preferences must be a regular file".into());
            }
            let mut bytes = Vec::new();
            file.take(4_194_305)
                .read_to_end(&mut bytes)
                .map_err(|_| "Cannot read Solar browser preferences")?;
            if bytes.len() > 4_194_304 {
                return Err("Solar browser preferences are too large".into());
            }
            serde_json::from_slice::<Value>(&bytes).map_err(|_| "Solar browser preferences are invalid; repair this Orbit's browser profile before reopening it")?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(_) => return Err("Cannot inspect Solar browser preferences".into()),
    };
    let settings = settings
        .as_object_mut()
        .ok_or("Solar browser preferences must be an object")?;
    // Do not offer password/passkey storage in a profile with no secret-backed key.
    for key in [
        "credentials_enable_service",
        "credentials_enable_passkeys",
        "credentials_enable_automatic_passkey_upgrades",
        "credentials_enable_autosignin",
    ] {
        settings.insert(key.into(), Value::Bool(false));
    }
    let bytes =
        serde_json::to_vec(settings).map_err(|_| "Cannot prepare Solar browser preferences")?;
    let temporary = format!(".solar-browser-{}.tmp", uuid::Uuid::new_v4());
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = profile
            .open_with(&temporary, &options)
            .map_err(|_| "Cannot save private browser preferences")?;
        file.write_all(&bytes)
            .map_err(|_| "Cannot save private browser preferences")?;
        file.sync_all()
            .map_err(|_| "Cannot save private browser preferences")?;
        profile
            .rename(&temporary, &profile, "Preferences")
            .map_err(|_| "Cannot finish private browser preferences")?;
        Ok(())
    })();
    if result.is_err() {
        let _ = profile.remove_file(&temporary);
    }
    result
}

struct BrowserSession {
    directory: PathBuf,
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    session_id: String,
    next_id: u64,
    loaded_documents: Vec<String>,
    _child: tokio::process::Child,
    _process_group: ProcessGroup,
}

impl BrowserSession {
    async fn launch(directory: &Path, data_directory: &Path) -> Result<Self, String> {
        let environment = managed_environment(data_directory)?;
        prepare_browser_credentials(directory, cfg!(target_os = "macos"))?;
        let active_port = directory.join("DevToolsActivePort");
        let _ = tokio::fs::remove_file(&active_port).await;
        let executable = browser_executable().ok_or("A compatible Chrome or Chromium installation is required. Set SOLAR_BROWSER_EXECUTABLE to its executable path if detection fails.")?;
        let mut command = tokio::process::Command::new(executable);
        command
            .envs(environment)
            .arg(format!("--user-data-dir={}", directory.display()))
            .args(browser_flags(cfg!(target_os = "macos")))
            .arg("about:blank")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Could not start browser: {e}"))?;
        let process_group = ProcessGroup(child.id());
        let mut endpoint = None;
        for _ in 0..100 {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                return Err(format!(
                    "Browser exited before remote control was ready ({status})"
                ));
            }
            if let Ok(port) = tokio::fs::read_to_string(&active_port).await {
                let mut lines = port.lines();
                if let (Some(port), Some(path)) = (lines.next(), lines.next())
                    && let Ok(port) = port.parse::<u16>()
                {
                    endpoint = Some(format!("ws://127.0.0.1:{port}{path}"));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let endpoint = endpoint.ok_or("Browser did not open its local automation endpoint")?;
        let (socket, _) = tokio_tungstenite::connect_async(endpoint)
            .await
            .map_err(|e| e.to_string())?;
        let mut browser = Self {
            directory: directory.to_owned(),
            socket,
            session_id: String::new(),
            next_id: 0,
            loaded_documents: Vec::new(),
            _child: child,
            _process_group: process_group,
        };
        let target = browser
            .call("Target.createTarget", json!({"url":"about:blank"}))
            .await?;
        let target_id = target
            .get("targetId")
            .and_then(Value::as_str)
            .ok_or("Browser target unavailable")?;
        let attached = browser
            .call(
                "Target.attachToTarget",
                json!({"targetId":target_id,"flatten":true}),
            )
            .await?;
        browser.session_id = attached
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or("Browser session unavailable")?
            .to_owned();
        browser.call("Page.enable", json!({})).await?;
        browser
            .call("Page.setLifecycleEventsEnabled", json!({"enabled":true}))
            .await?;
        browser.call("Runtime.enable", json!({})).await?;
        Ok(browser)
    }

    async fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.next_id += 1;
        let id = self.next_id;
        let mut message = json!({"id":id,"method":method,"params":params});
        if !self.session_id.is_empty() {
            message["sessionId"] = json!(self.session_id);
        }
        self.socket
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(|e| e.to_string())?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let message = self
                    .socket
                    .next()
                    .await
                    .ok_or("Browser connection closed")?
                    .map_err(|e| e.to_string())?;
                if let Message::Text(text) = message {
                    let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    if value.get("method").and_then(Value::as_str) == Some("Page.lifecycleEvent")
                        && matches!(
                            value["params"]["name"].as_str(),
                            Some("DOMContentLoaded" | "load")
                        )
                        && let Some(loader) = value["params"]["loaderId"].as_str()
                    {
                        self.loaded_documents.push(loader.into());
                        if self.loaded_documents.len() > 16 {
                            self.loaded_documents.remove(0);
                        }
                    }
                    if value.get("id").and_then(Value::as_u64) != Some(id) {
                        continue;
                    }
                    if let Some(error) = value.get("error") {
                        return Err(format!("Browser operation failed: {error}"));
                    }
                    return Ok(value["result"].clone());
                }
            }
        })
        .await
        .map_err(|_| "Browser operation timed out".to_owned())?
    }

    async fn evaluate(&mut self, expression: &str) -> Result<Value, String> {
        let response = self
            .call(
                "Runtime.evaluate",
                json!({"expression":expression,"returnByValue":true,"awaitPromise":true}),
            )
            .await?;
        if response.get("exceptionDetails").is_some() {
            return Err(
                "Page operation failed. Inspect the current document and choose a valid element."
                    .into(),
            );
        }
        Ok(response["result"]["value"].clone())
    }

    async fn wait_for_document(&mut self, loader: Option<&str>) -> Result<(), String> {
        for _ in 0..40 {
            let state = self
                .evaluate("document.readyState")
                .await
                .unwrap_or(Value::Null);
            let committed = loader
                .is_none_or(|loader| self.loaded_documents.iter().any(|loaded| loaded == loader));
            if committed && matches!(state.as_str(), Some("interactive" | "complete")) {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(())
    }

    async fn inspect(&mut self) -> Result<ToolOutput, String> {
        let expression = r#"(()=>({url:location.href,title:document.title,readyState:document.readyState,text:(document.body?.innerText||'').slice(0,24000),elements:Array.from(document.querySelectorAll('a,button,input,textarea,select,[role="button"],[contenteditable="true"]')).filter(e=>e.getBoundingClientRect().width>0&&e.getBoundingClientRect().height>0).slice(0,150).map((e,i)=>{e.setAttribute('data-solar-element',String(i));return {selector:'[data-solar-element="'+i+'"]',tag:e.tagName.toLowerCase(),text:(e.innerText||e.getAttribute('aria-label')||e.getAttribute('placeholder')||'').slice(0,250),type:e.getAttribute('type'),href:e.href||undefined,value:e.type==='password'?undefined:e.value};})}))()"#;
        self.evaluate(expression).await.map(ToolOutput::json)
    }
}

fn browser_executable() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SOLAR_BROWSER_EXECUTABLE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Some(path);
        }
    }
    for candidate in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(root: &Path, id: &str) -> ToolContext {
        ToolContext {
            orbit_id: id.into(),
            data_directory: root.into(),
            workspace: root.join("workspace"),
            allowed_roots: Vec::new(),
            denied_roots: vec![],
            allow_host_execution: false,
            screen_share_allowed: false,
            cancellation: Default::default(),
            progress: None,
        }
    }

    #[test]
    fn browser_profiles_live_beneath_each_orbit_directory() {
        let root = tempfile::tempdir().unwrap();
        let first = context(&root.path().join("sol"), "first");
        let second = context(&root.path().join("lyra"), "second");
        let first_profile = profile_directory(&first).unwrap();
        let second_profile = profile_directory(&second).unwrap();
        assert_eq!(first_profile, first.data_directory.join("browser"));
        assert_eq!(second_profile, second.data_directory.join("browser"));
        assert_ne!(first_profile, second_profile);
    }

    #[tokio::test]
    async fn denied_browser_use_does_not_create_a_profile() {
        let root = tempfile::tempdir().unwrap();
        let orbit = context(&root.path().join("sol"), "first");
        let result = BrowserManager::new()
            .execute(
                "browser_open",
                &json!({"url":"https://example.com"}),
                &orbit,
            )
            .await;
        assert!(result.is_err());
        assert!(!orbit.data_directory.exists());
    }

    #[cfg(unix)]
    #[test]
    fn browser_profile_cannot_redirect_through_a_symlink() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let orbit = context(root.path(), "first");
        std::os::unix::fs::symlink(outside.path(), root.path().join("browser")).unwrap();
        assert!(profile_directory(&orbit).is_err());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
    }
}

#[cfg(test)]
mod credential_tests {
    use super::*;
    #[test]
    fn keychain_free_browser_uses_only_private_preferences_and_keeps_other_settings() {
        let root = tempfile::tempdir().unwrap();
        let profile = managed_subdirectory(root.path(), "Default").unwrap();
        std::fs::write(
            profile.join("Preferences"),
            r#"{"theme":{"id":"keep"},"credentials_enable_service":true}"#,
        )
        .unwrap();
        prepare_browser_credentials(root.path(), true).unwrap();
        let settings: Value =
            serde_json::from_slice(&std::fs::read(profile.join("Preferences")).unwrap()).unwrap();
        assert_eq!(settings["theme"]["id"], "keep");
        assert_eq!(settings["credentials_enable_service"], false);
        assert_eq!(settings["credentials_enable_passkeys"], false);
        assert!(browser_flags(true).contains(&"--use-mock-keychain"));
        assert!(!browser_flags(false).contains(&"--use-mock-keychain"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(profile.join("Preferences"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn browser_preferences_never_follow_a_link_outside_the_profile() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(outside.path(), "preserve").unwrap();
        let profile = managed_subdirectory(root.path(), "Default").unwrap();
        std::os::unix::fs::symlink(outside.path(), profile.join("Preferences")).unwrap();
        assert!(prepare_browser_credentials(root.path(), true).is_err());
        assert_eq!(std::fs::read_to_string(outside.path()).unwrap(), "preserve");
    }
}
