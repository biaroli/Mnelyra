use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tauri::AppHandle;
use tokio::sync::{oneshot, Mutex, Notify};

use crate::error::{AppError, AppResult};

const LISTEN_ADDR: &str = "127.0.0.1:17841";
const OFFICIAL_CODEX_BACKEND: &str = "https://chatgpt.com/backend-api/codex";
const MAX_DECODED_REQUEST_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
struct ProxyState {
    app: Option<AppHandle>,
    client: reqwest::Client,
    helper_shutdown: Option<Arc<Notify>>,
    drain_owner_pid: Option<u32>,
}

pub(super) async fn active_mode() -> Option<String> {
    health_value().await.and_then(|value| {
        value
            .get("mode")
            .and_then(Value::as_str)
            .map(str::to_string)
    })
}

#[cfg(target_os = "windows")]
fn spawn_native_drain_process(owner_pid: u32) -> AppResult<()> {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let executable = std::env::current_exe().map_err(|error| {
        AppError::Message(format!(
            "could not resolve Mnelyra executable for native drain: {error}"
        ))
    })?;
    Command::new(executable)
        .arg("--web-models-native-drain")
        .arg(owner_pid.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|error| {
            AppError::Message(format!(
                "could not start detached Web Models native drain: {error}"
            ))
        })?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn spawn_native_drain_process(_owner_pid: u32) -> AppResult<()> {
    Err(AppError::Message(
        "detached Web Models native drain is currently implemented only on Windows".into(),
    ))
}

async fn wait_for_native_drain_helper(owner_pid: u32, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if health_value().await.is_some_and(|health| {
            health.get("mode").and_then(Value::as_str) == Some("native-drain")
                && health.get("drainPid").and_then(Value::as_u64).is_some()
                && health.get("ownerPid").and_then(Value::as_u64) == Some(owner_pid as u64)
        }) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

/// Transfer 17841 from the browser-owning Mnelyra process to a tiny detached
/// passthrough process. The helper lives only as long as the exact Codex
/// Desktop app-server PID that may still hold old 17841-backed sessions.
pub(super) async fn handoff_native_drain(app: &AppHandle, owner_pid: u32) -> AppResult<()> {
    MODE.store(MODE_NATIVE_DRAIN, Ordering::Release);
    stop().await;
    spawn_native_drain_process(owner_pid)?;
    if wait_for_native_drain_helper(owner_pid, Duration::from_secs(5)).await {
        return Ok(());
    }

    // Keep stale Desktop threads usable even if the detached handoff failed.
    // The in-process drain is a safe fallback while Mnelyra remains running.
    ensure_native_drain_started(app).await?;
    Ok(())
}

/// Spawn a detached drain before the main process exits. It retries binding
/// until the parent releases 17841, so there is no need to block Tauri's sync
/// ExitRequested callback.
pub(super) fn spawn_native_drain_for_exit(owner_pid: u32) -> AppResult<()> {
    spawn_native_drain_process(owner_pid)
}

#[cfg(target_os = "windows")]
pub(super) async fn run_native_drain_helper(owner_pid: u32) -> AppResult<()> {
    if !crate::platform::windows::process::is_process_alive(owner_pid) {
        return Ok(());
    }
    let owner_image = crate::platform::windows::process::process_image_path(owner_pid)?;
    let owner_is_codex = owner_image
        .as_deref()
        .and_then(|path| std::path::Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case("codex.exe"));
    if !owner_is_codex {
        return Err(AppError::Message(
            "native drain owner is not a live codex.exe process".into(),
        ));
    }

    MODE.store(MODE_NATIVE_DRAIN, Ordering::Release);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(6);
    let listener = loop {
        match tokio::net::TcpListener::bind(LISTEN_ADDR).await {
            Ok(listener) => break listener,
            Err(error) => {
                if !crate::platform::windows::process::is_process_alive(owner_pid) {
                    return Ok(());
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(AppError::Message(format!(
                        "native drain could not bind {LISTEN_ADDR}: {error}"
                    )));
                }
                tokio::time::sleep(Duration::from_millis(75)).await;
            }
        }
    };
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| {
            AppError::Message(format!("could not build native drain HTTP client: {error}"))
        })?;
    let shutdown = Arc::new(Notify::new());
    let state = ProxyState {
        app: None,
        client,
        helper_shutdown: Some(shutdown.clone()),
        drain_owner_pid: Some(owner_pid),
    };
    let router = build_router(state);
    let server = axum::serve(listener, router).with_graceful_shutdown(async move {
        loop {
            tokio::select! {
                _ = shutdown.notified() => break,
                _ = tokio::time::sleep(Duration::from_millis(500)) => {
                    if !crate::platform::windows::process::is_process_alive(owner_pid) {
                        break;
                    }
                }
            }
        }
    });
    server
        .await
        .map_err(|error| AppError::Message(format!("native drain server failed: {error}")))?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub(super) async fn run_native_drain_helper(_owner_pid: u32) -> AppResult<()> {
    Ok(())
}

pub(super) fn browser_request_seen() -> bool {
    BROWSER_REQUEST_SEEN.load(Ordering::Acquire)
}

struct RunningProxy {
    shutdown: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
}

static RUNTIME: OnceLock<Mutex<Option<RunningProxy>>> = OnceLock::new();
const MODE_BROWSER: u8 = 0;
const MODE_NATIVE_DRAIN: u8 = 1;
static MODE: AtomicU8 = AtomicU8::new(MODE_BROWSER);
static BROWSER_REQUEST_SEEN: AtomicBool = AtomicBool::new(false);

fn runtime() -> &'static Mutex<Option<RunningProxy>> {
    RUNTIME.get_or_init(|| Mutex::new(None))
}

pub(super) async fn ensure_started(app: &AppHandle) -> AppResult<()> {
    retire_external_native_drain().await?;
    ensure_started_with_mode(app, MODE_BROWSER).await
}

pub(super) async fn ensure_native_drain_started(app: &AppHandle) -> AppResult<()> {
    ensure_started_with_mode(app, MODE_NATIVE_DRAIN).await
}

async fn ensure_started_with_mode(app: &AppHandle, mode: u8) -> AppResult<()> {
    MODE.store(mode, Ordering::Release);
    if is_ready().await {
        return Ok(());
    }

    let mut guard = runtime().lock().await;
    if let Some(existing) = guard.as_ref() {
        if !existing.task.is_finished() {
            drop(guard);
            return wait_until_ready().await;
        }
    }
    if let Some(finished) = guard.take() {
        finished.task.abort();
    }

    let listener = tokio::net::TcpListener::bind(LISTEN_ADDR)
        .await
        .map_err(|error| {
            AppError::Message(format!(
                "Mnelyra could not bind Web Models Responses bridge on {LISTEN_ADDR}: {error}"
            ))
        })?;
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| {
            AppError::Message(format!("could not build Web Models HTTP client: {error}"))
        })?;
    let state = ProxyState {
        app: Some(app.clone()),
        client,
        helper_shutdown: None,
        drain_owner_pid: None,
    };
    let router = build_router(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tokio::spawn(async move {
        let server = axum::serve(listener, router).with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        });
        if let Err(error) = server.await {
            eprintln!("Mnelyra Web Models Responses bridge stopped: {error}");
        }
    });
    *guard = Some(RunningProxy {
        shutdown: shutdown_tx,
        task,
    });
    drop(guard);
    wait_until_ready().await
}

