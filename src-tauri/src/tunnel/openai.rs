use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

use crate::error::{AppError, AppResult};
use crate::mcp::{TUNNEL_MCP_SECRET_KEY, TUNNEL_SECRET_SCOPE, TUNNEL_TOKEN_HEADER};
use crate::platform::ProcessTreeGuard;
use crate::secret::SecretStore;
use crate::settings::{AppSettings, OpenAiConnectorConfig};
use crate::tunnel::download::download_release_asset;

pub const TUNNEL_CLIENT_VERSION: &str = "0.0.10";
pub const TUNNEL_RUNTIME_KEY: &str = "runtime_api_key";
const MAX_TUNNEL_ASSET_BYTES: usize = 100 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(35);
const HEALTH_TIMEOUT: Duration = Duration::from_secs(2);
const LOG_TAIL_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenAiConnectorStatus {
    pub configured: bool,
    pub has_runtime_key: bool,
    pub tunnel_id: String,
    pub alias: String,
    pub binary_installed: bool,
    pub binary_version: String,
    pub process_running: bool,
    pub healthy: bool,
    pub ready: bool,
    pub runtime_state: Option<String>,
    pub ui_url: Option<String>,
    pub detail: String,
}

struct RunningTunnel {
    child: Child,
    _process_tree: ProcessTreeGuard,
    health_url_file: PathBuf,
    log_tail: Arc<Mutex<String>>,
}

#[derive(Clone, Default)]
pub struct OpenAiTunnelManager {
    inner: Arc<Mutex<Option<RunningTunnel>>>,
}

pub fn validate_tunnel_id(value: &str) -> AppResult<()> {
    let valid = value.len() == 39
        && value.starts_with("tunnel_")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    if valid {
        Ok(())
    } else {
        Err(AppError::Message(
            "Tunnel ID must be tunnel_ followed by 32 lowercase hexadecimal characters".into(),
        ))
    }
}

pub fn validate_alias(value: &str) -> AppResult<()> {
    if !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(AppError::Message(
            "OpenAI tunnel alias may contain only letters, digits, dot, underscore and dash".into(),
        ))
    }
}

pub async fn ensure_tunnel_client(settings: &AppSettings) -> AppResult<PathBuf> {
    let binary = tunnel_client_binary()?;
    if binary.is_file() {
        return Ok(binary);
    }
    let asset = release_asset_name()?;
    let base = format!(
        "https://github.com/openai/tunnel-client/releases/download/v{TUNNEL_CLIENT_VERSION}"
    );
    let checksums = download_release_asset(
        settings,
        &format!("{base}/SHA256SUMS.txt"),
        "tunnel-client checksums",
    )
    .await?;
    let archive =
        download_release_asset(settings, &format!("{base}/{asset}"), "tunnel-client").await?;
    if archive.len() > MAX_TUNNEL_ASSET_BYTES {
        return Err(AppError::Message(
            "tunnel-client release archive exceeds the 100 MiB safety limit".into(),
        ));
    }
    let expected = checksum_for_asset(&checksums, &asset)?;
    let actual = format!("{:x}", Sha256::digest(&archive));
    if actual != expected {
        return Err(AppError::Message(format!(
            "tunnel-client checksum mismatch for {asset}"
        )));
    }
    extract_tunnel_client(&archive, &binary)?;
    Ok(binary)
}

