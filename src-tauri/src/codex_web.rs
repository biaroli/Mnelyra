use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::{AppError, AppResult};

const START_TIMEOUT: Duration = Duration::from_secs(35);
const SOL_CONTEXT_WINDOW: u64 = 1_050_000;
const CONTEXT_POLICY_MARKER: &str = "useNativeDefault:!0";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexWebBridgeStatus {
    pub launcher_installed: bool,
    pub cli_installed: bool,
    pub version: String,
    pub mode: String,
    pub app_name: Option<String>,
    pub route_installed: bool,
    pub route_active: bool,
    pub route_url: Option<String>,
    pub browser_ready: bool,
    pub browser_busy: bool,
    pub proxy_ready: bool,
    pub tunnel_id: Option<String>,
    pub tunnel_key_configured: bool,
    pub tunnel_ready: bool,
    pub ready: bool,
    pub detail: String,
}

fn launcher_owner_alive(launcher: &Path) -> bool {
    let Some(descriptor) = load_launcher_descriptor() else {
        return false;
    };
    if !same_executable_path(Path::new(&descriptor.helper.executable), launcher) {
        return false;
    }
    let platform = crate::platform::platform();
    if !platform.is_process_alive(descriptor.pid) {
        return false;
    }
    platform
        .process_image_path(descriptor.pid)
        .ok()
        .flatten()
        .is_some_and(|image| same_executable_path(Path::new(&image), launcher))
}

fn installed_runtime_bundle_path(launcher: &Path) -> Option<PathBuf> {
    let root = launcher.parent()?;
    Some(
        root.join("resources")
            .join("runtime")
            .join("app")
            .join("cli.js"),
    )
}

fn versioned_runtime_bundle_path(version: &str) -> Option<PathBuf> {
    let version = version.trim();
    if version.is_empty() {
        return None;
    }
    let versions = dirs::home_dir()?
        .join(".codex-chatgpt-web")
        .join("versions");
    let entries = std::fs::read_dir(versions).ok()?;
    let prefix = format!("{version}-");
    let mut candidates = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().ok().is_some_and(|kind| kind.is_dir()))
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .filter_map(|entry| {
            let bundle = entry.path().join("app").join("cli.js");
            bundle.is_file().then(|| {
                let modified = entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok());
                (modified, bundle)
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));
    candidates.into_iter().next().map(|(_, path)| path)
}

fn runtime_bundle_path(launcher: &Path, version: &str) -> Option<PathBuf> {
    versioned_runtime_bundle_path(version).or_else(|| installed_runtime_bundle_path(launcher))
}

fn restore_unused_installed_runtime_source(launcher: &Path, active_bundle: &Path) {
    let Some(installed) = installed_runtime_bundle_path(launcher) else {
        return;
    };
    if same_executable_path(&installed, active_bundle) {
        return;
    }
    let backup = installed.with_file_name("cli.js.bak-mnelyra-context-policy");
    if !backup.is_file() || !installed.is_file() {
        return;
    }
    let Ok(source) = std::fs::read_to_string(&installed) else {
        return;
    };
    if !source.contains(CONTEXT_POLICY_MARKER) {
        return;
    }
    let _ = std::fs::copy(backup, installed);
}

fn same_executable_path(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => {
            #[cfg(target_os = "windows")]
            {
                left.to_string_lossy()
                    .eq_ignore_ascii_case(&right.to_string_lossy())
            }
            #[cfg(not(target_os = "windows"))]
            {
                left == right
            }
        }
        _ => left == right,
    }
}

