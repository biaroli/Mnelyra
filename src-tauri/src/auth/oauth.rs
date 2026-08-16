use axum::http::HeaderMap;
use serde_json::{json, Value};

use crate::workspace::AuthConfig;

impl AuthConfig {
    pub fn oauth_enabled(&self) -> bool {
        self.auth_type == "oauth"
    }

    pub fn bearer_enabled(&self) -> bool {
        self.auth_type == "bearer"
    }

    pub fn auth_enabled(&self) -> bool {
        matches!(self.auth_type.as_str(), "oauth" | "bearer")
    }
}

/// Resolve the external OAuth/MCP base URL for a request.
/// Matches the Python server's behavior: prefer configured URL,
/// then `X-Forwarded-*` / `Host`, then localhost.
pub fn external_base_url(headers: &HeaderMap, bind_port: u16, configured_url: &str) -> String {
    let configured = configured_url.trim().trim_end_matches('/');

    // Quick Tunnel 的 hostname 是临时的，不能信任上一次保存的地址。
    // 对 trycloudflare.com 请求，改为从当前请求的 Forwarded / Host 推导公网地址。
    if !configured.is_empty() && !configured.contains(".trycloudflare.com") {
        return configured.to_string();
    }

    let proto = {
        let value = first_header_value(headers, "x-forwarded-proto");
        if value.is_empty() {
            forwarded_header_param(headers, "proto")
        } else {
            value
        }
    };
    let host = {
        let value = safe_external_host(&first_header_value(headers, "x-forwarded-host"));
        if !value.is_empty() {
            value
        } else {
            let value = safe_external_host(&forwarded_header_param(headers, "host"));
            if !value.is_empty() {
                value
            } else {
                safe_external_host(&first_header_value(headers, "host"))
            }
        }
    };

    let host = if host.is_empty() {
        format!("127.0.0.1:{bind_port}")
    } else {
        host
    };
    let proto = resolve_external_proto(
        if proto.is_empty() {
            None
        } else {
            Some(proto.as_str())
        },
        &host,
    );
    format!("{proto}://{host}")
}

fn first_header_value(headers: &HeaderMap, name: &str) -> String {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.split(',').next().unwrap_or("").trim().to_string())
        .unwrap_or_default()
}

fn forwarded_header_param(headers: &HeaderMap, name: &str) -> String {
    let first = first_header_value(headers, "forwarded");
    for part in first.split(';') {
        let part = part.trim();
        if let Some((key, value)) = part.split_once('=') {
            if key.trim().eq_ignore_ascii_case(name) {
                return value.trim().trim_matches('"').to_string();
            }
        }
    }
    String::new()
}

fn safe_external_host(host: &str) -> String {
    let host = host.trim();
    if host.is_empty()
        || host
            .chars()
            .any(|ch| matches!(ch, '\r' | '\n' | '/' | '\\'))
    {
        String::new()
    } else {
        host.to_string()
    }
}

fn resolve_external_proto(proto: Option<&str>, host: &str) -> &'static str {
    if let Some(proto) = proto {
        let proto = proto.trim().to_ascii_lowercase();
        if proto == "http" {
            return "http";
        }
        if proto == "https" {
            return "https";
        }
    }

    let host_without_port = host
        .rsplit_once(':')
        .map(|(value, _)| value.trim_matches('[').trim_matches(']'))
        .unwrap_or_else(|| host.trim_matches('[').trim_matches(']'));
    if is_loopback_host(host_without_port) {
        "http"
    } else {
        "https"
    }
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}

pub fn authorization_server_metadata(base_url: &str) -> Value {
    let base = base_url.trim_end_matches('/');
    json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["mcp", "offline_access"],
    })
}

pub fn protected_resource_metadata(base_url: &str) -> Value {
    let base = base_url.trim_end_matches('/');
    json!({
        "resource": base,
        "authorization_servers": [base],
        "bearer_methods_supported": ["header"],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_enabled_only_for_oauth_type() {
        let mut auth = AuthConfig::default();
        assert!(auth.oauth_enabled());
        auth.auth_type = "bearer".into();
        assert!(!auth.oauth_enabled());
        auth.auth_type = "internal".into();
        assert!(!auth.oauth_enabled());
    }

    #[test]
    fn authorization_metadata_uses_pkce_public_client() {
        let meta = authorization_server_metadata("https://example.com");
        assert_eq!(
            meta["token_endpoint_auth_methods_supported"],
            json!(["none"])
        );
    }

    #[test]
    fn protected_resource_metadata_lists_authorization_servers() {
        let meta = protected_resource_metadata("https://example.com");
        assert_eq!(
            meta["authorization_servers"],
            json!(["https://example.com"])
        );
    }

    #[test]
    fn external_base_url_prefers_configured_url() {
        let headers = HeaderMap::new();
        assert_eq!(
            external_base_url(&headers, 28767, "https://lb.frp-tx1.evwali.com"),
            "https://lb.frp-tx1.evwali.com"
        );
    }

    #[test]
    fn external_base_url_uses_forwarded_host() {
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", "https".parse().unwrap());
        headers.insert("x-forwarded-host", "lb.frp-tx1.evwali.com".parse().unwrap());
        assert_eq!(
            external_base_url(&headers, 28767, ""),
            "https://lb.frp-tx1.evwali.com"
        );
    }

    #[test]
    fn external_base_url_uses_host_header() {
        let mut headers = HeaderMap::new();
        headers.insert("host", "lb.frp-tx1.evwali.com".parse().unwrap());
        assert_eq!(
            external_base_url(&headers, 28767, ""),
            "https://lb.frp-tx1.evwali.com"
        );
    }
}
