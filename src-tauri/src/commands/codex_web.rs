use tauri::State;

use crate::app_state::AppState;
use crate::codex_web::{self, CodexWebBridgeStatus, CodexWebSetupCredentials};
use crate::error::AppResult;
use crate::secret::SecretStore;
use crate::tunnel::TUNNEL_RUNTIME_KEY;

#[tauri::command]
pub async fn get_codex_web_bridge_status(
    _state: State<'_, AppState>,
) -> AppResult<CodexWebBridgeStatus> {
    codex_web::status().await
}

#[tauri::command]
pub async fn start_codex_web_bridge(state: State<'_, AppState>) -> AppResult<CodexWebBridgeStatus> {
    let config = state.with_settings(|store| Ok(store.settings().openai_connector))?;
    let runtime_key = SecretStore::get_app("openai_connector", TUNNEL_RUNTIME_KEY)?;
    let credentials = (!config.tunnel_id.trim().is_empty() || runtime_key.is_some()).then_some({
        CodexWebSetupCredentials {
            tunnel_id: config.tunnel_id,
            runtime_key,
        }
    });
    codex_web::start_and_wait(credentials).await
}
