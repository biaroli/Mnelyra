#![cfg_attr(target_os = "windows", allow(linker_messages))]

mod activity;
mod app_state;
mod auth;
mod codex_web;
mod commands;
mod data;
mod error;
pub mod harness;
mod health;
mod mcp;
mod platform;
mod provider;
mod runtime;
mod secret;
mod session;
mod settings;
pub mod tools;
mod tunnel;
mod update;
mod workspace;

use app_state::AppState;
use commands::{
    activate_workspace, can_switch_workspace, cancel_session, check_app_update, compact_session,
    create_workspace, delete_frp_profile, delete_workspace, get_active_workspace_state,
    get_app_settings, get_codex_context_policy, get_codex_web_bridge_status, get_download_config,
    get_frp_snippet, get_global_auth, get_global_general, get_last_workspace_id,
    get_openai_connector_settings, get_openai_connector_status, get_pending_session_requests,
    get_provider_checkpoint, get_provider_status, get_runtime_status, get_session_event_page,
    get_session_events, get_shared_secret, get_webview_memory_sample, get_workspace_activity,
    get_workspace_memory_overview, hide_to_tray, install_openai_tunnel_client, install_software,
    list_frp_profiles, list_providers, list_sessions, list_software, list_workspaces, open_url,
    open_workspace_directory, quit_app, read_workspace_logs, recreate_ui_webview,
    regenerate_shared_secret, respond_session_request, restart_runtime, restart_tunnel,
    run_health_checks, save_frp_profile, save_openai_connector_settings, send_session_input,
    set_codex_auto_compact_limit, set_codex_context_policy, set_download_config, set_global_auth,
    set_global_general, set_last_workspace, set_permission_ceiling, set_shared_secret,
    show_main_window, start_background_services, start_codex_web_bridge, start_openai_connector,
    start_provider_task, start_runtime, start_tunnel, stop_openai_connector, stop_runtime,
    stop_tunnel, test_tunnel, uninstall_software, update_workspace,
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::webview::PageLoadEvent;
use tauri::{Emitter, Manager, WindowEvent};

#[cfg(target_os = "windows")]
fn signal_existing_instance() -> bool {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, EVENT_MODIFY_STATE,
    };

    let isolated = std::env::var_os("MNELYRA_DATA_DIR")
        .map(std::path::PathBuf::from)
        .is_some_and(|path| path.is_absolute());
    let (mutex_name, event_name) = if isolated {
        (
            "Local\\Mnelyra-IsolatedDev-SingleInstance",
            "Local\\Mnelyra-IsolatedDev-ShowWindow",
        )
    } else {
        ("Local\\Mnelyra-SingleInstance", "Local\\Mnelyra-ShowWindow")
    };
    let mutex_wide = mutex_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let event_wide = event_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();

    let Ok(mutex) = (unsafe { CreateMutexW(None, false, PCWSTR(mutex_wide.as_ptr())) }) else {
        eprintln!("创建应用单实例锁失败，为避免误清理其他实例的 frpc，本次启动已取消");
        return false;
    };
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        let _ = unsafe { CloseHandle(mutex) };
        if let Ok(event) =
            unsafe { OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(event_wide.as_ptr())) }
        {
            let _ = unsafe { SetEvent(event) };
            let _ = unsafe { CloseHandle(event) };
        }
        return false;
    }

    // Keep mutex handle for process lifetime (do not CloseHandle).
    let _ = INSTANCE_MUTEX.set(mutex.0 as usize);

    let Ok(event) = (unsafe { CreateEventW(None, false, false, PCWSTR(event_wide.as_ptr())) })
    else {
        return true;
    };
    // Pass the handle as usize so the waiter thread is Send (HANDLE is !Send).
    let event_bits = event.0 as usize;
    std::thread::spawn(move || {
        use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
        let event = HANDLE(event_bits as *mut std::ffi::c_void);
        loop {
            let _ = unsafe { WaitForSingleObject(event, INFINITE) };
            if let Some(app) = SHOW_APP_HANDLE.get() {
                let _ = commands::window_chrome::show_main_window(app.clone());
            }
        }
    });
    true
}

