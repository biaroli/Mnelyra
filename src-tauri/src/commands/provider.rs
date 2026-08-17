use std::path::PathBuf;

use tauri::State;

use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::provider::{ProviderDescriptor, ProviderStatus};
use crate::session::{
    list_provider_checkpoints, read_provider_checkpoint, PendingSessionRequest, ProviderCheckpoint,
    SessionEvent, SessionEventPage, TaskSession, WorkspaceMemoryOverview,
};

#[tauri::command]
pub fn list_providers(state: State<'_, AppState>) -> AppResult<Vec<ProviderDescriptor>> {
    Ok(state.providers.descriptors())
}

#[tauri::command]
pub fn get_session_event_page(
    state: State<'_, AppState>,
    session_id: String,
    cursor: Option<u64>,
    limit: Option<usize>,
) -> AppResult<SessionEventPage> {
    state
        .session_coordinator
        .event_page(&session_id, cursor.unwrap_or(0), limit)
}

#[tauri::command]
pub async fn get_provider_status(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<ProviderStatus> {
    state.providers.status(&id).await
}

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> AppResult<Vec<TaskSession>> {
    state.sessions.list()
}

#[tauri::command]
pub fn get_workspace_memory_overview(
    state: State<'_, AppState>,
    workspace_id: String,
) -> AppResult<WorkspaceMemoryOverview> {
    let profile = state.with_workspaces(|store| {
        store
            .get(&workspace_id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {workspace_id}")))
    })?;
    list_provider_checkpoints(&workspace_id, &PathBuf::from(profile.path))
}

#[tauri::command]
pub fn get_provider_checkpoint(
    state: State<'_, AppState>,
    workspace_id: String,
    checkpoint_id: String,
) -> AppResult<ProviderCheckpoint> {
    let profile = state.with_workspaces(|store| {
        store
            .get(&workspace_id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {workspace_id}")))
    })?;
    read_provider_checkpoint(&PathBuf::from(profile.path), &checkpoint_id)
}

#[tauri::command]
pub async fn start_provider_task(
    state: State<'_, AppState>,
    workspace_id: String,
    title: String,
    prompt: String,
) -> AppResult<TaskSession> {
    require_active_workspace(&state, &workspace_id)?;
    let profile = state.with_workspaces(|store| {
        store
            .get(&workspace_id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {workspace_id}")))
    })?;
    state
        .session_coordinator
        .start_codex_task(&workspace_id, &PathBuf::from(profile.path), &title, &prompt)
        .await
}

#[tauri::command]
pub async fn send_session_input(
    state: State<'_, AppState>,
    session_id: String,
    input: String,
) -> AppResult<TaskSession> {
    let session = state.sessions.get(&session_id)?;
    require_active_workspace(&state, &session.workspace_id)?;
    state
        .session_coordinator
        .send_input(&session_id, &input)
        .await
}

#[tauri::command]
pub async fn cancel_session(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<TaskSession> {
    state.session_coordinator.cancel(&session_id).await
}

#[tauri::command]
pub async fn compact_session(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<TaskSession> {
    let session = state.sessions.get(&session_id)?;
    require_active_workspace(&state, &session.workspace_id)?;
    state.session_coordinator.compact(&session_id).await
}

#[tauri::command]
pub async fn get_codex_context_policy(state: State<'_, AppState>) -> AppResult<serde_json::Value> {
    let cwd = state
        .active_workspace_state()?
        .workspace_id
        .and_then(|workspace_id| {
            state
                .with_workspaces(|store| Ok(store.get(&workspace_id).cloned()))
                .ok()
                .flatten()
                .map(|profile| PathBuf::from(profile.path))
        });
    state
        .codex_app_server
        .read_config(cwd.as_deref())
        .await
        .map_err(AppError::Message)
}

#[tauri::command]
pub async fn set_codex_context_policy(
    state: State<'_, AppState>,
    context_window: Option<u64>,
    auto_compact_token_limit: Option<u64>,
) -> AppResult<serde_json::Value> {
    state
        .codex_app_server
        .set_context_policy(context_window, auto_compact_token_limit)
        .await
        .map_err(AppError::Message)
}

#[tauri::command]
pub async fn set_permission_ceiling(state: State<'_, AppState>, mode: String) -> AppResult<()> {
    if !matches!(mode.as_str(), "automatic" | "read_only" | "custom") {
        return Err(AppError::Message(format!(
            "unsupported Codex permission mode: {mode}"
        )));
    }

    let previous = state.with_settings(|store| Ok(store.settings().general.permission_ceiling))?;
    if previous == mode {
        return Ok(());
    }

    let active_workspace_id = state.active_workspace_state()?.workspace_id;
    let mcp_was_running = if let Some(id) = active_workspace_id.as_deref() {
        state
            .with_runtime(|runtime| Ok(runtime.is_running(id, crate::runtime::ServiceKind::Mcp)))?
    } else {
        false
    };

    state
        .codex_app_server
        .reconfigure_permission_mode(&mode)
        .await
        .map_err(AppError::Message)?;

    if let Err(error) = state.with_settings(|store| {
        let mut settings = store.settings();
        settings.general.permission_ceiling = mode.clone();
        store.update_settings(settings)
    }) {
        let _ = state
            .codex_app_server
            .reconfigure_permission_mode(&previous)
            .await;
        return Err(error);
    }

    let apply_runtime_ceiling = async {
        if let Some(id) = active_workspace_id.as_deref() {
            if mcp_was_running {
                crate::commands::runtime::restart_mcp_by_id(&state, id).await?;
            }
        }
        Ok::<(), AppError>(())
    }
    .await;

    if let Err(error) = apply_runtime_ceiling {
        let _ = state.with_settings(|store| {
            let mut settings = store.settings();
            settings.general.permission_ceiling = previous.clone();
            store.update_settings(settings)
        });
        let _ = state
            .codex_app_server
            .reconfigure_permission_mode(&previous)
            .await;
        if let Some(id) = active_workspace_id.as_deref() {
            if mcp_was_running {
                let _ = crate::commands::runtime::restart_mcp_by_id(&state, id).await;
            }
        }
        return Err(error);
    }

    Ok(())
}

#[tauri::command]
pub async fn set_codex_auto_compact_limit(
    state: State<'_, AppState>,
    token_limit: Option<u64>,
) -> AppResult<serde_json::Value> {
    state
        .codex_app_server
        .set_auto_compact_token_limit(token_limit)
        .await
        .map_err(AppError::Message)
}

#[tauri::command]
pub fn get_session_events(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<SessionEvent>> {
    state.session_coordinator.events(&session_id)
}

#[tauri::command]
pub fn get_pending_session_requests(
    state: State<'_, AppState>,
    session_id: String,
) -> AppResult<Vec<PendingSessionRequest>> {
    state.session_coordinator.pending_requests(&session_id)
}

#[tauri::command]
pub async fn respond_session_request(
    state: State<'_, AppState>,
    session_id: String,
    request_id: String,
    action: String,
) -> AppResult<()> {
    state
        .session_coordinator
        .respond_to_request(&session_id, &request_id, &action)
        .await
}

fn require_active_workspace(state: &AppState, workspace_id: &str) -> AppResult<()> {
    let active = state.active_workspace_state()?;
    if active.workspace_id.as_deref() != Some(workspace_id)
        || active.phase != crate::activity::ActiveWorkspacePhase::Active
    {
        return Err(AppError::Message(format!(
            "workspace {workspace_id} is not the active authoritative root"
        )));
    }
    if state.activity.snapshot(workspace_id).drain_requested {
        return Err(AppError::Message(format!(
            "workspace {workspace_id} is draining"
        )));
    }
    Ok(())
}
