use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{broadcast, oneshot, Mutex};

use crate::platform::ProcessTreeGuard;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const EVENT_CHANNEL_CAPACITY: usize = 16 * 1024;
const SERVER_OVERLOADED_CODE: i64 = -32001;
const SERVER_OVERLOAD_RETRIES: u32 = 5;
const SERVER_OVERLOAD_BASE_DELAY_MS: u64 = 50;

#[derive(Debug, Clone)]
enum AppServerRequestFailure {
    Rpc { code: Option<i64>, message: String },
    Transport(String),
}

fn validate_context_policy(
    context_window: Option<u64>,
    auto_compact_token_limit: Option<u64>,
) -> Result<(), String> {
    if context_window.is_some_and(|value| value < 16_384) {
        return Err("context window must be at least 16384 tokens".into());
    }
    if auto_compact_token_limit.is_some_and(|value| value < 16_384) {
        return Err("auto-compaction threshold must be at least 16384 tokens".into());
    }
    if let (Some(context), Some(compact)) = (context_window, auto_compact_token_limit) {
        if compact >= context {
            return Err("auto-compaction threshold must be lower than the context window".into());
        }
    }
    Ok(())
}

type PendingResponse = Result<Value, AppServerRequestFailure>;
type PendingSender = oneshot::Sender<PendingResponse>;
type PendingRequests = Arc<Mutex<HashMap<u64, PendingSender>>>;

impl AppServerRequestFailure {
    fn is_overloaded(&self) -> bool {
        matches!(
            self,
            Self::Rpc {
                code: Some(SERVER_OVERLOADED_CODE),
                ..
            }
        )
    }

