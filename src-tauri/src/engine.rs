use crate::{model::*, store::Store};
use serde::{Deserialize, Serialize};
use solar_providers::{ProviderKind, ProviderProfile, ProviderService};
use solar_tools::{McpConfig, McpTransport, ToolBroker};
use std::collections::BTreeMap;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::{Notify, broadcast, oneshot};
use tokio_util::sync::CancellationToken;

pub struct Engine {
    pub(crate) store: Store,
    pub(crate) providers: ProviderService,
    pub(crate) tools: ToolBroker,
    pub(crate) data_dir: PathBuf,
    pub(crate) events: broadcast::Sender<AppEvent>,
    pub(crate) active: Mutex<HashMap<String, CancellationToken>>,
    pub(crate) approvals: Mutex<HashMap<String, (Approval, oneshot::Sender<bool>)>>,
    pub(crate) sharing: Mutex<HashSet<String>>,
    pub(crate) orbit_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    pub(crate) model_active: Mutex<usize>,
    pub(crate) model_notify: Notify,
    conversation_creation: Mutex<()>,
    mcp_connected: Mutex<HashSet<String>>,
    pub(crate) desktop_expanded: std::sync::atomic::AtomicBool,
    pub(crate) attention: Mutex<crate::attention::AttentionQueue>,
    pub(crate) settings_write: Mutex<()>,
    pub(crate) resetting: std::sync::atomic::AtomicBool,
    pub(crate) reset_operations: tokio::sync::RwLock<()>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDraft {
    #[serde(flatten)]
    pub profile: ProviderProfile,
    pub api_key: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct McpCredentials {
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerDraft {
    #[serde(flatten)]
    server: McpServer,
    env: Option<BTreeMap<String, String>>,
    headers: Option<BTreeMap<String, String>>,
    #[serde(default)]
    clear_credentials: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrbitExport {
    schema_version: u32,
    orbit: Orbit,
}

pub(crate) fn validate_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 80
        || !value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("Invalid identifier".into());
    }
    Ok(())
}

pub(crate) fn validate_text(value: &str, name: &str, max: usize) -> Result<(), String> {
    if value.trim().is_empty() || value.len() > max {
        return Err(format!("{name} must contain between 1 and {max} bytes"));
    }
    Ok(())
}

impl Engine {
    pub fn open(data_dir: PathBuf) -> Result<Arc<Self>, String> {
        std::fs::create_dir_all(&data_dir).map_err(|e| e.to_string())?;
        let settings_store = Store::open_profile(&data_dir)?;
        let (events, _) = broadcast::channel(256);
        let engine = Arc::new(Self {
            store: settings_store,
            providers: ProviderService::new(),
            tools: ToolBroker::new(data_dir.clone())?,
            data_dir,
            events,
            active: Mutex::new(HashMap::new()),
            approvals: Mutex::new(HashMap::new()),
            sharing: Mutex::new(HashSet::new()),
            orbit_locks: Mutex::new(HashMap::new()),
            model_active: Mutex::new(0),
            model_notify: Notify::new(),
            conversation_creation: Mutex::new(()),
            mcp_connected: Mutex::new(HashSet::new()),
            desktop_expanded: std::sync::atomic::AtomicBool::new(false),
            attention: Mutex::new(crate::attention::AttentionQueue::default()),
            settings_write: Mutex::new(()),
            resetting: std::sync::atomic::AtomicBool::new(false),
            reset_operations: tokio::sync::RwLock::new(()),
        });
        engine.seed()?;
        engine.recover()?;
        Ok(engine)
    }

    fn seed(&self) -> Result<(), String> {
        if self.store.get::<bool>("meta", "initialized")?.is_some() {
            return Ok(());
        }
        if !self.store.list::<Orbit>("orbits")?.is_empty() {
            return self.store.put("meta", "initialized", &true);
        }
        let tools = ["files", "web", "memory", "collaboration"];
        for (key, name, role, personality, color) in [
            (
                "assistant",
                "Sol",
                "Personal assistant",
                "Warm, organized, thoughtful. Turn open loops into clear next steps.",
                "#f5b45b",
            ),
            (
                "researcher",
                "Lyra",
                "Researcher",
                "Curious, rigorous, clear about uncertainty. Cite primary sources and distinguish evidence from inference.",
                "#8fdac6",
            ),
            (
                "qa",
                "Nova",
                "Quality assurance",
                "Methodical, observant, constructive. Reproduce issues and report exact evidence.",
                "#b5a1ec",
            ),
        ] {
            let orbit = Orbit {
                id: key.into(),
                name: name.into(),
                role: role.into(),
                personality: personality.into(),
                instructions: String::new(),
                color: color.into(),
                model: String::new(),
                provider_id: String::new(),
                autonomy: "workspace".into(),
                tool_grants: tools.iter().map(|s| (*s).into()).collect(),
                project_ids: Vec::new(),
                x: None,
                y: None,
                enabled: true,
            };
            self.store.put("orbits", key, &orbit)?;
        }
        if self.store.get::<Settings>("settings", "app")?.is_none() {
            self.store.put("settings", "app", &Settings::default())?;
        }
        self.store.put("meta", "initialized", &true)
    }

    fn recover(&self) -> Result<(), String> {
        for mut task in self.store.list::<Task>("tasks")? {
            if ["queued", "running", "waiting"].contains(&task.status.as_str()) {
                let recovery_error = self.recover_history(&task).err();
                task.status = "interrupted".into();
                task.error = Some(recovery_error.map(|error|format!("This task could not be restored: {error}. Review its activity before continuing.")).unwrap_or_else(||"Solar stopped before this task finished. Review its activity before starting a new request. External actions have not been replayed.".into()));
                self.store.put("tasks", &task.id, &task)?;
            }
        }
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.events.subscribe()
    }
    pub(crate) fn ensure_running(&self) -> Result<(), String> {
        if self.resetting.load(std::sync::atomic::Ordering::SeqCst) {
            Err("Solar is resetting. Wait for setup to restart.".into())
        } else {
            Ok(())
        }
    }
    pub(crate) fn changed(&self) {
        let _ = self.events.send(AppEvent::State);
    }
    pub fn settings(&self) -> Result<Settings, String> {
        Ok(self.store.get("settings", "app")?.unwrap_or_default())
    }

    pub fn bootstrap(&self) -> Result<Bootstrap, String> {
        let providers = self
            .store
            .list::<ProviderProfile>("providers")?
            .into_iter()
            .map(|profile| {
                let connected = self.providers.status(&profile).unwrap_or(false);
                ProviderView { profile, connected }
            })
            .collect();
        Ok(Bootstrap {
            orbits: self.store.list("orbits")?,
            providers,
            projects: self.store.list("projects")?,
            conversations: self.store.list("conversations")?,
            routines: self.store.list("routines")?,
            tasks: self
                .store
                .list::<Task>("tasks")?
                .into_iter()
                .take(100)
                .collect(),
            approvals: self
                .approvals
                .lock()
                .map_err(|_| "Approval lock unavailable")?
                .values()
                .map(|(a, _)| a.clone())
                .collect(),
            settings: self.settings()?,
            mcp_servers: self
                .store
                .list::<McpServer>("mcp")?
                .into_iter()
                .map(|mut server| {
                    server.connected = self
                        .mcp_connected
                        .lock()
                        .map(|ids| ids.contains(&server.id))
                        .unwrap_or(false);
                    server
                })
                .collect(),
            sharing_orbit_ids: self
                .sharing
                .lock()
                .map_err(|_| "Sharing lock unavailable")?
                .iter()
                .cloned()
                .collect(),
            desktop_expanded: self
                .desktop_expanded
                .load(std::sync::atomic::Ordering::SeqCst),
            attention_items: self.attention_items()?,
        })
    }

    pub fn save_orbit(&self, mut orbit: Orbit) -> Result<Orbit, String> {
        self.ensure_running()?;
        if orbit.id.is_empty() {
            orbit.id = id();
        }
        validate_id(&orbit.id)?;
        validate_text(&orbit.name, "Name", 100)?;
        validate_text(&orbit.role, "Role", 300)?;
        if orbit.instructions.len() > 32_000 || orbit.personality.len() > 8_000 {
            return Err("Orbit instructions are too long".into());
        }
        if !["ask", "workspace", "trusted"].contains(&orbit.autonomy.as_str()) {
            return Err("Unknown autonomy setting".into());
        }
        if orbit.x.is_some_and(|n| !n.is_finite()) || orbit.y.is_some_and(|n| !n.is_finite()) {
            return Err("Invalid desktop position".into());
        }
        for project in &orbit.project_ids {
            self.project(project)?;
        }
        if !orbit.provider_id.is_empty() {
            self.provider(&orbit.provider_id)?;
        }
        if self
            .store
            .get::<Orbit>("orbits", &orbit.id)?
            .is_some_and(|previous| {
                !orbit.enabled
                    || previous.provider_id != orbit.provider_id
                    || previous.model != orbit.model
                    || previous
                        .tool_grants
                        .iter()
                        .any(|grant| !orbit.tool_grants.contains(grant))
                    || previous
                        .project_ids
                        .iter()
                        .any(|project| !orbit.project_ids.contains(project))
                    || (previous.autonomy == "trusted" && orbit.autonomy != "trusted")
                    || (previous.autonomy == "workspace" && orbit.autonomy == "ask")
            })
        {
            self.cancel_orbit_tasks(&orbit.id)?;
        }
        self.store.put("orbits", &orbit.id, &orbit)?;
        std::fs::create_dir_all(self.workspace(&orbit.id)?).map_err(|e| e.to_string())?;
        if !orbit.enabled {
            self.clear_orbit_attention(&orbit.id);
        }
        self.changed();
        Ok(orbit)
    }

    pub fn delete_orbit(&self, orbit_id: &str) -> Result<(), String> {
        self.orbit(orbit_id)?;
        for task in self.store.list::<Task>("tasks")? {
            if task.orbit_id == orbit_id {
                self.stop_task(&task.id)?;
            }
        }
        self.store.remove("orbits", orbit_id)?;
        self.clear_orbit_attention(orbit_id);
        self.changed();
        Ok(())
    }

    fn cancel_orbit_tasks(&self, orbit_id: &str) -> Result<(), String> {
        for task in self.store.list::<Task>("tasks")? {
            if task.orbit_id == orbit_id {
                self.stop_task(&task.id)?;
            }
        }
        Ok(())
    }

    fn cancel_provider_tasks(&self, provider_id: &str) -> Result<(), String> {
        for orbit in self.store.list::<Orbit>("orbits")? {
            if orbit.provider_id == provider_id {
                self.cancel_orbit_tasks(&orbit.id)?;
            }
        }
        Ok(())
    }

    pub fn orbit(&self, orbit_id: &str) -> Result<Orbit, String> {
        self.store
            .get("orbits", orbit_id)?
            .ok_or_else(|| "Orbit no longer exists".into())
    }
    pub fn provider(&self, provider_id: &str) -> Result<ProviderProfile, String> {
        self.store
            .get("providers", provider_id)?
            .ok_or_else(|| "Connect a model provider in Settings first".into())
    }
    pub fn project(&self, project_id: &str) -> Result<Project, String> {
        self.store
            .get("projects", project_id)?
            .ok_or_else(|| "Project no longer exists".into())
    }
    pub(crate) fn workspace(&self, orbit_id: &str) -> Result<PathBuf, String> {
        validate_id(orbit_id)?;
        Ok(self.store.orbit_dir(orbit_id)?.join("workspace"))
    }

    pub fn save_provider(&self, mut draft: ProviderDraft) -> Result<ProviderView, String> {
        self.ensure_running()?;
        if draft.profile.id.is_empty() {
            draft.profile.id = id();
        }
        validate_id(&draft.profile.id)?;
        validate_text(&draft.profile.name, "Provider name", 100)?;
        if matches!(
            draft.profile.kind,
            ProviderKind::OpenAi | ProviderKind::LiteLlm
        ) {
            let endpoint = url::Url::parse(&draft.profile.base_url)
                .map_err(|_| "Enter a valid provider URL")?;
            if !["http", "https"].contains(&endpoint.scheme())
                || endpoint.host_str().is_none()
                || !endpoint.username().is_empty()
                || endpoint.password().is_some()
                || endpoint.query().is_some()
                || endpoint.fragment().is_some()
            {
                return Err("Provider URL must use HTTP(S) without embedded credentials, query, or fragment".into());
            }
        }
        if draft.profile.kind == ProviderKind::Claude && !self.settings()?.experimental_claude {
            return Err("Enable experimental Claude subscriptions and acknowledge the provider warning first".into());
        }
        if self
            .store
            .get::<ProviderProfile>("providers", &draft.profile.id)?
            .is_some()
        {
            self.cancel_provider_tasks(&draft.profile.id)?;
        }
        if let Some(key) = draft.api_key.filter(|s| !s.trim().is_empty()) {
            self.providers.set_api_key(&draft.profile, &key)?;
        }
        self.store
            .put("providers", &draft.profile.id, &draft.profile)?;
        let connected = self.providers.status(&draft.profile)?;
        self.changed();
        Ok(ProviderView {
            profile: draft.profile,
            connected,
        })
    }

    pub async fn discover_models(&self, provider_id: &str) -> Result<Vec<String>, String> {
        let _operation = self.reset_operations.read().await;
        self.ensure_running()?;
        let profile = self.provider(provider_id)?;
        let models = self.providers.discover_models(&profile).await?;
        let mut current = self.provider(provider_id)?;
        if current.kind != profile.kind || current.base_url != profile.base_url {
            return Err("Provider changed while model discovery was running. Try again.".into());
        }
        if !models.is_empty() {
            current.models = models.clone();
            self.store.put("providers", provider_id, &current)?;
        }
        self.changed();
        Ok(models)
    }

    pub async fn start_login(&self, provider_id: &str) -> Result<serde_json::Value, String> {
        let _operation = self.reset_operations.read().await;
        self.ensure_running()?;
        let profile = self.provider(provider_id)?;
        if profile.kind == ProviderKind::Claude && !self.settings()?.experimental_claude {
            return Err("Experimental Claude subscriptions are disabled".into());
        }
        let start = self.providers.begin_login(&profile).await?;
        serde_json::to_value(start).map_err(|e| e.to_string())
    }

    pub fn logout_provider(&self, provider_id: &str) -> Result<(), String> {
        self.cancel_provider_tasks(provider_id)?;
        self.providers.logout(&self.provider(provider_id)?)?;
        self.changed();
        Ok(())
    }

    pub fn save_project(&self, mut project: Project) -> Result<Project, String> {
        self.ensure_running()?;
        if project.id.is_empty() {
            project.id = id();
        }
        validate_id(&project.id)?;
        validate_text(&project.name, "Project name", 200)?;
        let directory = std::fs::canonicalize(&project.directory)
            .map_err(|e| format!("Select an existing project folder: {e}"))?;
        if !directory.is_dir() {
            return Err("The project path must be a folder".into());
        }
        project.directory = directory.to_string_lossy().into_owned();
        if self
            .store
            .get::<Project>("projects", &project.id)?
            .is_some_and(|previous| previous.directory != project.directory)
        {
            for orbit in self.store.list::<Orbit>("orbits")? {
                if orbit.project_ids.contains(&project.id) {
                    self.cancel_orbit_tasks(&orbit.id)?;
                }
            }
        }
        self.store.put("projects", &project.id, &project)?;
        self.changed();
        Ok(project)
    }

    pub fn create_conversation(
        &self,
        orbit_ids: Vec<String>,
        lead_orbit_id: String,
        title: String,
        project_id: Option<String>,
    ) -> Result<Conversation, String> {
        self.ensure_running()?;
        let _lock = self
            .conversation_creation
            .lock()
            .map_err(|_| "Conversation lock unavailable")?;
        let project_id = project_id.filter(|s| !s.is_empty());
        if orbit_ids.is_empty() || !orbit_ids.contains(&lead_orbit_id) {
            return Err("Choose at least one Orbit and a lead from the team".into());
        }
        for orbit_id in &orbit_ids {
            let orbit = self.orbit(orbit_id)?;
            if !orbit.enabled {
                return Err(format!("{} is disabled", orbit.name));
            }
            if let Some(project_id) = project_id.as_ref().filter(|s| !s.is_empty()) {
                self.project(project_id)?;
                if !orbit.project_ids.contains(project_id) {
                    return Err(format!(
                        "Grant {} access to this project before adding it to the conversation",
                        orbit.name
                    ));
                }
            }
        }
        if orbit_ids.len() == 1
            && !title.starts_with("Routine ·")
            && let Some(existing) = self
                .store
                .list::<Conversation>("conversations")?
                .into_iter()
                .find(|c| {
                    c.orbit_ids == orbit_ids
                        && c.project_id == project_id
                        && !c.title.starts_with("Routine ·")
                })
        {
            return Ok(existing);
        }
        let conversation = Conversation {
            id: id(),
            title: if title.trim().is_empty() {
                "New conversation".into()
            } else {
                title
            },
            orbit_ids,
            lead_orbit_id,
            project_id,
            created_at: now(),
        };
        self.store
            .put("conversations", &conversation.id, &conversation)?;
        self.changed();
        Ok(conversation)
    }

    pub fn messages(&self, conversation_id: &str) -> Result<Vec<Message>, String> {
        self.store.messages(conversation_id)
    }
    pub(crate) fn message(
        &self,
        conversation_id: &str,
        orbit_id: Option<String>,
        role: &str,
        content: String,
    ) -> Result<Message, String> {
        let message = Message {
            id: id(),
            conversation_id: conversation_id.into(),
            orbit_id,
            role: role.into(),
            content,
            created_at: now(),
        };
        self.store.save_message(&message)?;
        let _ = self.events.send(AppEvent::Message(message.clone()));
        Ok(message)
    }

    pub fn memories(
        &self,
        orbit_id: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<Vec<Memory>, String> {
        Ok(self
            .store
            .list::<Memory>("memories")?
            .into_iter()
            .filter(|m| {
                (orbit_id.is_some() && m.orbit_id.as_deref() == orbit_id)
                    || (project_id.is_some() && m.project_id.as_deref() == project_id)
            })
            .collect())
    }

    pub fn save_memory(&self, mut memory: Memory) -> Result<Memory, String> {
        self.ensure_running()?;
        if memory.id.is_empty() {
            memory.id = id();
        }
        validate_id(&memory.id)?;
        validate_text(&memory.content, "Memory", 16_000)?;
        match (&memory.orbit_id, &memory.project_id) {
            (Some(orbit_id), None) => {
                self.orbit(orbit_id)?;
            }
            (None, Some(project_id)) => {
                self.project(project_id)?;
            }
            _ => return Err("A memory belongs to exactly one Orbit or shared project".into()),
        }
        if memory.created_at.is_empty() {
            memory.created_at = now();
        }
        self.store.put("memories", &memory.id, &memory)?;
        self.changed();
        Ok(memory)
    }

    pub fn delete_memory(&self, memory_id: &str) -> Result<(), String> {
        self.store.remove("memories", memory_id)?;
        self.changed();
        Ok(())
    }

    pub fn export_orbit(&self, orbit_id: &str) -> Result<String, String> {
        let mut orbit = self.orbit(orbit_id)?;
        orbit.provider_id.clear();
        orbit.project_ids.clear();
        orbit.x = None;
        orbit.y = None;
        serde_json::to_string_pretty(&OrbitExport {
            schema_version: 1,
            orbit,
        })
        .map_err(|e| e.to_string())
    }

    pub fn import_orbit(&self, profile_json: &str) -> Result<Orbit, String> {
        if profile_json.len() > 64_000 {
            return Err("Orbit profile is too large".into());
        }
        let mut export: OrbitExport =
            serde_json::from_str(profile_json).map_err(|e| e.to_string())?;
        if export.schema_version != 1 {
            return Err("Unsupported Orbit profile version".into());
        }
        export.orbit.id = id();
        export.orbit.provider_id.clear();
        export.orbit.project_ids.clear();
        export.orbit.autonomy = "ask".into();
        export.orbit.x = None;
        export.orbit.y = None;
        self.save_orbit(export.orbit)
    }

    pub fn save_settings(&self, mut settings: Settings) -> Result<Settings, String> {
        self.ensure_running()?;
        let _write = self
            .settings_write
            .lock()
            .map_err(|_| "Settings are unavailable")?;
        settings.max_concurrent = settings.max_concurrent.clamp(1, 8);
        settings.attention_volume = if settings.attention_volume.is_finite() {
            settings.attention_volume.clamp(0.0, 1.0)
        } else {
            0.35
        };
        let previous = self.settings()?;
        // The explicit onboarding commands own progress. A settings form that
        // was opened on an earlier slide must not accidentally rewind it.
        settings.onboarding_complete = previous.onboarding_complete;
        settings.onboarding_step = previous.onboarding_step;
        settings.onboarding_volume = if settings.onboarding_volume.is_finite() {
            settings.onboarding_volume.clamp(0.0, 0.5)
        } else {
            0.12
        };
        if previous.experimental_claude && !settings.experimental_claude {
            for profile in self.store.list::<ProviderProfile>("providers")? {
                if profile.kind == ProviderKind::Claude {
                    self.cancel_provider_tasks(&profile.id)?;
                }
            }
        }
        self.store.put("settings", "app", &settings)?;
        self.model_notify.notify_waiters();
        self.model_notify.notify_one();
        self.changed();
        Ok(settings)
    }

    pub fn save_routine(&self, mut routine: Routine) -> Result<Routine, String> {
        self.ensure_running()?;
        if routine.id.is_empty() {
            routine.id = id();
        }
        if routine.last_run_at.is_none() {
            routine.last_run_at = Some(now());
        }
        validate_id(&routine.id)?;
        self.orbit(&routine.orbit_id)?;
        validate_text(&routine.prompt, "Routine prompt", 16_000)?;
        if !["interval", "folder", "completion"].contains(&routine.trigger.as_str()) {
            return Err("Unknown routine trigger".into());
        }
        routine.interval_minutes = routine.interval_minutes.clamp(1, 43_200);
        if routine.trigger == "folder" && routine.project_id.is_none() {
            return Err("Folder routines need a project".into());
        }
        if let Some(project_id) = &routine.project_id {
            self.project(project_id)?;
            if !self
                .orbit(&routine.orbit_id)?
                .project_ids
                .contains(project_id)
            {
                return Err("Grant the Orbit access to this project first".into());
            }
        }
        self.store.put("routines", &routine.id, &routine)?;
        self.changed();
        Ok(routine)
    }

    pub async fn start_screen_share(&self, orbit_id: &str) -> Result<(), String> {
        let _operation = self.reset_operations.read().await;
        self.ensure_running()?;
        self.orbit(orbit_id)?;
        self.tools.start_screen_share(orbit_id).await?;
        self.sharing
            .lock()
            .map_err(|_| "Sharing lock unavailable")?
            .insert(orbit_id.into());
        self.changed();
        Ok(())
    }

    pub async fn stop_screen_share(&self, orbit_id: &str) -> Result<(), String> {
        self.sharing
            .lock()
            .map_err(|_| "Sharing lock unavailable")?
            .remove(orbit_id);
        self.tools.stop_screen_share(orbit_id).await?;
        self.changed();
        Ok(())
    }

    pub async fn save_mcp_server(&self, draft: McpServerDraft) -> Result<McpServer, String> {
        let _operation = self.reset_operations.read().await;
        self.ensure_running()?;
        let mut server = draft.server;
        if server.id.is_empty() {
            server.id = id();
        }
        validate_id(&server.id)?;
        server.revision = self
            .store
            .get::<McpServer>("mcp", &server.id)?
            .map(|s| s.revision)
            .unwrap_or(0)
            .saturating_add(1);
        validate_text(&server.name, "Server name", 100)?;
        if !["stdio", "http"].contains(&server.transport.as_str()) {
            return Err("Unknown MCP transport".into());
        }
        self.cancel_mcp_tasks(&server.id)?;
        self.tools.disconnect_mcp(&server.id).await?;
        self.mcp_connected
            .lock()
            .map_err(|_| "MCP lock unavailable")?
            .remove(&server.id);
        if draft.clear_credentials {
            self.providers.delete_integration_secret(&server.id)?;
        }
        if draft.env.is_some() || draft.headers.is_some() {
            let mut secrets = self
                .providers
                .integration_secret(&server.id)?
                .map(|value| {
                    serde_json::from_str::<McpCredentials>(&value).map_err(|e| e.to_string())
                })
                .transpose()?
                .unwrap_or_default();
            if let Some(env) = draft.env {
                secrets.env = env;
            }
            if let Some(headers) = draft.headers {
                secrets.headers = headers;
            }
            self.providers.set_integration_secret(
                &server.id,
                &serde_json::to_string(&secrets).map_err(|e| e.to_string())?,
            )?;
        }
        server.has_credentials = self.providers.integration_secret(&server.id)?.is_some();
        server.connected = false;
        self.store.put("mcp", &server.id, &server)?;
        self.changed();
        Ok(server)
    }

    pub async fn connect_mcp(&self, server_id: &str) -> Result<usize, String> {
        let _operation = self.reset_operations.read().await;
        self.ensure_running()?;
        let server: McpServer = self
            .store
            .get("mcp", server_id)?
            .ok_or("MCP server not found")?;
        if !server.enabled {
            return Err("Enable this MCP server before connecting".into());
        }
        let revision = server.revision;
        let credentials = self
            .providers
            .integration_secret(server_id)?
            .map(|value| {
                serde_json::from_str::<McpCredentials>(&value)
                    .map_err(|_| "Stored MCP credentials are invalid. Save them again.".to_owned())
            })
            .transpose()?
            .unwrap_or_default();
        let transport = if server.transport == "stdio" {
            McpTransport::Stdio {
                command: server.command.ok_or("A command is required")?,
                args: server.args,
                env: credentials.env,
            }
        } else {
            McpTransport::Http {
                url: server.url.ok_or("A URL is required")?,
                headers: credentials.headers,
            }
        };
        let definitions = self
            .tools
            .connect_mcp(
                McpConfig {
                    id: server.id,
                    name: server.name,
                    transport,
                },
                true,
            )
            .await?;
        if self
            .store
            .get::<McpServer>("mcp", server_id)?
            .is_none_or(|current| !current.enabled || current.revision != revision)
        {
            self.tools.disconnect_mcp(server_id).await?;
            return Err("MCP configuration changed while connecting. Connect again with the current settings.".into());
        }
        self.mcp_connected
            .lock()
            .map_err(|_| "MCP lock unavailable")?
            .insert(server_id.into());
        self.changed();
        Ok(definitions.len())
    }

    pub async fn delete_mcp_server(&self, server_id: &str) -> Result<(), String> {
        self.cancel_mcp_tasks(server_id)?;
        self.tools.disconnect_mcp(server_id).await?;
        self.mcp_connected
            .lock()
            .map_err(|_| "MCP lock unavailable")?
            .remove(server_id);
        self.store.remove("mcp", server_id)?;
        self.providers.delete_integration_secret(server_id)?;
        self.changed();
        Ok(())
    }

    fn cancel_mcp_tasks(&self, server_id: &str) -> Result<(), String> {
        let prefix = format!("mcp__{server_id}__");
        for task in self.store.list::<Task>("tasks")? {
            if task.summary.contains(&prefix) {
                self.stop_task(&task.id)?;
            }
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        if let Ok(active) = self.active.lock() {
            for token in active.values() {
                token.cancel();
            }
        }
        self.tools.shutdown().await;
    }
}
