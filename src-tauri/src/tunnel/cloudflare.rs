use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
use tokio::time;

use crate::error::{AppError, AppResult};
use crate::platform::{legacy_app_config_dir, platform};

const READY_TIMEOUT: Duration = Duration::from_secs(30);

/// Handle to a supervised `cloudflared` child process.
pub struct CloudflareTunnelHandle {
    pub child: Child,
    pub public_url: String,
    pub pid: Option<u32>,
}

fn migrate_legacy_cached_cloudflared() -> Option<PathBuf> {
    let current = cached_cloudflared_path()?;
    if current.is_file() {
        return Some(current);
    }
    let legacy = legacy_app_config_dir()?
        .join("bin")
        .join(cloudflared_binary_name());
    if !legacy.is_file() {
        return None;
    }
    if let Some(parent) = current.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    std::fs::copy(legacy, &current).ok()?;
    current.is_file().then_some(current)
}

pub fn resolve_cloudflared() -> AppResult<PathBuf> {
    platform()
        .cloudflared_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .or_else(|| cached_cloudflared_path().filter(|path| path.is_file()))
        .or_else(migrate_legacy_cached_cloudflared)
        .ok_or_else(|| {
            AppError::Message(
                "未找到 cloudflared。请到「软件管理」安装，或自行安装 Cloudflare Tunnel CLI。\n\
                 Windows 可执行：winget install Cloudflare.cloudflared"
                    .into(),
            )
        })
}

/// Path where the app caches a self-managed cloudflared binary.
pub(crate) fn cached_cloudflared_path() -> Option<PathBuf> {
    platform()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("bin").join(cloudflared_binary_name()))
}

pub(crate) fn cloudflared_binary_name() -> &'static str {
    #[cfg(windows)]
    {
        "cloudflared.exe"
    }
    #[cfg(not(windows))]
    {
        "cloudflared"
    }
}

/// GitHub release asset name for the current platform.
fn cloudflared_release_asset() -> AppResult<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        Ok("cloudflared-windows-amd64.exe")
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        Ok("cloudflared-windows-arm64.exe")
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        Ok("cloudflared-linux-amd64")
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        Ok("cloudflared-linux-arm64")
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        Ok("cloudflared-darwin-amd64.tgz")
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        Ok("cloudflared-darwin-arm64.tgz")
    }
    #[cfg(not(any(
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        Err(AppError::Message(
            "当前平台暂不支持自动下载 cloudflared。".into(),
        ))
    }
}

/// Latest cloudflared release. Pinned for reproducibility; bump as needed.
const CLOUDFLARED_VERSION: &str = "2025.6.1";

/// Download cloudflared into the app cache `bin/` directory, honoring the
/// configured mirror + proxy. Windows/Linux assets are raw binaries; macOS
/// assets are `.tgz` archives that need extraction.
pub(crate) async fn download_cloudflared_to_cache() -> AppResult<PathBuf> {
    let settings = crate::settings::AppSettings::load_or_default();
    let asset = cloudflared_release_asset()?;
    let url = format!(
        "https://github.com/cloudflare/cloudflared/releases/download/{CLOUDFLARED_VERSION}/{asset}"
    );
    let dest =
        cached_cloudflared_path().ok_or_else(|| AppError::Message("无法解析缓存目录。".into()))?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let bytes =
        crate::tunnel::download::download_release_asset(&settings, &url, "cloudflared").await?;

    if asset.ends_with(".tgz") {
        extract_cloudflared_from_tar_gz(&bytes, &dest)?;
    } else {
        std::fs::write(&dest, &bytes)?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(&dest) {
            let mut perms = meta.permissions();
            perms.set_mode(0o755);
            let _ = std::fs::set_permissions(&dest, perms);
        }
    }

    if dest.is_file() {
        Ok(dest)
    } else {
        Err(AppError::Message("cloudflared 自动安装失败。".into()))
    }
}

#[cfg(target_os = "macos")]
fn extract_cloudflared_from_tar_gz(bytes: &[u8], dest: &Path) -> AppResult<()> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    for entry in archive
        .entries()
        .map_err(|err| AppError::Message(format!("解压 cloudflared 安装包失败: {err}")))?
    {
        let mut entry = entry
            .map_err(|err| AppError::Message(format!("读取 cloudflared 安装包失败: {err}")))?;
        let path = entry
            .path()
            .map_err(|err| AppError::Message(err.to_string()))?
            .to_string_lossy()
            .replace('\\', "/");
        if path.ends_with("cloudflared") {
            let mut out = std::fs::File::create(dest)?;
            std::io::copy(&mut entry, &mut out)?;
            return Ok(());
        }
    }
    Err(AppError::Message(
        "cloudflared 安装包中未找到可执行文件。".into(),
    ))
}

