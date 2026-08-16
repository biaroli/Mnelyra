use serde::Serialize;
use std::fmt;

use crate::activity::{ActiveWorkspacePhase, ActiveWorkspaceState};
use crate::app_state::AppState;
use crate::commands::runtime::{start_mcp_service, stop_mcp_service};
use crate::error::AppError;
use crate::runtime::ServiceKind;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceActivationError {
    pub source_workspace_id: Option<String>,
    pub target_workspace_id: String,
    pub primary_failure: String,
    pub rollback_attempted: bool,
    pub rollback_succeeded: bool,
    pub rollback_failure: Option<String>,
}

pub async fn deactivate_workspace(
    state: &AppState,
    workspace_id: &str,
) -> Result<ActiveWorkspaceState, WorkspaceActivationError> {
    let _activation_guard = state.workspace_activation.lock().await;
    ensure_workspace_exists(state, workspace_id).map_err(|error| {
        WorkspaceActivationError::new(
            Some(workspace_id.to_string()),
            workspace_id,
            error.to_string(),
        )
    })?;

    let current = state.active_workspace_state().map_err(|error| {
        WorkspaceActivationError::new(
            Some(workspace_id.to_string()),
            workspace_id,
            error.to_string(),
        )
    })?;
    let was_authoritative = current.workspace_id.as_deref() == Some(workspace_id);
    if was_authoritative {
        if let Err(error) = state.refresh_provider_activity(workspace_id).await {
            state
                .activity
                .set_provider_activity(workspace_id, 0, 0, false);
            return Err(WorkspaceActivationError::new(
                Some(workspace_id.to_string()),
                workspace_id,
                format!("provider activity probe failed: {error}"),
            ));
        }
        state
            .transition_active_workspace(
                Some(workspace_id.to_string()),
                ActiveWorkspacePhase::Draining,
                Some(format!("Draining {workspace_id} before stop")),
            )
            .map_err(|error| {
                WorkspaceActivationError::new(
                    Some(workspace_id.to_string()),
                    workspace_id,
                    error.to_string(),
                )
            })?;
    }

    let check = state.activity.begin_drain(workspace_id);
    if !check.allowed {
        state.activity.open_gate(workspace_id);
        if was_authoritative {
            let _ = state.set_active_workspace(workspace_id);
        }
        return Err(WorkspaceActivationError::new(
            Some(workspace_id.to_string()),
            workspace_id,
            format!("workspace is busy: {}", check.reasons.join("; ")),
        ));
    }

    if is_mcp_running(state, workspace_id).map_err(|error| {
        WorkspaceActivationError::new(
            Some(workspace_id.to_string()),
            workspace_id,
            error.to_string(),
        )
    })? {
        if let Err(error) = stop_mcp_service(state, workspace_id).await {
            state.activity.open_gate(workspace_id);
            if was_authoritative {
                let _ = state.set_active_workspace(workspace_id);
            }
            return Err(WorkspaceActivationError::new(
                Some(workspace_id.to_string()),
                workspace_id,
                format!("failed to stop workspace: {error}"),
            ));
        }
    }

    state.activity.open_gate(workspace_id);
    if was_authoritative {
        state
            .transition_active_workspace(None, ActiveWorkspacePhase::None, None)
            .map_err(|error| {
                WorkspaceActivationError::new(
                    Some(workspace_id.to_string()),
                    workspace_id,
                    error.to_string(),
                )
            })
    } else {
        state.active_workspace_state().map_err(|error| {
            WorkspaceActivationError::new(
                Some(workspace_id.to_string()),
                workspace_id,
                error.to_string(),
            )
        })
    }
}

impl WorkspaceActivationError {
    fn new(
        source_workspace_id: Option<String>,
        target_workspace_id: impl Into<String>,
        primary_failure: impl Into<String>,
    ) -> Self {
        Self {
            source_workspace_id,
            target_workspace_id: target_workspace_id.into(),
            primary_failure: primary_failure.into(),
            rollback_attempted: false,
            rollback_succeeded: false,
            rollback_failure: None,
        }
    }
}