fn build_router(state: ProxyState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/internal/drain/stop", post(stop_drain_helper))
        .route("/v1/models", get(models))
        .route(
            "/v1/responses",
            get(websocket_not_supported).post(responses),
        )
        .route("/v1/responses/compact", post(compact))
        .route("/v1/alpha/search", post(alpha_search))
        .with_state(state)
}

async fn health_value() -> Option<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(900))
        .build()
        .ok()?;
    let response = client
        .get("http://127.0.0.1:17841/healthz")
        .send()
        .await
        .ok()?;
    if !response.status().is_success()
        || response
            .headers()
            .get("x-mnelyra-web-models")
            .and_then(|value| value.to_str().ok())
            != Some("1")
    {
        return None;
    }
    response.json::<Value>().await.ok()
}

async fn wait_until_not_ready(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if health_value().await.is_none() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

async fn retire_external_native_drain() -> AppResult<()> {
    let Some(health) = health_value().await else {
        return Ok(());
    };
    if health.get("mode").and_then(Value::as_str) != Some("native-drain") {
        return Ok(());
    }

    if health.get("drainPid").and_then(Value::as_u64).is_some() {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .map_err(|error| {
                AppError::Message(format!("could not build drain control client: {error}"))
            })?;
        let response = client
            .post("http://127.0.0.1:17841/internal/drain/stop")
            .send()
            .await
            .map_err(|error| {
                AppError::Message(format!("could not stop stale native drain: {error}"))
            })?;
        if !response.status().is_success() {
            return Err(AppError::Message(format!(
                "stale native drain refused shutdown with HTTP {}",
                response.status()
            )));
        }
    } else {
        // In-process drain from the current Mnelyra instance.
        stop().await;
    }

    if wait_until_not_ready(Duration::from_secs(3)).await {
        Ok(())
    } else {
        Err(AppError::Message(
            "native drain did not release 127.0.0.1:17841 in time".into(),
        ))
    }
}

pub(super) fn enter_native_drain() -> bool {
    MODE.store(MODE_NATIVE_DRAIN, Ordering::Release);
    BROWSER_REQUEST_SEEN.swap(false, Ordering::AcqRel)
}

pub(super) fn enter_browser_mode() {
    MODE.store(MODE_BROWSER, Ordering::Release);
}

pub(super) fn is_native_drain() -> bool {
    MODE.load(Ordering::Acquire) == MODE_NATIVE_DRAIN
}

#[allow(dead_code)]
pub(super) async fn stop() {
    let mut guard = runtime().lock().await;
    if let Some(running) = guard.take() {
        let _ = running.shutdown.send(());
        let _ = tokio::time::timeout(Duration::from_secs(3), running.task).await;
    }
}

pub(super) async fn is_ready() -> bool {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_millis(700))
        .build()
    {
        Ok(client) => client,
        Err(_) => return false,
    };
    client
        .get("http://127.0.0.1:17841/healthz")
        .send()
        .await
        .ok()
        .is_some_and(|response| {
            response.status().is_success()
                && response
                    .headers()
                    .get("x-mnelyra-web-models")
                    .and_then(|value| value.to_str().ok())
                    == Some("1")
        })
}

