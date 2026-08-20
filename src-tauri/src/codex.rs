use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(target_os = "windows")]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "windows")]
static CHATGPT_APPX_CODEX: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

pub(crate) fn discover_executable() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("MNELYRA_CODEX_BIN") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err("MNELYRA_CODEX_BIN does not point to a Codex executable".into());
    }

    // On Windows, Mnelyra integrates with the Codex desktop app first. Prefer
    // the CLI bundled inside the installed AppX/MSIX package so model catalog
    // and app-server behavior match the desktop client the user is actually
    // running. Explicit MNELYRA_CODEX_BIN remains the escape hatch above.
    #[cfg(target_os = "windows")]
    if let Some(path) = discover_from_chatgpt_appx() {
        return Ok(path);
    }

    if let Ok(path) = which::which("codex") {
        if path.is_file() {
            return Ok(path);
        }
    }

    if let Some(path) = discover_from_npm_global_prefix() {
        return Ok(path);
    }

    for candidate in executable_candidates() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err("Codex was not detected. Install Codex or set MNELYRA_CODEX_BIN.".into())
}

/// Return the app-server directly owned by the installed Codex Desktop, if it
/// is currently running. Exact executable paths are used so standalone Codex
/// CLI/app-server processes are never mistaken for the Desktop runtime.
#[cfg(target_os = "windows")]
pub(crate) fn running_desktop_app_server_pid() -> Result<Option<u32>, String> {
    let codex = discover_executable()?;
    let app_dir = codex
        .parent()
        .and_then(|resources| resources.parent())
        .ok_or_else(|| "Could not resolve the Codex Desktop application directory".to_string())?;
    let desktop = app_dir.join("ChatGPT.exe");
    if !desktop.is_file() {
        return Ok(None);
    }
    crate::platform::windows::process::find_child_process_by_image_paths(&codex, &desktop)
        .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn running_desktop_app_server_pid() -> Result<Option<u32>, String> {
    Ok(None)
}

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("CODEX_HOME") {
        let raw = value.to_string_lossy();
        let path = if raw == "~" {
            dirs::home_dir()
                .ok_or_else(|| "Could not resolve the user home directory".to_string())?
        } else if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
            dirs::home_dir()
                .ok_or_else(|| "Could not resolve the user home directory".to_string())?
                .join(rest)
        } else {
            PathBuf::from(value)
        };
        return if path.is_absolute() {
            Ok(path)
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .map_err(|error| format!("Could not resolve relative CODEX_HOME: {error}"))
        };
    }
    dirs::home_dir()
        .map(|home| home.join(".codex"))
        .ok_or_else(|| "Could not resolve the user home directory".to_string())
}

pub(crate) fn config_path() -> Result<PathBuf, String> {
    Ok(home_dir()?.join("config.toml"))
}

pub(crate) fn clear_model_cache() -> Result<bool, String> {
    let path = home_dir()?.join("models_cache.json");
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!(
            "Could not clear Codex model cache {}: {error}",
            path.display()
        )),
    }
}

fn discover_from_npm_global_prefix() -> Option<PathBuf> {
    let npm = which::which("npm").ok()?;
    let output = run_capture(&npm, &["prefix", "-g"])?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8(output.stdout).ok()?;
    let prefix = PathBuf::from(prefix.trim());
    if !prefix.is_absolute() {
        return None;
    }

    #[cfg(target_os = "windows")]
    let names = ["codex.cmd", "codex.exe", "codex.ps1"];
    #[cfg(not(target_os = "windows"))]
    let names = ["codex"];

    names
        .into_iter()
        .map(|name| prefix.join(name))
        .find(|candidate| candidate.is_file())
}

fn run_capture(program: &std::path::Path, args: &[&str]) -> Option<std::process::Output> {
    #[cfg(target_os = "windows")]
    {
        let extension = program
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
            return Command::new("cmd.exe")
                .arg("/d")
                .arg("/c")
                .arg(program)
                .args(args)
                .stdin(Stdio::null())
                .stderr(Stdio::piped())
                .output()
                .ok();
        }
    }

    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()
}