#[cfg(not(target_os = "macos"))]
#[allow(dead_code)]
fn extract_cloudflared_from_tar_gz(_bytes: &[u8], _dest: &Path) -> AppResult<()> {
    Err(AppError::Message(
        "当前平台的 cloudflared 无需解压。".into(),
    ))
}

pub fn extract_trycloudflare_url(line: &str) -> Option<String> {
    const PREFIX: &str = "https://";
    const SUFFIX: &str = ".trycloudflare.com";
    let lower = line.to_ascii_lowercase();
    let mut search_from = 0;

    while let Some(rel) = lower[search_from..].find(PREFIX) {
        let start = search_from + rel;
        let Some(suffix_rel) = lower[start..].find(SUFFIX) else {
            break;
        };
        let end = start + suffix_rel + SUFFIX.len();
        let host = &line[start + PREFIX.len()..end - SUFFIX.len()];
        if host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') && !host.is_empty() {
            return Some(line[start..end].trim_end_matches('/').to_string());
        }
        search_from = start + PREFIX.len();
    }
    None
}

fn named_tunnel_connection_ready(line: &str) -> bool {
    line.to_ascii_lowercase()
        .contains("registered tunnel connection")
}

/// Spawn `cloudflared tunnel --url http://127.0.0.1:{port}` (quick) or named `tunnel run --token`.
pub async fn spawn_cloudflare_tunnel(
    port: u16,
    cwd: &Path,
    log_path: &Path,
    cloudflare_mode: &str,
    cloudflare_token: &str,
    named_public_url: &str,
) -> AppResult<CloudflareTunnelHandle> {
    let cloudflared = resolve_cloudflared()?;
    let quick = cloudflare_mode != "named";

    if !quick {
        if cloudflare_token.trim().is_empty() {
            return Err(AppError::Message(
                "Cloudflare 命名隧道模式需要填写 Tunnel Token。".into(),
            ));
        }
        if named_public_url.trim().is_empty() {
            return Err(AppError::Message(
                "Cloudflare 命名隧道模式需要填写固定公网地址。".into(),
            ));
        }
    }

    let mut cmd = Command::new(&cloudflared);
    cmd.current_dir(cwd);
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    #[cfg(windows)]
    {
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    }

    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    if quick {
        // Do not inherit a remotely-managed tunnel token into Quick mode.
        cmd.env_remove("TUNNEL_TOKEN");
        cmd.args(["tunnel", "--url", &format!("http://127.0.0.1:{port}")]);
    } else {
        // Use Cloudflare's environment form so the token is not exposed in
        // the process command line / Task Manager command-line details.
        cmd.env("TUNNEL_TOKEN", cloudflare_token.trim());
        cmd.args(["tunnel", "run"]);
    }

    let mut child = cmd
        .spawn()
        .map_err(|err| AppError::Message(format!("启动 cloudflared 失败: {err}")))?;
    let pid = child.id();

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let (ready_tx, ready_rx) = oneshot::channel();
    let log_path = log_path.to_path_buf();
    let named_url = named_public_url.trim_end_matches('/').to_string();
    let log_path_for_error = log_path.clone();

    if let Some(stdout) = child.stdout.take() {
        let stderr = child.stderr.take();
        tokio::spawn(async move {
            stream_cloudflare_output(stdout, stderr, &log_path, quick, named_url, ready_tx).await;
        });
    } else {
        // Missing output means we cannot prove that either Quick or Named is
        // actually connected. Never synthesize readiness here.
        let _ = ready_tx.send(QuickTunnelReady {
            public_url: None,
            named_ready: false,
        });
    }

    let ready = match time::timeout(READY_TIMEOUT, ready_rx).await {
        Ok(Ok(ready)) => ready,
        Ok(Err(_)) => {
            let _ = stop_child(child, pid).await;
            return Err(AppError::Message(
                "cloudflared 在连接就绪前结束了输出流。".into(),
            ));
        }
        Err(_) => {
            let detail = if quick {
                "没有返回 trycloudflare.com 公网地址"
            } else {
                "没有建立 Named Tunnel 连接"
            };
            let error = AppError::Message(format!(
                "cloudflared 已启动，但在 {} 秒内{detail}。\n\
                 请检查：1) MCP 服务是否已在本机端口 {port} 运行；2) Tunnel Token 与固定公网地址是否匹配；\
                 3) 查看日志 {log_hint}",
                READY_TIMEOUT.as_secs(),
                log_hint = log_path_for_error.display()
            ));
            let _ = stop_child(child, pid).await;
            return Err(error);
        }
    };

    if !quick && !ready.named_ready {
        let _ = stop_child(child, pid).await;
        return Err(AppError::Message(format!(
            "cloudflared 在建立 Named Tunnel 连接前退出。请检查 Tunnel Token，并查看日志：{}",
            log_path_for_error.display()
        )));
    }

    let public_url = if quick {
        match ready.public_url {
            Some(url) => url,
            None => {
                let error = AppError::Message(format!(
                    "cloudflared 已启动，但没有解析到 trycloudflare.com 地址。请查看日志：{}",
                    log_path_for_error.display()
                ));
                let _ = stop_child(child, pid).await;
                return Err(error);
            }
        }
    } else {
        named_public_url.trim_end_matches('/').to_string()
    };

    Ok(CloudflareTunnelHandle {
        child,
        public_url,
        pid,
    })
}