impl OpenAiTunnelManager {
    pub async fn status(&self, config: &OpenAiConnectorConfig) -> AppResult<OpenAiConnectorStatus> {
        let binary = tunnel_client_binary()?;
        let has_runtime_key = SecretStore::get_app(TUNNEL_SECRET_SCOPE, TUNNEL_RUNTIME_KEY)?
            .is_some_and(|value| !value.trim().is_empty());
        let configured = validate_tunnel_id(&config.tunnel_id).is_ok()
            && validate_alias(&config.alias).is_ok()
            && has_runtime_key;

        let running_snapshot = {
            let mut inner = self.inner.lock().await;
            let Some(runtime) = inner.as_mut() else {
                return Ok(offline_status(
                    config,
                    configured,
                    has_runtime_key,
                    binary.is_file(),
                    "OpenAI Tunnel is not running",
                ));
            };
            match runtime.child.try_wait() {
                Ok(None) => Some((
                    runtime.child.id(),
                    runtime.health_url_file.clone(),
                    runtime.log_tail.clone(),
                )),
                Ok(Some(exit)) => {
                    let tail = runtime.log_tail.clone();
                    let _finished = inner.take();
                    drop(inner);
                    let detail = tail.lock().await.clone();
                    return Ok(offline_status(
                        config,
                        configured,
                        has_runtime_key,
                        binary.is_file(),
                        &format!("tunnel-client exited with {exit}: {detail}"),
                    ));
                }
                Err(error) => {
                    let _finished = inner.take();
                    return Ok(offline_status(
                        config,
                        configured,
                        has_runtime_key,
                        binary.is_file(),
                        &format!("failed to query tunnel-client process: {error}"),
                    ));
                }
            }
        };
        let Some((pid, health_url_file, log_tail)) = running_snapshot else {
            unreachable!();
        };

        let base = std::fs::read_to_string(&health_url_file)
            .ok()
            .map(|value| value.trim().trim_end_matches('/').to_string())
            .filter(|value| {
                value.starts_with("http://127.0.0.1:") || value.starts_with("http://[::1]:")
            });
        let (healthy, ready) = if let Some(base) = base.as_deref() {
            let client = reqwest::Client::builder()
                .timeout(HEALTH_TIMEOUT)
                .no_proxy()
                .build()
                .map_err(|error| AppError::Message(error.to_string()))?;
            let health = client
                .get(format!("{base}/healthz"))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            let readiness = client
                .get(format!("{base}/readyz"))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            (health, readiness)
        } else {
            (false, false)
        };
        let runtime_state = Some(
            if ready {
                "ready"
            } else if healthy {
                "starting"
            } else {
                "degraded"
            }
            .to_string(),
        );
        let detail = if healthy && ready {
            format!("pid={pid:?} health=ok ready=yes")
        } else {
            let tail = log_tail.lock().await.clone();
            if tail.trim().is_empty() {
                format!("pid={pid:?} health={healthy} ready={ready}")
            } else {
                tail
            }
        };
        Ok(OpenAiConnectorStatus {
            configured,
            has_runtime_key,
            tunnel_id: config.tunnel_id.clone(),
            alias: config.alias.clone(),
            binary_installed: binary.is_file(),
            binary_version: TUNNEL_CLIENT_VERSION.into(),
            process_running: true,
            healthy,
            ready,
            runtime_state,
            ui_url: base.map(|base| format!("{base}/ui")),
            detail: redact_runtime_error(&detail, &config.tunnel_id),
        })
    }

    pub async fn start(
        &self,
        settings: &AppSettings,
        config: &OpenAiConnectorConfig,
        mcp_port: u16,
    ) -> AppResult<OpenAiConnectorStatus> {
        validate_tunnel_id(&config.tunnel_id)?;
        validate_alias(&config.alias)?;
        if mcp_port == 0 {
            return Err(AppError::Message(
                "active Mnelyra MCP port is invalid".into(),
            ));
        }
        let runtime_key = SecretStore::get_app(TUNNEL_SECRET_SCOPE, TUNNEL_RUNTIME_KEY)?
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| AppError::Message("OpenAI Tunnel runtime API key is missing".into()))?;
        let mcp_token = SecretStore::get_or_create_app(TUNNEL_SECRET_SCOPE, TUNNEL_MCP_SECRET_KEY)?;
        let binary = ensure_tunnel_client(settings).await?;
        self.shutdown().await;

