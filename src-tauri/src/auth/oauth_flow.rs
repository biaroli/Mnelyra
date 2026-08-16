use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{
    header::{AUTHORIZATION, WWW_AUTHENTICATE},
    HeaderMap, HeaderValue, StatusCode,
};
use axum::response::{Html, IntoResponse, Redirect, Response};
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::bearer::constant_time_eq_str;

pub const OAUTH_CODE_TTL_SECONDS: u64 = 300;
pub const OAUTH_TOKEN_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;
pub const OAUTH_REFRESH_TOKEN_TTL_SECONDS: i64 = 60 * 60 * 24 * 180;
#[allow(dead_code)]
pub const OAUTH_MAX_BODY_BYTES: usize = 8_192;

#[derive(Clone)]
pub struct OAuthRuntime {
    pub client_id: String,
    pub approval_code: String,
    pub token_secret: String,
    pending: Arc<Mutex<HashMap<String, PendingCode>>>,
}

#[derive(Clone)]
#[allow(dead_code)]
struct PendingCode {
    code_challenge: String,
    client_id: String,
    redirect_uri: String,
    state: String,
    expires_at: u64,
    server_url: String,
    scope: String,
    resource: String,
}

fn default_access_token_kind() -> String {
    "access".into()
}

#[derive(Serialize, Deserialize)]
struct TokenClaims {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    scope: String,
    #[serde(default = "default_access_token_kind")]
    token_kind: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct RegisteredClientClaims {
    kind: String,
    client_name: String,
    redirect_uris: Vec<String>,
    iat: i64,
    exp: i64,
}

#[derive(Debug, Deserialize)]
pub struct RegistrationRequest {
    #[serde(default)]
    pub client_name: String,
    pub redirect_uris: Vec<String>,
    #[serde(default)]
    pub grant_types: Vec<String>,
    #[serde(default)]
    pub response_types: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_method: String,
}

impl OAuthRuntime {
    pub fn new(
        _base_url: String,
        client_id: String,
        approval_code: String,
        token_secret: String,
    ) -> Self {
        Self {
            client_id,
            approval_code,
            token_secret,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn client_id_allowed(&self, client_id: &str) -> bool {
        if client_id.is_empty() {
            return false;
        }
        constant_time_eq_str(client_id, &self.client_id)
            || self.decode_registered_client(client_id).is_some()
    }

    pub fn redirect_uri_allowed(&self, client_id: &str, redirect_uri: &str) -> bool {
        if !safe_redirect_uri(redirect_uri) {
            return false;
        }
        if constant_time_eq_str(client_id, &self.client_id) {
            return true;
        }
        self.decode_registered_client(client_id)
            .map(|claims| {
                claims
                    .redirect_uris
                    .iter()
                    .any(|value| value == redirect_uri)
            })
            .unwrap_or(false)
    }

    pub fn client_display_name(&self, client_id: &str) -> String {
        self.decode_registered_client(client_id)
            .map(|claims| claims.client_name)
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| client_id.to_string())
    }

    fn decode_registered_client(&self, client_id: &str) -> Option<RegisteredClientClaims> {
        let encoded = client_id.strip_prefix("mnelyra-dcr-")?;
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_aud = false;
        decode::<RegisteredClientClaims>(
            encoded,
            &DecodingKey::from_secret(self.token_secret.as_bytes()),
            &validation,
        )
        .ok()
        .map(|data| data.claims)
        .filter(|claims| claims.kind == "oauth-client")
    }

    pub fn verify_access_token(&self, token: &str, server_url: &str) -> bool {
        self.verify_token_kind(token, server_url, "access")
    }

    pub fn verify_refresh_token(&self, token: &str, server_url: &str) -> bool {
        self.verify_token_kind(token, server_url, "refresh")
    }

    fn verify_token_kind(&self, token: &str, server_url: &str, expected_kind: &str) -> bool {
        let server_url = server_url.trim_end_matches('/');
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[server_url]);
        validation.set_issuer(&[server_url]);
        decode::<TokenClaims>(
            token,
            &DecodingKey::from_secret(self.token_secret.as_bytes()),
            &validation,
        )
        .map(|data| data.claims.token_kind == expected_kind)
        .unwrap_or(false)
    }
}

fn oauth_unauthorized(message: &'static str, server_url: &str) -> Response {
    let mut response = (StatusCode::UNAUTHORIZED, message).into_response();
    let resource_metadata = format!(
        "{}/.well-known/oauth-protected-resource",
        server_url.trim_end_matches('/')
    );
    let challenge = format!("Bearer resource_metadata=\"{resource_metadata}\", scope=\"mcp\"");
    if let Ok(value) = HeaderValue::from_str(&challenge) {
        response.headers_mut().insert(WWW_AUTHENTICATE, value);
    }
    response
}

pub fn verify_oauth_bearer_header(
    headers: &HeaderMap,
    oauth: &OAuthRuntime,
    server_url: &str,
) -> Option<Response> {
    let Some(header_value) = headers.get(AUTHORIZATION) else {
        return Some(oauth_unauthorized(
            "Missing Authorization header",
            server_url,
        ));
    };
    let Ok(header_str) = header_value.to_str() else {
        return Some(oauth_unauthorized(
            "Invalid Authorization header",
            server_url,
        ));
    };
    let Some(token) = header_str.strip_prefix("Bearer ").map(str::trim) else {
        return Some(oauth_unauthorized("Invalid bearer token", server_url));
    };
    if oauth.verify_access_token(token, server_url) {
        None
    } else {
        Some(oauth_unauthorized("Invalid bearer token", server_url))
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeParams {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub resource: String,
}

pub fn register_client(oauth: &OAuthRuntime, request: RegistrationRequest) -> Response {
    if request.redirect_uris.is_empty()
        || request
            .redirect_uris
            .iter()
            .any(|redirect| !safe_redirect_uri(redirect))
    {
        return registration_error(
            "invalid_redirect_uri",
            "redirect_uris must use HTTPS or loopback HTTP",
        );
    }
    if !request.token_endpoint_auth_method.is_empty()
        && request.token_endpoint_auth_method != "none"
    {
        return registration_error(
            "invalid_client_metadata",
            "Mnelyra supports public PKCE clients only",
        );
    }
    if !request.grant_types.is_empty()
        && !request
            .grant_types
            .iter()
            .any(|value| value == "authorization_code")
    {
        return registration_error(
            "invalid_client_metadata",
            "authorization_code grant is required",
        );
    }
    if !request.response_types.is_empty()
        && !request.response_types.iter().any(|value| value == "code")
    {
        return registration_error("invalid_client_metadata", "code response type is required");
    }

    let now = unix_now() as i64;
    let claims = RegisteredClientClaims {
        kind: "oauth-client".into(),
        client_name: if request.client_name.trim().is_empty() {
            "MCP client".into()
        } else {
            request.client_name.trim().chars().take(120).collect()
        },
        redirect_uris: request.redirect_uris.clone(),
        iat: now,
        exp: now + 60 * 60 * 24 * 3650,
    };
    let encoded = match encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(oauth.token_secret.as_bytes()),
    ) {
        Ok(value) => value,
        Err(_) => return registration_error("server_error", "Failed to register OAuth client"),
    };
    let client_id = format!("mnelyra-dcr-{encoded}");
    (
        StatusCode::CREATED,
        axum::Json(json!({
            "client_id": client_id,
            "client_name": claims.client_name,
            "redirect_uris": claims.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
            "client_id_issued_at": now
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeForm {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub scope: String,
    #[serde(default)]
    pub resource: String,
    pub approval_code: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct TokenForm {
    pub grant_type: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub redirect_uri: String,
    #[serde(default)]
    pub code_verifier: String,
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub refresh_token: String,
    #[serde(default)]
    pub resource: String,
}

pub fn authorize_get(
    oauth: &OAuthRuntime,
    params: AuthorizeParams,
    server_url: &str,
    workspace_label: Option<&str>,
) -> Response {
    if params.response_type != "code" {
        return html_error("response_type must be 'code'", StatusCode::BAD_REQUEST);
    }
    if !oauth.client_id_allowed(&params.client_id) {
        return html_error("Unknown client_id", StatusCode::BAD_REQUEST);
    }
    if !oauth.redirect_uri_allowed(&params.client_id, &params.redirect_uri) {
        return html_error("Invalid redirect_uri", StatusCode::BAD_REQUEST);
    }
    if params.code_challenge_method != "S256" || params.code_challenge.is_empty() {
        return html_error(
            "code_challenge_method must be S256 and code_challenge is required",
            StatusCode::BAD_REQUEST,
        );
    }
    let resource = match normalize_resource(&params.resource, server_url) {
        Some(value) => value,
        None => return html_error("Invalid resource", StatusCode::BAD_REQUEST),
    };
    let client_name = oauth.client_display_name(&params.client_id);
    Html(authorization_page(
        &params.client_id,
        &client_name,
        &params.redirect_uri,
        &params.code_challenge,
        &params.code_challenge_method,
        &params.state,
        &normalize_scope(&params.scope),
        &resource,
        "",
        workspace_label,
    ))
    .into_response()
}

pub fn authorize_post(
    oauth: &OAuthRuntime,
    form: AuthorizeForm,
    server_url: &str,
    workspace_label: Option<&str>,
) -> Response {
    if !oauth.client_id_allowed(&form.client_id) {
        return authorization_error_page(
            oauth,
            &form,
            "无法识别此客户端。",
            workspace_label,
            StatusCode::BAD_REQUEST,
        );
    }
    if !oauth.redirect_uri_allowed(&form.client_id, &form.redirect_uri) {
        return authorization_error_page(
            oauth,
            &form,
            "回调地址与客户端注册信息不匹配。",
            workspace_label,
            StatusCode::BAD_REQUEST,
        );
    }
    if form.code_challenge_method != "S256" || form.code_challenge.is_empty() {
        return authorization_error_page(
            oauth,
            &form,
            "授权请求中的 PKCE 参数无效。",
            workspace_label,
            StatusCode::BAD_REQUEST,
        );
    }
    if !constant_time_eq_str(form.approval_code.trim(), oauth.approval_code.trim()) {
        return authorization_error_page(
            oauth,
            &form,
            "授权码不正确。",
            workspace_label,
            StatusCode::UNAUTHORIZED,
        );
    }

    let server_url = server_url.trim_end_matches('/').to_string();
    let resource = match normalize_resource(&form.resource, &server_url) {
        Some(value) => value,
        None => {
            return authorization_error_page(
                oauth,
                &form,
                "授权资源与当前 Mnelyra 地址不匹配。",
                workspace_label,
                StatusCode::BAD_REQUEST,
            )
        }
    };
    let code = uuid::Uuid::new_v4().to_string().replace('-', "");
    let now = unix_now();
    {
        let mut pending = oauth.pending.lock().expect("oauth pending lock");
        pending.retain(|_, v| v.expires_at >= now);
        pending.insert(
            code.clone(),
            PendingCode {
                code_challenge: form.code_challenge.clone(),
                client_id: form.client_id.clone(),
                redirect_uri: form.redirect_uri.clone(),
                state: form.state.clone(),
                expires_at: now + OAUTH_CODE_TTL_SECONDS,
                server_url: server_url.clone(),
                scope: normalize_scope(&form.scope),
                resource,
            },
        );
    }

    let mut qs = format!("code={}", urlencoding_encode(&code));
    if !form.state.is_empty() {
        qs.push_str(&format!("&state={}", urlencoding_encode(&form.state)));
    }
    qs.push_str(&format!("&iss={}", urlencoding_encode(&server_url)));
    let sep = if form.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    Redirect::to(&format!("{}{}{}", form.redirect_uri, sep, qs)).into_response()
}

pub fn token_exchange(
    oauth: &OAuthRuntime,
    _headers: &HeaderMap,
    form: TokenForm,
    server_url: &str,
) -> Response {
    if !oauth.client_id_allowed(&form.client_id) {
        return token_error("invalid_client", "Unknown client_id");
    }
    let expected_resource = canonical_resource(server_url);
    let token_resource = match normalize_resource(&form.resource, server_url) {
        Some(value) => value,
        None => return token_error("invalid_target", "Invalid resource"),
    };
    if form.grant_type == "refresh_token" {
        if form.refresh_token.is_empty() {
            return token_error("invalid_grant", "refresh_token is required");
        }
        if !oauth.verify_refresh_token(&form.refresh_token, server_url) {
            return token_error("invalid_grant", "Invalid refresh token");
        }
        return issue_token_pair(
            oauth,
            &expected_resource,
            &token_resource,
            "mcp offline_access",
        );
    }
    if form.grant_type != "authorization_code" {
        return token_error(
            "unsupported_grant_type",
            "Only authorization_code and refresh_token are supported",
        );
    }
    if form.code.is_empty() {
        return token_error("invalid_grant", "code is required");
    }
    if !valid_code_verifier(&form.code_verifier) {
        return token_error("invalid_grant", "Invalid code_verifier");
    }

    let code_data = {
        let mut pending = oauth.pending.lock().expect("oauth pending lock");
        pending.remove(&form.code)
    };
    let Some(code_data) = code_data else {
        return token_error(
            "invalid_grant",
            "Unknown or already-used authorization code",
        );
    };
    if unix_now() > code_data.expires_at {
        return token_error("invalid_grant", "Authorization code expired");
    }
    if !constant_time_eq_str(&code_data.client_id, &form.client_id) {
        return token_error("invalid_grant", "client_id mismatch");
    }
    if !constant_time_eq_str(&code_data.redirect_uri, &form.redirect_uri) {
        return token_error("invalid_grant", "redirect_uri mismatch");
    }
    if !constant_time_eq_str(&code_data.resource, &token_resource) {
        return token_error("invalid_target", "resource mismatch");
    }
    if !verify_pkce(&form.code_verifier, &code_data.code_challenge) {
        return token_error("invalid_grant", "PKCE verification failed");
    }

    let issuer = if code_data.server_url.trim().is_empty() {
        server_url.trim_end_matches('/').to_string()
    } else {
        code_data.server_url.trim_end_matches('/').to_string()
    };
    issue_token_pair(oauth, &issuer, &code_data.resource, &code_data.scope)
}

fn normalize_scope(scope: &str) -> String {
    let mut scopes = scope
        .split_whitespace()
        .filter(|value| matches!(*value, "mcp" | "offline_access"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if !scopes.iter().any(|value| value == "mcp") {
        scopes.insert(0, "mcp".into());
    }
    if !scopes.iter().any(|value| value == "offline_access") {
        scopes.push("offline_access".into());
    }
    scopes.sort();
    scopes.dedup();
    scopes.join(" ")
}

fn canonical_resource(server_url: &str) -> String {
    server_url.trim().trim_end_matches('/').to_string()
}

fn normalize_resource(resource: &str, server_url: &str) -> Option<String> {
    let expected = canonical_resource(server_url);
    let provided = if resource.trim().is_empty() {
        expected.clone()
    } else {
        resource.trim().trim_end_matches('/').to_string()
    };
    if provided == expected {
        Some(expected)
    } else {
        None
    }
}

fn safe_redirect_uri(value: &str) -> bool {
    let value = value.trim();
    if value.is_empty() || value.contains('#') || value.chars().any(|ch| matches!(ch, '\r' | '\n'))
    {
        return false;
    }
    if value.starts_with("https://") {
        return value.len() > "https://".len();
    }
    ["http://localhost", "http://127.0.0.1", "http://[::1]"]
        .iter()
        .any(|base| {
            value == *base
                || value
                    .strip_prefix(base)
                    .is_some_and(|rest| rest.starts_with('/') || rest.starts_with(':'))
        })
}

fn registration_error(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({
            "error": error,
            "error_description": description
        })),
    )
        .into_response()
}

fn issue_token_pair(oauth: &OAuthRuntime, issuer: &str, resource: &str, scope: &str) -> Response {
    let scope = normalize_scope(scope);
    let access = create_token(
        issuer,
        resource,
        &oauth.token_secret,
        OAUTH_TOKEN_TTL_SECONDS,
        &scope,
        "access",
    );
    let refresh = create_token(
        issuer,
        resource,
        &oauth.token_secret,
        OAUTH_REFRESH_TOKEN_TTL_SECONDS,
        &scope,
        "refresh",
    );
    match (access, refresh) {
        (Ok(access_token), Ok(refresh_token)) => (
            StatusCode::OK,
            axum::Json(json!({
                "access_token": access_token,
                "refresh_token": refresh_token,
                "token_type": "Bearer",
                "expires_in": OAUTH_TOKEN_TTL_SECONDS,
                "scope": scope
            })),
        )
            .into_response(),
        _ => token_error("server_error", "Failed to issue OAuth token"),
    }
}

fn create_token(
    issuer: &str,
    resource: &str,
    token_secret: &str,
    ttl: i64,
    scope: &str,
    token_kind: &str,
) -> Result<String, ()> {
    let now = unix_now() as i64;
    let claims = TokenClaims {
        iss: issuer.to_string(),
        aud: resource.to_string(),
        iat: now,
        exp: now + ttl,
        scope: scope.into(),
        token_kind: token_kind.into(),
    };
    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(token_secret.as_bytes()),
    )
    .map_err(|_| ())
}

fn verify_pkce(code_verifier: &str, code_challenge: &str) -> bool {
    let digest = Sha256::digest(code_verifier.as_bytes());
    let expected = URL_SAFE_NO_PAD.encode(digest);
    constant_time_eq_str(&expected, code_challenge)
}

fn valid_code_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '.' | '_' | '~'))
}

fn token_error(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(json!({
            "error": error,
            "error_description": description
        })),
    )
        .into_response()
}

fn html_error(message: &str, status: StatusCode) -> Response {
    (status, Html(format!("<h2>Error</h2><p>{message}</p>"))).into_response()
}

fn authorization_error_page(
    oauth: &OAuthRuntime,
    form: &AuthorizeForm,
    message: &str,
    workspace_label: Option<&str>,
    status: StatusCode,
) -> Response {
    (
        status,
        Html(authorization_page(
            &form.client_id,
            &oauth.client_display_name(&form.client_id),
            &form.redirect_uri,
            &form.code_challenge,
            &form.code_challenge_method,
            &form.state,
            &normalize_scope(&form.scope),
            &form.resource,
            message,
            workspace_label,
        )),
    )
        .into_response()
}

#[allow(clippy::too_many_arguments)]
fn authorization_page(
    client_id: &str,
    client_name: &str,
    redirect_uri: &str,
    code_challenge: &str,
    code_challenge_method: &str,
    state: &str,
    scope: &str,
    resource: &str,
    error: &str,
    workspace_label: Option<&str>,
) -> String {
    let icon = STANDARD.encode(include_bytes!("../../icons/128x128.png"));
    let workspace = workspace_label
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            format!(
                "<div class='meta-row'><span>当前工作区</span><strong>{}</strong></div>",
                html_escape(value)
            )
        })
        .unwrap_or_default();
    let error_block = if error.is_empty() {
        String::new()
    } else {
        format!(
            "<div class='error' role='alert'><span>!</span><p>{}</p></div>",
            html_escape(error)
        )
    };

    format!(
        "<!doctype html><html lang='zh-CN'><head><meta charset='utf-8'>\
        <meta name='viewport' content='width=device-width,initial-scale=1'>\
        <meta name='color-scheme' content='dark'>\
        <title>授权 Mnelyra</title>\
        <style>\
        *{{box-sizing:border-box}}\
        html,body{{min-height:100%;margin:0}}\
        body{{font-family:Inter,-apple-system,BlinkMacSystemFont,'Segoe UI','Microsoft YaHei',sans-serif;background:#070b12;color:#f5f7fb;display:grid;place-items:center;padding:28px;overflow-x:hidden}}\
        body:before{{content:'';position:fixed;inset:0;pointer-events:none;background:radial-gradient(circle at 50% 18%,rgba(50,132,255,.14),transparent 38%),linear-gradient(rgba(255,255,255,.018) 1px,transparent 1px),linear-gradient(90deg,rgba(255,255,255,.018) 1px,transparent 1px);background-size:auto,32px 32px,32px 32px}}\
        .shell{{position:relative;width:min(100%,480px)}}\
        .card{{background:linear-gradient(180deg,rgba(19,27,40,.98),rgba(11,17,27,.98));border:1px solid rgba(125,168,225,.2);border-radius:22px;padding:28px;box-shadow:0 28px 80px rgba(0,0,0,.48),0 0 0 1px rgba(255,255,255,.015) inset}}\
        .brand{{display:flex;align-items:center;gap:14px;margin-bottom:24px}}\
        .brand img{{width:48px;height:48px;border-radius:13px;box-shadow:0 8px 28px rgba(50,132,255,.2)}}\
        .brand h1{{font-size:22px;line-height:1.15;margin:0;font-weight:680;letter-spacing:-.02em}}\
        .brand p{{margin:5px 0 0;color:#8ea0b8;font-size:13px}}\
        .intro{{margin:0 0 20px;color:#c9d3e1;font-size:14px;line-height:1.65}}\
        .meta{{border:1px solid rgba(125,168,225,.13);background:rgba(4,9,17,.42);border-radius:14px;padding:4px 14px;margin-bottom:20px}}\
        .meta-row{{display:flex;align-items:center;justify-content:space-between;gap:18px;min-height:42px;border-bottom:1px solid rgba(125,168,225,.09);font-size:12px}}\
        .meta-row:last-child{{border-bottom:0}}\
        .meta-row span{{color:#7f91aa;white-space:nowrap}}\
        .meta-row strong{{font-weight:560;color:#dce5f2;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;text-align:right}}\
        .error{{display:flex;gap:10px;align-items:flex-start;padding:12px 13px;margin-bottom:16px;border:1px solid rgba(255,99,116,.28);background:rgba(255,67,88,.09);border-radius:12px;color:#ff9eaa;font-size:13px}}\
        .error span{{display:grid;place-items:center;width:18px;height:18px;border-radius:50%;background:#ff5366;color:#fff;font-weight:800;font-size:11px;flex:0 0 auto}}\
        .error p{{margin:0;line-height:1.45}}\
        label{{display:block;color:#dce5f2;font-size:13px;font-weight:600;margin-bottom:8px}}\
        input.code{{width:100%;height:48px;border:1px solid rgba(126,166,218,.24);border-radius:12px;background:#080d15;color:#fff;padding:0 14px;font:600 15px ui-monospace,SFMono-Regular,Consolas,monospace;letter-spacing:.045em;outline:none;transition:.18s border-color,.18s box-shadow}}\
        input.code:focus{{border-color:#4b9cff;box-shadow:0 0 0 3px rgba(75,156,255,.13)}}\
        .hint{{margin:8px 0 18px;color:#73869f;font-size:12px;line-height:1.5}}\
        button{{width:100%;height:48px;border:0;border-radius:12px;background:linear-gradient(135deg,#2e8cff,#6377ff);color:white;font-size:14px;font-weight:680;cursor:pointer;box-shadow:0 10px 28px rgba(48,124,255,.22);transition:.16s transform,.16s filter}}\
        button:hover{{filter:brightness(1.06)}} button:active{{transform:translateY(1px)}}\
        .foot{{text-align:center;color:#596b82;font-size:11px;margin:14px 8px 0;line-height:1.5}}\
        @media(max-width:520px){{body{{padding:16px}}.card{{padding:22px;border-radius:18px}}.meta-row{{align-items:flex-start;flex-direction:column;gap:3px;padding:10px 0}}.meta-row strong{{white-space:normal;text-align:left;word-break:break-all}}}}\
        </style></head><body><main class='shell'><section class='card'>\
        <div class='brand'><img src='data:image/png;base64,{icon}' alt='Mnelyra'><div><h1>授权 Mnelyra</h1><p>Authorize MCP connection</p></div></div>\
        <p class='intro'>一个 MCP 客户端正在请求访问 Mnelyra。输入桌面端“认证”页面显示的授权码以继续。</p>\
        <div class='meta'>{workspace}<div class='meta-row'><span>客户端</span><strong>{}</strong></div></div>\
        {error_block}\
        <form method='post' action='/oauth/authorize'>\
        <input type='hidden' name='client_id' value='{}'>\
        <input type='hidden' name='redirect_uri' value='{}'>\
        <input type='hidden' name='code_challenge' value='{}'>\
        <input type='hidden' name='code_challenge_method' value='{}'>\
        <input type='hidden' name='state' value='{}'>\
        <input type='hidden' name='scope' value='{}'>\
        <input type='hidden' name='resource' value='{}'>\
        <label for='approval_code'>授权码</label>\
        <input class='code' id='approval_code' type='password' name='approval_code' autocomplete='one-time-code' spellcheck='false' autofocus required>\
        <p class='hint'>授权码由本机 Mnelyra 生成，可以随时重新生成。</p>\
        <button type='submit'>授权连接</button>\
        </form></section><p class='foot'>仅在你主动连接的客户端中输入此授权码。</p></main></body></html>",
        html_escape(client_name),
        html_escape(client_id),
        html_escape(redirect_uri),
        html_escape(code_challenge),
        html_escape(code_challenge_method),
        html_escape(state),
        html_escape(scope),
        html_escape(resource),
    )
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn urlencoding_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_exchange_without_client_secret() {
        use axum::http::HeaderMap;

        let oauth = OAuthRuntime::new(
            "https://lb.example.com".into(),
            "chatgpt-client-test".into(),
            "APPROVE-TEST".into(),
            "token-signing-secret".into(),
        );
        let verifier = "dBjftJeZ4CVP-mB92Kpru-AEJvkQlLgi3ThpmQ45N_Xyo";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let redirect_uri = "https://chatgpt.com/connector/oauth/test";
        let page = authorize_get(
            &oauth,
            AuthorizeParams {
                response_type: "code".into(),
                client_id: "chatgpt-client-test".into(),
                redirect_uri: redirect_uri.into(),
                code_challenge: challenge,
                code_challenge_method: "S256".into(),
                state: "state".into(),
                scope: "mcp offline_access".into(),
                resource: "https://lb.example.com".into(),
            },
            "https://lb.example.com",
            Some("Example Workspace"),
        );
        assert_eq!(page.status(), StatusCode::OK);
        let redirect = authorize_post(
            &oauth,
            AuthorizeForm {
                client_id: "chatgpt-client-test".into(),
                redirect_uri: redirect_uri.into(),
                code_challenge: URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                code_challenge_method: "S256".into(),
                state: "state".into(),
                scope: "mcp offline_access".into(),
                resource: "https://lb.example.com".into(),
                approval_code: "APPROVE-TEST".into(),
            },
            "https://lb.example.com",
            Some("Example Workspace"),
        );
        assert_eq!(redirect.status(), StatusCode::SEE_OTHER);
        let code = {
            let pending = oauth.pending.lock().expect("lock");
            pending.keys().next().cloned().unwrap()
        };

        let response = token_exchange(
            &oauth,
            &HeaderMap::new(),
            TokenForm {
                grant_type: "authorization_code".into(),
                code,
                redirect_uri: redirect_uri.into(),
                code_verifier: verifier.into(),
                client_id: "chatgpt-client-test".into(),
                refresh_token: String::new(),
                resource: "https://lb.example.com".into(),
            },
            "https://lb.example.com",
        );
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn missing_oauth_bearer_advertises_protected_resource_metadata() {
        let oauth = OAuthRuntime::new(
            "https://lb.example.com".into(),
            "chatgpt-client-test".into(),
            "APPROVE-TEST".into(),
            "token-signing-secret".into(),
        );
        let response =
            verify_oauth_bearer_header(&HeaderMap::new(), &oauth, "https://lb.example.com")
                .expect("missing bearer should be challenged");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response
                .headers()
                .get(WWW_AUTHENTICATE)
                .and_then(|value| value.to_str().ok()),
            Some(
                "Bearer resource_metadata=\"https://lb.example.com/.well-known/oauth-protected-resource\", scope=\"mcp\""
            )
        );
    }

    #[test]
    fn refresh_token_can_issue_a_new_access_token() {
        let oauth = OAuthRuntime::new(
            "https://lb.example.com".into(),
            "chatgpt-client-test".into(),
            "APPROVE-TEST".into(),
            "token-signing-secret".into(),
        );
        let refresh = create_token(
            "https://lb.example.com",
            "https://lb.example.com",
            &oauth.token_secret,
            OAUTH_REFRESH_TOKEN_TTL_SECONDS,
            "mcp offline_access",
            "refresh",
        )
        .expect("refresh token");
        let response = token_exchange(
            &oauth,
            &HeaderMap::new(),
            TokenForm {
                grant_type: "refresh_token".into(),
                client_id: "chatgpt-client-test".into(),
                refresh_token: refresh,
                resource: "https://lb.example.com".into(),
                ..TokenForm::default()
            },
            "https://lb.example.com",
        );
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn pkce_round_trip() {
        let verifier = "dBjftJeZ4CVP-mB92Kpru-AEJvkQlLgi3ThpmQ45N_Xyo";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!(verify_pkce(verifier, &challenge));
    }

    #[test]
    fn wrong_approval_code_does_not_issue_authorization_code() {
        let oauth = OAuthRuntime::new(
            "https://lb.example.com".into(),
            "chatgpt-client-test".into(),
            "RIGHT-CODE".into(),
            "token-signing-secret".into(),
        );
        let response = authorize_post(
            &oauth,
            AuthorizeForm {
                client_id: "chatgpt-client-test".into(),
                redirect_uri: "https://chatgpt.com/connector/oauth/test".into(),
                code_challenge: "challenge".into(),
                code_challenge_method: "S256".into(),
                state: "state".into(),
                scope: "mcp offline_access".into(),
                resource: "https://lb.example.com".into(),
                approval_code: "WRONG-CODE".into(),
            },
            "https://lb.example.com",
            Some("Example Workspace"),
        );
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(oauth.pending.lock().expect("lock").is_empty());
    }

    #[tokio::test]
    async fn dcr_client_survives_registration_and_can_authorize() {
        use axum::body::to_bytes;

        let oauth = OAuthRuntime::new(
            "https://lb.example.com".into(),
            "mnelyra-client-fixed".into(),
            "APPROVE-TEST".into(),
            "token-signing-secret".into(),
        );
        let redirect_uri = "https://chatgpt.com/connector/oauth/test";
        let response = register_client(
            &oauth,
            RegistrationRequest {
                client_name: "ChatGPT".into(),
                redirect_uris: vec![redirect_uri.into()],
                grant_types: vec!["authorization_code".into(), "refresh_token".into()],
                response_types: vec!["code".into()],
                token_endpoint_auth_method: "none".into(),
            },
        );
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("registration body");
        let payload: serde_json::Value = serde_json::from_slice(&body).expect("registration json");
        let client_id = payload["client_id"]
            .as_str()
            .expect("registered client_id")
            .to_string();
        assert!(client_id.starts_with("mnelyra-dcr-"));
        assert!(oauth.client_id_allowed(&client_id));
        assert!(oauth.redirect_uri_allowed(&client_id, redirect_uri));
        assert_eq!(oauth.client_display_name(&client_id), "ChatGPT");

        let verifier = "dBjftJeZ4CVP-mB92Kpru-AEJvkQlLgi3ThpmQ45N_Xyo";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let page = authorize_get(
            &oauth,
            AuthorizeParams {
                response_type: "code".into(),
                client_id: client_id.clone(),
                redirect_uri: redirect_uri.into(),
                code_challenge: challenge.clone(),
                code_challenge_method: "S256".into(),
                state: "state".into(),
                scope: "mcp offline_access".into(),
                resource: "https://lb.example.com".into(),
            },
            "https://lb.example.com",
            Some("Example Workspace"),
        );
        assert_eq!(page.status(), StatusCode::OK);

        let redirect = authorize_post(
            &oauth,
            AuthorizeForm {
                client_id: client_id.clone(),
                redirect_uri: redirect_uri.into(),
                code_challenge: challenge,
                code_challenge_method: "S256".into(),
                state: "state".into(),
                scope: "mcp offline_access".into(),
                resource: "https://lb.example.com".into(),
                approval_code: "APPROVE-TEST".into(),
            },
            "https://lb.example.com",
            Some("Example Workspace"),
        );
        assert_eq!(redirect.status(), StatusCode::SEE_OTHER);
        let code = oauth
            .pending
            .lock()
            .expect("lock")
            .keys()
            .next()
            .cloned()
            .expect("authorization code");
        let token = token_exchange(
            &oauth,
            &HeaderMap::new(),
            TokenForm {
                grant_type: "authorization_code".into(),
                code,
                redirect_uri: redirect_uri.into(),
                code_verifier: verifier.into(),
                client_id,
                refresh_token: String::new(),
                resource: "https://lb.example.com".into(),
            },
            "https://lb.example.com",
        );
        assert_eq!(token.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn http_oauth_flow_matches_chatgpt_mcp_sequence() {
        use axum::extract::{Form, Json, Query};
        use axum::routing::{get, post};
        use axum::Router;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener");
        let addr = listener.local_addr().expect("local addr");
        let base = format!("http://{addr}");
        let oauth = Arc::new(OAuthRuntime::new(
            base.clone(),
            "mnelyra-client-fixed".into(),
            "APPROVE-TEST".into(),
            "token-signing-secret".into(),
        ));

        let mcp_oauth = oauth.clone();
        let mcp_base = base.clone();
        let prm_base = base.clone();
        let metadata_base = base.clone();
        let register_oauth = oauth.clone();
        let authorize_get_oauth = oauth.clone();
        let authorize_get_base = base.clone();
        let authorize_post_oauth = oauth.clone();
        let authorize_post_base = base.clone();
        let token_oauth = oauth.clone();
        let token_base = base.clone();

        let app = Router::new()
            .route(
                "/mcp",
                post(move |headers: HeaderMap| {
                    let oauth = mcp_oauth.clone();
                    let base = mcp_base.clone();
                    async move {
                        verify_oauth_bearer_header(&headers, &oauth, &base)
                            .unwrap_or_else(|| StatusCode::NO_CONTENT.into_response())
                    }
                }),
            )
            .route(
                "/.well-known/oauth-protected-resource",
                get(move || {
                    let base = prm_base.clone();
                    async move { Json(crate::auth::protected_resource_metadata(&base)) }
                }),
            )
            .route(
                "/.well-known/oauth-authorization-server",
                get(move || {
                    let base = metadata_base.clone();
                    async move { Json(crate::auth::authorization_server_metadata(&base)) }
                }),
            )
            .route(
                "/oauth/register",
                post(move |Json(request): Json<RegistrationRequest>| {
                    let oauth = register_oauth.clone();
                    async move { register_client(&oauth, request) }
                }),
            )
            .route(
                "/oauth/authorize",
                get(move |Query(params): Query<AuthorizeParams>| {
                    let oauth = authorize_get_oauth.clone();
                    let base = authorize_get_base.clone();
                    async move { authorize_get(&oauth, params, &base, Some("Example Workspace")) }
                })
                .post(move |Form(form): Form<AuthorizeForm>| {
                    let oauth = authorize_post_oauth.clone();
                    let base = authorize_post_base.clone();
                    async move { authorize_post(&oauth, form, &base, Some("Example Workspace")) }
                }),
            )
            .route(
                "/oauth/token",
                post(move |headers: HeaderMap, Form(form): Form<TokenForm>| {
                    let oauth = token_oauth.clone();
                    let base = token_base.clone();
                    async move { token_exchange(&oauth, &headers, form, &base) }
                }),
            );

        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.expect("test OAuth server");
        });
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("test HTTP client");

        let challenge = client
            .post(format!("{base}/mcp"))
            .send()
            .await
            .expect("anonymous mcp request");
        assert_eq!(challenge.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert!(challenge
            .headers()
            .get("www-authenticate")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("oauth-protected-resource")));

        let metadata: serde_json::Value = client
            .get(format!("{base}/.well-known/oauth-authorization-server"))
            .send()
            .await
            .expect("authorization metadata")
            .json()
            .await
            .expect("authorization metadata json");
        assert_eq!(
            metadata["registration_endpoint"],
            format!("{base}/oauth/register")
        );
        assert_eq!(
            metadata["token_endpoint_auth_methods_supported"],
            json!(["none"])
        );

        let redirect_uri = "https://chatgpt.com/connector/oauth/test";
        let registration: serde_json::Value = client
            .post(format!("{base}/oauth/register"))
            .json(&json!({
                "client_name": "ChatGPT",
                "redirect_uris": [redirect_uri],
                "grant_types": ["authorization_code", "refresh_token"],
                "response_types": ["code"],
                "token_endpoint_auth_method": "none"
            }))
            .send()
            .await
            .expect("dcr registration")
            .json()
            .await
            .expect("dcr response json");
        let client_id = registration["client_id"]
            .as_str()
            .expect("registered client id")
            .to_string();

        let verifier = "dBjftJeZ4CVP-mB92Kpru-AEJvkQlLgi3ThpmQ45N_Xyo";
        let code_challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let authorize_url = format!("{base}/oauth/authorize");
        let page = client
            .get(&authorize_url)
            .query(&[
                ("response_type", "code"),
                ("client_id", client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("code_challenge", code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", "state-1"),
                ("scope", "mcp offline_access"),
                ("resource", base.as_str()),
            ])
            .send()
            .await
            .expect("authorization page");
        assert_eq!(page.status(), reqwest::StatusCode::OK);
        let page_html = page.text().await.expect("authorization page html");
        assert!(page_html.contains("授权 Mnelyra"));
        assert!(page_html.contains("授权码"));
        assert!(page_html.contains("ChatGPT"));

        let wrong = client
            .post(&authorize_url)
            .form(&[
                ("client_id", client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("code_challenge", code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", "state-1"),
                ("scope", "mcp offline_access"),
                ("resource", base.as_str()),
                ("approval_code", "WRONG-CODE"),
            ])
            .send()
            .await
            .expect("wrong authorization code");
        assert_eq!(wrong.status(), reqwest::StatusCode::UNAUTHORIZED);

        let approved = client
            .post(&authorize_url)
            .form(&[
                ("client_id", client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("code_challenge", code_challenge.as_str()),
                ("code_challenge_method", "S256"),
                ("state", "state-1"),
                ("scope", "mcp offline_access"),
                ("resource", base.as_str()),
                ("approval_code", "APPROVE-TEST"),
            ])
            .send()
            .await
            .expect("approved authorization");
        assert_eq!(approved.status(), reqwest::StatusCode::SEE_OTHER);
        let location = approved
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok())
            .expect("authorization redirect");
        assert!(location.starts_with(redirect_uri));
        let code = location
            .split("code=")
            .nth(1)
            .and_then(|value| value.split('&').next())
            .expect("authorization code");

        let token: serde_json::Value = client
            .post(format!("{base}/oauth/token"))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
                ("code_verifier", verifier),
                ("client_id", client_id.as_str()),
                ("resource", base.as_str()),
            ])
            .send()
            .await
            .expect("token exchange")
            .json()
            .await
            .expect("token response json");
        let access_token = token["access_token"].as_str().expect("access token");
        let refresh_token = token["refresh_token"].as_str().expect("refresh token");

        let authenticated = client
            .post(format!("{base}/mcp"))
            .bearer_auth(access_token)
            .send()
            .await
            .expect("authenticated mcp request");
        assert_eq!(authenticated.status(), reqwest::StatusCode::NO_CONTENT);

        let refreshed = client
            .post(format!("{base}/oauth/token"))
            .form(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
                ("client_id", client_id.as_str()),
                ("resource", base.as_str()),
            ])
            .send()
            .await
            .expect("refresh token exchange");
        assert_eq!(refreshed.status(), reqwest::StatusCode::OK);

        server.abort();
    }
}
