use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::ActivitySnapshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    McpRequest,
    ExecSession,
    ProviderTurn,
    ProviderOperation,
}

impl ActivityKind {
    fn label(self) -> &'static str {
        match self {
            Self::McpRequest => "MCP request",
            Self::ExecSession => "exec session",
            Self::ProviderTurn => "provider turn",
            Self::ProviderOperation => "provider operation",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActivityBlocked {
    workspace_id: String,
    kind: ActivityKind,
}

impl fmt::Display for ActivityBlocked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "workspace {} is draining; new {} work is temporarily blocked",
            self.workspace_id,
            self.kind.label()
        )
    }
}

impl std::error::Error for ActivityBlocked {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SwitchCheck {
    pub allowed: bool,
    pub reasons: Vec<String>,
    pub activity: ActivitySnapshot,
}

#[derive(Debug, Clone)]
struct WorkspaceCounters {
    active_mcp_requests: u32,
    running_exec_sessions: u32,
    provider_turn_guards: u32,
    observed_provider_turns: u32,
    provider_operation_guards: u32,
    observed_provider_operations: u32,
    provider_activity_known: bool,
    drain_requested: bool,
}

impl Default for WorkspaceCounters {
    fn default() -> Self {
        Self {
            active_mcp_requests: 0,
            running_exec_sessions: 0,
            provider_turn_guards: 0,
            observed_provider_turns: 0,
            provider_operation_guards: 0,
            observed_provider_operations: 0,
            provider_activity_known: true,
            drain_requested: false,
        }
    }
}

impl WorkspaceCounters {
    fn snapshot(&self, workspace_id: &str) -> ActivitySnapshot {
        ActivitySnapshot {
            workspace_id: workspace_id.to_string(),
            active_mcp_requests: self.active_mcp_requests,
            running_exec_sessions: self.running_exec_sessions,
            active_provider_turns: self.provider_turn_guards.max(self.observed_provider_turns),
            pending_provider_operations: self
                .provider_operation_guards
                .max(self.observed_provider_operations),
            provider_activity_known: self.provider_activity_known,
            drain_requested: self.drain_requested,
        }
    }

    fn idle_and_open(&self) -> bool {
        self.active_mcp_requests == 0
            && self.running_exec_sessions == 0
            && self.provider_turn_guards == 0
            && self.observed_provider_turns == 0
            && self.provider_operation_guards == 0
            && self.observed_provider_operations == 0
            && self.provider_activity_known
            && !self.drain_requested
    }
}

#[derive(Clone, Default)]
pub struct ActivityCoordinator {
    inner: Arc<Mutex<HashMap<String, WorkspaceCounters>>>,
}

impl ActivityCoordinator {
    pub fn acquire(
        &self,
        workspace_id: &str,
        kind: ActivityKind,
    ) -> Result<ActivityGuard, ActivityBlocked> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let counters = inner.entry(workspace_id.to_string()).or_default();
        if counters.drain_requested {
            return Err(ActivityBlocked {
                workspace_id: workspace_id.to_string(),
                kind,
            });
        }
        match kind {
            ActivityKind::McpRequest => {
                counters.active_mcp_requests = counters.active_mcp_requests.saturating_add(1)
            }
            ActivityKind::ExecSession => {
                counters.running_exec_sessions = counters.running_exec_sessions.saturating_add(1)
            }
            ActivityKind::ProviderTurn => {
                counters.provider_turn_guards = counters.provider_turn_guards.saturating_add(1)
            }
            ActivityKind::ProviderOperation => {
                counters.provider_operation_guards =
                    counters.provider_operation_guards.saturating_add(1)
            }
        }
        drop(inner);
        Ok(ActivityGuard {
            coordinator: self.clone(),
            workspace_id: workspace_id.to_string(),
            kind,
            released: false,
        })
    }

    pub fn snapshot(&self, workspace_id: &str) -> ActivitySnapshot {
        let inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        inner
            .get(workspace_id)
            .cloned()
            .unwrap_or_default()
            .snapshot(workspace_id)
    }

    pub fn can_switch(&self, workspace_id: &str) -> SwitchCheck {
        let activity = self.snapshot(workspace_id);
        let mut reasons = Vec::new();
        if activity.active_mcp_requests > 0 {
            reasons.push(format!(
                "{} MCP request(s) active",
                activity.active_mcp_requests
            ));
        }
        if activity.running_exec_sessions > 0 {
            reasons.push(format!(
                "{} exec session(s) running",
                activity.running_exec_sessions
            ));
        }
        if activity.active_provider_turns > 0 {
            reasons.push(format!(
                "{} provider turn(s) active",
                activity.active_provider_turns
            ));
        }
        if activity.pending_provider_operations > 0 {
            reasons.push(format!(
                "{} provider operation(s) pending",
                activity.pending_provider_operations
            ));
        }
        if !activity.provider_activity_known {
            reasons.push("provider activity cannot be proven idle".into());
        }
        SwitchCheck {
            allowed: reasons.is_empty(),
            reasons,
            activity,
        }
    }