    fn message(&self) -> String {
        match self {
            Self::Rpc { code, message } => match code {
                Some(code) => format!("Codex app-server error {code}: {message}"),
                None => format!("Codex app-server error: {message}"),
            },
            Self::Transport(message) => message.clone(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppServerEvent {
    pub payload: Value,
}

#[derive(Debug, Clone)]
pub struct CodexAppServerRuntimeStatus {
    pub available: bool,
    pub running: bool,
    pub pid: Option<u32>,
    pub active_turns: u32,
    pub message: String,
}

#[derive(Clone)]
struct AppServerConnection {
    stdin: Arc<Mutex<ChildStdin>>,
    pending: PendingRequests,
    events: broadcast::Sender<AppServerEvent>,
    next_id: Arc<AtomicU64>,
    alive: Arc<AtomicBool>,
}

impl AppServerConnection {
    async fn request(&self, method: &str, params: Value) -> Result<Value, String> {
        let mut overload_attempt = 0;
        loop {
            match self.request_once(method, params.clone()).await {
                Ok(value) => return Ok(value),
                Err(error)
                    if error.is_overloaded() && overload_attempt < SERVER_OVERLOAD_RETRIES =>
                {
                    let delay = overload_retry_delay(overload_attempt);
                    overload_attempt += 1;
                    tokio::time::sleep(delay).await;
                }
                Err(error) => return Err(error.message()),
            }
        }
    }

    async fn request_once(
        &self,
        method: &str,
        params: Value,
    ) -> Result<Value, AppServerRequestFailure> {
        if !self.alive.load(Ordering::Acquire) {
            return Err(AppServerRequestFailure::Transport(
                "Codex app-server is not running".into(),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let message = json!({ "id": id, "method": method, "params": params });
        if let Err(error) = self.write_message(&message).await {
            self.pending.lock().await.remove(&id);
            return Err(AppServerRequestFailure::Transport(error));
        }
        match tokio::time::timeout(REQUEST_TIMEOUT, rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(AppServerRequestFailure::Transport(format!(
                "Codex app-server request {method} was cancelled"
            ))),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(AppServerRequestFailure::Transport(format!(
                    "Codex app-server request {method} timed out"
                )))
            }
        }
    }

    async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write_message(&json!({ "method": method, "params": params }))
            .await
    }

    async fn respond(&self, id: Value, result: Value) -> Result<(), String> {
        if !id.is_u64() && !id.is_i64() && !id.is_string() {
            return Err("Codex app-server request id has an unsupported type".into());
        }
        self.write_message(&json!({ "id": id, "result": result }))
            .await
    }

    async fn write_message(&self, value: &Value) -> Result<(), String> {
        let mut encoded = serde_json::to_vec(value)
            .map_err(|error| format!("failed to serialize Codex app-server message: {error}"))?;
        encoded.push(b'\n');
        let mut stdin = self.stdin.lock().await;
        stdin
            .write_all(&encoded)
            .await
            .map_err(|error| format!("failed to write Codex app-server stdin: {error}"))?;
        stdin
            .flush()
            .await
            .map_err(|error| format!("failed to flush Codex app-server stdin: {error}"))
    }
}

struct RunningAppServer {
    child: Child,
    connection: AppServerConnection,
    _process_tree: ProcessTreeGuard,
}

#[derive(Clone)]
pub struct CodexAppServerManager {
    inner: Arc<Mutex<Option<RunningAppServer>>>,
    active_turns: Arc<AtomicU64>,
    permission_mode: Arc<StdRwLock<String>>,
}

impl Default for CodexAppServerManager {
    fn default() -> Self {
        Self::with_permission_mode("automatic")
    }
}

impl CodexAppServerManager {
    pub fn with_permission_mode(mode: impl Into<String>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(None)),
            active_turns: Arc::new(AtomicU64::new(0)),
            permission_mode: Arc::new(StdRwLock::new(mode.into())),
        }
    }

    pub async fn reconfigure_permission_mode(&self, mode: &str) -> Result<(), String> {
        validate_permission_mode(mode)?;
        let current = self
            .permission_mode
            .read()
            .map_err(|_| "Codex permission mode lock poisoned".to_string())?
            .clone();
        if current == mode {
            return Ok(());
        }
        if self.active_turns.load(Ordering::Acquire) > 0 {
            return Err("Codex is currently running a task; change permissions after the active turn finishes".into());
        }
        self.shutdown().await;
        *self
            .permission_mode
            .write()
            .map_err(|_| "Codex permission mode lock poisoned".to_string())? = mode.to_string();
        Ok(())
    }

    fn current_permission_mode(&self) -> Result<String, String> {
        self.permission_mode
            .read()
            .map(|value| value.clone())
            .map_err(|_| "Codex permission mode lock poisoned".to_string())
    }

    pub async fn runtime_status(&self) -> CodexAppServerRuntimeStatus {
        let executable = match discover_codex_executable() {
            Ok(path) => path,
            Err(error) => {
                return CodexAppServerRuntimeStatus {
                    available: false,
                    running: false,
                    pid: None,
                    active_turns: 0,
                    message: error,
                }
            }
        };

        let mut guard = self.inner.lock().await;
        let (running, pid) = if let Some(runtime) = guard.as_mut() {
            let alive = runtime.connection.alive.load(Ordering::Acquire);
            let child_running = alive && matches!(runtime.child.try_wait(), Ok(None));
            if child_running {
                (true, runtime.child.id())
            } else {
                *guard = None;
                self.active_turns.store(0, Ordering::Release);
                (false, None)
            }
        } else {
            (false, None)
        };
        let active_turns = self
            .active_turns
            .load(Ordering::Acquire)
            .min(u32::MAX as u64) as u32;
        CodexAppServerRuntimeStatus {
            available: true,
            running,
            pid,
            active_turns,
            message: if running {
                format!("Codex app-server running from {}", executable.display())
            } else {
                format!("Codex ready from {}", executable.display())
            },
        }
    }

    pub async fn ensure_started(&self) -> Result<(), String> {
        let mut guard = self.inner.lock().await;
        if let Some(running) = guard.as_mut() {
            if running.connection.alive.load(Ordering::Acquire) {
                if let Ok(None) = running.child.try_wait() {
                    return Ok(());
                }
            }
            *guard = None;
        }

        let executable = discover_codex_executable()?;
        let runtime_dir = app_server_runtime_dir()?;
        std::fs::create_dir_all(&runtime_dir).map_err(|error| {
            format!(
                "failed to create Mnelyra Codex runtime directory {}: {error}",
                runtime_dir.display()
            )
        })?;

        let mut command = codex_command(&executable);
        configure_permission_profile(&mut command, &self.current_permission_mode()?)?;
        command
            .arg("app-server")
            .arg("--listen")
            .arg("stdio://")
            .current_dir(&runtime_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Approval policy is always inherited. Mnelyra only applies an explicit
        // sandbox profile when the user selected one in Developer settings.
        let mut child = command
            .spawn()
            .map_err(|error| format!("failed to start Codex app-server: {error}"))?;
        let process_tree = ProcessTreeGuard::attach(&child)?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "Codex app-server stdin was not piped".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "Codex app-server stdout was not piped".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "Codex app-server stderr was not piped".to_string())?;

        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let alive = Arc::new(AtomicBool::new(true));
        let connection = AppServerConnection {
            stdin: Arc::new(Mutex::new(stdin)),
            pending: pending.clone(),
            events: events.clone(),
            next_id: Arc::new(AtomicU64::new(1)),
            alive: alive.clone(),
        };

        spawn_stdout_router(
            stdout,
            pending,
            events,
            alive.clone(),
            self.active_turns.clone(),
        );
        spawn_stderr_reader(stderr);

        let initialize = connection
            .request(
                "initialize",
                json!({
                    "clientInfo": {
                        "name": "mnelyra",
                        "title": "Mnelyra",
                        "version": env!("CARGO_PKG_VERSION")
                    },
                    "capabilities": {
                        "experimentalApi": true
                    }
                }),
            )
            .await;
        if let Err(error) = initialize {
            alive.store(false, Ordering::Release);
            let _ = child.kill().await;
            return Err(format!("Codex app-server initialize failed: {error}"));
        }
        connection.notify("initialized", json!({})).await?;

        *guard = Some(RunningAppServer {
            child,
            connection,
            _process_tree: process_tree,
        });
        Ok(())
    }

    pub async fn compact_thread(&self, thread_id: &str) -> Result<(), String> {
        self.ensure_started().await?;
        self.connection()
            .await?
            .request("thread/compact/start", json!({ "threadId": thread_id }))
            .await?;
        Ok(())
    }

    pub async fn read_config(&self, cwd: Option<&Path>) -> Result<Value, String> {
        self.ensure_started().await?;
        let cwd = match cwd {
            Some(path) => Some(canonical_thread_root(path)?),
            None => None,
        };
        self.connection()
            .await?
            .request(
                "config/read",
                json!({
                    "includeLayers": true,
                    "cwd": cwd
                }),
            )
            .await
    }

    pub async fn set_auto_compact_token_limit(&self, limit: Option<u64>) -> Result<Value, String> {
        if limit.is_some_and(|value| value < 16_384) {
            return Err("auto-compaction threshold must be at least 16384 tokens".into());
        }
        let value = limit.map(Value::from).unwrap_or(Value::Null);
        self.ensure_started().await?;
        self.connection()
            .await?
            .request(
                "config/batchWrite",
                json!({
                    "edits": [{
                        "keyPath": "model_auto_compact_token_limit",
                        "value": value,
                        "mergeStrategy": "replace"
                    }],
                    "reloadUserConfig": false
                }),
            )
            .await
    }

    pub async fn set_context_policy(
        &self,
        context_window: Option<u64>,
        auto_compact_token_limit: Option<u64>,
    ) -> Result<Value, String> {
        validate_context_policy(context_window, auto_compact_token_limit)?;
        self.ensure_started().await?;
        self.connection()
            .await?
            .request(
                "config/batchWrite",
                json!({
                    "edits": [
                        {
                            "keyPath": "model_context_window",
                            "value": context_window.map(Value::from).unwrap_or(Value::Null),
                            "mergeStrategy": "replace"
                        },
                        {
                            "keyPath": "model_auto_compact_token_limit",
                            "value": auto_compact_token_limit.map(Value::from).unwrap_or(Value::Null),
                            "mergeStrategy": "replace"
                        }
                    ],
                    "reloadUserConfig": false
                }),
            )
            .await
    }

    pub async fn start_thread(&self, cwd: &Path) -> Result<String, String> {
        let cwd = canonical_thread_root(cwd)?;
        self.ensure_started().await?;
        let connection = self.connection().await?;
        let result = connection
            .request(
                "thread/start",
                json!({
                    "cwd": cwd,
                    "approvalsReviewer": "user"
                }),
            )
            .await?;
        string_at(&result, &["thread", "id"])
            .ok_or_else(|| "Codex thread/start response has no thread.id".into())
    }

    pub async fn resume_thread(&self, thread_id: &str) -> Result<(), String> {
        self.ensure_started().await?;
        let connection = self.connection().await?;
        connection
            .request("thread/resume", json!({ "threadId": thread_id }))
            .await?;
        Ok(())
    }

    pub async fn start_turn(&self, thread_id: &str, input: &str) -> Result<String, String> {
        if input.trim().is_empty() {
            return Err("Codex turn input must not be empty".into());
        }
        self.ensure_started().await?;
        let connection = self.connection().await?;
        let result = connection
            .request(
                "turn/start",
                json!({
                    "threadId": thread_id,
                    "input": [{ "type": "text", "text": input }]
                }),
            )
            .await?;
        string_at(&result, &["turn", "id"])
            .ok_or_else(|| "Codex turn/start response has no turn.id".to_string())
    }

    pub async fn interrupt_turn(&self, thread_id: &str, turn_id: &str) -> Result<(), String> {
        self.ensure_started().await?;
        let connection = self.connection().await?;
        connection
            .request(
                "turn/interrupt",
                json!({ "threadId": thread_id, "turnId": turn_id }),
            )
            .await?;
        Ok(())
    }

    pub async fn read_thread(&self, thread_id: &str) -> Result<Value, String> {
        self.ensure_started().await?;
        self.connection()
            .await?
            .request(
                "thread/read",
                json!({ "threadId": thread_id, "includeTurns": true }),
            )
            .await
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<AppServerEvent>, String> {
        self.ensure_started().await?;
        Ok(self.connection().await?.events.subscribe())
    }

    pub async fn respond_to_server_request(&self, id: Value, result: Value) -> Result<(), String> {
        self.ensure_started().await?;
        self.connection().await?.respond(id, result).await
    }

    pub async fn shutdown(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(mut running) = guard.take() {
            running.connection.alive.store(false, Ordering::Release);
            let _ = running.child.kill().await;
            fail_pending(&running.connection.pending, "Codex app-server stopped").await;
        }
        self.active_turns.store(0, Ordering::Release);
    }

    async fn connection(&self) -> Result<AppServerConnection, String> {
        let guard = self.inner.lock().await;
        guard
            .as_ref()
            .filter(|running| running.connection.alive.load(Ordering::Acquire))
            .map(|running| running.connection.clone())
            .ok_or_else(|| "Codex app-server is not running".into())
    }
}

fn spawn_stdout_router(
    stdout: tokio::process::ChildStdout,
    pending: PendingRequests,
    events: broadcast::Sender<AppServerEvent>,
    alive: Arc<AtomicBool>,
    active_turns: Arc<AtomicU64>,
) {
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(payload) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(id) = payload.get("id").and_then(Value::as_u64) {
                if payload.get("result").is_some() || payload.get("error").is_some() {
                    if let Some(waiter) = pending.lock().await.remove(&id) {
                        let result = match payload.get("error") {
                            Some(error) => Err(AppServerRequestFailure::Rpc {
                                code: error.get("code").and_then(Value::as_i64),
                                message: error
                                    .get("message")
                                    .and_then(Value::as_str)
                                    .map(str::to_string)
                                    .unwrap_or_else(|| error.to_string()),
                            }),
                            None => Ok(payload.get("result").cloned().unwrap_or(Value::Null)),
                        };
                        let _ = waiter.send(result);
                        continue;
                    }
                }
            }
            match payload.get("method").and_then(Value::as_str) {
                Some("turn/started") => {
                    active_turns.fetch_add(1, Ordering::AcqRel);
                }
                Some("turn/completed") => {
                    let _ =
                        active_turns.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                            Some(value.saturating_sub(1))
                        });
                }
                _ => {}
            }
            let _ = events.send(AppServerEvent { payload });
        }
        alive.store(false, Ordering::Release);
        active_turns.store(0, Ordering::Release);
        fail_pending(&pending, "Codex app-server stdout closed").await;
    });
}

