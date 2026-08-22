use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::webview::WebviewWindowBuilder;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow};

use crate::error::{AppError, AppResult};

mod browser_turn;
mod codex_route;
mod proxy;

const BROWSER_LABEL: &str = "mnelyra-web-models-browser";
const CHATGPT_TEMPORARY_URL: &str = "https://chatgpt.com/?temporary-chat=true";
const CHATGPT_ORIGIN: &str = "https://chatgpt.com";
const BROWSER_READY_TIMEOUT: Duration = Duration::from_secs(35);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(180);

static BROWSER_BUSY: AtomicBool = AtomicBool::new(false);
static BROWSER_CLOSING: AtomicBool = AtomicBool::new(false);
static BROWSER_READY_CACHED: AtomicBool = AtomicBool::new(false);
static DEVTOOLS_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static DEVTOOLS_SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WebModelBridgeStatus {
    pub codex_detected: bool,
    pub route_installed: bool,
    pub route_active: bool,
    pub route_url: String,
    pub browser_running: bool,
    pub browser_ready: bool,
    pub browser_busy: bool,
    pub proxy_ready: bool,
    pub ready: bool,
    pub detail: String,
}

struct BrowserClosingGuard;

impl BrowserClosingGuard {
    fn acquire() -> Self {
        BROWSER_CLOSING.store(true, Ordering::Release);
        Self
    }
}

impl Drop for BrowserClosingGuard {
    fn drop(&mut self) {
        BROWSER_CLOSING.store(false, Ordering::Release);
    }
}

struct DevToolsCallGuard;

impl DevToolsCallGuard {
    fn acquire() -> AppResult<Self> {
        if BROWSER_CLOSING.load(Ordering::Acquire) {
            return Err(AppError::Message(
                "Mnelyra ChatGPT window is closing".into(),
            ));
        }
        DEVTOOLS_IN_FLIGHT.fetch_add(1, Ordering::AcqRel);
        if BROWSER_CLOSING.load(Ordering::Acquire) {
            DEVTOOLS_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
            return Err(AppError::Message(
                "Mnelyra ChatGPT window is closing".into(),
            ));
        }
        Ok(Self)
    }
}

impl Drop for DevToolsCallGuard {
    fn drop(&mut self) {
        DEVTOOLS_IN_FLIGHT.fetch_sub(1, Ordering::AcqRel);
    }
}

async fn wait_for_devtools_idle(timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while DEVTOOLS_IN_FLIGHT.load(Ordering::Acquire) != 0 {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

#[cfg(not(windows))]
async fn call_devtools(_window: &WebviewWindow, _method: &str, _params: Value) -> AppResult<Value> {
    Err(AppError::Message(
        "Mnelyra Web Models DevTools integration is not implemented for this platform yet".into(),
    ))
}

async fn browser_responses(app: &AppHandle, request: Value) -> axum::response::Response {
    browser_turn::responses(app, request).await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrowserSessionProbe {
    authenticated: bool,
    composer_ready: bool,
    temporary: bool,
    #[allow(dead_code)]
    url: String,
}

struct BusyGuard;

impl BusyGuard {
    fn acquire() -> AppResult<Self> {
        BROWSER_BUSY
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| AppError::Message("Mnelyra Web Models browser is busy".into()))?;
        Ok(Self)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        BROWSER_BUSY.store(false, Ordering::Release);
    }
}

fn browser_profile_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let root = app.path().app_data_dir().map_err(|error| {
        AppError::Message(format!("could not resolve Mnelyra app data: {error}"))
    })?;
    Ok(root.join("web-models").join("browser-profile"))
}

fn browser_window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(BROWSER_LABEL)
}

fn create_browser_window(app: &AppHandle, proxy_server: Option<&str>) -> AppResult<WebviewWindow> {
    if BROWSER_CLOSING.load(Ordering::Acquire) {
        return Err(AppError::Message(
            "Mnelyra ChatGPT window is closing".into(),
        ));
    }

    let url = CHATGPT_TEMPORARY_URL
        .parse()
        .map_err(|error| AppError::Message(format!("invalid ChatGPT URL: {error}")))?;
    let profile = browser_profile_dir(app)?;
    std::fs::create_dir_all(&profile)?;

    let mut builder = WebviewWindowBuilder::new(app, BROWSER_LABEL, WebviewUrl::External(url))
        .title("Mnelyra · ChatGPT")
        // Keep the hidden browser on ChatGPT's full desktop composer layout.
        // Narrower widths can collapse the reasoning picker into compact UI,
        // which makes deterministic model-effort selection impossible.
        .inner_size(1440.0, 900.0)
        .min_inner_size(960.0, 640.0)
        .data_directory(profile)
        .visible(false);
    #[cfg(windows)]
    {
        builder = builder.additional_browser_args(
        "--disable-background-timer-throttling --disable-renderer-backgrounding --disable-backgrounding-occluded-windows --disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection,CalculateNativeWinOcclusion",
        );
    }
    if let Some(proxy_server) = proxy_server {
        let proxy_url = proxy_server.parse().map_err(|error| {
            AppError::Message(format!("invalid Mnelyra Web Models proxy URL: {error}"))
        })?;
        builder = builder.proxy_url(proxy_url);
    }

    builder.build().map_err(|error| {
        AppError::Message(format!("could not create Mnelyra ChatGPT window: {error}"))
    })
}

