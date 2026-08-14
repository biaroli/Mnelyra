use tauri::{AppHandle, Manager, State};

use crate::app_state::AppState;
use crate::error::{AppError, AppResult};
use crate::settings::GlobalAuthConfig;

fn validate_auth(auth: &GlobalAuthConfig) -> AppResult<()> {
    if !matches!(auth.mcp_auth_type.as_str(), "oauth" | "bearer" | "noauth") {
        return Err(AppError::Message(format!(
            "不支持的 MCP 认证方式: {}",
            auth.mcp_auth_type
        )));
    }
    if !matches!(auth.actions_auth_type.as_str(), "api_key" | "oauth" | "none") {
        return Err(AppError::Message(format!(
            "不支持的 Actions 认证方式: {}",
            auth.actions_auth_type
        )));
    }
    Ok(())
}

#[tauri::command]
pub fn get_global_auth(state: State<'_, AppState>) -> AppResult<GlobalAuthConfig> {
    state.with_settings(|store| Ok(store.settings().auth))
}

#[tauri::command]
pub fn set_global_auth(
    app: AppHandle,
    state: State<'_, AppState>,
    auth: GlobalAuthConfig,
) -> AppResult<()> {
    validate_auth(&auth)?;
    let auth = GlobalAuthConfig {
        mcp_auth_type: auth.mcp_auth_type.trim().to_string(),
        actions_auth_type: auth.actions_auth_type.trim().to_string(),
        actions_oauth_scopes: auth.actions_oauth_scopes.trim().to_string(),
    };

    let (mcp_changed, actions_changed) = state.with_settings(|store| {
        let mut settings = store.settings();
        let mcp_changed = settings.auth.mcp_auth_type != auth.mcp_auth_type;
        let actions_changed = settings.auth.actions_auth_type != auth.actions_auth_type
            || settings.auth.actions_oauth_scopes != auth.actions_oauth_scopes;
        settings.auth = auth;
        store.update_settings(settings)?;
        Ok((mcp_changed, actions_changed))
    })?;

    if !mcp_changed && !actions_changed {
        return Ok(());
    }

    let workspace_ids = state.with_workspaces(|store| {
        Ok(store
            .list()
            .iter()
            .map(|profile| profile.id.clone())
            .collect::<Vec<_>>())
    })?;

    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        for id in workspace_ids {
            if mcp_changed {
                let running = state
                    .with_runtime(|runtime| {
                        Ok(runtime.is_running(&id, crate::runtime::ServiceKind::Mcp))
                    })
                    .unwrap_or(false);
                if running {
                    if let Err(error) = crate::commands::runtime::restart_mcp_by_id(&state, &id).await
                    {
                        eprintln!("MCP restart after global auth change failed for {id}: {error}");
                    }
                }
            }

            if actions_changed {
                let running = state
                    .with_runtime(|runtime| {
                        Ok(runtime.is_running(&id, crate::runtime::ServiceKind::Actions))
                    })
                    .unwrap_or(false);
                if running {
                    if let Err(error) =
                        crate::commands::runtime::restart_actions_by_id(&state, &id).await
                    {
                        eprintln!(
                            "Actions restart after global auth change failed for {id}: {error}"
                        );
                    }
                }
            }
        }
    });

    Ok(())
}