        let runtime_key_file = write_secret_file("runtime-api.key", runtime_key.as_bytes())?;
        let mcp_token_file = write_secret_file("mcp-runtime.token", mcp_token.as_bytes())?;
        let root = connector_root()?;
        let runtime_dir = root.join("runtime");
        std::fs::create_dir_all(&runtime_dir)?;
        let health_url_file = runtime_dir.join(format!("{}.health-url", config.alias));
        if health_url_file.exists() {
            let _ = std::fs::remove_file(&health_url_file);
        }
        let profile_path = runtime_dir.join(format!("{}.yaml", config.alias));
        let log_path = runtime_dir.join(format!("{}.log", config.alias));
        let pid_path = runtime_dir.join(format!("{}.pid", config.alias));
        let profile = render_profile(
            config,
            mcp_port,
            &runtime_key_file,
            &mcp_token_file,
            &health_url_file,
            &log_path,
            &pid_path,
        );
        std::fs::write(&profile_path, profile)?;
        set_private_file(&profile_path)?;

        let mut command = Command::new(&binary);
        command
            .arg("run")
            .arg("--profile-file")
            .arg(&profile_path)
            .current_dir(&runtime_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| {
            AppError::Message(format!("failed to start tunnel-client: {error}"))
        })?;
        let process_tree = ProcessTreeGuard::attach(&child).map_err(AppError::Message)?;
        let log_tail = Arc::new(Mutex::new(String::new()));
        if let Some(stdout) = child.stdout.take() {
            spawn_log_reader(stdout, log_tail.clone());
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_log_reader(stderr, log_tail.clone());
        }
        *self.inner.lock().await = Some(RunningTunnel {
            child,
            _process_tree: process_tree,
            health_url_file,
            log_tail,
        });

        let deadline = tokio::time::Instant::now() + CONNECT_TIMEOUT;
        loop {
            let status = self.status(config).await?;
            if status.process_running && status.healthy && status.ready {
                return Ok(status);
            }
            if !status.process_running {
                self.shutdown().await;
                return Err(AppError::Message(format!(
                    "OpenAI Tunnel exited before ready: {}",
                    status.detail
                )));
            }
            if tokio::time::Instant::now() >= deadline {
                self.shutdown().await;
                return Err(AppError::Message(format!(
                    "OpenAI Tunnel did not become ready: {}",
                    status.detail
                )));
            }
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
    }

    pub async fn shutdown(&self) {
        let running = self.inner.lock().await.take();
        if let Some(mut running) = running {
            let _ = running.child.kill().await;
            // Dropping the ProcessTreeGuard closes the Windows Job Object and
            // terminates any descendants even when the tunnel binary spawned them.
        }
    }
}

fn offline_status(
    config: &OpenAiConnectorConfig,
    configured: bool,
    has_runtime_key: bool,
    binary_installed: bool,
    detail: &str,
) -> OpenAiConnectorStatus {
    OpenAiConnectorStatus {
        configured,
        has_runtime_key,
        tunnel_id: config.tunnel_id.clone(),
        alias: config.alias.clone(),
        binary_installed,
        binary_version: TUNNEL_CLIENT_VERSION.into(),
        process_running: false,
        healthy: false,
        ready: false,
        runtime_state: Some("offline".into()),
        ui_url: None,
        detail: redact_runtime_error(detail, &config.tunnel_id),
    }
}

