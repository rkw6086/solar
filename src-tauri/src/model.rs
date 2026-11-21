use serde::{Deserialize, Serialize};
use serde_json::Value;
use solar_providers::ProviderProfile;

pub fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Orbit {
    pub id: String,
    pub name: String,
    pub role: String,
    #[serde(default)]
    pub tasks: String,
    pub personality: String,
    pub instructions: String,
    pub color: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_seed: Option<String>,
    pub model: String,
    pub provider_id: String,
    pub autonomy: String,
    pub tool_grants: Vec<String>,
    pub project_ids: Vec<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub enabled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub description: String,
}

/// An explicitly selected live link, available to all or selected Sprites.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedFolder {
    pub id: String,
    pub name: String,
    pub directory: String,
    pub mount_path: String,
    #[serde(default)]
    pub all_sprites: bool,
    #[serde(default)]
    pub orbit_ids: Vec<String>,
    #[serde(default)]
    pub writable: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub orbit_ids: Vec<String>,
    pub lead_orbit_id: String,
    pub project_id: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub conversation_id: String,
    pub orbit_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_activity: Option<ToolActivity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: String,
    pub patch: String,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivity {
    pub task_id: String,
    pub call_id: String,
    pub name: String,
    pub status: String,
    pub arguments: Value,
    pub output: String,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i64>,
    #[serde(default)]
    pub diffs: Vec<FileDiff>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub conversation_id: String,
    pub orbit_id: String,
    pub status: String,
    pub summary: String,
    pub error: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub user_message_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Approval {
    pub id: String,
    pub task_id: String,
    pub orbit_id: String,
    pub tool: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionItem {
    pub id: String,
    pub orbit_id: String,
    pub task_id: String,
    pub conversation_id: String,
    pub kind: String,
    pub title: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub id: String,
    #[serde(default)]
    pub orbit_ids: Vec<String>,
    #[serde(default)]
    pub project_ids: Vec<String>,
    pub orbit_id: Option<String>,
    pub project_id: Option<String>,
    pub content: String,
    pub created_at: String,
}

impl Memory {
    pub fn orbit_ids(&self) -> Vec<String> {
        let mut ids = self.orbit_ids.clone();
        ids.extend(self.orbit_id.iter().cloned());
        ids.sort();
        ids.dedup();
        ids
    }
    pub fn project_ids(&self) -> Vec<String> {
        let mut ids = self.project_ids.clone();
        ids.extend(self.project_id.iter().cloned());
        ids.sort();
        ids.dedup();
        ids
    }
    pub fn is_system(&self) -> bool {
        self.orbit_ids().is_empty() && self.project_ids().is_empty()
    }
    pub fn visible_to(&self, orbit_id: &str, project_ids: &[String]) -> bool {
        self.is_system()
            || self.orbit_ids().iter().any(|id| id == orbit_id)
            || self.project_ids().iter().any(|id| project_ids.contains(id))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cron: String,
    pub orbit_id: String,
    pub prompt: String,
    pub interval_minutes: u32,
    pub enabled: bool,
    pub project_id: Option<String>,
    pub trigger: String,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub observed_signature: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub desktop_enabled: bool,
    pub reduced_motion: bool,
    pub max_concurrent: usize,
    pub experimental_claude: bool,
    pub attention_sound: bool,
    pub attention_volume: f32,
    pub onboarding_complete: bool,
    pub onboarding_step: String,
    #[serde(alias = "onboardingMusic")]
    pub sound_effects: bool,
    #[serde(alias = "onboardingVolume")]
    pub sound_effects_volume: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            desktop_enabled: true,
            reduced_motion: false,
            max_concurrent: 2,
            experimental_claude: false,
            attention_sound: true,
            attention_volume: 0.35,
            onboarding_complete: false,
            onboarding_step: "birth".into(),
            sound_effects: true,
            sound_effects_volume: 0.25,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    pub url: Option<String>,
    pub enabled: bool,
    #[serde(default)]
    pub connected: bool,
    #[serde(default)]
    pub has_credentials: bool,
    #[serde(default)]
    pub revision: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderView {
    #[serde(flatten)]
    pub profile: ProviderProfile,
    pub connected: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub orbits: Vec<Orbit>,
    pub providers: Vec<ProviderView>,
    pub projects: Vec<Project>,
    pub shared_folders: Vec<SharedFolder>,
    pub conversations: Vec<Conversation>,
    pub routines: Vec<Routine>,
    pub tasks: Vec<Task>,
    pub approvals: Vec<Approval>,
    pub settings: Settings,
    pub mcp_servers: Vec<McpServer>,
    pub sharing_orbit_ids: Vec<String>,
    pub desktop_expanded: bool,
    pub attention_items: Vec<AttentionItem>,
}

#[derive(Clone, Debug, Serialize)]
pub enum AppEvent {
    State,
    Message(Message),
    Task(Task),
    Approval(Approval),
    Attention,
}