async fn wait_until_ready() -> AppResult<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if is_ready().await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(AppError::Message(
        "Mnelyra Web Models Responses bridge did not become ready on 127.0.0.1:17841".into(),
    ))
}

async fn health(State(state): State<ProxyState>) -> impl IntoResponse {
    let mode = if is_native_drain() {
        "native-drain"
    } else {
        "browser"
    };
    (
        [("x-mnelyra-web-models", "1")],
        Json(json!({
            "ok": true,
            "owner": "mnelyra",
            "port": 17841,
            "mode": mode,
            "drainPid": state.helper_shutdown.as_ref().map(|_| std::process::id()),
            "ownerPid": state.drain_owner_pid,
        })),
    )
}

async fn stop_drain_helper(State(state): State<ProxyState>) -> Response {
    if !is_native_drain() {
        return bridge_error(
            StatusCode::CONFLICT,
            "bridge is not in native-drain mode".into(),
        );
    }
    let Some(shutdown) = state.helper_shutdown else {
        return bridge_error(
            StatusCode::CONFLICT,
            "native drain is owned by the main Mnelyra process".into(),
        );
    };
    shutdown.notify_one();
    StatusCode::NO_CONTENT.into_response()
}

async fn websocket_not_supported() -> Response {
    (
        StatusCode::UPGRADE_REQUIRED,
        [("content-type", "application/json")],
        Json(json!({
            "error": {
                "message": "WebSocket transport is not supported by the Mnelyra Web Models bridge; use Responses HTTP streaming",
                "type": "unsupported_transport"
            }
        })),
    )
        .into_response()
}

