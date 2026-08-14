use tauri::State;

use std::sync::LazyLock;
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::platform::platform;
use crate::runtime::{
    await_listener_shutdown, port_busy_message, try_reclaim_previous_macos_app_port,
    wait_for_port_free, ServiceKind,
};
use crate::tunnel::{
    maybe_start_for_runtime, stop_for_runtime, sync_managed_runtime_routes, TunnelServiceKind,
};
use crate::workspace::RuntimeStatusDto;

/// Serialize MCP/Actions restarts so secret-save and form-save cannot tear down
/// the same listener concurrently (that race could abort the process on Windows).
static RESTART_GATE: LazyLock<AsyncMutex<()>> = LazyLock::new(|| AsyncMutex::new(()));

fn profile_by_id(state: &AppState, id: &str) -> AppResult<crate::workspace::WorkspaceProfile> {
    state.with_workspaces(|store| {
        let mut profile = store
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Message(format!("workspace not found: {id}")))?;
        store.settings().apply_global_config(&mut profile);
        Ok(profile)
    })
}

/// Start MCP for saved workspaces when the desktop app launches.
/// A bad path, port conflict, or tunnel failure in one workspace must not block the others.
pub(crate) async fn auto_start_configured_mcp(state: &AppState) {
    let selected_id = match state.with_workspaces(|store| {
        let settings = store.settings();
        if !settings.general.configured {
            return Ok(None);
        }
        let selected = store
            .list()
            .iter()
            .find(|profile| profile.id == settings.last_workspace_id)
            .or_else(|| store.list().first())
            .filter(|profile| {
                let path = profile.path.trim();
                !path.is_empty() && std::path::Path::new(path).is_dir()
            })
            .map(|profile| profile.id.clone());
        Ok(selected)
    }) {
        Ok(id) => id,
        Err(error) => {
            eprintln!("MCP auto-start skipped: {error}");
            return;
        }
    };

    let Some(id) = selected_id else {
        return;
    };
    let already_running = state
        .with_runtime(|runtime| Ok(runtime.is_running(&id, ServiceKind::Mcp)))
        .unwrap_or(false);
    if already_running {
        return;
    }
    if let Err(error) = start_mcp_service(state, &id).await {
        let message = error.to_string();
        let _ = state.with_runtime(|runtime| {
            runtime.mark_error(&id, ServiceKind::Mcp, message);
            Ok(())
        });
        eprintln!("MCP auto-start failed for {id}: {error}");
    }
}

fn persist_tunnel_url(
    state: &AppState,
    _id: &str,
    kind: TunnelServiceKind,
    url: &str,
) -> AppResult<()> {
    if url.is_empty() {
        return Ok(());
    }

    state.with_settings(|store| {
        let mut settings = store.settings();
        match kind {
            TunnelServiceKind::Mcp => settings.general.mcp_tunnel.public_url = url.to_string(),
            TunnelServiceKind::Actions => settings.general.actions.public_url = url.to_string(),
        }
        store.update_settings(settings)
    })
}

async fn sync_tunnel_routes_from_runtime(state: &AppState) -> AppResult<()> {
    let active_keys = state.with_runtime(|runtime| Ok(runtime.active_tunnel_service_keys()))?;
    sync_managed_runtime_routes(active_keys).await
}

#[allow(clippy::collapsible_if)]
async fn ensure_port_available(port: u16, service_label: &str) -> AppResult<()> {
    let Some(pid) = platform().find_pid_listening_on_port(port)? else {
        return Ok(());
    };

    if crate::runtime::is_own_process(pid) {
        if wait_for_port_free(port, Duration::from_secs(3)).await {
            return Ok(());
        }
    }

    if try_reclaim_previous_macos_app_port(port) {
        return Ok(());
    }

    if let Some(pid) = platform().find_pid_listening_on_port(port)? {
        return Err(AppError::Message(port_busy_message(
            port,
            service_label,
            pid,
        )));
    }

    Ok(())
}

pub(crate) async fn stop_mcp_service(state: &AppState, id: &str) -> AppResult<RuntimeStatusDto> {
    let profile = profile_by_id(state, id)?;
    let port = profile.runtime.local_port;
    let handle = state.with_runtime(|runtime| Ok(runtime.begin_stop(id, ServiceKind::Mcp)))?;
    await_listener_shutdown(handle, port).await;
    state.with_runtime(|runtime| {
        runtime.finish_stop(id, ServiceKind::Mcp);
        Ok(runtime.mcp_status(&profile))
    })?;
    stop_for_runtime(&profile, TunnelServiceKind::Mcp).await?;
    sync_tunnel_routes_from_runtime(state).await?;
    state.with_runtime(|runtime| Ok(runtime.mcp_status(&profile)))
}

async fn start_mcp_service(state: &AppState, id: &str) -> AppResult<RuntimeStatusDto> {
    let profile = profile_by_id(state, id)?;
    ensure_port_available(profile.runtime.local_port, "本地 MCP").await?;
    state.with_runtime(|runtime| runtime.start_mcp(&profile))?;
    sync_tunnel_routes_from_runtime(state).await?;

    match maybe_start_for_runtime(&profile, TunnelServiceKind::Mcp).await {
        Ok(Some(url)) => {
            persist_tunnel_url(state, id, TunnelServiceKind::Mcp, &url)?;
        }
        Ok(None) => {}
        Err(error) => {
            let message = error.to_string();
            let _ = stop_mcp_service(state, id).await;
            state.with_runtime(|runtime| {
                runtime.mark_error(id, ServiceKind::Mcp, message);
                Ok(())
            })?;
            return Err(error);
        }
    }

    let profile = profile_by_id(state, id)?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    state.with_runtime(|runtime| {
        runtime.refresh_mcp(&profile);
        Ok(runtime.mcp_status(&profile))
    })
}