fn executable_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            candidates.push(PathBuf::from(appdata).join("npm").join("codex.cmd"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            candidates.push(
                local
                    .join("Microsoft")
                    .join("WinGet")
                    .join("Links")
                    .join("codex.exe"),
            );
            candidates.push(local.join("Programs").join("Codex").join("codex.exe"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("scoop").join("shims").join("codex.exe"));
            candidates.push(home.join(".local").join("bin").join("codex.exe"));
        }
        if let Some(prefix) = std::env::var_os("NPM_CONFIG_PREFIX") {
            let prefix = PathBuf::from(prefix);
            candidates.push(prefix.join("codex.cmd"));
            candidates.push(prefix.join("codex.exe"));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join(".local").join("bin").join("codex"));
        }
        if let Some(prefix) = std::env::var_os("NPM_CONFIG_PREFIX") {
            candidates.push(PathBuf::from(prefix).join("bin").join("codex"));
        }
    }
    candidates
}

#[cfg(target_os = "windows")]
fn discover_from_chatgpt_appx() -> Option<PathBuf> {
    let cache = CHATGPT_APPX_CODEX.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some(path) = guard.as_ref().filter(|path| path.is_file()) {
            return Some(path.clone());
        }
    }

    let powershell = which::which("powershell.exe")
        .or_else(|_| which::which("powershell"))
        .or_else(|_| which::which("pwsh"))
        .ok()?;
    // Prefer the dedicated Codex package, while retaining a narrow fallback
    // for older/future OpenAI package names that may still bundle codex.exe.
    let script = r#"$preferred = @(Get-AppxPackage -Name 'OpenAI.Codex' -ErrorAction SilentlyContinue); if ($preferred.Count -gt 0) { $preferred | Select-Object -ExpandProperty InstallLocation } else { Get-AppxPackage | Where-Object { $_.Name -match 'Codex|ChatGPT|OpenAI' -or $_.PackageFullName -match 'Codex|ChatGPT|OpenAI' } | Select-Object -ExpandProperty InstallLocation }"#;
    let output = Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let roots = String::from_utf8(output.stdout).ok()?;
    let mut best: Option<(u64, PathBuf)> = None;
    for root in roots.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let root = PathBuf::from(root);
        if !root.is_dir() {
            continue;
        }

        // The packaged Codex desktop app keeps the real CLI under app/resources.
        // Probe stable relative locations first so startup never recursively walks
        // a large WindowsApps package tree just to find one binary.
        let direct = [
            root.join("app").join("resources").join("codex.exe"),
            root.join("resources").join("codex.exe"),
            root.join("codex.exe"),
        ];
        for candidate in direct {
            let size = candidate
                .metadata()
                .ok()
                .map(|meta| meta.len())
                .unwrap_or(0);
            if candidate.is_file()
                && size >= 10 * 1024 * 1024
                && best.as_ref().is_none_or(|(current, _)| size > *current)
            {
                best = Some((size, candidate));
            }
        }
        if best.is_some() {
            continue;
        }

        // Keep a shallow fallback for future package layout changes, but never
        // scan the full app bundle recursively on startup.
        let app_root = root.join("app");
        for entry in walkdir::WalkDir::new(app_root)
            .max_depth(3)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if !entry.file_type().is_file()
                || !entry
                    .file_name()
                    .to_string_lossy()
                    .eq_ignore_ascii_case("codex.exe")
            {
                continue;
            }
            let size = entry.metadata().ok().map(|meta| meta.len()).unwrap_or(0);
            if size >= 10 * 1024 * 1024 && best.as_ref().is_none_or(|(current, _)| size > *current)
            {
                best = Some((size, entry.into_path()));
            }
        }
    }
    let path = best.map(|(_, path)| path)?;
    if let Ok(mut guard) = cache.lock() {
        *guard = Some(path.clone());
    }
    Some(path)
}
