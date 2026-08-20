use tauri::{AppHandle, State};

use crate::app_state::AppState;
use crate::error::AppResult;
use crate::web_models::{self, WebModelBridgeStatus};

#[tauri::command]
pub async fn get_web_model_bridge_status(
    app: AppHandle,
    _state: State<'_, AppState>,
) -> AppResult<WebModelBridgeStatus> {
    web_models::status(&app).await
}

#[tauri::command]
pub async fn start_web_model_bridge(
    app: AppHandle,
    _state: State<'_, AppState>,
) -> AppResult<WebModelBridgeStatus> {
    web_models::start_browser_only(&app).await
}

#[tauri::command]
pub async fn stop_web_model_bridge(
    app: AppHandle,
    _state: State<'_, AppState>,
) -> AppResult<WebModelBridgeStatus> {
    web_models::stop_browser_only(&app).await
}