fn spawn_stderr_reader(stderr: tokio::process::ChildStderr) {
    tauri::async_runtime::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(_line)) = lines.next_line().await {}
    });
}

async fn fail_pending(pending: &PendingRequests, message: &str) {
    let drained = {
        let mut pending = pending.lock().await;
        pending
            .drain()
            .map(|(_, sender)| sender)
            .collect::<Vec<_>>()
    };
    for sender in drained {
        let _ = sender.send(Err(AppServerRequestFailure::Transport(message.to_string())));
    }
}

fn overload_retry_delay(attempt: u32) -> Duration {
    let exponential = SERVER_OVERLOAD_BASE_DELAY_MS.saturating_mul(1_u64 << attempt.min(6));
    // Deterministic per-attempt jitter keeps synchronized clients from retrying on the exact
    // same boundary without introducing a runtime RNG dependency.
    let jitter = u64::from(attempt.wrapping_mul(17) % 31);
    Duration::from_millis(exponential.saturating_add(jitter))
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for part in path {
        current = current.get(*part)?;
    }
    current.as_str().map(str::to_string)
}

fn canonical_thread_root(path: &Path) -> Result<String, String> {
    let canonical = std::fs::canonicalize(path).map_err(|error| {
        format!(
            "failed to canonicalize task workspace {}: {error}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "task workspace is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical.to_string_lossy().into_owned())
}

fn validate_permission_mode(mode: &str) -> Result<(), String> {
    match mode {
        "automatic" | "read_only" | "custom" => Ok(()),
        other => Err(format!("unsupported Codex permission mode: {other}")),
    }
}

fn config_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn push_config_override(command: &mut Command, key: &str, value: impl AsRef<str>) {
    command.arg("-c").arg(format!("{key}={}", value.as_ref()));
}

fn configure_permission_profile(command: &mut Command, mode: &str) -> Result<(), String> {
    validate_permission_mode(mode)?;
    match mode {
        "automatic" => {}
        "read_only" => {
            push_config_override(command, "sandbox_mode", "\"read-only\"");
        }
        "custom" => {
            // Mnelyra Custom: workspace-write with network enabled, plus the
            // validated Windows elevated sandbox and MiKTeX compatibility.
            // This remains distinct from Full Access because filesystem scope
            // is still bounded to the workspace (plus exact compatibility roots).
            push_config_override(command, "sandbox_mode", "\"workspace-write\"");
            push_config_override(command, "sandbox_workspace_write.network_access", "true");
            #[cfg(target_os = "windows")]
            {
                push_config_override(command, "windows.sandbox", "\"elevated\"");
                if let Some(miktex) = prepare_miktex_sandbox_compat()? {
                    push_config_override(
                        command,
                        "sandbox_workspace_write.writable_roots",
                        format!("[{}]", config_string(&miktex.root.to_string_lossy())),
                    );
                    let shell_set = format!(
                        "{{ MIKTEX_USERSTARTUPFILE = {}, PATH = {} }}",
                        config_string(&miktex.startup.to_string_lossy()),
                        config_string(&miktex.path),
                    );
                    push_config_override(command, "shell_environment_policy.set", shell_set);
                    command.env("MIKTEX_USERSTARTUPFILE", &miktex.startup);
                    command.env("PATH", &miktex.path);
                }
            }
        }
        _ => unreachable!("validated permission mode"),
    }
    Ok(())
}

#[cfg(target_os = "windows")]
struct MiktexSandboxCompat {
    root: PathBuf,
    startup: PathBuf,
    path: String,
}

#[cfg(target_os = "windows")]
fn prepare_miktex_sandbox_compat() -> Result<Option<MiktexSandboxCompat>, String> {
    let local_app_data = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .ok_or_else(|| "could not resolve LOCALAPPDATA for MiKTeX compatibility".to_string())?;
    let install_root = local_app_data.join("Programs").join("MiKTeX");
    let lualatex = install_root
        .join("miktex")
        .join("bin")
        .join("x64")
        .join("lualatex.exe");
    if !lualatex.is_file() {
        return Ok(None);
    }

    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| {
            "could not resolve Mnelyra data directory for MiKTeX compatibility".to_string()
        })?;
    let root = base.join("Mnelyra").join("runtime").join("miktex-sandbox");
    let config_root = root.join("config");
    let data_root = root.join("data");
    let startup = root.join("miktexstartup.ini");
    let miktex_ini = config_root.join("miktex").join("config").join("miktex.ini");
    let Some(parent) = miktex_ini.parent() else {
        return Err("invalid MiKTeX compatibility config path".into());
    };
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create MiKTeX config directory: {error}"))?;
    std::fs::create_dir_all(&data_root)
        .map_err(|error| format!("failed to create MiKTeX data directory: {error}"))?;

    std::fs::write(
        &miktex_ini,
        ";;; Managed by Mnelyra for the isolated Codex sandbox.\r\n\r\n[Core]\r\nNoRegistry=true\r\n",
    )
    .map_err(|error| format!("failed to write MiKTeX sandbox config: {error}"))?;

    let startup_text = format!(
        ";;; Managed by Mnelyra for the isolated Codex sandbox.\r\n\r\n[Auto]\r\nConfig=Regular\r\n\r\n[Setup]\r\nVersion=25.12\r\n\r\n[Paths]\r\nUserInstall={}\r\nUserConfig={}\r\nUserData={}\r\n",
        install_root.display(),
        config_root.display(),
        data_root.display(),
    );
    std::fs::write(&startup, startup_text)
        .map_err(|error| format!("failed to write MiKTeX startup file: {error}"))?;

    let path = std::env::var("PATH")
        .unwrap_or_default()
        .split(';')
        .filter(|entry| {
            let normalized = entry.trim().replace('/', "\\").to_ascii_lowercase();
            !normalized.ends_with("\\transformers.exe")
                && !normalized.ends_with("\\transformers-cli.exe")
        })
        .collect::<Vec<_>>()
        .join(";");

    Ok(Some(MiktexSandboxCompat {
        root,
        startup,
        path,
    }))
}

fn app_server_runtime_dir() -> Result<PathBuf, String> {
    let base = dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .ok_or_else(|| "could not resolve a local application data directory".to_string())?;
    Ok(base
        .join("Mnelyra")
        .join("runtime")
        .join("codex-app-server"))
}

fn discover_codex_executable() -> Result<PathBuf, String> {
    if let Some(value) = std::env::var_os("MNELYRA_CODEX_BIN") {
        let path = PathBuf::from(value);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "MNELYRA_CODEX_BIN does not point to a file: {}",
            path.display()
        ));
    }
    if let Ok(path) = which::which("codex") {
        return Ok(path);
    }

    for candidate in codex_executable_candidates() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(
        "Codex executable was not found. Install Codex CLI or set MNELYRA_CODEX_BIN to the exact codex executable/script path."
            .into(),
    )
}