impl fmt::Display for WorkspaceActivationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "workspace activation failed: {} -> {}: {}",
            self.source_workspace_id.as_deref().unwrap_or("<none>"),
            self.target_workspace_id,
            self.primary_failure
        )?;
        if self.rollback_attempted {
            if self.rollback_succeeded {
                write!(f, "; rollback succeeded")?;
            } else if let Some(failure) = &self.rollback_failure {
                write!(f, "; rollback failed: {failure}")?;
            } else {
                write!(f, "; rollback failed")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for WorkspaceActivationError {}

pub async fn activate_workspace(
    state: &AppState,
    target_id: &str,
) -> Result<ActiveWorkspaceState, WorkspaceActivationError> {
    activate_workspace_with_options(state, target_id, false).await
}

pub async fn activate_workspace_with_options(
    state: &AppState,
    target_id: &str,
    force_restart: bool,
) -> Result<ActiveWorkspaceState, WorkspaceActivationError> {
    let _activation_guard = state.workspace_activation.lock().await;

    ensure_workspace_exists(state, target_id)
        .map_err(|error| WorkspaceActivationError::new(None, target_id, error.to_string()))?;

    let current = state
        .active_workspace_state()
        .map_err(|error| WorkspaceActivationError::new(None, target_id, error.to_string()))?;
    let source_id = current
        .workspace_id
        .clone()
        .or_else(|| find_running_workspace(state, target_id));

    if source_id.as_deref() == Some(target_id) && !force_restart {
        state.activity.open_gate(target_id);
        let running = is_mcp_running(state, target_id).map_err(|error| {
            WorkspaceActivationError::new(source_id.clone(), target_id, error.to_string())
        })?;
        if !running {
            state
                .transition_active_workspace(
                    source_id.clone(),
                    ActiveWorkspacePhase::Activating,
                    Some(format!("Starting {target_id}")),
                )
                .map_err(|error| {
                    WorkspaceActivationError::new(source_id.clone(), target_id, error.to_string())
                })?;
            let status = match start_mcp_service(state, target_id).await {
                Ok(status) => status,
                Err(error) => {
                    let message = error.to_string();
                    let _ = state.transition_active_workspace(
                        Some(target_id.to_string()),
                        ActiveWorkspacePhase::Error,
                        Some(message.clone()),
                    );
                    return Err(WorkspaceActivationError::new(
                        source_id.clone(),
                        target_id,
                        message,
                    ));
                }
            };
            if status.state != "running" {
                let message = format!("target runtime verification returned {}", status.state);
                let _ = state.transition_active_workspace(
                    Some(target_id.to_string()),
                    ActiveWorkspacePhase::Error,
                    Some(message.clone()),
                );
                return Err(WorkspaceActivationError::new(source_id, target_id, message));
            }
        }
        if let Err(error) = persist_last_workspace(state, target_id) {
            let message =
                format!("active runtime is healthy but restart preference commit failed: {error}");
            let _ = state.transition_active_workspace(
                Some(target_id.to_string()),
                ActiveWorkspacePhase::Error,
                Some(message.clone()),
            );
            return Err(WorkspaceActivationError::new(source_id, target_id, message));
        }
        return state.set_active_workspace(target_id).map_err(|error| {
            WorkspaceActivationError::new(source_id, target_id, error.to_string())
        });
    }

    if source_id.as_deref() == Some(target_id) && force_restart {
        if let Err(error) = state.refresh_provider_activity(target_id).await {
            state.activity.set_provider_activity(target_id, 0, 0, false);
            let _ = state.set_active_workspace(target_id);
            return Err(WorkspaceActivationError::new(
                source_id,
                target_id,
                format!("provider activity probe failed: {error}"),
            ));
        }
        state
            .transition_active_workspace(
                Some(target_id.to_string()),
                ActiveWorkspacePhase::Draining,
                Some(format!("Draining {target_id} before restart")),
            )
            .map_err(|error| {
                WorkspaceActivationError::new(source_id.clone(), target_id, error.to_string())
            })?;
        let check = state.activity.begin_drain(target_id);
        if !check.allowed {
            state.activity.open_gate(target_id);
            let _ = state.set_active_workspace(target_id);
            return Err(WorkspaceActivationError::new(
                source_id,
                target_id,
                format!("workspace is busy: {}", check.reasons.join("; ")),
            ));
        }

        let was_running = match is_mcp_running(state, target_id) {
            Ok(running) => running,
            Err(error) => {
                state.activity.open_gate(target_id);
                let _ = state.set_active_workspace(target_id);
                return Err(WorkspaceActivationError::new(
                    source_id,
                    target_id,
                    error.to_string(),
                ));
            }
        };
        if was_running {
            if let Err(error) = stop_mcp_service(state, target_id).await {
                state.activity.open_gate(target_id);
                let _ = state.set_active_workspace(target_id);
                return Err(WorkspaceActivationError::new(
                    source_id,
                    target_id,
                    format!("failed to stop workspace for restart: {error}"),
                ));
            }
        }
        state
            .transition_active_workspace(
                Some(target_id.to_string()),
                ActiveWorkspacePhase::Activating,
                Some(format!("Restarting {target_id}")),
            )
            .map_err(|error| {
                WorkspaceActivationError::new(source_id.clone(), target_id, error.to_string())
            })?;
        state.activity.open_gate(target_id);
        let status = match start_mcp_service(state, target_id).await {
            Ok(status) => status,
            Err(error) => {
                let message = error.to_string();
                let mut activation_error =
                    WorkspaceActivationError::new(source_id.clone(), target_id, message.clone());
                rollback_source(
                    state,
                    target_id,
                    was_running,
                    target_id,
                    true,
                    &mut activation_error,
                )
                .await;
                return Err(activation_error);
            }
        };
        if status.state != "running" {
            let message = format!("restart verification returned {}", status.state);
            let _ = state.transition_active_workspace(
                Some(target_id.to_string()),
                ActiveWorkspacePhase::Error,
                Some(message.clone()),
            );
            return Err(WorkspaceActivationError::new(source_id, target_id, message));
        }
        if let Err(error) = persist_last_workspace(state, target_id) {
            let mut activation_error = WorkspaceActivationError::new(
                source_id.clone(),
                target_id,
                format!("restart succeeded but commit failed: {error}"),
            );
            rollback_source(
                state,
                target_id,
                was_running,
                target_id,
                true,
                &mut activation_error,
            )
            .await;
            return Err(activation_error);
        }
        return state.set_active_workspace(target_id).map_err(|error| {
            WorkspaceActivationError::new(source_id, target_id, error.to_string())
        });
    }

    let source_was_running = if let Some(source_id) = source_id.as_deref() {
        if let Err(error) = state.refresh_provider_activity(source_id).await {
            state.activity.set_provider_activity(source_id, 0, 0, false);
            let _ = state.set_active_workspace(source_id);
            return Err(WorkspaceActivationError::new(
                Some(source_id.to_string()),
                target_id,
                format!("provider activity probe failed: {error}"),
            ));
        }
        state
            .transition_active_workspace(
                Some(source_id.to_string()),
                ActiveWorkspacePhase::Draining,
                Some(format!(
                    "Draining {source_id} before switching to {target_id}"
                )),
            )
            .map_err(|error| {
                WorkspaceActivationError::new(
                    Some(source_id.to_string()),
                    target_id,
                    error.to_string(),
                )
            })?;

        let check = state.activity.begin_drain(source_id);
        if !check.allowed {
            state.activity.open_gate(source_id);
            let _ = state.set_active_workspace(source_id);
            return Err(WorkspaceActivationError::new(
                Some(source_id.to_string()),
                target_id,
                format!("workspace is busy: {}", check.reasons.join("; ")),
            ));
        }

        match is_mcp_running(state, source_id) {
            Ok(running) => running,
            Err(error) => {
                state.activity.open_gate(source_id);
                let _ = state.set_active_workspace(source_id);
                return Err(WorkspaceActivationError::new(
                    Some(source_id.to_string()),
                    target_id,
                    error.to_string(),
                ));
            }
        }
    } else {
        false
    };

    if let Some(source_id) = source_id.as_deref() {
        if source_was_running {
            if let Err(error) = stop_mcp_service(state, source_id).await {
                let mut activation_error = WorkspaceActivationError::new(
                    Some(source_id.to_string()),
                    target_id,
                    format!("failed to stop source workspace: {error}"),
                );
                rollback_source(
                    state,
                    source_id,
                    source_was_running,
                    target_id,
                    false,
                    &mut activation_error,
                )
                .await;
                return Err(activation_error);
            }
        }
        state
            .transition_active_workspace(
                Some(source_id.to_string()),
                ActiveWorkspacePhase::Switching,
                Some(format!("Starting {target_id}")),
            )
            .map_err(|error| {
                WorkspaceActivationError::new(
                    Some(source_id.to_string()),
                    target_id,
                    error.to_string(),
                )
            })?;
    } else {
        state
            .transition_active_workspace(
                None,
                ActiveWorkspacePhase::Activating,
                Some(format!("Starting {target_id}")),
            )
            .map_err(|error| WorkspaceActivationError::new(None, target_id, error.to_string()))?;
    }

    state.activity.open_gate(target_id);
    let target_start = start_mcp_service(state, target_id).await;
    let target_started = target_start.is_ok();
    let target_status = match target_start {
        Ok(status) if status.state == "running" => status,
        Ok(status) => {
            let mut activation_error = WorkspaceActivationError::new(
                source_id.clone(),
                target_id,
                format!("target runtime verification returned {}", status.state),
            );
            rollback_after_target_failure(
                state,
                source_id.as_deref(),
                source_was_running,
                target_id,
                true,
                &mut activation_error,
            )
            .await;
            return Err(activation_error);
        }
        Err(error) => {
            let mut activation_error =
                WorkspaceActivationError::new(source_id.clone(), target_id, error.to_string());
            rollback_after_target_failure(
                state,
                source_id.as_deref(),
                source_was_running,
                target_id,
                target_started,
                &mut activation_error,
            )
            .await;
            return Err(activation_error);
        }
    };

    if let Err(error) = persist_last_workspace(state, target_id) {
        let mut activation_error = WorkspaceActivationError::new(
            source_id.clone(),
            target_id,
            format!("target started but commit failed: {error}"),
        );
        rollback_after_target_failure(
            state,
            source_id.as_deref(),
            source_was_running,
            target_id,
            true,
            &mut activation_error,
        )
        .await;
        return Err(activation_error);
    }

    let _ = target_status;
    let committed = state.set_active_workspace(target_id).map_err(|error| {
        WorkspaceActivationError::new(source_id.clone(), target_id, error.to_string())
    })?;
    if let Some(source_id) = source_id.as_deref() {
        state.activity.open_gate(source_id);
    }
    Ok(committed)
}

fn ensure_workspace_exists(state: &AppState, id: &str) -> Result<(), AppError> {
    state.with_workspaces(|store| {
        if store.get(id).is_some() {
            Ok(())
        } else {
            Err(AppError::Message(format!("workspace not found: {id}")))
        }
    })
}

fn persist_last_workspace(state: &AppState, id: &str) -> Result<(), AppError> {
    state.with_settings(|store| {
        let mut settings = store.settings();
        settings.last_workspace_id = id.to_string();
        store.update_settings(settings)
    })
}

fn is_mcp_running(state: &AppState, id: &str) -> Result<bool, AppError> {
    state.with_runtime(|runtime| Ok(runtime.is_running(id, ServiceKind::Mcp)))
}

fn find_running_workspace(state: &AppState, target_id: &str) -> Option<String> {
    if is_mcp_running(state, target_id).unwrap_or(false) {
        return Some(target_id.to_string());
    }
    let ids = state
        .with_workspaces(|store| {
            Ok(store
                .list()
                .iter()
                .filter(|profile| profile.id != target_id)
                .map(|profile| profile.id.clone())
                .collect::<Vec<_>>())
        })
        .ok()?;
    ids.into_iter().find(|id| {
        state
            .with_runtime(|runtime| Ok(runtime.is_running(id, ServiceKind::Mcp)))
            .unwrap_or(false)
    })
}

async fn rollback_after_target_failure(
    state: &AppState,
    source_id: Option<&str>,
    source_was_running: bool,
    target_id: &str,
    target_may_be_running: bool,
    error: &mut WorkspaceActivationError,
) {
    if target_may_be_running || is_mcp_running(state, target_id).unwrap_or(false) {
        let _ = stop_mcp_service(state, target_id).await;
    }
    if let Some(source_id) = source_id {
        rollback_source(state, source_id, source_was_running, target_id, true, error).await;
    } else {
        let _ = state.transition_active_workspace(
            None,
            ActiveWorkspacePhase::Error,
            Some(error.primary_failure.clone()),
        );
    }
}

async fn rollback_source(
    state: &AppState,
    source_id: &str,
    source_was_running: bool,
    _target_id: &str,
    mark_attempted: bool,
    error: &mut WorkspaceActivationError,
) {
    error.rollback_attempted = mark_attempted || source_was_running;
    let rollback_result = if source_was_running {
        match is_mcp_running(state, source_id) {
            Ok(true) => Ok(()),
            Ok(false) => match start_mcp_service(state, source_id).await {
                Ok(status) if status.state == "running" => Ok(()),
                Ok(status) => Err(AppError::Message(format!(
                    "rollback source returned non-running status: {}",
                    status.state
                ))),
                Err(error) => Err(error),
            },
            Err(error) => Err(error),
        }
    } else {
        Ok(())
    };

    state.activity.open_gate(source_id);
    match rollback_result {
        Ok(()) => {
            let _ = state.set_active_workspace(source_id);
            error.rollback_succeeded = true;
        }
        Err(rollback_error) => {
            let message = rollback_error.to_string();
            error.rollback_failure = Some(message.clone());
            let _ = state.transition_active_workspace(
                Some(source_id.to_string()),
                ActiveWorkspacePhase::Error,
                Some(format!("rollback failed: {message}")),
            );
        }
    }
}
