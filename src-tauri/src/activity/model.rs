use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActiveWorkspacePhase {
    None,
    Activating,
    Active,
    Draining,
    Switching,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveWorkspaceState {
    pub workspace_id: Option<String>,
    pub phase: ActiveWorkspacePhase,
    pub generation: u64,
    pub since_unix_ms: Option<u64>,
    pub message: Option<String>,
}

impl Default for ActiveWorkspaceState {
    fn default() -> Self {
        Self {
            workspace_id: None,
            phase: ActiveWorkspacePhase::None,
            generation: 0,
            since_unix_ms: None,
            message: None,
        }
    }
}

impl ActiveWorkspaceState {
    pub fn transition(
        &mut self,
        workspace_id: Option<String>,
        phase: ActiveWorkspacePhase,
        message: Option<String>,
    ) {
        self.workspace_id = workspace_id;
        self.phase = phase;
        self.generation = self.generation.saturating_add(1);
        self.since_unix_ms = Some(now_unix_ms());
        self.message = message;
    }

    pub fn set_active(&mut self, workspace_id: impl Into<String>) {
        self.transition(
            Some(workspace_id.into()),
            ActiveWorkspacePhase::Active,
            None,
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshot {
    pub workspace_id: String,
    pub active_mcp_requests: u32,
    pub running_exec_sessions: u32,
    pub active_provider_turns: u32,
    pub pending_provider_operations: u32,
    pub provider_activity_known: bool,
    pub drain_requested: bool,
}

impl ActivitySnapshot {
    pub fn is_busy(&self) -> bool {
        self.active_mcp_requests > 0
            || self.running_exec_sessions > 0
            || self.active_provider_turns > 0
            || self.pending_provider_operations > 0
            || !self.provider_activity_known
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