fn codex_executable_candidates() -> Vec<PathBuf> {
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
            candidates.push(PathBuf::from(prefix).join("codex.cmd"));
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

fn codex_command(executable: &Path) -> Command {
    #[cfg(target_os = "windows")]
    {
        let extension = executable
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat") {
            let mut command = Command::new("cmd.exe");
            command.arg("/D").arg("/S").arg("/C").arg(executable);
            return command;
        }
    }
    Command::new(executable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_string_extraction_is_strict() {
        let payload = json!({ "thread": { "id": "thr_123" } });
        assert_eq!(
            string_at(&payload, &["thread", "id"]).as_deref(),
            Some("thr_123")
        );
        assert!(string_at(&payload, &["turn", "id"]).is_none());
    }

    #[test]
    fn canonical_thread_root_rejects_missing_paths() {
        let missing = PathBuf::from("definitely-missing-mnelyra-workspace-root");
        assert!(canonical_thread_root(&missing).is_err());
    }

    #[test]
    fn overload_backoff_is_bounded_and_increasing() {
        let first = overload_retry_delay(0);
        let second = overload_retry_delay(1);
        let last = overload_retry_delay(SERVER_OVERLOAD_RETRIES);
        assert!(second > first);
        assert!(last < Duration::from_secs(5));
    }

    #[test]
    fn context_policy_accepts_one_million_preset() {
        assert!(validate_context_policy(Some(1_000_000), Some(900_000)).is_ok());
    }

    #[test]
    fn context_policy_rejects_compaction_at_or_above_window() {
        assert!(validate_context_policy(Some(100_000), Some(100_000)).is_err());
        assert!(validate_context_policy(Some(100_000), Some(110_000)).is_err());
    }
}