#[cfg(target_os = "windows")]
static SHOW_APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

#[cfg(target_os = "windows")]
static INSTANCE_MUTEX: std::sync::OnceLock<usize> = std::sync::OnceLock::new();

#[cfg(target_os = "windows")]
fn acquire_single_instance() -> bool {
    signal_existing_instance()
}

#[cfg(not(target_os = "windows"))]
fn acquire_single_instance() -> bool {
    true
}

fn setup_tray(app: &tauri::App) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "显示窗口", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &quit_i])?;

    let mut builder = TrayIconBuilder::with_id("main-tray")
        .menu(&menu)
        .tooltip("Mnelyra")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                let _ = commands::window_chrome::show_main_window(app.clone());
            }
            "quit" => {
                commands::window_chrome::arm_allow_exit();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = commands::window_chrome::show_main_window(tray.app_handle().clone());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    builder.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if !acquire_single_instance() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Finished {
                commands::schedule_after_page_load(webview.app_handle().clone());
            }
        })
        .setup(|app| {
            app.manage(AppState::new().expect("failed to load app state"));
            commands::schedule_fallback(app.handle().clone());
            // Recover FRP clients that stay alive while the public proxy dies
            // (common after install/restart network blips).
            tunnel::ensure_frp_health_loop();
            setup_tray(app)?;
            #[cfg(target_os = "windows")]
            {
                let _ = SHOW_APP_HANDLE.set(app.handle().clone());
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_active_workspace_state,
            get_workspace_activity,
            can_switch_workspace,
            activate_workspace,
            list_providers,
            get_provider_status,
            get_codex_context_policy,
            set_codex_auto_compact_limit,
            set_codex_context_policy,
            set_permission_ceiling,
            start_background_services,
            get_openai_connector_settings,
            save_openai_connector_settings,
            get_openai_connector_status,
            install_openai_tunnel_client,
            start_openai_connector,
            stop_openai_connector,
            get_codex_web_bridge_status,
            start_codex_web_bridge,
            list_sessions,
            get_workspace_memory_overview,
            get_provider_checkpoint,
            start_provider_task,
            send_session_input,
            cancel_session,
            compact_session,
            get_session_events,
            get_session_event_page,
            get_pending_session_requests,
            respond_session_request,
            list_workspaces,
            create_workspace,
            update_workspace,
            open_workspace_directory,
            open_url,
            check_app_update,
            delete_workspace,
            start_runtime,
            stop_runtime,
            get_runtime_status,
            restart_runtime,
            get_frp_snippet,
            start_tunnel,
            stop_tunnel,
            run_health_checks,
            get_shared_secret,
            set_shared_secret,
            regenerate_shared_secret,
            read_workspace_logs,
            list_frp_profiles,
            save_frp_profile,
            delete_frp_profile,
            get_app_settings,
            get_global_auth,
            set_global_auth,
            get_global_general,
            set_global_general,
            restart_tunnel,
            test_tunnel,
            set_last_workspace,
            get_last_workspace_id,
            list_software,
            install_software,
            uninstall_software,
            get_download_config,
            set_download_config,
            get_webview_memory_sample,
            recreate_ui_webview,
            hide_to_tray,
            show_main_window,
            quit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                // While recreating the UI WebView we temporarily destroy the main
                // window; without prevent_exit Tauri would quit the whole process
                // and take MCP/FRP down with it (0.1.30 regression).
                if commands::ui_memory::should_prevent_exit() {
                    api.prevent_exit();
                }
            }
            tauri::RunEvent::WindowEvent { label, event, .. } => {
                if label != "main" {
                    return;
                }
                if let WindowEvent::CloseRequested { api, .. } = event {
                    if commands::window_chrome::should_intercept_close() {
                        api.prevent_close();
                        let _ = app_handle.emit("close-requested", ());
                    }
                }
            }
            _ => {}
        });
}