async fn models(State(state): State<ProxyState>, headers: HeaderMap) -> Response {
    forward_native_stream(&state, Method::GET, "models", headers, None).await
}

async fn responses(State(state): State<ProxyState>, headers: HeaderMap, body: Bytes) -> Response {
    let json_body = match decode_request_body(&headers, &body) {
        Ok(body) => body,
        Err(error) => return bridge_error(StatusCode::BAD_REQUEST, error),
    };
    let decoded = match serde_json::from_slice::<Value>(&json_body) {
        Ok(value) => value,
        Err(error) => {
            let content_type = header_text(&headers, "content-type");
            let content_encoding = header_text(&headers, "content-encoding");
            eprintln!(
                "Mnelyra Web Models rejected Responses request: body_bytes={} decoded_bytes={} content_type={content_type:?} content_encoding={content_encoding:?} error={error}",
                body.len(),
                json_body.len(),
            );
            return bridge_error(
                StatusCode::BAD_REQUEST,
                format!(
                    "invalid Responses JSON: {error} (body_bytes={}, decoded_bytes={}, content_encoding={})",
                    body.len(),
                    json_body.len(),
                    content_encoding.as_deref().unwrap_or("identity")
                ),
            );
        }
    };
    let model = decoded
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if is_browser_model(model) && !is_native_drain() {
        BROWSER_REQUEST_SEEN.store(true, Ordering::Release);
        let Some(app) = state.app.as_ref() else {
            return bridge_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "browser bridge is unavailable in the native-drain helper".into(),
            );
        };
        return super::browser_responses(app, decoded).await;
    }
    forward_native_stream(&state, Method::POST, "responses", headers, Some(body)).await
}

async fn compact(State(state): State<ProxyState>, headers: HeaderMap, body: Bytes) -> Response {
    let model = serde_json::from_slice::<Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("model")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default();
    if model.starts_with("mnelyra-web/") {
        return bridge_error(
            StatusCode::NOT_IMPLEMENTED,
            "Web-model compaction is not enabled yet".into(),
        );
    }
    forward_native_stream(
        &state,
        Method::POST,
        "responses/compact",
        headers,
        Some(body),
    )
    .await
}

async fn alpha_search(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward_native_stream(&state, Method::POST, "alpha/search", headers, Some(body)).await
}

async fn forward_native_stream(
    state: &ProxyState,
    method: Method,
    endpoint: &str,
    headers: HeaderMap,
    body: Option<Bytes>,
) -> Response {
    let request = match build_native_request(state, method, endpoint, headers, body) {
        Ok(request) => request,
        Err(error) => return bridge_error(StatusCode::BAD_REQUEST, error.to_string()),
    };
    let response = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return bridge_error(
                StatusCode::BAD_GATEWAY,
                format!("native Codex request failed: {error}"),
            );
        }
    };
    let status = response.status();
    let headers = end_to_end_headers(response.headers());
    let mut builder = Response::builder().status(status);
    if let Some(target) = builder.headers_mut() {
        *target = headers;
    }
    builder
        .body(Body::from_stream(response.bytes_stream()))
        .unwrap_or_else(|error| {
            bridge_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not stream native Codex response: {error}"),
            )
        })
}

fn build_native_request(
    state: &ProxyState,
    method: Method,
    endpoint: &str,
    headers: HeaderMap,
    body: Option<Bytes>,
) -> AppResult<reqwest::RequestBuilder> {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.starts_with("Bearer ") && value.len() > "Bearer ".len())
        .ok_or_else(|| {
            AppError::Message(
                "Codex did not provide its ChatGPT bearer authorization to the local bridge".into(),
            )
        })?;
    let mut outgoing = end_to_end_headers(&headers);
    outgoing.insert(
        HeaderName::from_static("authorization"),
        authorization.parse().map_err(|error| {
            AppError::Message(format!("invalid Codex authorization header: {error}"))
        })?,
    );
    let url = format!("{OFFICIAL_CODEX_BACKEND}/{endpoint}");
    let request = state.client.request(method, url).headers(outgoing);
    Ok(if let Some(body) = body {
        request.body(body)
    } else {
        request
    })
}

