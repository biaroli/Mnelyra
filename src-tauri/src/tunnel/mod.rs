mod access;
mod cloudflare;
mod download;
mod frp;
mod openai;
mod software;
mod supervisor;

use crate::settings::AppSettings;
use crate::workspace::WorkspaceProfile;

pub use access::{
    cleanup_orphan_for_runtime, drop_workspace, ensure_frp_health_loop, maybe_start_for_runtime,
    stop_for_runtime, supervisor, sync_managed_runtime_routes,
};

pub(crate) use cloudflare::managed_cloudflared_update_needed;
#[allow(unused_imports)]
pub use cloudflare::{
    extract_trycloudflare_url, resolve_cloudflared, spawn_cloudflare_tunnel, stop_child,
};
#[allow(unused_imports)]
pub use frp::mcp_frp_snippet;
pub use openai::{
    ensure_tunnel_client as ensure_openai_tunnel_client,
    validate_alias as validate_openai_tunnel_alias,
    validate_tunnel_id as validate_openai_tunnel_id, OpenAiConnectorStatus, OpenAiTunnelManager,
    TUNNEL_CLIENT_VERSION, TUNNEL_RUNTIME_KEY,
};
#[allow(unused_imports)]
pub use software::{install_software, list_software, uninstall_software, SoftwareStatus};
#[allow(unused_imports)]
pub use supervisor::{
    append_profile_log, log_dir_for_profile, TunnelServiceKind, TunnelStatus, TunnelSupervisor,
};

pub fn frp_snippet(profile: &WorkspaceProfile, kind: TunnelServiceKind) -> String {
    let settings = AppSettings::load_or_default();
    frp::frp_snippet(profile, kind, &settings)
}