async fn stop_actions_service(state: &AppState, id: &str) -> AppResult<RuntimeStatusDto> {
    let profile = profile_by_id(state, id)?;
    let port = profile.actions.local_port;
    let handle = state.with_runtime(|runtime| Ok(runtime.begin_stop(id, ServiceKind::Actions)))?;
    await_listener_shutdown(handle, port).await;
    state.with_runtime(|runtime| {
        runtime.finish_stop(id, ServiceKind::Actions);
        Ok(runtime.actions_status(&profile))
    })?;
    stop_for_runtime(&profile, TunnelServiceKind::Actions).await?;
    sync_tunnel_routes_from_runtime(state).await?;
    state.with_runtime(|runtime| Ok(runtime.actions_status(&profile)))
}

async fn start_actions_service(state: &AppState, id: &str) -> AppResult<RuntimeStatusDto> {
    let profile = profile_by_id(state, id)?;
    ensure_port_available(profile.actions.local_port, "本地 Actions").await?;
    state.with_runtime(|runtime| runtime.start_actions(&profile))?;
    sync_tunnel_routes_from_runtime(state).await?;

    match maybe_start_for_runtime(&profile, TunnelServiceKind::Actions).await {
        Ok(Some(url)) => {
            persist_tunnel_url(state, id, TunnelServiceKind::Actions, &url)?;
        }
        Ok(None) => {}
        Err(error) => {
            eprintln!("actions tunnel auto-start failed for {id}: {error}");
        }
    }

    let profile = profile_by_id(state, id)?;
    tokio::time::sleep(Duration::from_millis(250)).await;
    state.with_runtime(|runtime| {
        runtime.refresh_actions(&profile);
        Ok(runtime.actions_status(&profile))
    })
}

pub(crate) async fn activate_workspace_mcp(
    state: &AppState,
    id: &str,
    force_restart: bool,
) -> AppResult<RuntimeStatusDto> {
    // 全局 MCP 只有一套：切换工作区时，先停止其他工作区的 MCP。
    let other_ids = state.with_workspaces(|store| {
        Ok(store
            .list()
            .iter()
            .filter(|profile| profile.id != id)
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>())
    })?;

    for other_id in other_ids {
        let running = state.with_runtime(|runtime| {
            Ok(runtime.is_running(&other_id, ServiceKind::Mcp))
        })?;

        if running {
            let _ = stop_mcp_service(state, &other_id).await;
        }
    }

    let selected_running = state.with_runtime(|runtime| {
        Ok(runtime.is_running(id, ServiceKind::Mcp))
    })?;

    if force_restart {
        return restart_mcp_by_id(state, id).await;
    }

    if selected_running {
        let profile = profile_by_id(state, id)?;
        return state.with_runtime(|runtime| Ok(runtime.mcp_status(&profile)));
    }

    start_mcp_service(state, id).await
}

/// Async stop→start for MCP. Used by the Tauri command and secret-change hooks.
pub(crate) async fn restart_mcp_by_id(
    state: &AppState,
    id: &str,
) -> AppResult<RuntimeStatusDto> {
    let _guard = RESTART_GATE.lock().await;
    let was_running = state.with_runtime(|runtime| {
        Ok(runtime.is_running(id, ServiceKind::Mcp))
    })?;
    if was_running {
        let _ = stop_mcp_service(state, id).await?;
    }
    start_mcp_service(state, id).await
}

/// Async stop→start for Actions. Used by the Tauri command and secret-change hooks.
pub(crate) async fn restart_actions_by_id(
    state: &AppState,
    id: &str,
) -> AppResult<RuntimeStatusDto> {
    let _guard = RESTART_GATE.lock().await;
    let was_running = state.with_runtime(|runtime| {
        Ok(runtime.is_running(id, ServiceKind::Actions))
    })?;
    if was_running {
        let _ = stop_actions_service(state, id).await?;
    }
    start_actions_service(state, id).await
}

#[tauri::command]
pub async fn start_runtime(state: State<'_, AppState>, id: String) -> AppResult<RuntimeStatusDto> {
    start_mcp_service(&state, &id).await
}

#[tauri::command]
pub async fn stop_runtime(state: State<'_, AppState>, id: String) -> AppResult<RuntimeStatusDto> {
    stop_mcp_service(&state, &id).await
}

#[tauri::command]
pub fn get_runtime_status(state: State<'_, AppState>, id: String) -> AppResult<RuntimeStatusDto> {
    let profile = profile_by_id(&state, &id)?;
    state.with_runtime(|runtime| {
        runtime.refresh_mcp(&profile);
        Ok(runtime.mcp_status(&profile))
    })
}

#[tauri::command]
pub async fn start_actions_runtime(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RuntimeStatusDto> {
    start_actions_service(&state, &id).await
}

#[tauri::command]
pub async fn stop_actions_runtime(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RuntimeStatusDto> {
    stop_actions_service(&state, &id).await
}

#[tauri::command]
pub fn get_actions_runtime_status(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RuntimeStatusDto> {
    let profile = profile_by_id(&state, &id)?;
    state.with_runtime(|runtime| {
        runtime.refresh_actions(&profile);
        Ok(runtime.actions_status(&profile))
    })
}

#[tauri::command]
pub async fn restart_runtime(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RuntimeStatusDto> {
    restart_mcp_by_id(&state, &id).await
}

#[tauri::command]
pub async fn restart_actions_runtime(
    state: State<'_, AppState>,
    id: String,
) -> AppResult<RuntimeStatusDto> {
    restart_actions_by_id(&state, &id).await
}