fn ensure_browser_window(app: &AppHandle) -> AppResult<WebviewWindow> {
    if let Some(window) = browser_window(app) {
        return Ok(window);
    }
    create_browser_window(app, None)
}

#[cfg(windows)]
fn parse_loopback_proxy_server(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    let selected = if raw.contains(';') {
        let mut https = None;
        let mut http = None;
        let mut socks = None;
        for segment in raw
            .split(';')
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let Some((kind, endpoint)) = segment.split_once('=') else {
                continue;
            };
            match kind.trim().to_ascii_lowercase().as_str() {
                "https" => https = Some(endpoint.trim()),
                "http" => http = Some(endpoint.trim()),
                "socks" | "socks5" => socks = Some(endpoint.trim()),
                _ => {}
            }
        }
        https.or(http).or(socks)?
    } else {
        raw
    };

    let selected = selected
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_start_matches("socks5://")
        .trim_start_matches("socks://");

    let (host, port_text) = if let Some(rest) = selected.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        (format!("[{host}]"), port)
    } else {
        let (host, port) = selected.rsplit_once(':')?;
        (host.to_string(), port)
    };
    let normalized_host = host.trim().to_ascii_lowercase();
    if !matches!(
        normalized_host.as_str(),
        "127.0.0.1" | "localhost" | "[::1]" | "::1"
    ) {
        return None;
    }

    let port: u16 = port_text.trim().parse().ok()?;
    let socket = if matches!(normalized_host.as_str(), "[::1]" | "::1") {
        std::net::SocketAddr::new(std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST), port)
    } else {
        std::net::SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), port)
    };
    if std::net::TcpStream::connect_timeout(&socket, Duration::from_millis(350)).is_err() {
        return None;
    }

    let url_host = if matches!(normalized_host.as_str(), "[::1]" | "::1") {
        "[::1]"
    } else if normalized_host == "localhost" {
        "localhost"
    } else {
        "127.0.0.1"
    };
    Some(format!("http://{url_host}:{port}"))
}

#[cfg(windows)]
fn configured_loopback_proxy_server() -> Option<String> {
    let output = std::process::Command::new("reg.exe")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyServer",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text.lines().find(|line| line.contains("ProxyServer"))?;
    parse_loopback_proxy_server(line.split_whitespace().last()?)
}

#[cfg(windows)]
async fn recreate_browser_with_loopback_proxy(
    app: &AppHandle,
    window: &WebviewWindow,
) -> AppResult<Option<WebviewWindow>> {
    let Some(proxy_server) = configured_loopback_proxy_server() else {
        return Ok(None);
    };
    eprintln!("[web-models] retrying managed ChatGPT WebView through detected loopback proxy");
    if !wait_for_devtools_idle(Duration::from_secs(2)).await {
        return Err(AppError::Message(
            "Mnelyra ChatGPT WebView is still busy while preparing local proxy recovery".into(),
        ));
    }
    let _ = window.destroy();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while browser_window(app).is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if browser_window(app).is_some() {
        return Err(AppError::Message(
            "could not recreate the Mnelyra ChatGPT window for local proxy recovery".into(),
        ));
    }
    let replacement = create_browser_window(app, Some(&proxy_server))?;
    wait_for_chatgpt_document(&replacement).await?;
    Ok(Some(replacement))
}

#[cfg(not(windows))]
async fn recreate_browser_with_loopback_proxy(
    _app: &AppHandle,
    _window: &WebviewWindow,
) -> AppResult<Option<WebviewWindow>> {
    Ok(None)
}

fn is_chatgpt_url(window: &WebviewWindow) -> bool {
    window
        .url()
        .ok()
        .is_some_and(|url| url.as_str().starts_with(CHATGPT_ORIGIN))
}