    pub fn begin_drain(&self, workspace_id: &str) -> SwitchCheck {
        {
            let mut inner = self
                .inner
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            inner
                .entry(workspace_id.to_string())
                .or_default()
                .drain_requested = true;
        }
        self.can_switch(workspace_id)
    }

    pub fn open_gate(&self, workspace_id: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(counters) = inner.get_mut(workspace_id) {
            counters.drain_requested = false;
            if counters.idle_and_open() {
                inner.remove(workspace_id);
            }
        }
    }

    pub fn set_provider_activity(
        &self,
        workspace_id: &str,
        active_turns: u32,
        pending: u32,
        activity_known: bool,
    ) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let counters = inner.entry(workspace_id.to_string()).or_default();
        counters.observed_provider_turns = active_turns;
        counters.observed_provider_operations = pending;
        counters.provider_activity_known = activity_known;
        if counters.idle_and_open() {
            inner.remove(workspace_id);
        }
    }

    pub fn remove_workspace(&self, workspace_id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(workspace_id);
    }

    fn release(&self, workspace_id: &str, kind: ActivityKind) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(counters) = inner.get_mut(workspace_id) else {
            return;
        };
        match kind {
            ActivityKind::McpRequest => {
                counters.active_mcp_requests = counters.active_mcp_requests.saturating_sub(1)
            }
            ActivityKind::ExecSession => {
                counters.running_exec_sessions = counters.running_exec_sessions.saturating_sub(1)
            }
            ActivityKind::ProviderTurn => {
                counters.provider_turn_guards = counters.provider_turn_guards.saturating_sub(1)
            }
            ActivityKind::ProviderOperation => {
                counters.provider_operation_guards =
                    counters.provider_operation_guards.saturating_sub(1)
            }
        }
        if counters.idle_and_open() {
            inner.remove(workspace_id);
        }
    }
}

pub struct ActivityGuard {
    coordinator: ActivityCoordinator,
    workspace_id: String,
    kind: ActivityKind,
    released: bool,
}

impl ActivityGuard {
    pub fn release(mut self) {
        if !self.released {
            self.coordinator.release(&self.workspace_id, self.kind);
            self.released = true;
        }
    }
}

impl Drop for ActivityGuard {
    fn drop(&mut self) {
        if !self.released {
            self.coordinator.release(&self.workspace_id, self.kind);
            self.released = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_guard_drop_releases_counter() {
        let coordinator = ActivityCoordinator::default();
        {
            let _guard = coordinator
                .acquire("workspace-a", ActivityKind::McpRequest)
                .expect("activity guard");
            assert_eq!(coordinator.snapshot("workspace-a").active_mcp_requests, 1);
        }
        assert_eq!(coordinator.snapshot("workspace-a").active_mcp_requests, 0);
    }

    #[test]
    fn drain_gate_blocks_new_work_and_reports_busy_reason() {
        let coordinator = ActivityCoordinator::default();
        let _guard = coordinator
            .acquire("workspace-a", ActivityKind::ExecSession)
            .expect("exec guard");
        let check = coordinator.begin_drain("workspace-a");
        assert!(!check.allowed);
        assert_eq!(check.activity.running_exec_sessions, 1);
        assert!(coordinator
            .acquire("workspace-a", ActivityKind::McpRequest)
            .is_err());
        coordinator.open_gate("workspace-a");
    }

    #[test]
    fn provider_turns_block_workspace_switching() {
        let coordinator = ActivityCoordinator::default();
        coordinator.set_provider_activity("workspace-a", 2, 0, true);
        let check = coordinator.can_switch("workspace-a");
        assert!(!check.allowed);
        assert_eq!(check.activity.active_provider_turns, 2);
        assert!(check
            .reasons
            .iter()
            .any(|reason| reason.contains("provider turn")));
    }

    #[test]
    fn unknown_provider_activity_fails_closed() {
        let coordinator = ActivityCoordinator::default();
        coordinator.set_provider_activity("workspace-a", 0, 0, false);
        let check = coordinator.can_switch("workspace-a");
        assert!(!check.allowed);
        assert!(!check.activity.provider_activity_known);
        assert!(check
            .reasons
            .iter()
            .any(|reason| reason.contains("cannot be proven idle")));
    }

    #[test]
    fn known_idle_provider_allows_switch_when_other_activity_is_zero() {
        let coordinator = ActivityCoordinator::default();
        coordinator.set_provider_activity("workspace-a", 0, 0, true);
        let check = coordinator.can_switch("workspace-a");
        assert!(check.allowed);
        assert!(check.activity.provider_activity_known);
    }
}
