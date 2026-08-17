use tauri::State;

use crate::activity::ActivityKind;
use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::runtime::ServiceKind;
use crate::secret::SecretStore;
use crate::settings::OpenAiConnectorConfig;
use crate::tunnel::{
    ensure_openai_tunnel_client, validate_openai_tunnel_alias, validate_openai_tunnel_id,
    OpenAiConnectorStatus, TUNNEL_CLIENT_VERSION, TUNNEL_RUNTIME_KEY,
};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiConnectorSettingsDto {
    pub enabled: bool,
    pub tunnel_id: String,
    pub alias: String,
    pub has_runtime_key: bool,
    pub tunnel_client_version: String,
}

#[tauri::command]
pub fn get_openai_connector_settings(
    state: State<'_, AppState>,
) -> AppResult<OpenAiConnectorSettingsDto> {
    let config = state.with_settings(|store| Ok(store.settings().openai_connector))?;
    settings_dto(config)
}

#[tauri::command]
pub fn save_openai_connector_settings(
    state: State<'_, AppState>,
    tunnel_id: String,
    alias: String,
    runtime_api_key: Option<String>,
) -> AppResult<OpenAiConnectorSettingsDto> {
    let tunnel_id = tunnel_id.trim().to_string();
    let alias = alias.trim().to_string();
    validate_openai_tunnel_id(&tunnel_id)?;
    validate_openai_tunnel_alias(&alias)?;
    let runtime_api_key = runtime_api_key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if let Some(key) = runtime_api_key {
        if key.len() > 65_536 {
            return Err(AppError::Message(
                "OpenAI Tunnel runtime API key is unexpectedly large".into(),
            ));
        }
    }
    let config = OpenAiConnectorConfig {
        enabled: state.with_settings(|store| Ok(store.settings().openai_connector.enabled))?,
        tunnel_id,
        alias,
    };
    state.with_settings(|store| {
        if let Some(key) = runtime_api_key {
            // Keep the in-memory DataStore and the on-disk secret payload in sync.
            // Writing through SecretStore here would update only the file; the next
            // ordinary settings save could then overwrite that fresh secret with the
            // stale in-memory snapshot.
            store.set_app_secret("openai_connector", TUNNEL_RUNTIME_KEY, key)?;
        }
        let mut settings = store.settings();
        settings.openai_connector = config.clone();
        store.update_settings(settings)
    })?;
    settings_dto(config)
}

#[tauri::command]
pub async fn install_openai_tunnel_client(
    state: State<'_, AppState>,
) -> AppResult<OpenAiConnectorStatus> {
    let settings = state.with_settings(|store| Ok(store.settings()))?;
    ensure_openai_tunnel_client(&settings).await?;
    state.openai_tunnel.status(&settings.openai_connector).await
}

#[tauri::command]
pub async fn get_openai_connector_status(
    state: State<'_, AppState>,
) -> AppResult<OpenAiConnectorStatus> {
    let config = state.with_settings(|store| Ok(store.settings().openai_connector))?;
    state.openai_tunnel.status(&config).await
}

#[tauri::command]
pub async fn start_openai_connector(
    state: State<'_, AppState>,
) -> AppResult<OpenAiConnectorStatus> {
    let active = state.active_workspace_state()?;
    let workspace_id = active
        .workspace_id
        .ok_or_else(|| AppError::Message("No authoritative Mnelyra workspace is active".into()))?;
    let (settings, profile) = state.with_settings(|store| {
        let settings = store.settings();
        let mut profile = store
            .get(&workspace_id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {workspace_id}")))?;
        settings.apply_global_config(&mut profile);
        Ok((settings, profile))
    })?;
    if !state.with_runtime(|runtime| Ok(runtime.is_running(&workspace_id, ServiceKind::Mcp)))? {
        return Err(AppError::Message(
            "Start the active Mnelyra MCP runtime before connecting OpenAI Tunnel".into(),
        ));
    }
    let _operation = state
        .activity
        .acquire(&workspace_id, ActivityKind::ProviderOperation)
        .map_err(|error| AppError::Message(error.to_string()))?;
    let status = state
        .openai_tunnel
        .start(
            &settings,
            &settings.openai_connector,
            profile.runtime.local_port,
        )
        .await?;
    state.with_settings(|store| {
        let mut settings = store.settings();
        settings.openai_connector.enabled = true;
        store.update_settings(settings)
    })?;
    Ok(status)
}

#[tauri::command]
pub async fn stop_openai_connector(state: State<'_, AppState>) -> AppResult<OpenAiConnectorStatus> {
    state.openai_tunnel.shutdown().await;
    let config = state.with_settings(|store| {
        let mut settings = store.settings();
        settings.openai_connector.enabled = false;
        let config = settings.openai_connector.clone();
        store.update_settings(settings)?;
        Ok(config)
    })?;
    state.openai_tunnel.status(&config).await
}

fn settings_dto(config: OpenAiConnectorConfig) -> AppResult<OpenAiConnectorSettingsDto> {
    Ok(OpenAiConnectorSettingsDto {
        enabled: config.enabled,
        tunnel_id: config.tunnel_id,
        alias: config.alias,
        has_runtime_key: SecretStore::get_app("openai_connector", TUNNEL_RUNTIME_KEY)?
            .is_some_and(|value| !value.trim().is_empty()),
        tunnel_client_version: TUNNEL_CLIENT_VERSION.into(),
    })
}