async fn wait_for_chatgpt_document(window: &WebviewWindow) -> AppResult<()> {
    let deadline = tokio::time::Instant::now() + BROWSER_READY_TIMEOUT;
    let mut navigation_retries = 0_u8;
    while tokio::time::Instant::now() < deadline {
        let current_url = window.url().ok();
        if current_url
            .as_ref()
            .is_some_and(|url| url.as_str().starts_with("chrome-error://"))
        {
            if navigation_retries < 2 {
                navigation_retries += 1;
                let url = CHATGPT_TEMPORARY_URL.parse().map_err(|error| {
                    AppError::Message(format!("invalid ChatGPT recovery URL: {error}"))
                })?;
                window.navigate(url).map_err(|error| {
                    AppError::Message(format!(
                        "could not retry the Mnelyra ChatGPT navigation after a WebView load error: {error}"
                    ))
                })?;
                tokio::time::sleep(Duration::from_millis(750)).await;
                continue;
            }
            return Err(AppError::Message(
                "Mnelyra ChatGPT WebView could not reach ChatGPT after navigation retries".into(),
            ));
        }

        if is_chatgpt_url(window) {
            let expression =
                r#"(() => ({ readyState: document.readyState, url: location.href }))()"#;
            if let Ok(value) = evaluate_browser_value(window, expression).await {
                if value
                    .get("readyState")
                    .and_then(Value::as_str)
                    .is_some_and(|state| matches!(state, "interactive" | "complete"))
                {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Err(AppError::Message(
        "ChatGPT did not finish loading in the Mnelyra browser window".into(),
    ))
}

async fn probe_browser_session(window: &WebviewWindow) -> AppResult<BrowserSessionProbe> {
    let expression = format!(
        r#"(async () => {{
          const visible = (element) => {{
            if (!element || !element.isConnected) return false;
            const style = getComputedStyle(element);
            const rect = element.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0
              && style.display !== 'none' && style.visibility !== 'hidden' && style.opacity !== '0';
          }};
          const composer = Array.from(document.querySelectorAll(
            '[data-testid="prompt-textarea"], #prompt-textarea, [contenteditable="true"][data-lexical-editor="true"], [contenteditable="true"][role="textbox"], textarea'
          )).find(visible);
          const current = new URL(location.href);
          let authenticated = false;
          try {{
            const controller = new AbortController();
            const timer = setTimeout(() => controller.abort(), 5000);
            try {{
              const response = await fetch('/api/auth/session', {{
                credentials: 'include', cache: 'no-store', headers: {{ accept: 'application/json' }}, signal: controller.signal
              }});
              const payload = response.ok && response.headers.get('content-type')?.includes('application/json')
                ? await response.json() : null;
              const user = payload?.user && typeof payload.user === 'object' && !Array.isArray(payload.user)
                ? payload.user : null;
              const expires = payload?.expires;
              const expiryValid = expires == null || (typeof expires === 'string'
                && Number.isFinite(Date.parse(expires)) && Date.parse(expires) > Date.now());
              authenticated = Boolean(user && Object.keys(user).length > 0 && !payload?.error && expiryValid);
            }} finally {{ clearTimeout(timer); }}
          }} catch {{}}
          return {{
            authenticated,
            composerReady: Boolean(composer),
            temporary: current.origin === {origin} && current.pathname === '/'
              && current.searchParams.get('temporary-chat') === 'true',
            url: current.href,
          }};
        }})()"#,
        origin = serde_json::to_string(CHATGPT_ORIGIN)?
    );
    let value = evaluate_browser_value(window, &expression).await?;
    serde_json::from_value(value).map_err(AppError::from)
}

fn sign_in_window_closed(app: &AppHandle, window: &WebviewWindow) -> bool {
    browser_window(app).is_none() || window.is_visible().is_err() || window.url().is_err()
}

async fn wait_for_sign_in_window_close(app: &AppHandle, window: &WebviewWindow) {
    while !sign_in_window_closed(app, window) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn wait_for_sign_in(
    app: &AppHandle,
    window: &WebviewWindow,
) -> AppResult<BrowserSessionProbe> {
    let deadline = tokio::time::Instant::now() + SIGN_IN_TIMEOUT;
    while tokio::time::Instant::now() < deadline {
        if sign_in_window_closed(app, window) {
            return Err(AppError::Message(
                "The Mnelyra ChatGPT sign-in window was closed. Click Start again to reopen it"
                    .into(),
            ));
        }
        let probe = tokio::select! {
            probe = probe_browser_session(window) => probe,
            _ = wait_for_sign_in_window_close(app, window) => {
                return Err(AppError::Message(
                    "The Mnelyra ChatGPT sign-in window was closed. Click Start again to reopen it"
                        .into(),
                ));
            }
        };
        if let Ok(probe) = probe {
            if probe.authenticated && probe.composer_ready && probe.temporary {
                return Ok(probe);
            }
        }
        tokio::time::sleep(Duration::from_millis(750)).await;
    }
    Err(AppError::Message(
        "ChatGPT sign-in was not completed in the Mnelyra browser window".into(),
    ))
}

pub async fn status(app: &AppHandle) -> AppResult<WebModelBridgeStatus> {
    let codex_detected = crate::codex::discover_executable().is_ok();
    let route = codex_route::status(app)?;
    let proxy_ready = proxy::is_ready().await;
    let proxy_mode = if proxy_ready {
        proxy::active_mode().await
    } else {
        None
    };
    let browser_busy = BROWSER_BUSY.load(Ordering::Acquire);
    // A WebView can remain in Tauri's window map briefly while Windows is
    // tearing its HWND down. Do not issue URL/DevTools probes during that gap.
    let window = if BROWSER_CLOSING.load(Ordering::Acquire) {
        None
    } else {
        browser_window(app)
    };
    let browser_running = window.is_some();
    let browser_ready = if let Some(window) = window.as_ref() {
        // Do not race the active browser turn with a health/status probe.
        // Both paths use WebView2 DevTools on the same hidden HWND; probing
        // during navigation/effort selection can enqueue work against a
        // transiently replaced WebView2 window and produce invalid-HWND
        // PostMessage failures. The last verified ready state is sufficient
        // while a turn owns the browser.
        if browser_busy {
            BROWSER_READY_CACHED.load(Ordering::Acquire)
        } else if is_chatgpt_url(window) {
            let ready = probe_browser_session(window)
                .await
                .is_ok_and(|probe| probe.authenticated && probe.composer_ready && probe.temporary);
            BROWSER_READY_CACHED.store(ready, Ordering::Release);
            ready
        } else {
            BROWSER_READY_CACHED.store(false, Ordering::Release);
            false
        }
    } else {
        BROWSER_READY_CACHED.store(false, Ordering::Release);
        false
    };

    let ready = codex_detected && route.active && browser_ready && proxy_ready;
    let draining = proxy_mode.as_deref() == Some("native-drain") && !route.active;
    let detail = if !codex_detected {
        "Codex was not detected".into()
    } else if draining {
        "Mnelyra Web Models are disconnected; already-loaded Codex threads are draining through the official Codex backend"
            .into()
    } else if !browser_ready {
        if browser_running {
            "Sign in to ChatGPT in the Mnelyra browser window".into()
        } else {
            "Mnelyra Web Models browser has not been started".into()
        }
    } else if !proxy_ready {
        "Mnelyra Responses bridge is not running on 127.0.0.1:17841".into()
    } else if !route.installed {
        "Mnelyra Web Models are ready, but the Codex route is not installed".into()
    } else if !route.active {
        route
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "The Codex route is installed but not active".into())
    } else {
        "Mnelyra Web Models are ready in Codex".into()
    };

    Ok(WebModelBridgeStatus {
        codex_detected,
        route_installed: route.installed,
        route_active: route.active,
        route_url: route.route_url,
        browser_running,
        browser_ready,
        browser_busy,
        proxy_ready,
        ready,
        detail,
    })
}

async fn start_web_models_impl(
    app: &AppHandle,
    allow_interactive_sign_in: bool,
) -> AppResult<WebModelBridgeStatus> {
    let _busy = BusyGuard::acquire()?;
    crate::codex::discover_executable().map_err(AppError::Message)?;

    let mut window = ensure_browser_window(app)?;
    let mut proxy_recovered = false;
    if let Err(direct_error) = wait_for_chatgpt_document(&window).await {
        match recreate_browser_with_loopback_proxy(app, &window).await {
            Ok(Some(replacement)) => {
                window = replacement;
                proxy_recovered = true;
            }
            Ok(None) => return Err(direct_error),
            Err(recovery_error) => {
                return Err(AppError::Message(format!(
                    "Mnelyra ChatGPT direct navigation failed: {direct_error}; local proxy recovery also failed: {recovery_error}"
                )));
            }
        }
    }
    let mut probe = probe_browser_session(&window).await?;
    if probe.url.starts_with("chrome-error://") && !proxy_recovered {
        if let Some(replacement) = recreate_browser_with_loopback_proxy(app, &window).await? {
            window = replacement;
            proxy_recovered = true;
            probe = probe_browser_session(&window).await?;
        }
    }
    #[cfg(debug_assertions)]
    eprintln!(
        "[web-models] browser probe authenticated={} composer_ready={} temporary={} proxy_recovered={} url={}",
        probe.authenticated, probe.composer_ready, probe.temporary, proxy_recovered, probe.url
    );
    #[cfg(debug_assertions)]
    if probe.url.starts_with("chrome-error://") {
        let detail = evaluate_browser_value(
            &window,
            r#"(() => ({ title: document.title, text: (document.body?.innerText || '').slice(0, 1200) }))()"#,
        )
        .await
        .unwrap_or(Value::Null);
        eprintln!("[web-models] browser error page detail={detail}");
    }
    if probe.url.starts_with("chrome-error://") {
        return Err(AppError::Message(
            "Mnelyra ChatGPT WebView could not reach ChatGPT through the available network routes"
                .into(),
        ));
    }
    if !(probe.authenticated && probe.composer_ready && probe.temporary) {
        if !allow_interactive_sign_in {
            return Err(AppError::Message(
                "Mnelyra Web Models self-test requires an already authenticated hidden browser profile"
                    .into(),
            ));
        }
        window.show().map_err(|error| {
            AppError::Message(format!("could not show Mnelyra ChatGPT window: {error}"))
        })?;
        let _ = window.set_focus();
        wait_for_sign_in(app, &window).await?;
    }
    BROWSER_READY_CACHED.store(true, Ordering::Release);
    let _ = window.hide();
    proxy::ensure_started(app).await?;
    let route = codex_route::install(app)?;
    if !route.active {
        return Err(AppError::Message(
            route.errors.first().cloned().unwrap_or_else(|| {
                "Mnelyra installed the Codex route, but it is not active".into()
            }),
        ));
    }
    status(app).await
}

pub async fn start_web_models(app: &AppHandle) -> AppResult<WebModelBridgeStatus> {
    start_web_models_impl(app, true).await
}

#[cfg(debug_assertions)]
fn collect_sse_output_text(body: &str) -> AppResult<String> {
    let mut output = String::new();
    for line in body.lines() {
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if value.get("type").and_then(Value::as_str) == Some("response.failed") {
            let message = value
                .pointer("/response/error/message")
                .and_then(Value::as_str)
                .or_else(|| value.pointer("/error/message").and_then(Value::as_str))
                .unwrap_or("Mnelyra Web Models response failed without an error message");
            return Err(AppError::Message(format!(
                "Web Models direct probe response.failed: {message}"
            )));
        }
        if value.get("type").and_then(Value::as_str) == Some("response.output_text.delta") {
            if let Some(delta) = value.get("delta").and_then(Value::as_str) {
                output.push_str(delta);
            }
        }
    }
    Ok(output)
}

#[cfg(debug_assertions)]
async fn debug_direct_responses_probe(
    model: &str,
    effort: &str,
    streaming_probe: bool,
) -> AppResult<String> {
    let input = if streaming_probe {
        "Write a structured Markdown answer of at least 1200 characters with one title, at least four prose paragraphs, and one bullet list containing at least four items. Do not use tools. Keep the answer substantive enough to observe streaming and end with the exact marker MNELYRA_STREAM_E2E_OK."
    } else {
        "Reply with exactly: MNELYRA_BRIDGE_E2E_OK"
    };
    let request = serde_json::json!({
        "model": model,
        "reasoning": { "effort": effort },
        "input": input,
        "stream": true,
    });
    let response = tokio::time::timeout(
        Duration::from_secs(300),
        reqwest::Client::new()
            .post("http://127.0.0.1:17841/v1/responses")
            .json(&request)
            .send(),
    )
    .await
    .map_err(|_| AppError::Message("Web Models direct probe timed out".into()))?
    .map_err(|error| AppError::Message(format!("Web Models direct probe failed: {error}")))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| AppError::Message(format!("Could not read direct probe SSE: {error}")))?;
    if !status.is_success() {
        return Err(AppError::Message(format!(
            "Web Models direct probe returned HTTP {status}: {body}"
        )));
    }
    let output = collect_sse_output_text(&body)?;
    let valid = if streaming_probe {
        output.chars().count() >= 700 && output.trim_end().ends_with("MNELYRA_STREAM_E2E_OK")
    } else {
        output.trim() == "MNELYRA_BRIDGE_E2E_OK"
    };
    if !valid {
        return Err(AppError::Message(format!(
            "Web Models direct probe returned unexpected text: {:?}",
            output.trim()
        )));
    }
    Ok(output)
}

