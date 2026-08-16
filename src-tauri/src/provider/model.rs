use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Status,
    Sessions,
    StartTask,
    SendInput,
    CancelTask,
    Compaction,
    Drain,
    Resume,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderDescriptor {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<ProviderCapability>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderState {
    Unavailable,
    Ready,
    Busy,
    VersionMismatch,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub provider_id: String,
    pub state: ProviderState,
    pub configured: bool,
    pub activity_known: bool,
    pub version: Option<String>,
    pub mode: Option<String>,
    pub pid: Option<u32>,
    pub accepting_tasks: Option<bool>,
    pub active_turns: u32,
    pub active_http_turns: u32,
    pub active_browser_turns: u32,
    pub session_ready: Option<bool>,
    pub message: String,
}

impl ProviderStatus {
    pub fn not_configured(provider_id: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            state: ProviderState::Unavailable,
            configured: false,
            activity_known: true,
            version: None,
            mode: None,
            pid: None,
            accepting_tasks: None,
            active_turns: 0,
            active_http_turns: 0,
            active_browser_turns: 0,
            session_ready: None,
            message: message.into(),
        }
    }
}
