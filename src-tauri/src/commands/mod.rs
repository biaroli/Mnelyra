mod activity;
mod app_info;
mod auth_settings;
mod frp_profiles;
mod general_settings;
mod health;
mod logs;
mod openai_connector;
mod provider;
pub(crate) mod runtime;
mod secrets;
mod software;
mod startup;
mod tunnel;
pub(crate) mod ui_memory;
mod web_models;
pub(crate) mod window_chrome;
mod workspace;

pub use activity::{
    activate_workspace, can_switch_workspace, get_active_workspace_state, get_workspace_activity,
};
pub use app_info::{check_app_update, open_url};
pub use auth_settings::{get_global_auth, set_global_auth};
pub use frp_profiles::{
    delete_frp_profile, get_app_settings, get_last_workspace_id, list_frp_profiles,
    save_frp_profile, set_last_workspace,
};
pub use general_settings::{get_global_general, set_global_general};
pub use health::run_health_checks;
pub use logs::read_workspace_logs;
pub use openai_connector::{
    get_openai_connector_settings, get_openai_connector_status, install_openai_tunnel_client,
    save_openai_connector_settings, start_openai_connector, stop_openai_connector,
};
pub use provider::{
    cancel_session, compact_session, get_codex_context_policy, get_pending_session_requests,
    get_provider_checkpoint, get_provider_status, get_session_event_page, get_session_events,
    get_workspace_memory_overview, list_providers, list_sessions, respond_session_request,
    send_session_input, set_codex_auto_compact_limit, set_codex_context_policy,
    set_permission_ceiling, start_provider_task,
};
pub use runtime::{get_runtime_status, restart_runtime, start_runtime, stop_runtime};
pub use secrets::{get_shared_secret, regenerate_shared_secret, set_shared_secret};
pub use software::{
    get_download_config, install_software, list_software, set_download_config, uninstall_software,
};
pub use startup::start_background_services;
pub(crate) use startup::{schedule_after_page_load, schedule_fallback};
pub use tunnel::{get_frp_snippet, restart_tunnel, start_tunnel, stop_tunnel, test_tunnel};
pub use ui_memory::{get_webview_memory_sample, recreate_ui_webview};
pub use web_models::{get_web_model_bridge_status, start_web_model_bridge, stop_web_model_bridge};
pub use window_chrome::{hide_to_tray, quit_app, show_main_window};
pub use workspace::{
    create_workspace, delete_workspace, list_workspaces, open_workspace_directory, update_workspace,
};