#[cfg(debug_assertions)]
async fn debug_codex_exec_probe(
    model: &str,
    effort: &str,
    streaming_probe: bool,
    tool_probe: bool,
) -> AppResult<String> {
    let executable = crate::codex::discover_executable().map_err(AppError::Message)?;
    let cwd = std::env::current_dir()?;
    let output_file = std::env::temp_dir().join(format!(
        "mnelyra-web-models-e2e-{}.txt",
        uuid::Uuid::new_v4().simple()
    ));
    let tool_probe_file = tool_probe.then(|| {
        let marker = format!("MNELYRA_CODEX_TOOL_E2E_{}", uuid::Uuid::new_v4().simple());
        let relative = format!(
            ".rootrelay/web-models-tool-e2e-{}.txt",
            uuid::Uuid::new_v4().simple()
        );
        let path = cwd.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        (path, relative, marker)
    });
    if let Some((path, _, marker)) = tool_probe_file.as_ref() {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, format!("{marker}\n"))?;
    }

    let mut command = tokio::process::Command::new(&executable);
    let prompt = if let Some((_, relative, marker)) = tool_probe_file.as_ref() {
        format!(
            "Use an available Codex local read-only tool to read the file `{relative}` relative to the current workdir. You MUST actually call a tool and use its returned contents; do not guess or infer the file. Prefer forward slashes in tool arguments. After the tool result is returned, reply with exactly the file contents, which must be the marker {marker}, and nothing else."
        )
    } else if streaming_probe {
        "Write a structured Markdown answer of at least 1200 characters with one title, at least four prose paragraphs, and one bullet list containing at least four items. Do not use tools. Keep the answer substantive enough to observe streaming and end with the exact marker MNELYRA_CODEX_STREAM_E2E_OK.".to_string()
    } else {
        "Reply with exactly: MNELYRA_CODEX_E2E_OK. Do not use tools.".to_string()
    };
    command
        .arg("-c")
        .arg(format!("model_reasoning_effort={effort:?}"))
        .arg("exec")
        .arg("--ephemeral")
        .arg("--skip-git-repo-check")
        .arg("--color")
        .arg("never")
        .arg("--sandbox")
        .arg("read-only")
        .arg("--model")
        .arg(model)
        .arg("--cd")
        .arg(&cwd)
        .arg("--output-last-message")
        .arg(&output_file)
        .arg(&prompt)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(Duration::from_secs(300), command.output())
        .await
        .map_err(|_| AppError::Message("Codex Web Models E2E probe timed out".into()))?
        .map_err(|error| AppError::Message(format!("Could not start Codex E2E probe: {error}")))?;
    let final_text = std::fs::read_to_string(&output_file).unwrap_or_default();
    let _ = std::fs::remove_file(&output_file);
    if let Some((path, _, _)) = tool_probe_file.as_ref() {
        let _ = std::fs::remove_file(path);
    }
    if !output.status.success() {
        return Err(AppError::Message(format!(
            "Codex Web Models E2E probe exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let valid = if let Some((_, _, marker)) = tool_probe_file.as_ref() {
        final_text.trim() == marker
    } else if streaming_probe {
        final_text.chars().count() >= 1200
            && final_text
                .trim_end()
                .ends_with("MNELYRA_CODEX_STREAM_E2E_OK")
    } else {
        final_text.trim().trim_end_matches(['.', '。']).trim_end() == "MNELYRA_CODEX_E2E_OK"
    };
    if !valid {
        return Err(AppError::Message(format!(
            "Codex Web Models E2E probe returned unexpected text: {:?}; stderr: {}",
            final_text.trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(final_text)
}

/// Debug-only opt-in smoke test. Normal application startup never calls Web
/// Models unless the user clicks Start. Developers may set
/// `MNELYRA_WEB_MODELS_SELFTEST=1` for one process to exercise the real browser
/// bridge and a real non-interactive Codex CLI request end-to-end.
#[cfg(debug_assertions)]
pub(crate) fn debug_self_test_enabled() -> bool {
    std::env::var("MNELYRA_WEB_MODELS_SELFTEST").ok().as_deref() == Some("1")
        || std::env::args().any(|arg| {
            matches!(
                arg.as_str(),
                "--web-models-selftest"
                    | "--web-models-selftest-low"
                    | "--web-models-selftest-medium"
                    | "--web-models-selftest-high"
                    | "--web-models-selftest-stream"
                    | "--web-models-selftest-tool"
                    | "--web-models-selftest-hold"
            )
        })
}

#[cfg(debug_assertions)]
fn debug_self_test_model() -> &'static str {
    "gpt-5.6-sol"
}

#[cfg(debug_assertions)]
fn debug_self_test_effort() -> &'static str {
    if std::env::args().any(|arg| arg == "--web-models-selftest-low") {
        "low"
    } else if std::env::args().any(|arg| {
        matches!(
            arg.as_str(),
            "--web-models-selftest-medium" | "--web-models-selftest"
        )
    }) {
        "medium"
    } else {
        "high"
    }
}

#[cfg(debug_assertions)]
fn debug_self_test_streaming() -> bool {
    std::env::args().any(|arg| arg == "--web-models-selftest-stream")
}

#[cfg(debug_assertions)]
fn debug_self_test_tool() -> bool {
    std::env::args().any(|arg| arg == "--web-models-selftest-tool")
}

#[cfg(debug_assertions)]
fn debug_self_test_hold() -> bool {
    std::env::args().any(|arg| arg == "--web-models-selftest-hold")
}

#[cfg(debug_assertions)]
pub(crate) fn schedule_debug_self_test(app: AppHandle) {
    if !debug_self_test_enabled() {
        return;
    }
    let model = debug_self_test_model();
    let effort = debug_self_test_effort();
    let streaming_probe = debug_self_test_streaming();
    let tool_probe = debug_self_test_tool();
    let hold_only = debug_self_test_hold();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_millis(800)).await;
        let started = std::time::Instant::now();
        eprintln!("[web-models selftest] START model={model} effort={effort}");
        let result = async {
            let status = start_web_models_impl(&app, false).await?;
            if !status.ready {
                return Err(AppError::Message(format!(
                    "Web Models did not become ready: {}",
                    status.detail
                )));
            }
            eprintln!(
                "[web-models selftest] bridge_ready_ms={}",
                started.elapsed().as_millis()
            );
            if hold_only {
                eprintln!("[web-models selftest] HOLD ready_for_external_probe=true");
                tokio::time::sleep(Duration::from_secs(600)).await;
                return AppResult::Ok(());
            }
            if !tool_probe {
                let direct_started = std::time::Instant::now();
                let direct = debug_direct_responses_probe(model, effort, streaming_probe).await?;
                eprintln!(
                    "[web-models selftest] direct_pass_ms={} text={:?}",
                    direct_started.elapsed().as_millis(),
                    direct.trim()
                );
            }
            let codex_started = std::time::Instant::now();
            let codex = debug_codex_exec_probe(model, effort, streaming_probe, tool_probe).await?;
            eprintln!(
                "[web-models selftest] codex_pass_ms={} text={:?}",
                codex_started.elapsed().as_millis(),
                codex.trim()
            );
            AppResult::Ok(())
        }
        .await;

        let stop_result = stop_web_models(&app).await;
        match result {
            Ok(()) => eprintln!(
                "[web-models selftest] PASS total_ms={}",
                started.elapsed().as_millis()
            ),
            Err(error) => eprintln!("[web-models selftest] FAIL: {error}"),
        }
        if let Err(error) = stop_result {
            eprintln!("[web-models selftest] cleanup failed: {error}");
        }
        app.exit(0);
    });
}

/// Restore only the reversible Codex route owned by Web Models.
///
/// This deliberately does not create the ChatGPT window or start the Responses
/// proxy. It is safe to call during application startup to recover from an
/// earlier unclean exit and during a real application exit so Codex never
/// remains pointed at an inactive 127.0.0.1:17841 endpoint.
pub fn restore_native_route_only(app: &AppHandle) -> AppResult<()> {
    let route = codex_route::status(app)?;
    if route.installed && route.active {
        let restored = codex_route::restore(app)?;
        if restored.active {
            return Err(AppError::Message(
                "Mnelyra could not restore the previous Codex route".into(),
            ));
        }
    }
    Ok(())
}

pub async fn stop_web_models(app: &AppHandle) -> AppResult<WebModelBridgeStatus> {
    let _busy = BusyGuard::acquire()?;
    let _closing = BrowserClosingGuard::acquire();
    BROWSER_READY_CACHED.store(false, Ordering::Release);
    // Codex freezes the provider endpoint when a thread is first loaded. New
    // threads read config.toml immediately, but an already-loaded Web thread
    // can keep calling 17841 after the route is restored. Flip the bridge to
    // native passthrough before changing config so such a stale thread falls
    // back to the official Codex backend instead of breaking or continuing to
    // consume ChatGPT Web.
    let browser_request_seen = proxy::enter_native_drain();
    let route = codex_route::restore(app)?;
    if route.active {
        proxy::enter_browser_mode();
        return Err(AppError::Message(
            "Mnelyra could not restore the previous Codex route".into(),
        ));
    }
    if !wait_for_devtools_idle(Duration::from_secs(3)).await {
        return Err(AppError::Message(
            "Mnelyra ChatGPT window still has an in-flight DevTools operation; retry disconnect after the current browser probe finishes"
                .into(),
        ));
    }
    let mut destroy_error = None;
    if let Some(window) = browser_window(app) {
        if let Err(error) = window.destroy() {
            destroy_error = Some(error.to_string());
        }
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while browser_window(app).is_some() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let still_registered = browser_window(app).is_some();
    if still_registered {
        return Err(AppError::Message(
            destroy_error
                .map(|error| format!("could not destroy Mnelyra ChatGPT window: {error}"))
                .unwrap_or_else(|| {
                    "Mnelyra ChatGPT window did not finish closing within two seconds".into()
                }),
        ));
    }
    if !browser_request_seen {
        proxy::stop().await;
    } else if let Some(owner_pid) =
        crate::codex::running_desktop_app_server_pid().map_err(AppError::Message)?
    {
        proxy::handoff_native_drain(app, owner_pid).await?;
    } else {
        proxy::stop().await;
    }
    status(app).await
}

/// Handle the tiny browserless native-drain subprocess before Tauri's
/// single-instance gate. Returning true means this process was the helper and
/// has fully handled its lifecycle.
pub(crate) fn run_native_drain_helper_if_requested() -> bool {
    let mut args = std::env::args();
    let _ = args.next();
    while let Some(arg) = args.next() {
        if arg != "--web-models-native-drain" {
            continue;
        }
        let result = args
            .next()
            .ok_or_else(|| AppError::Message("native drain owner PID is missing".into()))
            .and_then(|raw| {
                raw.parse::<u32>()
                    .map_err(|_| AppError::Message("native drain owner PID is invalid".into()))
            })
            .and_then(|owner_pid| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .map_err(|error| {
                        AppError::Message(format!("could not start native drain runtime: {error}"))
                    })?;
                runtime.block_on(proxy::run_native_drain_helper(owner_pid))
            });
        if let Err(error) = result {
            eprintln!("Mnelyra Web Models native drain stopped: {error}");
        }
        return true;
    }
    false
}

/// Restore the reversible route before a normal application exit. If the
/// Desktop app-server may still hold sessions created while Web Models were
/// active, start a detached native drain that will take 17841 as soon as this
/// process releases the listener and exit automatically with that app-server.
pub fn prepare_native_route_for_exit(app: &AppHandle) -> AppResult<()> {
    let had_browser_requests = proxy::browser_request_seen();
    restore_native_route_only(app)?;
    if had_browser_requests {
        if let Some(owner_pid) =
            crate::codex::running_desktop_app_server_pid().map_err(AppError::Message)?
        {
            proxy::spawn_native_drain_for_exit(owner_pid)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
async fn evaluate_browser_value(window: &WebviewWindow, expression: &str) -> AppResult<Value> {
    let params = serde_json::json!({
        "expression": expression,
        "awaitPromise": true,
        "returnByValue": true,
        "userGesture": true,
    });
    let raw = call_devtools(window, "Runtime.evaluate", params).await?;
    decode_devtools_response(&raw)
}

#[cfg(windows)]
async fn call_devtools(window: &WebviewWindow, method: &str, params: Value) -> AppResult<Value> {
    // Tauri dispatches `with_webview` onto the WebView/UI thread. Serializing
    // these calls prevents status probes and browser-turn input from posting
    // concurrent work to the same hidden WebView2 HWND during navigation.
    let _serial = DEVTOOLS_SERIAL.lock().await;
    let _call = DevToolsCallGuard::acquire()?;
    let (tx, rx) = tokio::sync::oneshot::channel();
    let method = method.to_string();
    let params = params.to_string();
    window
        .with_webview(move |platform| {
            let result = call_devtools_windows(platform, &method, &params);
            let _ = tx.send(result);
        })
        .map_err(|error| AppError::Message(format!("could not access Mnelyra WebView: {error}")))?;

    tokio::time::timeout(Duration::from_secs(30), rx)
        .await
        .map_err(|_| AppError::Message("Mnelyra WebView JavaScript evaluation timed out".into()))?
        .map_err(|_| AppError::Message("Mnelyra WebView JavaScript result channel closed".into()))?
}

#[cfg(windows)]
fn call_devtools_windows(
    platform: tauri::webview::PlatformWebview,
    method: &str,
    params: &str,
) -> AppResult<Value> {
    use std::sync::{Arc, Mutex};
    use webview2_com::{CallDevToolsProtocolMethodCompletedHandler, CoTaskMemPWSTR};

    let controller = platform.controller();
    let webview = unsafe { controller.CoreWebView2() }
        .map_err(|error| AppError::Message(format!("could not access WebView2 core: {error}")))?;
    let captured = Arc::new(Mutex::new(None::<Result<String, String>>));
    let captured_callback = captured.clone();
    let webview_for_call = webview.clone();
    let method_for_call = method.to_string();
    let params_for_call = params.to_string();
    CallDevToolsProtocolMethodCompletedHandler::wait_for_async_operation(
        Box::new(move |handler| unsafe {
            let method = CoTaskMemPWSTR::from(method_for_call.as_str());
            let params = CoTaskMemPWSTR::from(params_for_call.as_str());
            webview_for_call
                .CallDevToolsProtocolMethod(
                    *method.as_ref().as_pcwstr(),
                    *params.as_ref().as_pcwstr(),
                    &handler,
                )
                .map_err(webview2_com::Error::WindowsError)
        }),
        Box::new(move |error_code, result| {
            let value = if error_code.is_ok() {
                Ok(result.to_string())
            } else {
                Err(format!("WebView2 DevTools call failed with {error_code:?}"))
            };
            if let Ok(mut slot) = captured_callback.lock() {
                *slot = Some(value);
            }
            Ok(())
        }),
    )
    .map_err(|error| {
        AppError::Message(format!("WebView2 JavaScript evaluation failed: {error}"))
    })?;

    let raw = captured
        .lock()
        .map_err(|_| AppError::Message("WebView2 JavaScript result lock was poisoned".into()))?
        .take()
        .ok_or_else(|| AppError::Message("WebView2 returned no JavaScript result".into()))?
        .map_err(AppError::Message)?;
    serde_json::from_str(&raw).map_err(AppError::from)
}

#[cfg(not(windows))]
async fn evaluate_browser_value(_window: &WebviewWindow, _expression: &str) -> AppResult<Value> {
    Err(AppError::Message(
        "Mnelyra Web Models browser evaluation is not implemented for this platform yet".into(),
    ))
}

#[cfg(test)]
fn decode_devtools_value(raw: &str) -> AppResult<Value> {
    let response: Value = serde_json::from_str(raw)?;
    decode_devtools_response(&response)
}

fn decode_devtools_response(response: &Value) -> AppResult<Value> {
    if let Some(exception) = response.get("exceptionDetails") {
        return Err(AppError::Message(format!(
            "ChatGPT browser JavaScript failed: {exception}"
        )));
    }
    if let Some(exception) = response.pointer("/result/exceptionDetails") {
        return Err(AppError::Message(format!(
            "ChatGPT browser JavaScript failed: {exception}"
        )));
    }
    response
        .pointer("/result/value")
        .or_else(|| response.pointer("/result/result/value"))
        .cloned()
        .ok_or_else(|| {
            AppError::Message(format!(
                "ChatGPT browser JavaScript returned no serializable value: {response}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_runtime_evaluate_value() {
        let value =
            decode_devtools_value(r#"{"result":{"type":"object","value":{"authenticated":true}}}"#)
                .expect("decode DevTools value");
        assert_eq!(value["authenticated"], true);
    }

    #[test]
    fn rejects_runtime_evaluate_exception() {
        assert!(decode_devtools_value(
            r#"{"exceptionDetails":{"text":"boom"},"result":{"type":"undefined"}}"#
        )
        .is_err());
    }

    #[cfg(windows)]
    #[test]
    fn accepts_only_live_loopback_proxy_servers() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind loopback test port");
        let port = listener.local_addr().expect("local addr").port();
        assert_eq!(
            parse_loopback_proxy_server(&format!("127.0.0.1:{port}")),
            Some(format!("http://127.0.0.1:{port}"))
        );
        assert_eq!(
            parse_loopback_proxy_server(&format!("http=127.0.0.1:{port};https=127.0.0.1:{port}")),
            Some(format!("http://127.0.0.1:{port}"))
        );
        assert_eq!(
            parse_loopback_proxy_server(&format!("example.com:{port}")),
            None
        );
    }
}
