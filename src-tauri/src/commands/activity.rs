use tauri::State;

use crate::activity::{ActiveWorkspaceState, ActivitySnapshot, SwitchCheck};
use crate::app_state::AppState;
use crate::error::AppResult;

#[tauri::command]
pub fn get_active_workspace_state(state: State<'_, AppState>) -> AppResult<ActiveWorkspaceState> {
    state.active_workspace_state()
}

#[tauri::command]
pub async fn get_workspace_activity(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<ActivitySnapshot> {
    let active = state.active_workspace_state()?;
    if active.workspace_id.as_deref() == Some(id.as_str()) {
        let _ = state.refresh_provider_activity(&id).await?;
    }
    Ok(state.activity.snapshot(&id))
}

#[tauri::command]
pub async fn can_switch_workspace(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<SwitchCheck> {
    let _ = state.refresh_provider_activity(&id).await?;
    Ok(state.activity.can_switch(&id))
}

#[tauri::command]
pub async fn activate_workspace(
    state: State<'_, AppState>,
    id: String,
) -> Result<ActiveWorkspaceState, crate::workspace::activation::WorkspaceActivationError> {
    crate::workspace::activation::activate_workspace(&state, &id).await
}
