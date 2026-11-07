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
    pub personality: String,
    pub instructions: String,
    pub color: String,
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
    pub orbit_id: Option<String>,
    pub project_id: Option<String>,
    pub content: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Routine {
    pub id: String,
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