struct QuickTunnelReady {
    public_url: Option<String>,
    #[allow(dead_code)]
    named_ready: bool,
}

async fn stream_cloudflare_output<R, E>(
    stdout: R,
    stderr: Option<E>,
    log_path: &Path,
    quick: bool,
    named_url: String,
    ready_tx: oneshot::Sender<QuickTunnelReady>,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
    E: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let mut ready_tx = Some(ready_tx);
    let mut public_url: Option<String> = None;

    // Logging is best-effort only. A log-file failure must not be converted
    // into a fake Named Tunnel success, and must not stop us from consuming
    // cloudflared output to detect the real readiness line.
    let mut log = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .await
        .ok();

    let send_ready = |tx: &mut Option<oneshot::Sender<QuickTunnelReady>>,
                      url: Option<String>,
                      named_ready: bool| {
        if let Some(sender) = tx.take() {
            let _ = sender.send(QuickTunnelReady {
                public_url: url,
                named_ready,
            });
        }
    };

    let handle_line =
        |line: &str,
         public_url: &mut Option<String>,
         ready_tx: &mut Option<oneshot::Sender<QuickTunnelReady>>| {
            if quick {
                if public_url.is_none() {
                    if let Some(url) = extract_trycloudflare_url(line) {
                        *public_url = Some(url.clone());
                        send_ready(ready_tx, Some(url), false);
                    }
                }
            } else if named_tunnel_connection_ready(line) {
                send_ready(ready_tx, Some(named_url.clone()), true);
            }
        };

    // cloudflared logs primarily to stderr; read stdout and stderr concurrently.
    let (line_tx, mut line_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let stderr_line_tx = line_tx.clone();

    tokio::spawn(async move {
        let mut stdout = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = stdout.next_line().await {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    if let Some(stderr) = stderr {
        tokio::spawn(async move {
            let mut stderr = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = stderr.next_line().await {
                if stderr_line_tx.send(line).is_err() {
                    break;
                }
            }
        });
    }

    while let Some(line) = line_rx.recv().await {
        if let Some(log) = log.as_mut() {
            let _ = log.write_all(line.as_bytes()).await;
            let _ = log.write_all(b"\n").await;
            let _ = log.flush().await;
        }
        handle_line(&line, &mut public_url, &mut ready_tx);
    }

    // Stream EOF before a readiness line means startup failed. In particular,
    // an invalid Named Tunnel token must never be treated as a successful handover.
    send_ready(&mut ready_tx, public_url, false);
}

pub async fn stop_child(mut child: Child, pid: Option<u32>) -> AppResult<()> {
    if let Some(pid) = pid {
        let _ = platform().terminate_process_tree(pid);
    }

    let _ = child.kill().await;
    let _ = time::timeout(Duration::from_secs(3), child.wait()).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{extract_trycloudflare_url, named_tunnel_connection_ready};

    #[test]
    fn extracts_trycloudflare_url_from_log_line() {
        let line = "INF | https://abc-def.trycloudflare.com is your tunnel URL";
        assert_eq!(
            extract_trycloudflare_url(line).as_deref(),
            Some("https://abc-def.trycloudflare.com")
        );
    }

    #[test]
    fn ignores_invalid_hosts() {
        let line = "https://bad_host.trycloudflare.com";
        assert!(extract_trycloudflare_url(line).is_none());
    }

    #[test]
    fn named_tunnel_requires_registered_connection_line() {
        assert!(named_tunnel_connection_ready(
            "INF Registered tunnel connection connIndex=0 connection=abc"
        ));
        assert!(!named_tunnel_connection_ready(
            "INF Starting metrics server on 127.0.0.1:20241/metrics"
        ));
    }
}
