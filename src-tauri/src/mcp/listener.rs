use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Form, Query, State};
use axum::http::{header::CACHE_CONTROL, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tower_http::cors::CorsLayer;

use crate::activity::ActivityCoordinator;
use crate::auth::{
    authorization_server_metadata, authorize_get, external_base_url, protected_resource_metadata,
    token_exchange, verify_bearer_header, verify_oauth_bearer_header, AuthorizeParams,
    OAuthRuntime, TokenForm,
};
use crate::mcp::server::{handle_request, new_state, SharedState};
use crate::mcp::{TUNNEL_MCP_SECRET_KEY, TUNNEL_SECRET_SCOPE, TUNNEL_TOKEN_HEADER};
use crate::secret::SecretStore;
use crate::session::SessionCoordinator;
use crate::tools::policy::PolicySettings;
use crate::tools::wrap_mcp_tool_result;
use crate::tools::Workspace;
use crate::tunnel::append_profile_log;
use crate::workspace::{AuthConfig, RuntimeConfig};

pub type ShutdownSender = oneshot::Sender<()>;

#[derive(Clone)]
struct ListenerState {
    mcp: SharedState,
    sessions: SessionCoordinator,
    auth: AuthConfig,
    workspace_id: String,
    workspace_path: String,
    bind_port: u16,
    configured_public_url: String,
    bearer_token: Option<String>,
    oauth: Option<Arc<OAuthRuntime>>,
    oauth_client_secret: Option<String>,
}

fn is_codex_control_tool(name: &str) -> bool {
    matches!(
        name,
        "codex_start_task"
            | "codex_list_sessions"
            | "codex_send_input"
            | "codex_cancel_session"
            | "codex_compact_session"
            | "codex_session_events"
            | "codex_pending_requests"
            | "codex_respond_request"
    )
}

fn is_read_only_codex_control_tool(name: &str) -> bool {
    matches!(
        name,
        "codex_list_sessions" | "codex_session_events" | "codex_pending_requests"
    )
}

async fn handle_codex_tool(state: &ListenerState, body: &Value) -> Value {
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let params = body.get("params").cloned().unwrap_or(Value::Null);
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));

    if state.mcp.policy.permission_ceiling == "read_only" && !is_read_only_codex_control_tool(name)
    {
        let structured = json!({
            "ok": false,
            "status": "error",
            "summary": format!("{name} is blocked by the Mnelyra read-only ceiling"),
            "error": {
                "code": "MASTER_PERMISSION_READ_ONLY",
                "message": format!("{name} is blocked by the Mnelyra read-only ceiling"),
                "category": "permission",
                "retryable": false,
                "details": { "permission_ceiling": "read_only" }
            }
        });
        return json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": wrap_mcp_tool_result(name, &args, structured)
        });
    }

    let structured = match call_codex_tool(state, name, &args).await {
        Ok(value) => value,
        Err(message) => json!({
            "ok": false,
            "status": "error",
            "summary": message,
            "error": {
                "code": "CODEX_CONTROL_FAILED",
                "message": message,
                "category": "provider",
                "retryable": true,
                "details": {}
            }
        }),
    };
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": wrap_mcp_tool_result(name, &args, structured)
    })
}

