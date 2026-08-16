use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::data::AppData;
use crate::workspace::{RuntimeConfig, TunnelConfig, WorkspaceProfile};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrpProfile {
    pub id: String,
    pub name: String,
    pub server: String,
    #[serde(default = "default_frp_server_port", alias = "serverPort")]
    pub server_port: u16,
}

fn default_openai_tunnel_alias() -> String {
    "mnelyra".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiConnectorConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub tunnel_id: String,
    #[serde(default = "default_openai_tunnel_alias")]
    pub alias: String,
}

impl Default for OpenAiConnectorConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tunnel_id: String::new(),
            alias: default_openai_tunnel_alias(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalAuthConfig {
    #[serde(default = "default_mcp_auth_type")]
    pub mcp_auth_type: String,
}

impl Default for GlobalAuthConfig {
    fn default() -> Self {
        Self {
            mcp_auth_type: default_mcp_auth_type(),
        }
    }
}

impl GlobalAuthConfig {
    pub(crate) fn from_profile(profile: &WorkspaceProfile) -> Self {
        Self {
            mcp_auth_type: profile.auth.auth_type.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalGeneralConfig {
    #[serde(default)]
    pub configured: bool,
    #[serde(default = "default_permission_ceiling", alias = "codexPermissionMode")]
    pub permission_ceiling: String,
    #[serde(default)]
    pub mcp_tunnel: TunnelConfig,
    #[serde(default)]
    pub mcp_runtime: RuntimeConfig,
}

impl GlobalGeneralConfig {
    pub(crate) fn from_profile(profile: &WorkspaceProfile) -> Self {
        Self {
            configured: true,
            permission_ceiling: default_permission_ceiling(),
            mcp_tunnel: profile.tunnel.clone(),
            mcp_runtime: profile.runtime.clone(),
        }
    }
}

/// Download settings for fetching frpc / cloudflared binaries.
///
/// GitHub is slow/unreliable from some networks, so downloads try a mirror
/// prefix first (ghproxy-style: `{mirror}/{full_github_url}`) and fall back to
/// the direct GitHub URL.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadConfig {
    /// Mirror prefix applied before the full GitHub URL. Empty = direct.
    #[serde(default = "default_github_mirror")]
    pub github_mirror: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            github_mirror: default_github_mirror(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppSettings {
    #[serde(default)]
    pub frp_profiles: Vec<FrpProfile>,
    #[serde(default)]
    pub last_workspace_id: String,
    #[serde(default)]
    pub download: DownloadConfig,
    /// One authentication policy for the whole desktop app. Workspace auth
    /// fields are legacy compatibility data only and are overridden at runtime.
    #[serde(default)]
    pub auth: GlobalAuthConfig,
    /// Global service, tunnel and policy settings. Workspace copies are legacy only.
    #[serde(default)]
    pub general: GlobalGeneralConfig,
    /// Optional OpenAI Secure MCP Tunnel connector. Runtime/API keys remain in app_secrets.
    #[serde(default)]
    pub openai_connector: OpenAiConnectorConfig,
    /// Shared secrets indexed by key name (e.g. "bearer_token").
    /// Persisted alongside other app settings in app_settings.json.
    #[serde(default)]
    pub shared_secrets: HashMap<String, String>,
    /// Per-workspace secrets: workspace_id -> secret_key -> value.
    #[serde(default)]
    pub workspace_secrets: HashMap<String, HashMap<String, String>>,
    /// App-scoped secrets: scope -> item_id -> value (e.g. frp profile tokens).
    #[serde(default)]
    pub app_secrets: HashMap<String, HashMap<String, String>>,
}

fn default_frp_server_port() -> u16 {
    7000
}

fn default_github_mirror() -> String {
    "https://gh-proxy.com".to_string()
}

fn default_mcp_auth_type() -> String {
    "oauth".to_string()
}

fn default_permission_ceiling() -> String {
    "automatic".to_string()
}

impl AppSettings {
    /// Apply the single app-wide authentication policy to a workspace clone.
    /// Workspace auth fields are retained on disk only for backward compatibility;
    /// runtime services always receive this effective global configuration.
    pub fn apply_global_auth(&self, profile: &mut WorkspaceProfile) {
        profile.auth.auth_type = self.auth.mcp_auth_type.clone();
        profile.auth.use_shared_secrets = true;
        if let Some(client_id) = self.shared_secrets.get("oauth_client_id") {
            profile.auth.oauth_client_id = client_id.clone();
        }
    }

    pub fn apply_global_config(&self, profile: &mut WorkspaceProfile) {
        profile.tunnel = self.general.mcp_tunnel.clone();
        profile.runtime = self.general.mcp_runtime.clone();
        self.apply_global_auth(profile);
    }

    pub fn from_data(data: &AppData) -> Self {
        Self {
            frp_profiles: data.frp_profiles.clone(),
            last_workspace_id: data.last_workspace_id.clone(),
            download: data.download.clone(),
            auth: data.auth.clone(),
            general: data.general.clone(),
            openai_connector: data.openai_connector.clone(),
            shared_secrets: data.shared_secrets.clone(),
            workspace_secrets: data.workspace_secrets.clone(),
            app_secrets: data.app_secrets.clone(),
        }
    }

    pub fn apply_to(&self, data: &mut AppData) {
        data.frp_profiles = self.frp_profiles.clone();
        data.last_workspace_id = self.last_workspace_id.clone();
        data.download = self.download.clone();
        data.auth = self.auth.clone();
        data.general = self.general.clone();
        data.openai_connector = self.openai_connector.clone();
        data.shared_secrets = self.shared_secrets.clone();
        data.workspace_secrets = self.workspace_secrets.clone();
        data.app_secrets = self.app_secrets.clone();
    }

    pub fn load_or_default() -> Self {
        crate::data::DataStore::read_file(|data| Ok(Self::from_data(data))).unwrap_or_default()
    }

    pub fn find_frp_profile(&self, id: &str) -> Option<&FrpProfile> {
        if id.trim().is_empty() {
            return None;
        }
        self.frp_profiles.iter().find(|profile| profile.id == id)
    }
}

#[allow(dead_code)]
impl FrpProfile {
    pub fn new(name: String, server: String, server_port: u16) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string().replace('-', ""),
            name,
            server: server.trim().to_string(),
            server_port,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FrpProfile;

    #[test]
    fn accepts_frontend_camel_case_server_port() {
        let profile: FrpProfile = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "公司 FRP",
            "server": "frp.example.com",
            "serverPort": 7004
        }))
        .expect("FRP profile should deserialize");

        assert_eq!(profile.server_port, 7004);
    }

    #[test]
    fn keeps_legacy_snake_case_server_port_compatible() {
        let profile: FrpProfile = serde_json::from_value(serde_json::json!({
            "id": "p1",
            "name": "公司 FRP",
            "server": "frp.example.com",
            "server_port": 7005
        }))
        .expect("legacy FRP profile should deserialize");

        assert_eq!(profile.server_port, 7005);
    }
}
