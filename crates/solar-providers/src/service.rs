//! Solar-owned credentials and provider construction. Tokens never cross into UI state.
use crate::credential_store::CredentialStore;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures::StreamExt;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use solar_agent_core::AgentRuntime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::{
    AnthropicRuntime, AnthropicRuntimeConfig, ChatGPTSubscriptionCredentials,
    ChatGPTSubscriptionRuntime, ChatGPTSubscriptionRuntimeConfig, OpenAICompatibleRuntime,
    OpenAICompatibleRuntimeConfig,
};

pub const CLAUDE_SUBSCRIPTION_NOTICE: &str = "Direct Claude subscription access is experimental and unsupported by Anthropic for third-party apps. Anthropic may restrict your account or disable access. Enable only if you accept this risk.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    #[serde(rename = "openai")]
    OpenAi,
    #[serde(rename = "litellm")]
    LiteLlm,
    #[serde(rename = "chatgpt")]
    ChatGpt,
    #[serde(rename = "claude")]
    Claude,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderProfile {
    pub id: String,
    pub name: String,
    pub kind: ProviderKind,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub experimental: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStart {
    pub url: String,
    pub expires_in_seconds: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginStatus {
    pub connected: bool,
    pub pending: bool,
    pub message: Option<String>,
    pub persistent_storage: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct CredentialRecord {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
    #[serde(default)]
    expires_at: Option<u64>,
}

impl std::fmt::Debug for CredentialRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialRecord")
            .field("has_refresh", &self.refresh_token.is_some())
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
struct PendingLogin {
    generation: u64,
    verifier: String,
    state: String,
    redirect_uri: String,
    expires_at: u64,
}

#[derive(Default)]
struct AuthState {
    generation: u64,
    record: Option<CredentialRecord>,
    pending: Option<PendingLogin>,
    error: Option<String>,
}

struct AuthSlot {
    state: Mutex<AuthState>,
    refresh: tokio::sync::Mutex<()>,
    cancel_login: tokio::sync::Notify,
}

#[derive(Clone)]
pub struct ProviderService {
    slots: Arc<Mutex<HashMap<String, Arc<AuthSlot>>>>,
    resetting: Arc<AtomicBool>,
    reset_lock: Arc<Mutex<()>>,
    client: reqwest::Client,
    storage: Arc<CredentialStore>,
    #[cfg(test)]
    token_override: Option<String>,
}

impl ProviderService {
    pub fn new(data_directory: &Path) -> Result<Self, String> {
        let storage = Arc::new(CredentialStore::open(data_directory)?);
        Ok(Self {
            slots: Arc::new(Mutex::new(HashMap::new())),
            resetting: Arc::new(AtomicBool::new(false)),
            reset_lock: Arc::new(Mutex::new(())),
            storage,
            #[cfg(test)]
            token_override: None,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|_| "Cannot initialize Solar's provider connection service.")?,
        })
    }

    fn load_secret(
        &self,
        key: &str,
        replace_invalid: bool,
    ) -> Result<Option<CredentialRecord>, String> {
        let Some(bytes) = self.storage.load(key)? else {
            return Ok(None);
        };
        match serde_json::from_slice(&bytes) {
            Ok(record) => Ok(Some(record)),
            Err(_) if replace_invalid => Ok(None),
            Err(_) => Err("Stored credentials are invalid. Reconnect this connection.".into()),
        }
    }
    fn save_secret(&self, key: &str, record: &CredentialRecord) -> Result<(), String> {
        let bytes = serde_json::to_vec(record).map_err(|_| "Cannot encode Solar's credential.")?;
        self.storage.save(key, &bytes)
    }

    fn token_url(&self, kind: ProviderKind) -> &str {
        #[cfg(test)]
        if let Some(url) = &self.token_override {
            return url;
        }
        match kind {
            ProviderKind::ChatGpt => "https://auth.openai.com/oauth/token",
            ProviderKind::Claude => "https://platform.claude.com/v1/oauth/token",
            _ => unreachable!("only subscription providers have a token endpoint"),
        }
    }

    fn key(profile: &ProviderProfile) -> Result<String, String> {
        if profile.id.is_empty()
            || profile.id.len() > 128
            || !profile
                .id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        {
            return Err("Provider identifier is invalid.".into());
        }
        Ok(format!("provider::{}::{:?}", profile.id, profile.kind))
    }

    fn slot(&self, profile: &ProviderProfile) -> Result<Arc<AuthSlot>, String> {
        self.slot_for_key(Self::key(profile)?)
    }

    fn slot_replacing(&self, profile: &ProviderProfile) -> Result<Arc<AuthSlot>, String> {
        self.slot_for_key_mode(Self::key(profile)?, true)
    }

    fn slot_for_key(&self, key: String) -> Result<Arc<AuthSlot>, String> {
        self.slot_for_key_mode(key, false)
    }

    fn slot_for_key_mode(
        &self,
        key: String,
        replace_invalid: bool,
    ) -> Result<Arc<AuthSlot>, String> {
        if self.resetting.load(Ordering::SeqCst) {
            return Err("Solar credentials are being reset.".into());
        }
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        if self.resetting.load(Ordering::SeqCst) {
            return Err("Solar credentials are being reset.".into());
        }
        if let Some(slot) = slots.get(&key) {
            return Ok(slot.clone());
        }
        let record = self.load_secret(&key, replace_invalid)?;
        let slot = Arc::new(AuthSlot {
            state: Mutex::new(AuthState {
                record,
                ..Default::default()
            }),
            refresh: tokio::sync::Mutex::new(()),
            cancel_login: tokio::sync::Notify::new(),
        });
        slots.insert(key, slot.clone());
        Ok(slot)
    }

    /// Store integration configuration containing secrets outside ordinary application profiles.
    pub fn set_integration_secret(&self, id: &str, value: &str) -> Result<(), String> {
        let key = integration_key(id)?;
        if value.len() > 128 * 1024 {
            return Err("Integration secret exceeds the size limit.".into());
        }
        if value.is_empty() {
            return self.delete_integration_secret(id);
        }
        let record = CredentialRecord {
            access_token: value.to_owned(),
            refresh_token: None,
            account_id: None,
            expires_at: None,
        };
        let slot = self.slot_for_key_mode(key.clone(), true)?;
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        if self.resetting.load(Ordering::SeqCst) {
            return Err("Solar credentials are being reset.".into());
        }
        self.save_secret(&key, &record)?;
        state.generation = state.generation.wrapping_add(1);
        state.record = Some(record);
        Ok(())
    }

    /// Returns integration secrets to trusted Rust code only. Never expose this as a UI command.
    pub fn integration_secret(&self, id: &str) -> Result<Option<String>, String> {
        let slot = self.slot_for_key(integration_key(id)?)?;
        let state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        Ok(state
            .record
            .as_ref()
            .map(|record| record.access_token.clone()))
    }

    pub fn delete_integration_secret(&self, id: &str) -> Result<(), String> {
        let key = integration_key(id)?;
        let slot = self.slot_for_key_mode(key.clone(), true)?;
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        let deletion = self.storage.delete(&key);
        state.generation = state.generation.wrapping_add(1);
        state.record = None;
        state.error = deletion.as_ref().err().cloned();
        deletion
    }

    pub fn status(&self, profile: &ProviderProfile) -> Result<bool, String> {
        Ok(self.login_status(profile)?.connected)
    }

    pub fn login_status(&self, profile: &ProviderProfile) -> Result<LoginStatus, String> {
        let slot = self.slot(profile)?;
        let state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        let connected = state.record.as_ref().is_some_and(|record| {
            !record.access_token.is_empty()
                && (record.expires_at.is_none_or(|expiry| expiry > now())
                    || record.refresh_token.is_some())
        });
        Ok(LoginStatus {
            connected,
            pending: state
                .pending
                .as_ref()
                .is_some_and(|pending| pending.expires_at > now()),
            message: state.error.clone(),
            persistent_storage: true,
        })
    }

    /// An empty key explicitly clears credentials. Unauthenticated local endpoints remain usable.
    pub fn set_api_key(&self, profile: &ProviderProfile, key: &str) -> Result<(), String> {
        if key.trim().is_empty() {
            return self.logout(profile);
        }
        if key.len() > 32768 || key.chars().any(char::is_control) {
            return Err("Credential is not valid.".into());
        }
        if profile.kind == ProviderKind::ChatGpt {
            return Err("Use browser sign-in for a ChatGPT subscription.".into());
        }
        if profile.kind == ProviderKind::Claude
            && crate::is_claude_subscription_token(key)
            && !profile.experimental
        {
            return Err(CLAUDE_SUBSCRIPTION_NOTICE.into());
        }
        let record = CredentialRecord {
            access_token: key.trim().into(),
            refresh_token: None,
            account_id: None,
            expires_at: None,
        };
        let slot = self.slot_replacing(profile)?;
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        if self.resetting.load(Ordering::SeqCst) {
            return Err("Solar credentials are being reset.".into());
        }
        self.save_secret(&Self::key(profile)?, &record)?;
        state.generation = state.generation.wrapping_add(1);
        slot.cancel_login.notify_waiters();
        state.pending = None;
        state.record = Some(record);
        state.error = None;
        Ok(())
    }

    pub fn logout(&self, profile: &ProviderProfile) -> Result<(), String> {
        let slot = self.slot_replacing(profile)?;
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        let deletion = self.storage.delete(&Self::key(profile)?);
        state.generation = state.generation.wrapping_add(1);
        slot.cancel_login.notify_waiters();
        state.record = None;
        state.pending = None;
        state.error = deletion.as_ref().err().cloned();
        deletion
    }

    /// Clear only Solar's dedicated credential namespace. This also covers
    /// orphaned keys from providers or MCP servers no longer in the database.
    /// Successful reset keeps this service closed until the app restarts.
    pub fn reset_credentials(&self) -> Result<(), String> {
        self.reset_credentials_with(|| self.storage.clear())
    }

    fn reset_credentials_with(
        &self,
        delete: impl FnOnce() -> Result<(), String>,
    ) -> Result<(), String> {
        let _reset = self
            .reset_lock
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        if self.resetting.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let result = (|| {
            let slots = self
                .slots
                .lock()
                .map_err(|_| "Credential service is unavailable.")?;
            let mut states = Vec::with_capacity(slots.len());
            for slot in slots.values() {
                let mut state = slot
                    .state
                    .lock()
                    .map_err(|_| "Credential service is unavailable.")?;
                state.generation = state.generation.wrapping_add(1);
                state.record = None;
                state.pending = None;
                state.error = None;
                slot.cancel_login.notify_waiters();
                states.push(state);
            }
            // Keep the slot and state locks through deletion. Late auth writes
            // see a changed generation or the reset gate, never restore a key.
            let deletion = delete();
            if let Err(error) = &deletion {
                for state in &mut states {
                    state.error = Some(error.clone());
                }
            }
            deletion
        })();
        if result.is_err() {
            self.resetting.store(false, Ordering::SeqCst);
        }
        result
    }

    pub async fn begin_login(&self, profile: &ProviderProfile) -> Result<LoginStart, String> {
        if !matches!(profile.kind, ProviderKind::ChatGpt | ProviderKind::Claude) {
            return Err("This provider uses an API key, not browser sign-in.".into());
        }
        if profile.kind == ProviderKind::Claude && !profile.experimental {
            return Err(CLAUDE_SUBSCRIPTION_NOTICE.into());
        }
        let slot = self.slot_replacing(profile)?;
        let (port, path) = match profile.kind {
            ProviderKind::ChatGpt => (1455, "/auth/callback"),
            ProviderKind::Claude => (53692, "/callback"),
            _ => unreachable!(),
        };
        {
            let mut state = slot
                .state
                .lock()
                .map_err(|_| "Credential service is unavailable.")?;
            state.generation = state.generation.wrapping_add(1);
            state.pending = None;
            slot.cancel_login.notify_waiters();
        }
        // Give a cancelled listener a chance to release its fixed provider callback port.
        tokio::task::yield_now().await;
        let mut listener = None;
        for _ in 0..10 {
            match TcpListener::bind(("127.0.0.1", port)).await {
                Ok(bound) => {
                    listener = Some(bound);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
        let listener = listener.ok_or_else(|| {
            format!("Sign-in port {port} is busy. Finish other sign-ins and try again.")
        })?;
        let redirect_uri = format!("http://localhost:{port}{path}");
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        state.generation = state.generation.wrapping_add(1);
        let pending = PendingLogin {
            generation: state.generation,
            verifier: random_secret(),
            state: random_secret(),
            redirect_uri,
            expires_at: now() + 300,
        };
        let url = authorize_url(profile.kind, &pending)?;
        state.pending = Some(pending.clone());
        state.error = None;
        drop(state);
        let service = self.clone();
        let profile = profile.clone();
        tokio::spawn(async move {
            let result = service
                .listen_for_callback(&profile, pending.clone(), listener, path)
                .await;
            if let Err(error) = result
                && let Ok(mut state) = slot.state.lock()
                && state.generation == pending.generation
                && state.pending.is_some()
            {
                state.pending = None;
                state.error = Some(error);
            }
        });
        Ok(LoginStart {
            url,
            expires_in_seconds: 300,
        })
    }

    async fn listen_for_callback(
        &self,
        profile: &ProviderProfile,
        pending: PendingLogin,
        listener: TcpListener,
        expected_path: &str,
    ) -> Result<(), String> {
        tokio::time::timeout(Duration::from_secs(300), async {
            loop {
                let slot = self.slot(profile)?;
                if slot.state.lock().map_err(|_| "Credential service is unavailable.")?.generation != pending.generation {
                    return Err("Sign-in was cancelled or replaced.".into());
                }
                let (mut stream, _) = tokio::select! {
                    accepted = listener.accept() => accepted.map_err(|_| "Sign-in listener failed.")?,
                    _ = slot.cancel_login.notified() => return Err("Sign-in was cancelled or replaced.".into()),
                };
                let mut data = vec![0u8; 8192];
                let size = match tokio::time::timeout(Duration::from_secs(3), stream.read(&mut data)).await {
                    Ok(Ok(size)) => size,
                    _ => continue,
                };
                let line = std::str::from_utf8(&data[..size]).ok().and_then(|request| request.lines().next());
                let target = line.and_then(|line| line.strip_prefix("GET ")).and_then(|rest| rest.split_once(' ')).map(|v|v.0);
                let Some(target) = target else { continue; };
                let callback = match Url::parse(&pending.redirect_uri).and_then(|base| base.join(target)) {
                    Ok(url) if url.path() == expected_path => url,
                    _ => continue,
                };
                // Ignore unrelated browser requests and callbacks from an older attempt.
                if callback.query_pairs().find(|(k,_)|k == "state").map(|(_,v)|v.into_owned()).as_deref() != Some(&pending.state) {
                    let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 21\r\nConnection: close\r\n\r\nInvalid sign-in state.").await;
                    continue;
                }
                let result = self.complete_login(profile, callback.as_str()).await;
                let body = if result.is_ok() { "Solar is connected. You can close this window." } else { "Solar could not complete sign-in. Return to Solar for details." };
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes()).await;
                return result;
            }
        }).await.map_err(|_| "Sign-in expired. Try again.".to_string())?
    }

    pub async fn complete_login(
        &self,
        profile: &ProviderProfile,
        callback: &str,
    ) -> Result<(), String> {
        let slot = self.slot(profile)?;
        let _gate = slot.refresh.lock().await;
        let pending = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?
            .pending
            .clone()
            .ok_or("No sign-in is pending.")?;
        if pending.expires_at <= now() {
            return Err("Sign-in expired. Try again.".into());
        }
        let (code, callback_state) = parse_callback(callback, &pending.redirect_uri)?;
        if callback_state != pending.state {
            return Err("Sign-in state did not match. Try again.".into());
        }
        let record = self.exchange_code(profile.kind, &pending, &code).await?;
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        if state.generation != pending.generation || state.pending.is_none() {
            return Err("This sign-in was cancelled or replaced.".into());
        }
        publish_record(&mut state, pending.generation, record, |record| {
            self.save_secret(&Self::key(profile)?, record)
        })?;
        state.pending = None;
        state.error = None;
        Ok(())
    }

    async fn exchange_code(
        &self,
        kind: ProviderKind,
        pending: &PendingLogin,
        code: &str,
    ) -> Result<CredentialRecord, String> {
        let response = match kind {
            ProviderKind::ChatGpt => self.client.post(self.token_url(ProviderKind::ChatGpt))
                .form(&[("grant_type", "authorization_code"), ("client_id", CHATGPT_CLIENT),
                    ("code", code), ("code_verifier", &pending.verifier), ("redirect_uri", &pending.redirect_uri)])
                .send().await,
            ProviderKind::Claude => self.client.post(self.token_url(ProviderKind::Claude))
                .json(&serde_json::json!({"grant_type":"authorization_code", "client_id":CLAUDE_CLIENT,
                    "code":code, "state":pending.state, "code_verifier":pending.verifier,
                    "redirect_uri":pending.redirect_uri})).send().await,
            _ => return Err("Provider does not support subscription sign-in.".into()),
        }.map_err(|_| "Could not reach the sign-in server. Try again.".to_string())?;
        parse_token_response(response, None).await
    }

    async fn credential(&self, profile: &ProviderProfile) -> Result<CredentialRecord, String> {
        let slot = self.slot(profile)?;
        let _gate = slot.refresh.lock().await;
        let (generation, previous) = {
            let state = slot
                .state
                .lock()
                .map_err(|_| "Credential service is unavailable.")?;
            (
                state.generation,
                state.record.clone().ok_or("Connect this provider first.")?,
            )
        };
        if previous
            .expires_at
            .is_none_or(|expiry| expiry > now() + 300)
        {
            return Ok(previous);
        }
        let refresh = previous
            .refresh_token
            .as_deref()
            .ok_or("Subscription sign-in expired. Reconnect this provider.")?;
        let response = match profile.kind {
            ProviderKind::ChatGpt => {
                self.client
                    .post(self.token_url(ProviderKind::ChatGpt))
                    .form(&[
                        ("grant_type", "refresh_token"),
                        ("client_id", CHATGPT_CLIENT),
                        ("refresh_token", refresh),
                    ])
                    .send()
                    .await
            }
            ProviderKind::Claude => {
                self.client
                    .post(self.token_url(ProviderKind::Claude))
                    .json(
                        &serde_json::json!({"grant_type":"refresh_token", "client_id":CLAUDE_CLIENT,
                    "refresh_token":refresh}),
                    )
                    .send()
                    .await
            }
            _ => return Err("API credentials expired. Reconnect this provider.".into()),
        }
        .map_err(|_| "Credential refresh could not reach the provider. Try again.".to_string())?;
        let rejected = matches!(response.status().as_u16(), 400 | 401 | 403);
        let record = match parse_token_response(response, Some(&previous)).await {
            Ok(record) => record,
            Err(error) => {
                if rejected {
                    let mut state = slot
                        .state
                        .lock()
                        .map_err(|_| "Credential service is unavailable.")?;
                    if state.generation == generation {
                        let deletion = self.storage.delete(&Self::key(profile)?);
                        state.record = None;
                        state.error = Some(error.clone());
                        deletion?;
                    }
                }
                return Err(error);
            }
        };
        let mut state = slot
            .state
            .lock()
            .map_err(|_| "Credential service is unavailable.")?;
        if state.generation != generation {
            return Err("The provider session was replaced.".into());
        }
        publish_record(&mut state, generation, record.clone(), |record| {
            self.save_secret(&Self::key(profile)?, record)
        })?;
        Ok(record)
    }

    pub async fn runtime(
        &self,
        profile: &ProviderProfile,
        model: &str,
    ) -> Result<Arc<dyn AgentRuntime>, String> {
        if model.trim().is_empty() {
            return Err("Choose a model for this Orbit.".into());
        }
        match profile.kind {
            ProviderKind::OpenAi | ProviderKind::LiteLlm => {
                let base_url = validate_endpoint(&profile.base_url)?;
                let key = if self.status(profile)? {
                    Some(self.credential(profile).await?.access_token)
                } else {
                    None
                };
                Ok(Arc::new(OpenAICompatibleRuntime::new(
                    OpenAICompatibleRuntimeConfig {
                        base_url,
                        api_key: key,
                        model: model.into(),
                        max_output_tokens: None,
                        supports_system_messages: true,
                    },
                )))
            }
            ProviderKind::ChatGpt => {
                let service = self.clone();
                let profile = profile.clone();
                Ok(Arc::new(
                    ChatGPTSubscriptionRuntime::new(ChatGPTSubscriptionRuntimeConfig {
                        model: model.into(),
                        reasoning_effort: None,
                        max_output_tokens: None,
                    })
                    .with_credential_resolver(Arc::new(move || {
                        let service = service.clone();
                        let profile = profile.clone();
                        Box::pin(async move {
                            let record = service.credential(&profile).await?;
                            Ok(ChatGPTSubscriptionCredentials {
                                access_token: record.access_token,
                                account_id: record.account_id,
                            })
                        })
                    })),
                ))
            }
            ProviderKind::Claude => {
                let service = self.clone();
                let profile = profile.clone();
                let experimental = profile.experimental;
                Ok(Arc::new(
                    AnthropicRuntime::new(AnthropicRuntimeConfig {
                        api_key: String::new(),
                        model: model.into(),
                        max_output_tokens: None,
                    })
                    .with_credential_resolver(Arc::new(move || {
                        let service = service.clone();
                        let profile = profile.clone();
                        Box::pin(async move {
                            let record = service.credential(&profile).await?;
                            if crate::is_claude_subscription_token(&record.access_token)
                                && !experimental
                            {
                                return Err(CLAUDE_SUBSCRIPTION_NOTICE.into());
                            }
                            Ok(record.access_token)
                        })
                    })),
                ))
            }
        }
    }

    pub async fn discover_models(&self, profile: &ProviderProfile) -> Result<Vec<String>, String> {
        match profile.kind {
            ProviderKind::Claude => {
                let credential = self.credential(profile).await?;
                if crate::is_claude_subscription_token(&credential.access_token)
                    && !profile.experimental
                {
                    return Err(CLAUDE_SUBSCRIPTION_NOTICE.into());
                }
                Ok(crate::discover_anthropic_models(&credential.access_token)
                    .await?
                    .into_iter()
                    .map(|m| m.id)
                    .collect())
            }
            ProviderKind::ChatGpt => {
                let credential = self.credential(profile).await?;
                let release = self
                    .client
                    .get("https://api.github.com/repos/openai/codex/releases/latest")
                    .header(reqwest::header::USER_AGENT, "Solar")
                    .send()
                    .await
                    .map_err(|_| "Could not retrieve the Codex catalog version.")?;
                if !release.status().is_success() {
                    return Err("Could not retrieve the Codex catalog version. Existing models are retained.".into());
                }
                let release = bounded_json(release, 1024 * 1024).await?;
                let version = release["tag_name"]
                    .as_str()
                    .and_then(normalize_codex_version)
                    .ok_or("Codex catalog version was invalid.")?;
                let mut request = self
                    .client
                    .get("https://chatgpt.com/backend-api/codex/models")
                    .query(&[("client_version", version)])
                    .bearer_auth(credential.access_token)
                    .header(reqwest::header::USER_AGENT, "Solar")
                    .header(reqwest::header::ACCEPT, "application/json");
                if let Some(account_id) = credential.account_id {
                    request = request.header("ChatGPT-Account-ID", account_id);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| "Could not retrieve ChatGPT models.")?;
                if !response.status().is_success() {
                    return Err(format!(
                        "ChatGPT model discovery returned HTTP {}.",
                        response.status().as_u16()
                    ));
                }
                let body = bounded_json(response, 2 * 1024 * 1024).await?;
                chatgpt_model_ids(&body)
            }
            _ => {
                let base_url = validate_endpoint(&profile.base_url)?;
                let mut request = self
                    .client
                    .get(format!("{}/models", base_url.trim_end_matches('/')));
                if self.status(profile)? {
                    request = request.bearer_auth(self.credential(profile).await?.access_token);
                }
                let response = request
                    .send()
                    .await
                    .map_err(|_| "Could not reach the model endpoint.".to_string())?;
                if !response.status().is_success() {
                    return Err(format!(
                        "Model discovery returned HTTP {}.",
                        response.status().as_u16()
                    ));
                }
                let body = bounded_json(response, 2 * 1024 * 1024).await?;
                let data = body["data"]
                    .as_array()
                    .ok_or("Model discovery did not return a model list.")?;
                let mut models: Vec<_> = data
                    .iter()
                    .filter_map(|entry| entry["id"].as_str())
                    .map(str::to_owned)
                    .collect();
                models.sort();
                models.dedup();
                Ok(models)
            }
        }
    }
}

const CHATGPT_CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const CLAUDE_CLIENT: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";

fn integration_key(id: &str) -> Result<String, String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
    {
        return Err("Integration identifier is invalid.".into());
    }
    Ok(format!("integration::{id}"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn authorize_url(kind: ProviderKind, pending: &PendingLogin) -> Result<String, String> {
    let (url, client, scope) = match kind {
        ProviderKind::ChatGpt => (
            "https://auth.openai.com/oauth/authorize",
            CHATGPT_CLIENT,
            "openid profile email offline_access",
        ),
        ProviderKind::Claude => (
            "https://claude.ai/oauth/authorize",
            CLAUDE_CLIENT,
            "org:create_api_key user:profile user:inference user:sessions:claude_code user:mcp_servers user:file_upload",
        ),
        _ => return Err("Provider does not support subscription sign-in.".into()),
    };
    let mut url = Url::parse(url).map_err(|_| "Invalid provider sign-in URL.")?;
    url.query_pairs_mut()
        .append_pair("client_id", client)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &pending.redirect_uri)
        .append_pair("scope", scope)
        .append_pair("state", &pending.state)
        .append_pair("code_challenge_method", "S256")
        .append_pair(
            "code_challenge",
            &URL_SAFE_NO_PAD.encode(Sha256::digest(pending.verifier.as_bytes())),
        );
    if kind == ProviderKind::ChatGpt {
        url.query_pairs_mut()
            .append_pair("id_token_add_organizations", "true")
            .append_pair("codex_cli_simplified_flow", "true")
            .append_pair("originator", "solar");
    }
    Ok(url.into())
}

fn parse_callback(callback: &str, expected_redirect: &str) -> Result<(String, String), String> {
    let url = Url::parse(callback.trim()).map_err(|_| "Paste the full browser callback URL.")?;
    let expected =
        Url::parse(expected_redirect).map_err(|_| "Invalid sign-in callback configuration.")?;
    if url.scheme() != expected.scheme()
        || url.host_str() != expected.host_str()
        || url.port_or_known_default() != expected.port_or_known_default()
        || url.path() != expected.path()
    {
        return Err("Sign-in callback does not match this attempt.".into());
    }
    let values: HashMap<_, _> = url.query_pairs().into_owned().collect();
    if values.contains_key("error") {
        return Err("Provider sign-in was declined. Try again when ready.".into());
    }
    let code = values
        .get("code")
        .filter(|value| !value.is_empty())
        .ok_or("Sign-in callback is missing its code.")?;
    let state = values
        .get("state")
        .filter(|value| !value.is_empty())
        .ok_or("Sign-in callback is missing its state.")?;
    Ok((code.clone(), state.clone()))
}

async fn parse_token_response(
    response: reqwest::Response,
    previous: Option<&CredentialRecord>,
) -> Result<CredentialRecord, String> {
    if !response.status().is_success() {
        return Err(match response.status().as_u16() {
            400 | 401 | 403 => {
                "Provider sign-in expired or was rejected. Reconnect this provider.".into()
            }
            429 => "Provider sign-in is rate limited. Try again later.".into(),
            _ => format!(
                "Provider sign-in returned HTTP {}.",
                response.status().as_u16()
            ),
        });
    }
    let data = bounded_json(response, 128 * 1024).await?;
    credential_from_json(&data, previous)
}

fn credential_from_json(
    data: &serde_json::Value,
    previous: Option<&CredentialRecord>,
) -> Result<CredentialRecord, String> {
    let access_token = data["access_token"]
        .as_str()
        .filter(|token| !token.is_empty() && token.len() <= 32768)
        .ok_or("Provider did not return a valid access token.")?
        .to_owned();
    if access_token.chars().any(char::is_control) {
        return Err("Provider returned an invalid access token.".into());
    }
    let access_claims = decode_claims(&access_token);
    let claims = data["id_token"]
        .as_str()
        .map(decode_claims)
        .unwrap_or_else(|| access_claims.clone());
    let expires_at = data["expires_in"]
        .as_u64()
        .filter(|seconds| *seconds > 0)
        .map(|seconds| now().saturating_add(seconds))
        .or_else(|| access_claims["exp"].as_u64())
        .filter(|expiry| *expiry > now())
        .ok_or("Provider returned missing or expired token validity.")?;
    let refresh_token = data["refresh_token"]
        .as_str()
        .filter(|token| {
            !token.is_empty() && token.len() <= 32768 && !token.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .or_else(|| previous.and_then(|p| p.refresh_token.clone()));
    let account_id = claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_account_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| previous.and_then(|p| p.account_id.clone()));
    Ok(CredentialRecord {
        access_token,
        refresh_token,
        account_id,
        expires_at: Some(expires_at),
    })
}

fn normalize_codex_version(version: &str) -> Option<&str> {
    let version = version
        .strip_prefix("rust-v")
        .or_else(|| version.strip_prefix('v'))
        .unwrap_or(version);
    if version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+'))
    {
        Some(version)
    } else {
        None
    }
}

fn chatgpt_model_ids(body: &serde_json::Value) -> Result<Vec<String>, String> {
    let models = body["models"]
        .as_array()
        .ok_or("ChatGPT model response omitted its catalog.")?;
    let mut ids: Vec<_> = models
        .iter()
        .filter(|model| model["visibility"].as_str() == Some("list"))
        .filter_map(|model| model["slug"].as_str())
        .filter(|id| !id.trim().is_empty())
        .map(str::to_owned)
        .collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err("ChatGPT returned no visible models. Existing models are retained.".into());
    }
    Ok(ids)
}

fn publish_record(
    state: &mut AuthState,
    generation: u64,
    record: CredentialRecord,
    persist: impl FnOnce(&CredentialRecord) -> Result<(), String>,
) -> Result<(), String> {
    if state.generation != generation {
        return Err("The provider session was replaced.".into());
    }
    persist(&record)?;
    state.record = Some(record);
    Ok(())
}

async fn bounded_json(
    response: reqwest::Response,
    limit: usize,
) -> Result<serde_json::Value, String> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err("Provider response exceeded the size limit.".into());
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Provider response could not be read.")?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err("Provider response exceeded the size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| "Provider returned invalid JSON.".into())
}

fn decode_claims(token: &str) -> serde_json::Value {
    token
        .split('.')
        .nth(1)
        .and_then(|data| URL_SAFE_NO_PAD.decode(data).ok())
        .and_then(|data| serde_json::from_slice(&data).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn validate_endpoint(endpoint: &str) -> Result<String, String> {
    let url = Url::parse(endpoint.trim()).map_err(|_| "Enter a valid HTTP model endpoint.")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Model endpoint must be HTTP(S), without embedded credentials, query, or fragment."
                .into(),
        );
    }
    Ok(endpoint.trim().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn profile() -> ProviderProfile {
        ProviderProfile {
            id: uuid::Uuid::new_v4().to_string(),
            name: "test".into(),
            kind: ProviderKind::OpenAi,
            base_url: "http://localhost:1234/v1".into(),
            models: vec![],
            experimental: false,
        }
    }

    #[test]
    fn reset_invalidates_loaded_credentials_and_prevents_late_reconnect() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderService::new(root.path()).unwrap();
        let profile = profile();
        let slot = service.slot(&profile).unwrap();
        let before = slot.state.lock().unwrap().generation;
        slot.state.lock().unwrap().record = Some(CredentialRecord {
            access_token: "test-only-memory-token".into(),
            refresh_token: None,
            account_id: None,
            expires_at: None,
        });
        // Test the reset state gate independently of storage failure handling.
        service.reset_credentials_with(|| Ok(())).unwrap();
        service
            .reset_credentials_with(|| panic!("A completed credential reset must be idempotent"))
            .unwrap();
        let state = slot.state.lock().unwrap();
        assert!(state.record.is_none());
        assert_ne!(state.generation, before);
        drop(state);
        assert!(service.slot(&profile).is_err());
        assert!(service.set_api_key(&profile, "replacement").is_err());
        assert!(
            service
                .set_integration_secret("mcp-test", "replacement")
                .is_err()
        );
    }

    #[test]
    fn failed_credential_reset_can_be_retried() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderService::new(root.path()).unwrap();
        assert!(
            service
                .reset_credentials_with(|| Err("test deletion failure".into()))
                .is_err()
        );
        assert!(!service.resetting.load(Ordering::SeqCst));
        service.reset_credentials_with(|| Ok(())).unwrap();
        assert!(service.resetting.load(Ordering::SeqCst));
    }
    #[test]
    fn credentials_never_serialize_through_profile_or_debug() {
        let mut profile = profile();
        profile.kind = ProviderKind::Claude;
        let record = CredentialRecord {
            access_token: "secret-value".into(),
            refresh_token: Some("refresh-value".into()),
            account_id: None,
            expires_at: None,
        };
        assert!(!format!("{record:?}").contains("secret-value"));
        assert!(
            !serde_json::to_string(&profile)
                .unwrap()
                .contains("access_token")
        );
    }
    #[test]
    fn provider_and_integration_credentials_survive_restart_and_logout_stays_scoped() {
        let root = tempfile::tempdir().unwrap();
        let profile = profile();
        let service = ProviderService::new(root.path()).unwrap();
        service.set_api_key(&profile, "private-api-key").unwrap();
        service
            .set_integration_secret("mcp-one", r#"{"TOKEN":"private-mcp-key"}"#)
            .unwrap();
        drop(service);
        let reopened = ProviderService::new(root.path()).unwrap();
        assert!(reopened.status(&profile).unwrap());
        assert!(reopened.login_status(&profile).unwrap().persistent_storage);
        assert_eq!(
            reopened.integration_secret("mcp-one").unwrap().as_deref(),
            Some(r#"{"TOKEN":"private-mcp-key"}"#)
        );
        reopened.logout(&profile).unwrap();
        let third = ProviderService::new(root.path()).unwrap();
        assert!(!third.status(&profile).unwrap());
        assert!(third.integration_secret("mcp-one").unwrap().is_some());
        third.reset_credentials().unwrap();
        let fourth = ProviderService::new(root.path()).unwrap();
        assert!(!fourth.status(&profile).unwrap());
        assert!(fourth.integration_secret("mcp-one").unwrap().is_none());
    }

    #[test]
    fn oauth_record_persists_and_failed_rotation_preserves_loaded_session() {
        let root = tempfile::tempdir().unwrap();
        let mut profile = profile();
        profile.kind = ProviderKind::ChatGpt;
        let service = ProviderService::new(root.path()).unwrap();
        let original = CredentialRecord {
            access_token: "access-before".into(),
            refresh_token: Some("refresh-before".into()),
            account_id: Some("account".into()),
            expires_at: Some(now() + 300),
        };
        service
            .save_secret(&ProviderService::key(&profile).unwrap(), &original)
            .unwrap();
        let reopened = ProviderService::new(root.path()).unwrap();
        let slot = reopened.slot(&profile).unwrap();
        let mut state = slot.state.lock().unwrap();
        assert_eq!(
            state.record.as_ref().unwrap().refresh_token.as_deref(),
            Some("refresh-before")
        );
        let generation = state.generation;
        let rotated = CredentialRecord {
            access_token: "access-after".into(),
            refresh_token: Some("refresh-after".into()),
            account_id: original.account_id,
            expires_at: original.expires_at,
        };
        assert!(
            publish_record(&mut state, generation, rotated.clone(), |_| Err(
                "private storage unavailable".into()
            ))
            .is_err()
        );
        assert_eq!(state.record.as_ref().unwrap().access_token, "access-before");
        publish_record(&mut state, generation, rotated, |record| {
            reopened.save_secret(&ProviderService::key(&profile)?, record)
        })
        .unwrap();
        drop(state);
        let after = ProviderService::new(root.path()).unwrap();
        let after_slot = after.slot(&profile).unwrap();
        let after_state = after_slot.state.lock().unwrap();
        assert_eq!(
            after_state.record.as_ref().unwrap().access_token,
            "access-after"
        );
        assert_eq!(
            after_state
                .record
                .as_ref()
                .unwrap()
                .refresh_token
                .as_deref(),
            Some("refresh-after")
        );
    }

    #[test]
    fn corrupt_credential_errors_never_echo_saved_contents() {
        let root = tempfile::tempdir().unwrap();
        let profile = profile();
        let service = ProviderService::new(root.path()).unwrap();
        service
            .storage
            .save(
                &ProviderService::key(&profile).unwrap(),
                b"not-json-private-value",
            )
            .unwrap();
        let error = match service.slot(&profile) {
            Ok(_) => panic!("Corrupt record was accepted"),
            Err(error) => error,
        };
        assert!(!error.contains("not-json-private-value"));
        assert!(!error.contains(root.path().to_string_lossy().as_ref()));
        service.reset_credentials().unwrap();
        assert!(
            ProviderService::new(root.path())
                .unwrap()
                .slot(&profile)
                .unwrap()
                .state
                .lock()
                .unwrap()
                .record
                .is_none()
        );
    }

    #[test]
    fn reconnect_and_logout_can_repair_malformed_regular_credentials() {
        let root = tempfile::tempdir().unwrap();
        let profile = profile();
        let service = ProviderService::new(root.path()).unwrap();
        let key = ProviderService::key(&profile).unwrap();
        service.storage.save(&key, b"broken-secret-json").unwrap();
        assert!(service.status(&profile).is_err());
        service.set_api_key(&profile, "replacement-key").unwrap();
        assert!(
            ProviderService::new(root.path())
                .unwrap()
                .status(&profile)
                .unwrap()
        );
        service.storage.save(&key, b"broken-again").unwrap();
        let reopened = ProviderService::new(root.path()).unwrap();
        assert!(reopened.status(&profile).is_err());
        reopened.logout(&profile).unwrap();
        assert!(
            !ProviderService::new(root.path())
                .unwrap()
                .status(&profile)
                .unwrap()
        );
        reopened
            .storage
            .save(&integration_key("mcp-one").unwrap(), b"broken-integration")
            .unwrap();
        let another = ProviderService::new(root.path()).unwrap();
        another
            .set_integration_secret("mcp-one", "valid-integration")
            .unwrap();
        assert_eq!(
            another.integration_secret("mcp-one").unwrap().as_deref(),
            Some("valid-integration")
        );
    }

    #[test]
    fn callback_is_bound_to_redirect_and_state() {
        assert!(
            parse_callback(
                "http://localhost:1455/auth/callback?code=abc&state=ok",
                "http://localhost:1455/auth/callback"
            )
            .is_ok()
        );
        assert!(
            parse_callback(
                "http://evil.test:1455/auth/callback?code=abc&state=ok",
                "http://localhost:1455/auth/callback"
            )
            .is_err()
        );
        assert!(
            parse_callback(
                "http://localhost:1455/wrong?code=abc&state=ok",
                "http://localhost:1455/auth/callback"
            )
            .is_err()
        );
    }
    #[test]
    fn oauth_has_state_and_pkce_and_product_origin() {
        let pending = PendingLogin {
            generation: 1,
            verifier: "verifier".into(),
            state: "state".into(),
            redirect_uri: "http://localhost:1455/auth/callback".into(),
            expires_at: now() + 60,
        };
        let url = Url::parse(&authorize_url(ProviderKind::ChatGpt, &pending).unwrap()).unwrap();
        let values: HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(values["state"], "state");
        assert_eq!(values["code_challenge_method"], "S256");
        assert_eq!(values["originator"], "solar");
        assert!(!url.as_str().contains("verifier="));
    }
    #[test]
    fn endpoint_validation_rejects_embedded_secrets() {
        assert!(validate_endpoint("http://localhost:4000/v1").is_ok());
        assert!(validate_endpoint("https://key@example.com/v1").is_err());
        assert!(validate_endpoint("file:///tmp/model").is_err());
    }
    #[test]
    fn logout_invalidates_credential_and_pending_generation() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderService::new(root.path()).unwrap();
        let profile = profile();
        service.set_api_key(&profile, "test-token").unwrap();
        assert!(service.status(&profile).unwrap());
        let slot = service.slot(&profile).unwrap();
        let generation = slot.state.lock().unwrap().generation;
        service.logout(&profile).unwrap();
        assert!(!service.status(&profile).unwrap());
        assert!(slot.state.lock().unwrap().generation > generation);
    }
    #[test]
    fn rotated_credentials_are_persisted_before_becoming_visible() {
        let mut state = AuthState {
            generation: 4,
            ..Default::default()
        };
        let record = CredentialRecord {
            access_token: "new".into(),
            refresh_token: None,
            account_id: None,
            expires_at: Some(now() + 60),
        };
        assert!(
            publish_record(&mut state, 4, record.clone(), |_| Err(
                "credential file unavailable".into()
            ))
            .is_err()
        );
        assert!(state.record.is_none());
        let mut persisted = false;
        publish_record(&mut state, 4, record, |_| {
            persisted = true;
            Ok(())
        })
        .unwrap();
        assert!(persisted);
        assert_eq!(state.record.as_ref().unwrap().access_token, "new");
    }

    #[test]
    fn old_refresh_cannot_replace_new_session() {
        let mut state = AuthState {
            generation: 5,
            ..Default::default()
        };
        let record = CredentialRecord {
            access_token: "old".into(),
            refresh_token: None,
            account_id: None,
            expires_at: Some(now() + 60),
        };
        assert!(
            publish_record(&mut state, 4, record, |_| panic!(
                "stale token must not persist"
            ))
            .is_err()
        );
        assert!(state.record.is_none());
    }

    #[test]
    fn refresh_rotation_preserves_omitted_refresh_token() {
        let previous = CredentialRecord {
            access_token: "old".into(),
            refresh_token: Some("refresh".into()),
            account_id: Some("account".into()),
            expires_at: Some(now() + 60),
        };
        let record = credential_from_json(
            &serde_json::json!({"access_token":"new", "expires_in":3600}),
            Some(&previous),
        )
        .unwrap();
        assert_eq!(record.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(record.account_id.as_deref(), Some("account"));
        assert!(credential_from_json(&serde_json::json!({"access_token":"secret"}), None).is_err());
    }
    #[tokio::test]
    async fn concurrent_requests_share_a_single_refresh() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            tokio::time::sleep(Duration::from_millis(20)).await;
            let body =
                r#"{"access_token":"new-token","refresh_token":"rotated","expires_in":3600}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let mut service = ProviderService::new(root.path()).unwrap();
        service.token_override = Some(endpoint);
        let mut profile = profile();
        profile.kind = ProviderKind::ChatGpt;
        service.slot(&profile).unwrap().state.lock().unwrap().record = Some(CredentialRecord {
            access_token: "expired".into(),
            refresh_token: Some("refresh".into()),
            account_id: None,
            expires_at: Some(now() - 1),
        });
        let (first, second) =
            tokio::join!(service.credential(&profile), service.credential(&profile));
        assert_eq!(first.unwrap().access_token, "new-token");
        assert_eq!(second.unwrap().refresh_token.as_deref(), Some("rotated"));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn logout_during_refresh_cannot_restore_credentials() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/token", listener.local_addr().unwrap());
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let release = Arc::new(tokio::sync::Notify::new());
        let server_release = release.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            started_tx.send(()).unwrap();
            server_release.notified().await;
            let body = r#"{"access_token":"late-token","expires_in":3600}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let mut service = ProviderService::new(root.path()).unwrap();
        service.token_override = Some(endpoint);
        let mut profile = profile();
        profile.kind = ProviderKind::ChatGpt;
        service.slot(&profile).unwrap().state.lock().unwrap().record = Some(CredentialRecord {
            access_token: "expired".into(),
            refresh_token: Some("refresh".into()),
            account_id: None,
            expires_at: Some(now() - 1),
        });
        let worker_service = service.clone();
        let worker_profile = profile.clone();
        let worker = tokio::spawn(async move { worker_service.credential(&worker_profile).await });
        started_rx.await.unwrap();
        service.logout(&profile).unwrap();
        release.notify_one();
        assert!(worker.await.unwrap().is_err());
        assert!(!service.status(&profile).unwrap());
        server.await.unwrap();
    }
    #[test]
    fn chatgpt_catalog_only_includes_visible_entries() {
        let models = chatgpt_model_ids(&serde_json::json!({"models":[
            {"slug":"visible-model","visibility":"list"},
            {"slug":"hidden-model","visibility":"hide"},
            {"slug":"","visibility":"list"}
        ]}))
        .unwrap();
        assert_eq!(models, vec!["visible-model"]);
        assert_eq!(normalize_codex_version("rust-v0.167.0"), Some("0.167.0"));
        assert!(normalize_codex_version("../invalid").is_none());
    }
    #[test]
    fn integration_secrets_are_namespaced_and_removed() {
        let root = tempfile::tempdir().unwrap();
        let service = ProviderService::new(root.path()).unwrap();
        let profile = profile();
        service.set_api_key(&profile, "provider-key").unwrap();
        service
            .set_integration_secret(&profile.id, r#"{"TOKEN":"mcp-key"}"#)
            .unwrap();
        assert_eq!(
            service.integration_secret(&profile.id).unwrap().as_deref(),
            Some(r#"{"TOKEN":"mcp-key"}"#)
        );
        service.delete_integration_secret(&profile.id).unwrap();
        assert!(service.integration_secret(&profile.id).unwrap().is_none());
        assert!(service.status(&profile).unwrap());
        assert!(
            service
                .set_integration_secret("../invalid", "value")
                .is_err()
        );
        service.set_integration_secret("empty", "value").unwrap();
        service.set_integration_secret("empty", "").unwrap();
        assert!(service.integration_secret("empty").unwrap().is_none());
    }
}