fn render_profile(
    config: &OpenAiConnectorConfig,
    mcp_port: u16,
    runtime_key_file: &Path,
    mcp_token_file: &Path,
    health_url_file: &Path,
    log_path: &Path,
    pid_path: &Path,
) -> String {
    fn q(value: impl AsRef<str>) -> String {
        serde_json::to_string(value.as_ref()).unwrap_or_else(|_| "\"\"".into())
    }
    let mcp_url = format!("http://127.0.0.1:{mcp_port}/mcp");
    let runtime_key = q(format!("file:{}", runtime_key_file.to_string_lossy()));
    let mcp_token = q(format!("file:{}", mcp_token_file.to_string_lossy()));
    let lines = vec![
        "config_version: 1".to_string(),
        "control_plane:".to_string(),
        format!("  tunnel_id: {}", q(&config.tunnel_id)),
        format!("  api_key: {runtime_key}"),
        "mcp:".to_string(),
        "  server_urls:".to_string(),
        "    - channel: main".to_string(),
        format!("      url: {}", q(mcp_url)),
        "  extra_headers:".to_string(),
        format!("    {TUNNEL_TOKEN_HEADER}: {mcp_token}"),
        "  discovery_extra_headers:".to_string(),
        format!("    {TUNNEL_TOKEN_HEADER}: {mcp_token}"),
        "health:".to_string(),
        "  listen_addr: \"127.0.0.1:0\"".to_string(),
        format!("  url_file: {}", q(health_url_file.to_string_lossy())),
        "admin_ui:".to_string(),
        "  open_browser: false".to_string(),
        "process:".to_string(),
        format!("  pid_file: {}", q(pid_path.to_string_lossy())),
        "log:".to_string(),
        "  level: info".to_string(),
        "  format: json".to_string(),
        format!("  file: {}", q(log_path.to_string_lossy())),
    ];
    format!("{}\n", lines.join("\n"))
}

fn spawn_log_reader<R>(reader: R, tail: Arc<Mutex<String>>)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let mut tail = tail.lock().await;
            tail.push_str(&line);
            tail.push('\n');
            if tail.len() > LOG_TAIL_BYTES {
                let trim = tail.len() - LOG_TAIL_BYTES;
                let boundary = tail
                    .char_indices()
                    .find_map(|(index, _)| (index >= trim).then_some(index))
                    .unwrap_or(trim);
                tail.drain(..boundary);
            }
        }
    });
}

fn checksum_for_asset(checksums: &[u8], asset: &str) -> AppResult<String> {
    let text = std::str::from_utf8(checksums)
        .map_err(|_| AppError::Message("tunnel-client checksum file is not UTF-8".into()))?;
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(hash) = parts.next() else { continue };
        let Some(name) = parts.next() else { continue };
        if name.trim_start_matches('*') == asset
            && hash.len() == 64
            && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Ok(hash.to_ascii_lowercase());
        }
    }
    Err(AppError::Message(format!(
        "tunnel-client checksum list has no entry for {asset}"
    )))
}

fn extract_tunnel_client(archive: &[u8], destination: &Path) -> AppResult<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut zip = zip::ZipArchive::new(Cursor::new(archive))
        .map_err(|error| AppError::Message(format!("invalid tunnel-client zip: {error}")))?;
    let expected_name = if cfg!(target_os = "windows") {
        "tunnel-client.exe"
    } else {
        "tunnel-client"
    };
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|error| AppError::Message(error.to_string()))?;
        let name = Path::new(entry.name())
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if name != expected_name {
            continue;
        }
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if bytes.is_empty() || bytes.len() > MAX_TUNNEL_ASSET_BYTES {
            return Err(AppError::Message(
                "tunnel-client binary is empty or unexpectedly large".into(),
            ));
        }
        std::fs::write(destination, bytes)?;
        set_executable(destination)?;
        return Ok(());
    }
    Err(AppError::Message(format!(
        "tunnel-client archive does not contain {expected_name}"
    )))
}

fn write_secret_file(name: &str, bytes: &[u8]) -> AppResult<PathBuf> {
    if bytes.is_empty() || bytes.len() > 65_536 {
        return Err(AppError::Message(
            "OpenAI Tunnel secret is empty or unexpectedly large".into(),
        ));
    }
    let directory = connector_root()?.join("secrets");
    std::fs::create_dir_all(&directory)?;
    let path = directory.join(name);
    std::fs::write(&path, bytes)?;
    set_private_file(&path)?;
    Ok(path)
}