fn end_to_end_headers(source: &HeaderMap) -> HeaderMap {
    const HOP_BY_HOP: &[&str] = &[
        "connection",
        "keep-alive",
        "proxy-authenticate",
        "proxy-authorization",
        "te",
        "trailer",
        "transfer-encoding",
        "upgrade",
        "host",
        "content-length",
    ];
    let mut headers = source.clone();
    for name in HOP_BY_HOP {
        headers.remove(*name);
    }
    headers
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn read_decoded(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    reader
        .by_ref()
        .take(MAX_DECODED_REQUEST_BYTES + 1)
        .read_to_end(&mut decoded)
        .map_err(|error| format!("could not decode Responses request body: {error}"))?;
    if decoded.len() as u64 > MAX_DECODED_REQUEST_BYTES {
        return Err(format!(
            "decoded Responses request body exceeds {} MiB",
            MAX_DECODED_REQUEST_BYTES / (1024 * 1024)
        ));
    }
    Ok(decoded)
}

fn decode_one_encoding(encoding: &str, body: &[u8]) -> Result<Vec<u8>, String> {
    match encoding {
        "identity" => Ok(body.to_vec()),
        "gzip" | "x-gzip" => read_decoded(flate2::read::GzDecoder::new(body)),
        "deflate" => read_decoded(flate2::read::ZlibDecoder::new(body))
            .or_else(|_| read_decoded(flate2::read::DeflateDecoder::new(body))),
        "br" => read_decoded(brotli::Decompressor::new(body, 4096)),
        "zstd" => {
            let decoder = zstd::stream::read::Decoder::new(body)
                .map_err(|error| format!("could not initialize zstd request decoder: {error}"))?;
            read_decoded(decoder)
        }
        other => Err(format!("unsupported Responses Content-Encoding: {other}")),
    }
}

fn decode_request_body(headers: &HeaderMap, body: &[u8]) -> Result<Vec<u8>, String> {
    let Some(raw) = header_text(headers, "content-encoding") else {
        return Ok(body.to_vec());
    };
    let encodings = raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    let mut decoded = body.to_vec();
    for encoding in encodings.iter().rev() {
        decoded = decode_one_encoding(encoding, &decoded)?;
    }
    Ok(decoded)
}

fn is_browser_model(model: &str) -> bool {
    model == "gpt-5.6-sol" || model.starts_with("mnelyra-web/")
}

fn bridge_error(status: StatusCode, message: String) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "message": message,
                "type": "mnelyra_web_models_error"
            }
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_sol_uses_browser_bridge() {
        assert!(is_browser_model("gpt-5.6-sol"));
        assert!(is_browser_model("mnelyra-web/high"));
        assert!(!is_browser_model("gpt-5.6-terra"));
    }

    #[test]
    fn drain_mode_is_explicit_and_reversible() {
        enter_browser_mode();
        assert!(!is_native_drain());
        let _ = enter_native_drain();
        assert!(is_native_drain());
        enter_browser_mode();
        assert!(!is_native_drain());
    }

    #[test]
    fn strips_hop_by_hop_headers() {
        let mut headers = HeaderMap::new();
        headers.insert("connection", "keep-alive".parse().unwrap());
        headers.insert("authorization", "Bearer token".parse().unwrap());
        let clean = end_to_end_headers(&headers);
        assert!(!clean.contains_key("connection"));
        assert!(clean.contains_key("authorization"));
    }

    #[test]
    fn decodes_gzip_responses_request_body() {
        use std::io::Write;

        let raw = br#"{"model":"gpt-5.6-sol","reasoning":{"effort":"high"},"input":"ping"}"#;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(raw).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("content-encoding", "gzip".parse().unwrap());
        assert_eq!(decode_request_body(&headers, &compressed).unwrap(), raw);
    }
}
