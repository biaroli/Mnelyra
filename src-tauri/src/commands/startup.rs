use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};

use crate::app_state::AppState;
use crate::tunnel;

static BACKGROUND_START_STARTED: AtomicBool = AtomicBool::new(false);

fn start_background_once(app: AppHandle) -> bool {
    if BACKGROUND_START_STARTED
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return false;
    }

    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let required_kind = state
            .with_settings(|store| {
                let settings = store.settings();
                Ok(match settings.general.mcp_tunnel.tunnel_type.as_str() {
                    "frp" => Some("frpc".to_string()),
                    "cloudflare" => Some("cloudflared".to_string()),
                    _ => None,
                })
            })
            .unwrap_or(None);

        if let Some(required_kind) = required_kind.as_deref() {
            let missing = tunnel::list_software()
                .into_iter()
                .find(|status| status.kind == required_kind)
                .is_some_and(|status| !status.installed);
            let stale_managed_cloudflared = required_kind == "cloudflared"
                && crate::tunnel::managed_cloudflared_update_needed();
            if missing || stale_managed_cloudflared {
                if let Err(error) = tunnel::install_software(required_kind).await {
                    eprintln!("automatic {required_kind} install failed: {error}");
                }
            }
        }

        super::runtime::auto_start_configured_mcp(state.inner()).await;

        for status in tunnel::list_software() {
            if !status.installed && Some(status.kind.as_str()) != required_kind.as_deref() {
                if let Err(error) = tunnel::install_software(&status.kind).await {
                    eprintln!("background {} install failed: {error}", status.kind);
                }
            }
        }
    });

    true
}

pub(crate) fn schedule_fallback(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(6)).await;
        let _ = start_background_once(app);
    });
}

pub(crate) fn schedule_after_page_load(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let _ = start_background_once(app);
    });
}

#[tauri::command]
pub fn start_background_services(app: AppHandle) -> bool {
    start_background_once(app)
}