fn patch_runtime_context_policy_source(source: &str) -> Result<Option<String>, String> {
    let mut patched = source.to_string();
    let mut changed = false;

    if !patched.contains(CONTEXT_POLICY_MARKER) {
        let marker = "ChatGPT Plus context limit is not defined for unavailable effort";
        let marker_pos = patched
            .find(marker)
            .ok_or_else(|| "runtime context-limit marker was not found".to_string())?;
        let start = marker_pos.saturating_sub(1600);
        let region = &patched[start..marker_pos];
        let branch = Regex::new(
            r#"if\(([A-Za-z_$][A-Za-z0-9_$]*)==="medium"\|\|([A-Za-z_$][A-Za-z0-9_$]*)==="high"\)return ([A-Za-z_$][A-Za-z0-9_$]*)\(([0-9]+),([0-9]+)\);"#,
        )
        .map_err(|error| error.to_string())?;
        let captures = branch
            .captures(region)
            .ok_or_else(|| "runtime Plus High/Medium context branch was not found".to_string())?;
        if captures.get(1).map(|value| value.as_str())
            != captures.get(2).map(|value| value.as_str())
        {
            return Err("runtime context branch uses mismatched effort variables".into());
        }
        let effort = captures
            .get(1)
            .ok_or_else(|| "runtime effort variable is missing".to_string())?
            .as_str();
        let matched = captures
            .get(0)
            .ok_or_else(|| "runtime context branch match is missing".to_string())?;
        let replacement = format!(
            "if({effort}===\"medium\"||{effort}===\"high\")return{{contextWindow:{SOL_CONTEXT_WINDOW},maxContextWindow:{SOL_CONTEXT_WINDOW},{CONTEXT_POLICY_MARKER}}};"
        );
        patched.replace_range(start + matched.start()..start + matched.end(), &replacement);
        changed = true;
    }

    if !patched.contains("auto_compact_token_limit:")
        || !patched.contains(".useNativeDefault?void 0:")
    {
        let template_marker = "native Codex model template";
        let template_pos = patched
            .find(template_marker)
            .ok_or_else(|| "runtime native model template marker was not found".to_string())?;
        let prefix = &patched[..template_pos];
        let assignment = prefix
            .rfind("=G4(")
            .ok_or_else(|| "runtime native model template assignment was not found".to_string())?;
        let native_var = identifier_before(&patched, assignment)
            .ok_or_else(|| "runtime native model template variable was not found".to_string())?;

        let fields = Regex::new(
            r#"context_window:([A-Za-z_$][A-Za-z0-9_$]*)\.contextWindow,max_context_window:([A-Za-z_$][A-Za-z0-9_$]*)\.contextWindow,effective_context_window_percent:([A-Za-z_$][A-Za-z0-9_$]*)\.effectiveContextWindowPercent,auto_compact_token_limit:([A-Za-z_$][A-Za-z0-9_$]*)\.autoCompactTokenLimit"#,
        )
        .map_err(|error| error.to_string())?;
        let tail = &patched[template_pos..];
        let captures = fields
            .captures(tail)
            .ok_or_else(|| "runtime web-model context fields were not found".to_string())?;
        let budget_var = captures
            .get(1)
            .ok_or_else(|| "runtime context budget variable is missing".to_string())?
            .as_str();
        for index in 2..=4 {
            if captures.get(index).map(|value| value.as_str()) != Some(budget_var) {
                return Err(
                    "runtime web-model context fields use mismatched budget variables".into(),
                );
            }
        }
        let matched = captures
            .get(0)
            .ok_or_else(|| "runtime web-model context field match is missing".to_string())?;
        let replacement = format!(
            "context_window:{budget_var}.useNativeDefault?({native_var}.context_window??{budget_var}.contextWindow):{budget_var}.contextWindow,max_context_window:{budget_var}.maxContextWindow??{budget_var}.contextWindow,effective_context_window_percent:{budget_var}.useNativeDefault?({native_var}.effective_context_window_percent??95):{budget_var}.effectiveContextWindowPercent,auto_compact_token_limit:{budget_var}.useNativeDefault?void 0:{budget_var}.autoCompactTokenLimit"
        );
        patched.replace_range(
            template_pos + matched.start()..template_pos + matched.end(),
            &replacement,
        );
        changed = true;
    }

    Ok(changed.then_some(patched))
}

fn identifier_before(source: &str, end: usize) -> Option<&str> {
    if end == 0 || end > source.len() {
        return None;
    }
    let bytes = source.as_bytes();
    let mut start = end;
    while start > 0 {
        let byte = bytes[start - 1];
        if byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$') {
            start -= 1;
        } else {
            break;
        }
    }
    (start < end).then_some(&source[start..end])
}

fn codex_model_cache_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex").join("models_cache.json"))
}

fn cached_web_model_policy_is_stale(value: &serde_json::Value) -> bool {
    let models = value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    models.iter().any(|model| {
        let slug = model
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        if !matches!(slug, "chatgpt-web/medium" | "chatgpt-web/high") {
            return false;
        }
        let explicit_compaction = model
            .get("auto_compact_token_limit")
            .is_some_and(|value| !value.is_null());
        let max_context = model
            .get("max_context_window")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        explicit_compaction || max_context < SOL_CONTEXT_WINDOW
    })
}

fn clear_codex_model_cache() {
    let Some(path) = codex_model_cache_path() else {
        return;
    };
    let _ = std::fs::remove_file(path);
}

fn clear_stale_codex_model_cache() {
    let Some(path) = codex_model_cache_path() else {
        return;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return;
    };
    if cached_web_model_policy_is_stale(&value) {
        let _ = std::fs::remove_file(path);
    }
}

