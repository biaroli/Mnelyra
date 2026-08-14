use tauri::State;

use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::settings::GlobalGeneralConfig;

#[tauri::command]
pub fn get_global_general(state: State<'_, AppState>) -> AppResult<GlobalGeneralConfig> {
    state.with_settings(|store| Ok(store.settings().general))
}

#[tauri::command]
pub async fn set_global_general(
    state: State<'_, AppState>,
    mut general: GlobalGeneralConfig,
) -> AppResult<()> {
    if general.mcp_runtime.local_port == 0 {
        return Err(AppError::Message("MCP 端口不能为 0。".into()));
    }
    if general.actions.local_port == 0 {
        return Err(AppError::Message("Actions 端口不能为 0。".into()));
    }

    general.configured = true;
    let selected_id = state.with_settings(|store| {
        let mut settings = store.settings();
        settings.general = general;
        let selected_id = settings.last_workspace_id.clone();
        store.update_settings(settings)?;
        Ok(selected_id)
    })?;

    if !selected_id.trim().is_empty() {
        crate::commands::runtime::activate_workspace_mcp(&state, &selected_id, true).await?;
    }
    Ok(())
}