async fn call_codex_tool(state: &ListenerState, name: &str, args: &Value) -> Result<Value, String> {
    match name {
        "codex_start_task" => {
            let title = required_string(args, "title")?;
            let prompt = required_string(args, "prompt")?;
            let session = state
                .sessions
                .start_codex_task(
                    &state.workspace_id,
                    &PathBuf::from(&state.workspace_path),
                    &title,
                    &prompt,
                )
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true, "session": session }))
        }
        "codex_compact_session" => {
            let session_id = required_string(args, "session_id")?;
            ensure_session_workspace(state, &session_id)?;
            let session = state
                .sessions
                .compact(&session_id)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true, "session": session }))
        }
        "codex_list_sessions" => {
            let mut sessions = state
                .sessions
                .list_sessions()
                .map_err(|error| error.to_string())?;
            sessions.retain(|session| session.workspace_id == state.workspace_id);
            Ok(json!({ "ok": true, "sessions": sessions }))
        }
        "codex_send_input" => {
            let session_id = required_string(args, "session_id")?;
            ensure_session_workspace(state, &session_id)?;
            let input = required_string(args, "input")?;
            let session = state
                .sessions
                .send_input(&session_id, &input)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true, "session": session }))
        }
        "codex_cancel_session" => {
            let session_id = required_string(args, "session_id")?;
            ensure_session_workspace(state, &session_id)?;
            let session = state
                .sessions
                .cancel(&session_id)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true, "session": session }))
        }
        "codex_session_events" => {
            let session_id = required_string(args, "session_id")?;
            ensure_session_workspace(state, &session_id)?;
            let cursor = args.get("cursor").and_then(Value::as_u64).unwrap_or(0);
            let limit = args
                .get("limit")
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok());
            let page = state
                .sessions
                .event_page(&session_id, cursor, limit)
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true, "page": page }))
        }
        "codex_pending_requests" => {
            let session_id = required_string(args, "session_id")?;
            ensure_session_workspace(state, &session_id)?;
            let requests = state
                .sessions
                .pending_requests(&session_id)
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true, "requests": requests }))
        }
        "codex_respond_request" => {
            let session_id = required_string(args, "session_id")?;
            ensure_session_workspace(state, &session_id)?;
            let request_id = required_string(args, "request_id")?;
            let action = required_string(args, "action")?;
            state
                .sessions
                .respond_to_request(&session_id, &request_id, &action)
                .await
                .map_err(|error| error.to_string())?;
            Ok(json!({ "ok": true }))
        }
        _ => Err(format!("unknown Codex control tool: {name}")),
    }
}

fn required_string(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("missing required field: {key}"))
}

