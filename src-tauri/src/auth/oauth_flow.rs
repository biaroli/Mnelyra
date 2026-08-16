use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::{header::AUTHORIZATION, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::bearer::constant_time_eq_str;

pub const OAUTH_CODE_TTL_SECONDS: u64 = 300;
pub const OAUTH_TOKEN_TTL_SECONDS: i64 = 60 * 60 * 24 * 30;
#[allow(dead_code)]
pub const OAUTH_MAX_BODY_BYTES: usize = 8_192;

#[derive(Clone)]
pub struct OAuthRuntime {
    pub client_id: String,
    pub client_secret: Option<String>,
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
}

#[derive(Serialize, Deserialize)]
struct TokenClaims {
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    scope: String,
}

impl OAuthRuntime {
    pub fn new(
        _base_url: String,
        client_id: String,
        client_secret: Option<String>,
        token_secret: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            token_secret,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn client_id_allowed(&self, client_id: &str) -> bool {
        if client_id.is_empty() {
            return false;
        }
        if self.client_id.is_empty() {
            return true;
        }
        constant_time_eq_str(client_id, &self.client_id)
    }

    pub fn verify_access_token(&self, token: &str, server_url: &str) -> bool {
        let server_url = server_url.trim_end_matches('/');
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_audience(&[server_url]);
        validation.set_issuer(&[server_url]);
        decode::<TokenClaims>(
            token,
            &DecodingKey::from_secret(self.token_secret.as_bytes()),
            &validation,
        )
        .is_ok()
    }
}

pub fn verify_oauth_bearer_header(
    headers: &HeaderMap,
    oauth: &OAuthRuntime,
    server_url: &str,
) -> Option<Response> {
    let Some(header_value) = headers.get(AUTHORIZATION) else {
        return Some((StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response());
    };
    let Ok(header_str) = header_value.to_str() else {
        return Some((StatusCode::UNAUTHORIZED, "Invalid Authorization header").into_response());
    };
    let Some(token) = header_str.strip_prefix("Bearer ").map(str::trim) else {
        return Some((StatusCode::UNAUTHORIZED, "Invalid bearer token").into_response());
    };
    if oauth.verify_access_token(token, server_url) {
        None
    } else {
        Some((StatusCode::UNAUTHORIZED, "Invalid bearer token").into_response())
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
}

#[derive(Debug, Deserialize, Default)]
pub struct TokenForm {
    pub grant_type: String,
    pub code: String,
    pub redirect_uri: String,
    pub code_verifier: String,
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}

pub fn authorize_get(oauth: &OAuthRuntime, params: AuthorizeParams, server_url: &str) -> Response {
    if params.response_type != "code" {
        return html_error("response_type must be 'code'", StatusCode::BAD_REQUEST);
    }
    if !oauth.client_id_allowed(&params.client_id) {
        return html_error("Unknown client_id", StatusCode::BAD_REQUEST);
    }
    if params.code_challenge_method != "S256" || params.code_challenge.is_empty() {
        return html_error(
            "code_challenge_method must be S256 and code_challenge is required",
            StatusCode::BAD_REQUEST,
        );
    }
    let server_url = server_url.trim_end_matches('/').to_string();
    let code = uuid::Uuid::new_v4().to_string().replace('-', "");
    let now = unix_now();
    {
        let mut pending = oauth.pending.lock().expect("oauth pending lock");
        pending.retain(|_, v| v.expires_at >= now);
        pending.insert(
            code.clone(),
            PendingCode {
                code_challenge: params.code_challenge.clone(),
                client_id: params.client_id.clone(),
                redirect_uri: params.redirect_uri.clone(),
                state: params.state.clone(),
                expires_at: now + OAUTH_CODE_TTL_SECONDS,
                server_url: server_url.clone(),
            },
        );
    }

    let mut qs = format!("code={}", urlencoding_encode(&code));
    if !params.state.is_empty() {
        qs.push_str(&format!("&state={}", urlencoding_encode(&params.state)));
    }
    let sep = if params.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    Redirect::to(&format!("{}{}{}", params.redirect_uri, sep, qs)).into_response()
}

pub fn token_exchange(
    oauth: &OAuthRuntime,
    headers: &HeaderMap,
    mut form: TokenForm,
    server_url: &str,
) -> Response {
    if form.grant_type != "authorization_code" {
        return token_error(
            "unsupported_grant_type",
            "Only authorization_code is supported",
        );
    }

    if let Some((id, secret)) = basic_auth_credentials(headers) {
        if form.client_id.is_empty() {
            form.client_id = id;
        }
        if form.client_secret.is_empty() {
            form.client_secret = secret;
        }
    }

    if !oauth.client_id_allowed(&form.client_id) {
        return token_error("invalid_client", "Unknown client_id");
    }
    if let Some(expected) = oauth.client_secret.as_deref() {
        if !constant_time_eq_str(&form.client_secret, expected) {
            return token_error("invalid_client", "Invalid client_secret");
        }
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
    if !verify_pkce(&form.code_verifier, &code_data.code_challenge) {
        return token_error("invalid_grant", "PKCE verification failed");
    }

    let issuer = if code_data.server_url.trim().is_empty() {
        server_url.trim_end_matches('/').to_string()
    } else {
        code_data.server_url.trim_end_matches('/').to_string()
    };
    match create_access_token(&issuer, &oauth.token_secret, OAUTH_TOKEN_TTL_SECONDS) {
        Ok(access_token) => (
            StatusCode::OK,
            axum::Json(json!({
                "access_token": access_token,
                "token_type": "Bearer",
                "expires_in": OAUTH_TOKEN_TTL_SECONDS
            })),
        )
            .into_response(),
        Err(_) => token_error("server_error", "Failed to issue access token"),
    }
}

fn create_access_token(server_url: &str, token_secret: &str, ttl: i64) -> Result<String, ()> {
    let now = unix_now() as i64;
    let claims = TokenClaims {
        iss: server_url.to_string(),
        aud: server_url.to_string(),
        iat: now,
        exp: now + ttl,
        scope: "mcp".into(),
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

fn basic_auth_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let header = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let encoded = header.strip_prefix("Basic ")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (id, secret) = text.split_once(':')?;
    Some((id.to_string(), secret.to_string()))
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
            None,
            "token-signing-secret".into(),
        );
        let verifier = "dBjftJeZ4CVP-mB92Kpru-AEJvkQlLgi3ThpmQ45N_Xyo";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let redirect_uri = "https://chatgpt.com/connector/oauth/test";
        let redirect = authorize_get(
            &oauth,
            AuthorizeParams {
                response_type: "code".into(),
                client_id: "chatgpt-client-test".into(),
                redirect_uri: redirect_uri.into(),
                code_challenge: challenge,
                code_challenge_method: "S256".into(),
                state: "state".into(),
            },
            "https://lb.example.com",
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
                client_secret: String::new(),
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
}