fn release_asset_name() -> AppResult<String> {
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        return Err(AppError::Message(
            "OpenAI tunnel-client is unsupported on this platform".into(),
        ));
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        return Err(AppError::Message(
            "OpenAI tunnel-client is unsupported on this CPU architecture".into(),
        ));
    };
    Ok(format!(
        "tunnel-client-v{TUNNEL_CLIENT_VERSION}-{platform}-{arch}.zip"
    ))
}

fn connector_root() -> AppResult<PathBuf> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| AppError::Message("could not resolve local application data".into()))?;
    Ok(base.join("Mnelyra").join("openai-tunnel"))
}

fn tunnel_client_binary() -> AppResult<PathBuf> {
    let name = if cfg!(target_os = "windows") {
        "tunnel-client.exe"
    } else {
        "tunnel-client"
    };
    Ok(connector_root()?
        .join("bin")
        .join(TUNNEL_CLIENT_VERSION)
        .join(name))
}

#[cfg(unix)]
fn set_executable(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> AppResult<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> AppResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> AppResult<()> {
    Ok(())
}

fn redact_runtime_error(value: &str, tunnel_id: &str) -> String {
    let mut text = value.replace(tunnel_id, "[tunnel-id]");
    if let Ok(pattern) = regex::Regex::new(r"sk-[A-Za-z0-9_-]{12,}") {
        text = pattern.replace_all(&text, "[redacted-key]").into_owned();
    }
    text.chars().take(2400).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_id_contract_matches_openai_runtime_shape() {
        assert!(validate_tunnel_id("tunnel_0123456789abcdef0123456789abcdef").is_ok());
        assert!(validate_tunnel_id("tunnel_0123456789ABCDEF0123456789abcdef").is_err());
        assert!(validate_tunnel_id("not-a-tunnel").is_err());
    }

    #[test]
    fn release_asset_name_matches_supported_distribution() {
        let asset = release_asset_name().expect("asset name");
        assert!(asset.starts_with("tunnel-client-v0.0.10-"));
        assert!(asset.ends_with(".zip"));
    }

    #[test]
    fn checksum_parser_selects_exact_asset() {
        let list = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other.zip\n\
bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb  wanted.zip\n";
        assert_eq!(
            checksum_for_asset(list, "wanted.zip").expect("checksum"),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        );
    }

    #[test]
    fn rendered_profile_uses_secret_file_references_only() {
        let config = OpenAiConnectorConfig {
            enabled: true,
            tunnel_id: "tunnel_0123456789abcdef0123456789abcdef".into(),
            alias: "mnelyra".into(),
        };
        let profile = render_profile(
            &config,
            3000,
            Path::new("C:/mnelyra/runtime-api.key"),
            Path::new("C:/mnelyra/mcp.token"),
            Path::new("C:/mnelyra/health.url"),
            Path::new("C:/mnelyra/runtime.log"),
            Path::new("C:/mnelyra/runtime.pid"),
        );
        assert!(profile.contains("api_key: \"file:C:/mnelyra/runtime-api.key\""));
        assert!(profile.contains(&format!(
            "{TUNNEL_TOKEN_HEADER}: \"file:C:/mnelyra/mcp.token\""
        )));
        assert!(profile.contains(&format!(
            "\n  extra_headers:\n    {TUNNEL_TOKEN_HEADER}: \"file:C:/mnelyra/mcp.token\"\n"
        )));
        assert!(profile.contains(&format!(
            "\n  discovery_extra_headers:\n    {TUNNEL_TOKEN_HEADER}: \"file:C:/mnelyra/mcp.token\"\n"
        )));
        assert!(!profile.contains(&format!("\n{TUNNEL_TOKEN_HEADER}:")));
        assert!(profile.contains("url: \"http://127.0.0.1:3000/mcp\""));
        assert!(profile.contains("listen_addr: \"127.0.0.1:0\""));
        assert!(!profile.contains("sk-"));
        assert!(!profile.contains("test-local-token"));
    }
}
