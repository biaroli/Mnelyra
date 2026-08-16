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
    if !matches!(
        general.permission_ceiling.as_str(),
        "automatic" | "read_only" | "custom"
    ) {
        return Err(AppError::Message(format!(
            "不支持的权限总阀门模式: {}",
            general.permission_ceiling
        )));
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
        crate::workspace::activation::activate_workspace_with_options(&state, &selected_id, true)
            .await
            .map_err(|error| AppError::Message(error.to_string()))?;
        // Global MCP port changes invalidate Mode B's loopback target. If the
        // connector is enabled, rebuild its owned runtime against the freshly
        // verified authoritative listener instead of leaving a stale "ready"
        // tunnel pointed at the old port.
        crate::commands::auto_start_openai_connector(&state).await;
    }
    Ok(())
}