fn ensure_session_workspace(state: &ListenerState, session_id: &str) -> Result<(), String> {
    let session = state
        .sessions
        .get_session(session_id)
        .map_err(|error| error.to_string())?;
    if session.workspace_id != state.workspace_id {
        return Err(format!(
            "session {session_id} belongs to a different workspace"
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_listener(
    port: u16,
    workspace_path: PathBuf,
    workspace_id: String,
    auth: AuthConfig,
    public_base_url: String,
    oauth_client_secret: Option<String>,
    oauth_token_secret: Option<String>,
    runtime: RuntimeConfig,
    permission_ceiling: String,
    activity: ActivityCoordinator,
    sessions: SessionCoordinator,
) -> Result<(ShutdownSender, tauri::async_runtime::JoinHandle<()>), String> {
    let workspace_display = workspace_path.display().to_string();
    let workspace = Workspace::new(workspace_path).map_err(|e| e.message())?;
    let policy =
        PolicySettings::from_runtime(&runtime).with_permission_ceiling(&permission_ceiling);
    let mcp = new_state(
        workspace,
        workspace_id.clone(),
        activity,
        auth.clone(),
        policy,
        runtime.tool_profile.clone(),
        runtime.permission_mode.clone(),
    );
    let bearer_token = if auth.bearer_enabled() {
        SecretStore::get_shared("bearer_token").map_err(|e| e.to_string())?
    } else {
        None
    };
    let configured_public_url = public_base_url.trim().to_string();
    let oauth = if auth.oauth_enabled() {
        let token_secret = oauth_token_secret.unwrap_or_default();
        let oauth_base = external_base_url(&HeaderMap::new(), port, &configured_public_url);
        Some(Arc::new(OAuthRuntime::new(
            oauth_base,
            auth.oauth_client_id.clone(),
            oauth_client_secret.clone(),
            token_secret,
        )))
    } else {
        None
    };
    let state = ListenerState {
        mcp,
        sessions,
        auth,
        workspace_id,
        workspace_path: workspace_display,
        bind_port: port,
        configured_public_url,
        bearer_token,
        oauth,
        oauth_client_secret,
    };
    // 在返回 Running 之前完成 bind，避免后台任务里的端口冲突被伪装成启动成功。
    let listener = bind_listener(port)?;
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let profile_id = state.workspace_id.clone();
    let handle = tauri::async_runtime::spawn(async move {
        let result = serve(listener, port, state, shutdown_rx).await;
        if let Err(err) = &result {
            append_profile_log(
                &profile_id,
                "stderr.log",
                &format!("[mcp] listener stopped: {err}"),
            );
            eprintln!("mcp listener stopped: {err}");
        } else {
            append_profile_log(&profile_id, "stderr.log", "[mcp] listener stopped");
        }
    });
    Ok((shutdown_tx, handle))
}

async fn serve(
    listener: tokio::net::TcpListener,
    port: u16,
    state: ListenerState,
    shutdown: oneshot::Receiver<()>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let profile_id = state.workspace_id.clone();
    let app = Router::new()
        .route("/mcp", get(mcp_discovery).post(mcp_post))
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_authorization_server_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource",
            get(oauth_protected_resource_metadata),
        )
        .route("/oauth/authorize", get(oauth_authorize_get))
        .route("/oauth/token", post(oauth_token_post))
        .with_state(state)
        .layer(CorsLayer::permissive());

    append_profile_log(
        &profile_id,
        "stdout.log",
        &format!("[mcp] listening on http://127.0.0.1:{port}/mcp"),
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await?;
    Ok(())
}

fn bind_listener(port: u16) -> Result<tokio::net::TcpListener, String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let listener = std::net::TcpListener::bind(addr)
        .map_err(|err| format!("MCP 本地端口 {port} 绑定失败: {err}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|err| format!("MCP 本地端口 {port} 设置非阻塞失败: {err}"))?;
    tokio::net::TcpListener::from_std(listener)
        .map_err(|err| format!("MCP 本地监听器初始化失败: {err}"))
}

async fn mcp_discovery() -> Response {
    ([(CACHE_CONTROL, "no-store")], Json(mcp_discovery_payload())).into_response()
}

fn mcp_discovery_payload() -> Value {
    json!({
        "name": "mnelyra",
        "version": env!("CARGO_PKG_VERSION"),
        "protocolVersion": "2025-06-18"
    })
}

fn resolve_oauth_base(state: &ListenerState, headers: &HeaderMap) -> String {
    external_base_url(headers, state.bind_port, &state.configured_public_url)
}

async fn mcp_post(
    State(state): State<ListenerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if let Some(response) = require_mcp_auth(&state, &headers) {
        return response;
    }
    let method = body
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let request_id = body.get("id").cloned().unwrap_or(Value::Null);
    let tool_name = body
        .get("params")
        .and_then(|params| params.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    append_profile_log(
        &state.workspace_id,
        "mcp-requests.log",
        &format!(
            "[rpc] request id={} method={} tool={}",
            request_id, method, tool_name
        ),
    );

    let _activity_guard = if method == "tools/call" {
        match state
            .mcp
            .acquire_activity(crate::activity::ActivityKind::McpRequest)
        {
            Ok(guard) => guard,
            Err(error) => {
                return Json(json!({
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {
                        "code": -32002,
                        "message": error.message(),
                        "data": {
                            "reason": "workspace_draining",
                            "retryable": true
                        }
                    }
                }))
                .into_response();
            }
        }
    } else {
        None
    };

    let mcp = state.mcp.clone();
    let profile_id = state.workspace_id.clone();
    let result = if method == "tools/call" && is_codex_control_tool(&tool_name) {
        Ok(handle_codex_tool(&state, &body).await)
    } else {
        tokio::task::spawn_blocking(move || handle_request(&mcp, &body)).await
    };
    match result {
        Ok(response) => {
            append_profile_log(
                &profile_id,
                "mcp-requests.log",
                &format!(
                    "[rpc] completed id={} method={} tool={}",
                    request_id, method, tool_name
                ),
            );
            if tool_name == "exec_command" || tool_name == "exec_health_check" {
                let structured = response
                    .get("result")
                    .and_then(|result| result.get("structuredContent"));
                let status = structured
                    .and_then(|value| value.get("status"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let termination_reason = structured
                    .and_then(|value| value.get("termination_reason"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let exit_code = structured
                    .and_then(|value| value.get("exit_code"))
                    .map(Value::to_string)
                    .unwrap_or_default();
                let is_error = response
                    .get("result")
                    .and_then(|result| result.get("isError"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                append_profile_log(
                    &profile_id,
                    "mcp-requests.log",
                    &format!(
                        "[exec] id={} tool={} is_error={} status={} termination_reason={} exit_code={}",
                        request_id, tool_name, is_error, status, termination_reason, exit_code
                    ),
                );
            }
            Json(response).into_response()
        }
        Err(error) => {
            append_profile_log(
                &profile_id,
                "mcp-requests.log",
                &format!(
                    "[rpc] worker_failed id={} method={} tool={} error={error}",
                    request_id, method, tool_name
                ),
            );
            Json(json!({
                "jsonrpc": "2.0",
                "id": request_id,
                "error": {
                    "code": -32603,
                    "message": "Exec RPC worker failed",
                    "data": {
                        "stage": "rpc_worker",
                        "reason": "worker_failed",
                        "retryable": true,
                        "suggestion": "重试请求或重启 MCP 运行时"
                    }
                }
            }))
            .into_response()
        }
    }
}

fn require_mcp_auth(state: &ListenerState, headers: &HeaderMap) -> Option<Response> {
    if tunnel_client_authorized(headers) {
        return None;
    }
    if state.auth.bearer_enabled() {
        let expected = state.bearer_token.as_deref().unwrap_or("");
        return verify_bearer_header(headers, expected);
    }
    if state.auth.oauth_enabled() {
        if let Some(oauth) = state.oauth.as_ref() {
            let server_url = resolve_oauth_base(state, headers);
            return verify_oauth_bearer_header(headers, oauth, &server_url);
        }
    }
    None
}

fn tunnel_client_authorized(headers: &HeaderMap) -> bool {
    let Some(provided) = headers
        .get(TUNNEL_TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Ok(Some(expected)) = SecretStore::get_app(TUNNEL_SECRET_SCOPE, TUNNEL_MCP_SECRET_KEY)
    else {
        return false;
    };
    constant_time_eq(provided.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (&a, &b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

async fn oauth_authorization_server_metadata(
    State(state): State<ListenerState>,
    headers: HeaderMap,
) -> Response {
    if tunnel_client_authorized(&headers) {
        return oauth_not_configured();
    }
    if !state.auth.oauth_enabled() {
        return oauth_not_configured();
    }
    let base = resolve_oauth_base(&state, &headers);
    Json(authorization_server_metadata(
        &base,
        state.oauth_client_secret.as_deref(),
    ))
    .into_response()
}

async fn oauth_protected_resource_metadata(
    State(state): State<ListenerState>,
    headers: HeaderMap,
) -> Response {
    if tunnel_client_authorized(&headers) {
        return oauth_not_configured();
    }
    if !state.auth.oauth_enabled() {
        return oauth_not_configured();
    }
    Json(protected_resource_metadata(&resolve_oauth_base(
        &state, &headers,
    )))
    .into_response()
}

async fn oauth_authorize_get(
    State(state): State<ListenerState>,
    headers: HeaderMap,
    Query(params): Query<AuthorizeParams>,
) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return oauth_not_configured();
    };
    authorize_get(oauth, params, &resolve_oauth_base(&state, &headers))
}

async fn oauth_token_post(
    State(state): State<ListenerState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    let Some(oauth) = state.oauth.as_ref() else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "unsupported_grant_type" })),
        )
            .into_response();
    };
    token_exchange(oauth, &headers, form, &resolve_oauth_base(&state, &headers))
}

fn oauth_not_configured() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": "OAuth not configured" })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use axum::http::header::CACHE_CONTROL;
    use axum::response::IntoResponse;

    use super::{bind_listener, mcp_discovery, mcp_discovery_payload};

    #[test]
    fn bind_listener_reports_port_conflict_synchronously() {
        let occupied = std::net::TcpListener::bind(("127.0.0.1", 0)).expect("占用测试端口");
        let port = occupied.local_addr().expect("读取测试端口").port();

        assert!(bind_listener(port).is_err());
    }

    #[tokio::test]
    async fn discovery_reports_the_current_package_version() {
        let discovery = mcp_discovery_payload();

        assert_eq!(discovery["version"], env!("CARGO_PKG_VERSION"));
    }

    #[tokio::test]
    async fn discovery_prevents_stale_tool_catalog_caching() {
        let response = mcp_discovery().await.into_response();

        assert_eq!(response.headers()[CACHE_CONTROL], "no-store");
    }
}