fn apply_runtime_context_policy(launcher: &Path, version: &str) -> AppResult<bool> {
    let Some(bundle) = runtime_bundle_path(launcher, version) else {
        return Ok(false);
    };
    if !bundle.is_file() {
        return Ok(false);
    }
    restore_unused_installed_runtime_source(launcher, &bundle);
    let source = std::fs::read_to_string(&bundle).map_err(|error| {
        AppError::Message(format!(
            "failed to read Web-model runtime bundle {}: {error}",
            bundle.display()
        ))
    })?;
    let Some(patched) = patch_runtime_context_policy_source(&source).map_err(|error| {
        AppError::Message(format!(
            "Web-model runtime context policy is unsupported: {error}"
        ))
    })?
    else {
        return Ok(false);
    };

    let backup = bundle.with_file_name("cli.js.bak-mnelyra-context-policy");
    if !backup.exists() {
        std::fs::copy(&bundle, &backup).map_err(|error| {
            AppError::Message(format!(
                "failed to back up Web-model runtime bundle {}: {error}",
                backup.display()
            ))
        })?;
    }
    if let Err(error) = std::fs::write(&bundle, patched) {
        let _ = std::fs::copy(&backup, &bundle);
        return Err(AppError::Message(format!(
            "failed to update Web-model runtime context policy: {error}"
        )));
    }
    clear_codex_model_cache();
    Ok(true)
}

fn load_launcher_descriptor() -> Option<LauncherDescriptor> {
    let path = launcher_descriptor_path().ok()?;
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn same_executable(left: &Path, right: &Path) -> bool {
    match (std::fs::canonicalize(left), std::fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => {
            #[cfg(target_os = "windows")]
            {
                left.to_string_lossy()
                    .eq_ignore_ascii_case(&right.to_string_lossy())
            }
            #[cfg(not(target_os = "windows"))]
            {
                left == right
            }
        }
        _ => false,
    }
}

async fn terminate_launcher_owner(launcher: &Path) -> AppResult<bool> {
    let Some(descriptor) = load_launcher_descriptor() else {
        return Ok(false);
    };
    if !same_executable(Path::new(&descriptor.helper.executable), launcher) {
        return Err(AppError::Message(
            "Web-model launcher descriptor points to a different executable".into(),
        ));
    }
    let platform = crate::platform::platform();
    if !platform.is_process_alive(descriptor.pid) {
        return Ok(false);
    }
    let image = match platform.process_image_path(descriptor.pid) {
        Ok(image) => image,
        Err(_error) if !platform.is_process_alive(descriptor.pid) => return Ok(false),
        Err(error) => return Err(error),
    };
    let Some(image) = image else {
        return Ok(false);
    };
    if !same_executable(Path::new(&image), launcher) {
        return Err(AppError::Message(
            "Web-model launcher PID no longer belongs to the configured launcher".into(),
        ));
    }

    #[cfg(target_os = "windows")]
    {
        let status = Command::new("taskkill.exe")
            .arg("/PID")
            .arg(descriptor.pid.to_string())
            .arg("/T")
            .arg("/F")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map_err(|error| {
                AppError::Message(format!("failed to restart Web-model launcher: {error}"))
            })?;
        if !status.success() {
            return Err(AppError::Message(
                "failed to stop the previous Web-model launcher process".into(),
            ));
        }
    }

    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(descriptor.pid as i32, libc::SIGTERM) };
        if result != 0 {
            return Err(AppError::Message(
                "failed to stop the previous Web-model launcher process".into(),
            ));
        }
    }

    tokio::time::sleep(Duration::from_millis(450)).await;
    Ok(true)
}

fn schedule_context_policy_reload(launcher: PathBuf, cli: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30 * 60);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let busy = run_cli_json::<DoctorReport>(&cli, &["doctor", "--json"])
                .await
                .ok()
                .is_some_and(|report| browser_host_busy(&report));
            if busy {
                continue;
            }
            match terminate_launcher_owner(&launcher).await {
                Ok(_) => {
                    if let Err(error) = launch_hidden(&launcher) {
                        eprintln!("Web-model context policy reload failed: {error}");
                    }
                }
                Err(error) => eprintln!("Web-model context policy reload failed: {error}"),
            }
            return;
        }
        eprintln!("Web-model context policy reload remained pending while the browser stayed busy");
    });
}

async fn ensure_runtime_context_policy(launcher: &Path, cli: &Path) {
    let version = run_cli(cli, &["--version"])
        .await
        .map(|value| normalize_version(&value))
        .unwrap_or_default();
    let applied = match apply_runtime_context_policy(launcher, &version) {
        Ok(applied) => applied,
        Err(error) => {
            eprintln!("Web-model context policy update skipped: {error}");
            return;
        }
    };
    clear_stale_codex_model_cache();
    if !applied {
        return;
    }

    let busy = run_cli_json::<DoctorReport>(cli, &["doctor", "--json"])
        .await
        .ok()
        .is_some_and(|report| browser_host_busy(&report));
    if busy {
        schedule_context_policy_reload(launcher.to_path_buf(), cli.to_path_buf());
        return;
    }

    match terminate_launcher_owner(launcher).await {
        Ok(true) => {
            if let Err(error) = launch_hidden(launcher) {
                eprintln!("Web-model context policy launcher restart failed: {error}");
            }
        }
        Ok(false) => {}
        Err(error) => eprintln!("Web-model context policy launcher restart failed: {error}"),
    }
}

