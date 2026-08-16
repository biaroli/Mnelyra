use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskSessionState {
    Queued,
    Starting,
    Running,
    WaitingForUser,
    WaitingForTool,
    Compacting,
    Draining,
    Completed,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskSession {
    pub id: String,
    pub workspace_id: String,
    pub canonical_workspace_path: String,
    pub provider_id: String,
    pub provider_session_id: Option<String>,
    pub title: String,
    pub state: TaskSessionState,
    pub created_at: String,
    pub updated_at: String,
    pub last_activity_at: String,
}