#[derive(Debug, Deserialize)]
struct LauncherDescriptor {
    pid: u32,
    helper: LauncherHelper,
}

#[derive(Debug, Deserialize)]
struct LauncherHelper {
    executable: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredConfig {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    app_name: Option<String>,
    #[serde(default)]
    tunnel: Option<StoredTunnelConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StoredTunnelConfig {
    #[serde(default)]
    tunnel_id: String,
    #[serde(default)]
    runtime_key_file: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CodexWebSetupCredentials {
    pub tunnel_id: String,
    pub runtime_key: Option<String>,
}

struct TemporaryRuntimeKey {
    path: PathBuf,
}

impl TemporaryRuntimeKey {
    fn create(value: &str) -> AppResult<Self> {
        let path = std::env::temp_dir().join(format!(
            "mnelyra-codex-web-runtime-{}.key",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, value).map_err(|error| {
            AppError::Message(format!(
                "failed to prepare Codex Web runtime key file: {error}"
            ))
        })?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryRuntimeKey {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn stored_config_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex-chatgpt-web").join("config.json"))
}

fn load_stored_config() -> StoredConfig {
    let Some(path) = stored_config_path() else {
        return StoredConfig::default();
    };
    let Ok(raw) = std::fs::read_to_string(path) else {
        return StoredConfig::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn browser_host_busy(report: &DoctorReport) -> bool {
    report
        .checks
        .iter()
        .find(|check| check.id == "browser-host")
        .is_some_and(|check| {
            check.status == "error"
                && check
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("ChatGPT browser is running Codex turn"))
        })
}

fn launcher_descriptor_path() -> AppResult<PathBuf> {
    let home = dirs::home_dir()
        .ok_or_else(|| AppError::Message("could not resolve the user home directory".into()))?;
    Ok(home
        .join(".codex-chatgpt-web")
        .join("runtime")
        .join("launcher-browser.json"))
}

async fn run_cli_owned(cli: &Path, args: &[String]) -> AppResult<String> {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_cli(cli, &refs).await
}

async fn wait_for_browser_owner(cli: &Path) -> AppResult<()> {
    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if let Ok(report) = run_cli_json::<DoctorReport>(cli, &["doctor", "--json"]).await {
            if check_ok(&report, "browser-host") {
                return Ok(());
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    Err(AppError::Message(
        "Codex Web browser owner did not become ready. Open the launcher once and finish ChatGPT sign-in/onboarding."
            .into(),
    ))
}

async fn install_route_from_existing_config(
    cli: &Path,
    credentials: Option<&CodexWebSetupCredentials>,
) -> AppResult<()> {
    let report = run_cli_json::<DoctorReport>(cli, &["doctor", "--json"]).await?;
    let stored = load_stored_config();
    let mode = report
        .mode
        .as_deref()
        .filter(|value| matches!(*value, "full" | "browser-only"))
        .or_else(|| {
            matches!(stored.mode.as_str(), "full" | "browser-only")
                .then_some(stored.mode.as_str())
        })
        .or_else(|| credentials.map(|_| "full"))
        .ok_or_else(|| {
            AppError::Message(
                "Codex Web has no reusable setup mode. Finish the one-time ChatGPT browser sign-in or provide full-mode Tunnel credentials in Mnelyra."
                    .into(),
            )
        })?;
    let descriptor = launcher_descriptor_path()?;
    let descriptor = descriptor.to_string_lossy().to_string();
    let mut args = vec![
        "setup".to_string(),
        format!("--{mode}"),
        "--browser-host-descriptor".to_string(),
        descriptor,
        "--acknowledge-unofficial".to_string(),
        "--replace-codex-route".to_string(),
    ];
    let mut temporary_key = None;
    if mode == "full" {
        let tunnel_id = credentials
            .map(|value| value.tunnel_id.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| {
                stored.tunnel.as_ref().and_then(|tunnel| {
                    let value = tunnel.tunnel_id.trim();
                    (!value.is_empty()).then(|| value.to_string())
                })
            })
            .ok_or_else(|| {
                AppError::Message(
                    "Codex Web full mode requires an OpenAI Tunnel ID. Save it in the Codex Web setup section first."
                        .into(),
                )
            })?;

        let runtime_key_path = if let Some(value) = credentials
            .and_then(|value| value.runtime_key.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            temporary_key = Some(TemporaryRuntimeKey::create(value)?);
            temporary_key
                .as_ref()
                .expect("temporary key was just created")
                .path
                .clone()
        } else {
            stored
                .tunnel
                .as_ref()
                .and_then(|tunnel| tunnel.runtime_key_file.as_deref())
                .map(PathBuf::from)
                .filter(|path| path.is_file())
                .ok_or_else(|| {
                    AppError::Message(
                        "Codex Web full mode requires an OpenAI Tunnel Runtime Key. Save it in Mnelyra or complete the upstream setup once."
                            .into(),
                    )
                })?
        };

        args.extend([
            "--tunnel-id".to_string(),
            tunnel_id,
            "--runtime-key-file".to_string(),
            runtime_key_path.to_string_lossy().to_string(),
        ]);
    }
    let output = run_cli_owned(cli, &args).await?;
    drop(temporary_key);
    if !output.contains("Setup complete:") {
        return Err(AppError::Message(
            "Codex Web setup did not report a completed installation".into(),
        ));
    }
    Ok(())
}

/// Recover an already-installed Web-model route when Mnelyra starts.
///
/// This is intentionally conservative: first-time setup remains explicit because it can require
/// ChatGPT sign-in and connector authorization. Once the reversible Codex route exists, Mnelyra
/// owns keeping its browser/Responses sidecar alive instead of leaving Codex pointed at 17841 with
/// no daemon behind it.
pub async fn auto_start_if_installed() {
    let runtime = CodexWebRuntime::discover();
    let (Some(launcher), Some(cli)) = (runtime.launcher.as_deref(), runtime.cli.as_deref()) else {
        return;
    };
    ensure_runtime_context_policy(launcher, cli).await;
    let Ok(route) = run_cli_json::<RouteStatus>(cli, &["route", "status"]).await else {
        return;
    };
    if !route.installed || !route.active {
        return;
    }

    // A full doctor probe can take several seconds when the previous launcher descriptor points
    // at a dead process. Check the owned PID first so cold-start recovery can launch Electron
    // immediately, then use doctor only for readiness polling.
    if !launcher_owner_alive(launcher) {
        if let Err(error) = launch_hidden(launcher) {
            eprintln!("Codex Web auto-start failed: {error}");
            return;
        }
    } else if let Ok(current) = status().await {
        if current.ready {
            return;
        }
    }

    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        match status().await {
            Ok(current) if current.ready => return,
            Ok(_) => tokio::time::sleep(Duration::from_millis(900)).await,
            Err(error) => {
                eprintln!("Codex Web auto-start health check failed: {error}");
                return;
            }
        }
    }
    if let Ok(current) = status().await {
        eprintln!("Codex Web auto-start timed out: {}", current.detail);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteStatus {
    #[serde(default)]
    installed: bool,
    #[serde(default)]
    active: bool,
    route_url: Option<String>,
    #[serde(default)]
    errors: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DoctorReport {
    #[serde(default)]
    ok: bool,
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    checks: Vec<DoctorCheck>,
}

#[derive(Debug, Deserialize)]
struct DoctorCheck {
    id: String,
    status: String,
    message: String,
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexWebRuntime {
    launcher: Option<PathBuf>,
    cli: Option<PathBuf>,
}

impl CodexWebRuntime {
    fn discover() -> Self {
        let launcher = discover_launcher();
        let cli = discover_cli(launcher.as_deref());
        Self { launcher, cli }
    }
}

pub async fn status() -> AppResult<CodexWebBridgeStatus> {
    let runtime = CodexWebRuntime::discover();
    let Some(cli) = runtime.cli.as_deref() else {
        return Ok(CodexWebBridgeStatus {
            launcher_installed: runtime.launcher.is_some(),
            cli_installed: false,
            version: String::new(),
            mode: String::new(),
            app_name: None,
            route_installed: false,
            route_active: false,
            route_url: None,
            browser_ready: false,
            browser_busy: false,
            proxy_ready: false,
            tunnel_id: None,
            tunnel_key_configured: false,
            tunnel_ready: false,
            ready: false,
            detail: "Codex Web runtime is not installed".into(),
        });
    };
    let stored = load_stored_config();

    let version = run_cli(cli, &["--version"]).await.unwrap_or_default();
    let route = run_cli_json::<RouteStatus>(cli, &["route", "status"])
        .await
        .unwrap_or(RouteStatus {
            installed: false,
            active: false,
            route_url: None,
            errors: vec!["Unable to read Codex route status".into()],
        });
    let doctor = run_cli_json::<DoctorReport>(cli, &["doctor", "--json"])
        .await
        .unwrap_or(DoctorReport {
            ok: false,
            mode: None,
            checks: Vec::new(),
        });

    // Upstream doctor cannot perform its normal browser verification while a Codex turn owns the
    // launcher browser. In that state it reports browser-host as `error`, even though the bridge is
    // demonstrably live and serving that turn. Treat that specific busy state as usable instead of
    // making Mnelyra claim the runtime is down precisely while Codex is working.
    let browser_busy = browser_host_busy(&doctor);
    let browser_ready = check_ok(&doctor, "browser-host") || browser_busy;
    let proxy_ready = check_ok(&doctor, "proxy");
    let tunnel_check = doctor
        .checks
        .iter()
        .find(|check| check.id == "tunnel-runtime");
    let tunnel_key_configured = doctor
        .checks
        .iter()
        .find(|check| check.id == "tunnel-key")
        .is_some_and(|check| check.status == "ok");
    // browser-only mode intentionally has no tunnel check; in that mode the proxy can be ready
    // without a tunnel. Treat an absent tunnel-runtime row as not required.
    let tunnel_ready = tunnel_check.is_none_or(|check| check.status == "ok");
    let blocking_doctor_error = doctor
        .checks
        .iter()
        .any(|check| check.status == "error" && !(check.id == "browser-host" && browser_busy));
    let ready = route.installed
        && route.active
        && browser_ready
        && proxy_ready
        && tunnel_ready
        && (doctor.ok || browser_busy)
        && !blocking_doctor_error;

    let detail = if ready {
        "Codex Web models are routed through the live local Responses bridge".into()
    } else {
        summarize_failure(&runtime, &route, &doctor)
    };

    Ok(CodexWebBridgeStatus {
        launcher_installed: runtime.launcher.is_some(),
        cli_installed: true,
        version: normalize_version(&version),
        mode: doctor
            .mode
            .clone()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(stored.mode),
        app_name: stored.app_name,
        route_installed: route.installed,
        route_active: route.active,
        route_url: route.route_url,
        browser_ready,
        browser_busy,
        proxy_ready,
        tunnel_id: stored.tunnel.and_then(|tunnel| {
            let value = tunnel.tunnel_id.trim().to_string();
            (!value.is_empty()).then_some(value)
        }),
        tunnel_key_configured,
        tunnel_ready,
        ready,
        detail,
    })
}

pub async fn start_and_wait(
    credentials: Option<CodexWebSetupCredentials>,
) -> AppResult<CodexWebBridgeStatus> {
    let runtime = CodexWebRuntime::discover();
    let launcher = runtime.launcher.as_deref().ok_or_else(|| {
        AppError::Message(
            "Codex Web GPT launcher is not installed. Mnelyra found no embedded-browser owner to start."
                .into(),
        )
    })?;
    let cli = runtime.cli.as_deref().ok_or_else(|| {
        AppError::Message("Codex Web runtime CLI is missing from the launcher installation".into())
    })?;

    ensure_runtime_context_policy(launcher, cli).await;

    let mut route = run_cli_json::<RouteStatus>(cli, &["route", "status"]).await?;
    if !route.installed {
        // Existing codex-chatgpt-web configuration is enough to reinstall its reversible Codex
        // route. The launcher browser must be alive first because Windows/Linux setup probes that
        // exact owned surface rather than starting a second browser.
        launch_hidden(launcher)?;
        wait_for_browser_owner(cli).await?;
        install_route_from_existing_config(cli, credentials.as_ref()).await?;
        route = run_cli_json::<RouteStatus>(cli, &["route", "status"]).await?;
        if !route.installed {
            return Err(AppError::Message(
                "Codex Web setup completed without installing a Codex model route".into(),
            ));
        }
    }
    if !route.active {
        let output = run_cli(cli, &["route", "connect"]).await?;
        if output.trim().is_empty() {
            return Err(AppError::Message(
                "Codex Web route activation returned no result".into(),
            ));
        }
    }

    if !launcher_owner_alive(launcher) {
        launch_hidden(launcher)?;
    }

    let deadline = tokio::time::Instant::now() + START_TIMEOUT;
    loop {
        let current = status().await?;
        if current.ready {
            return Ok(current);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AppError::Message(format!(
                "Codex Web bridge did not become ready: {}",
                current.detail
            )));
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

fn check_ok(report: &DoctorReport, id: &str) -> bool {
    report
        .checks
        .iter()
        .find(|check| check.id == id)
        .is_some_and(|check| check.status == "ok")
}

fn summarize_failure(
    runtime: &CodexWebRuntime,
    route: &RouteStatus,
    report: &DoctorReport,
) -> String {
    if !route.installed {
        return "Codex Web model route is not installed".into();
    }
    if !route.active {
        return "Codex Web model route is installed but disconnected".into();
    }
    if runtime.launcher.is_none() {
        return "Codex route is active, but the embedded browser launcher is not installed".into();
    }
    if let Some(check) = report.checks.iter().find(|check| check.status == "error") {
        return format!("{}: {}", check.id, check.message);
    }
    if let Some(error) = route.errors.first() {
        return error.clone();
    }
    "Codex Web route exists but the local Responses bridge is not ready".into()
}

async fn run_cli_json<T>(cli: &Path, args: &[&str]) -> AppResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let raw = run_cli_allow_failure(cli, args).await?;
    serde_json::from_str(raw.trim()).map_err(|error| {
        AppError::Message(format!(
            "Codex Web runtime returned invalid JSON for {}: {error}",
            args.join(" ")
        ))
    })
}

async fn run_cli(cli: &Path, args: &[&str]) -> AppResult<String> {
    let output = command_for_cli(cli, args)
        .output()
        .await
        .map_err(|error| AppError::Message(format!("failed to run Codex Web runtime: {error}")))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Message(if detail.is_empty() {
            format!("Codex Web runtime exited with {}", output.status)
        } else {
            detail
        }));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

async fn run_cli_allow_failure(cli: &Path, args: &[&str]) -> AppResult<String> {
    let output = command_for_cli(cli, args)
        .output()
        .await
        .map_err(|error| AppError::Message(format!("failed to run Codex Web runtime: {error}")))?;
    // `doctor --json` intentionally exits 1 while reporting a useful structured degraded state.
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::Message(if stderr.is_empty() {
            format!("Codex Web runtime returned no output ({})", output.status)
        } else {
            stderr
        }));
    }
    Ok(stdout)
}

fn command_for_cli(cli: &Path, args: &[&str]) -> Command {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd.exe");
        command.arg("/d").arg("/c").arg(cli).args(args);
        command.stdin(Stdio::null());
        command
    }
    #[cfg(not(target_os = "windows"))]
    {
        let mut command = Command::new(cli);
        command.args(args).stdin(Stdio::null());
        command
    }
}

fn launch_hidden(launcher: &Path) -> AppResult<()> {
    let mut command = std::process::Command::new(launcher);
    command
        .arg("--hidden")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command.spawn().map_err(|error| {
        AppError::Message(format!("failed to start Codex Web browser owner: {error}"))
    })?;
    Ok(())
}

fn discover_launcher() -> Option<PathBuf> {
    if let Some(path) = absolute_file_from_env("MNELYRA_CODEX_WEB_LAUNCHER") {
        return Some(path);
    }

    #[cfg(target_os = "windows")]
    if let Some(local) = dirs::data_local_dir() {
        let candidate = local
            .join("Programs")
            .join("codex-web-gpt-launcher")
            .join("Codex Web GPT.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn discover_cli(launcher: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = absolute_file_from_env("MNELYRA_CODEX_WEB_BIN") {
        return Some(path);
    }

    if let Some(launcher) = launcher {
        let root = launcher.parent()?;
        #[cfg(target_os = "windows")]
        let candidate = root
            .join("resources")
            .join("runtime")
            .join("bin")
            .join("codex-chatgpt-web.cmd");
        #[cfg(not(target_os = "windows"))]
        let candidate = root
            .join("resources")
            .join("runtime")
            .join("bin")
            .join("codex-chatgpt-web");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    which::which(if cfg!(target_os = "windows") {
        "codex-chatgpt-web.cmd"
    } else {
        "codex-chatgpt-web"
    })
    .ok()
}

fn absolute_file_from_env(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os(name).map(PathBuf::from)?;
    (path.is_absolute() && path.is_file()).then_some(path)
}

fn normalize_version(raw: &str) -> String {
    raw.lines()
        .next()
        .unwrap_or_default()
        .trim()
        .strip_prefix("codex-chatgpt-web ")
        .unwrap_or(raw.trim())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_runtime_version_banner() {
        assert_eq!(normalize_version("codex-chatgpt-web 2.1.10\n"), "2.1.10");
        assert_eq!(normalize_version("2.1.11"), "2.1.11");
    }

    #[test]
    fn stored_full_mode_config_tracks_tunnel_setup() {
        let stored: StoredConfig = serde_json::from_str(
            r#"{
                "mode":"full",
                "appName":"Codex Native2",
                "tunnel":{
                    "tunnelId":"tunnel_example",
                    "runtimeKeyFile":"C:\\\\tmp\\\\runtime.key"
                }
            }"#,
        )
        .unwrap();

        assert_eq!(stored.mode, "full");
        assert_eq!(stored.app_name.as_deref(), Some("Codex Native2"));
        let tunnel = stored.tunnel.unwrap();
        assert_eq!(tunnel.tunnel_id, "tunnel_example");
        assert!(tunnel.runtime_key_file.is_some());
    }

    #[test]
    fn doctor_rows_drive_readiness_without_detail_scraping() {
        let report = DoctorReport {
            ok: true,
            mode: Some("full".into()),
            checks: vec![DoctorCheck {
                id: "proxy".into(),
                status: "ok".into(),
                message: "ready".into(),
                detail: None,
            }],
        };
        assert!(check_ok(&report, "proxy"));
        assert!(!check_ok(&report, "browser-host"));
    }

    #[test]
    fn active_codex_turn_is_a_usable_browser_host_not_an_outage() {
        let report = DoctorReport {
            ok: false,
            mode: Some("full".into()),
            checks: vec![
                DoctorCheck {
                    id: "browser-host".into(),
                    status: "error".into(),
                    message: "Embedded launcher browser is unavailable".into(),
                    detail: Some(
                        "Launcher ChatGPT session could not be verified: ChatGPT browser is running Codex turn a8b1ee4bb69f"
                            .into(),
                    ),
                },
                DoctorCheck {
                    id: "proxy".into(),
                    status: "ok".into(),
                    message: "Responses proxy is healthy on 127.0.0.1:17841".into(),
                    detail: None,
                },
                DoctorCheck {
                    id: "tunnel-runtime".into(),
                    status: "ok".into(),
                    message: "Tunnel runtime reports healthy and ready".into(),
                    detail: None,
                },
            ],
        };

        assert!(browser_host_busy(&report));
        assert!(!check_ok(&report, "browser-host"));
        assert!(!report.checks.iter().any(|check| {
            check.status == "error" && !(check.id == "browser-host" && browser_host_busy(&report))
        }));
    }

    #[test]
    fn runtime_context_policy_delegates_default_compaction_to_codex() {
        let source = concat!(
            "function B2($,Q,Z){if(Z.proAvailable)return W2(111193,95000);",
            "if(Q===\"low\")return W2(41000,32000);",
            "if(Q===\"medium\"||Q===\"high\")return W2(140000,125000);",
            "throw Error(`ChatGPT Plus context limit is not defined for unavailable effort: ${Q}`)}",
            "function UX($,Q,Z){let Y=G4($,\"native Codex model template\"),X=I2(Y);",
            "let J=B2(Q.backendModel,Q.adapterEffort,Z),K={...structuredClone(Y),",
            "context_window:J.contextWindow,max_context_window:J.contextWindow,",
            "effective_context_window_percent:J.effectiveContextWindowPercent,",
            "auto_compact_token_limit:J.autoCompactTokenLimit,additional_speed_tiers:[]};return K}"
        );

        let patched = patch_runtime_context_policy_source(source)
            .unwrap()
            .expect("legacy runtime should be patched");
        assert!(patched.contains(
            "return{contextWindow:1050000,maxContextWindow:1050000,useNativeDefault:!0};"
        ));
        assert!(patched.contains(
            "context_window:J.useNativeDefault?(Y.context_window??J.contextWindow):J.contextWindow"
        ));
        assert!(patched.contains("max_context_window:J.maxContextWindow??J.contextWindow"));
        assert!(patched.contains(
            "auto_compact_token_limit:J.useNativeDefault?void 0:J.autoCompactTokenLimit"
        ));
        assert!(patch_runtime_context_policy_source(&patched)
            .unwrap()
            .is_none());
    }

    #[test]
    fn runtime_context_policy_accepts_newer_legacy_budget_values() {
        let source = concat!(
            "function z1(J,Q,Z){if(Q===\"medium\"||Q===\"high\")return W2(90000,80000);",
            "throw Error(`ChatGPT Plus context limit is not defined for unavailable effort: ${Q}`)}",
            "function U1($,Q,Z){let T=G4($,\"native Codex model template\"),X=I2(T);",
            "let C=z1(Q.backendModel,Q.adapterEffort,Z),K={...structuredClone(T),",
            "context_window:C.contextWindow,max_context_window:C.contextWindow,",
            "effective_context_window_percent:C.effectiveContextWindowPercent,",
            "auto_compact_token_limit:C.autoCompactTokenLimit,additional_speed_tiers:[]};return K}"
        );

        let patched = patch_runtime_context_policy_source(source)
            .unwrap()
            .expect("newer legacy budget should be patched");
        assert!(patched.contains("contextWindow:1050000"));
        assert!(patched.contains("context_window:C.useNativeDefault?(T.context_window"));
    }

    #[test]
    fn stale_model_cache_detects_legacy_web_compaction_policy() {
        let stale = serde_json::json!({
            "models": [
                {
                    "slug": "chatgpt-web/high",
                    "context_window": 140000,
                    "max_context_window": 140000,
                    "auto_compact_token_limit": 125000
                }
            ]
        });
        let current = serde_json::json!({
            "models": [
                {
                    "slug": "chatgpt-web/high",
                    "context_window": 272000,
                    "max_context_window": 1050000,
                    "auto_compact_token_limit": null
                },
                {
                    "slug": "chatgpt-web/light",
                    "context_window": 41000,
                    "max_context_window": 41000,
                    "auto_compact_token_limit": 32000
                }
            ]
        });

        assert!(cached_web_model_policy_is_stale(&stale));
        assert!(!cached_web_model_policy_is_stale(&current));
    }
}
