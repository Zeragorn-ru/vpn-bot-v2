#![allow(clippy::format_push_string)]

use std::collections::BTreeMap;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::FromRow;
use url::Url;
use uuid::Uuid;
use vpn_domain::{ProxyNode, SubscriptionFormat, SubscriptionProfile};
use vpn_storage::{decrypt_app_secret, hash_token};

use super::{ApiError, AppState};

const CLIENT_MARKERS: &[&str] = &[
    "clash",
    "mihomo",
    "hiddify",
    "karing",
    "happ",
    "shadowrocket",
    "sing-box",
    "singbox",
    "streisand",
    "loon",
    "nekobox",
    "nekoray",
    "v2ray",
    "xray",
    "incycloud",
];

#[derive(Debug, Deserialize)]
pub struct FormatQuery {
    pub format: Option<String>,
}

#[derive(Debug, FromRow)]
struct EntitlementRow {
    status: String,
    expires_at: Option<DateTime<Utc>>,
    traffic_used_bytes: i64,
    traffic_limit_bytes: Option<i64>,
    snapshot: Option<Vec<u8>>,
    snapshot_fresh_until: Option<DateTime<Utc>>,
}

#[derive(Debug)]
struct ParsedNode {
    name: String,
    scheme: String,
    host: String,
    port: u16,
    credential: String,
    query: BTreeMap<String, String>,
    path: String,
}

#[derive(Debug, Clone, Copy)]
enum ClientCategory {
    Browser,
    VpnClient,
}

impl ClientCategory {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::VpnClient => "vpn_client",
        }
    }
}

pub async fn get(
    State(state): State<AppState>,
    Path(token): Path<String>,
    Query(query): Query<FormatQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let token_hash = hash_token(&token);
    let entitlement = sqlx::query_as::<_, EntitlementRow>(
        "SELECT s.status, s.expires_at, s.traffic_used_bytes,
                s.traffic_limit_bytes, p.profile AS snapshot, p.fresh_until AS snapshot_fresh_until
         FROM subscription_access_tokens a
         JOIN subscriptions s ON s.id = a.subscription_id
         LEFT JOIN provider_snapshots p ON p.subscription_id = s.id
         WHERE a.token_hash = $1
           AND a.revoked_at IS NULL
           AND (a.expires_at IS NULL OR a.expires_at > now())",
    )
    .bind(&token_hash)
    .fetch_optional(&state.database)
    .await
    .map_err(|_| ApiError::internal())?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Subscription was not found.",
        )
    })?;

    if entitlement.status != "active" {
        return Err(ApiError::new(
            StatusCode::GONE,
            "subscription_unavailable",
            "Subscription is not active.",
        ));
    }

    let explicit_format = query.format.as_deref().map(SubscriptionFormat::parse);
    if query.format.is_some() && explicit_format.flatten().is_none() {
        return Err(ApiError::invalid("Unsupported subscription format."));
    }
    let category = classify_client(&headers);
    let selected_format = explicit_format
        .flatten()
        .unwrap_or(SubscriptionFormat::Base64);

    let profile = profile_from_row(&entitlement, &state.encryption_key);
    let should_render_browser =
        query.format.is_none() && matches!(category, ClientCategory::Browser);
    let response = if should_render_browser {
        browser_response(&headers, &token, &profile)
    } else {
        if !snapshot_is_fresh(&entitlement) {
            return Err(ApiError::unavailable(
                "Provider snapshot is unavailable or stale.",
            ));
        }
        let snapshot = entitlement
            .snapshot
            .as_deref()
            .ok_or_else(|| ApiError::unavailable("Provider snapshot is not ready."))?;
        let snapshot_profile = decrypt_app_secret(&state.encryption_key, snapshot)
            .ok()
            .and_then(|value| serde_json::from_str::<SubscriptionProfile>(&value).ok())
            .ok_or_else(|| ApiError::unavailable("Provider snapshot is invalid."))?;
        let body = render(&snapshot_profile, selected_format)?;
        config_response(&snapshot_profile, selected_format, body)
    };

    sqlx::query(
        "UPDATE subscription_access_tokens SET last_used_at = now()
         WHERE token_hash = $1",
    )
    .bind(&token_hash)
    .execute(&state.database)
    .await
    .map_err(|_| ApiError::internal())?;
    sqlx::query(
        "INSERT INTO subscription_request_logs
         (id, token_hash, format, client_category, status_code, node_count)
         VALUES ($1, $2, $3, $4, 200, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(token_hash)
    .bind(selected_format.as_str())
    .bind(category.as_str())
    .bind(i32::try_from(profile.nodes.len()).unwrap_or(i32::MAX))
    .execute(&state.database)
    .await
    .map_err(|_| ApiError::internal())?;
    Ok(response)
}

pub async fn head(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
            SELECT 1
            FROM subscription_access_tokens a
            JOIN subscriptions s ON s.id = a.subscription_id
            WHERE a.token_hash = $1
              AND a.revoked_at IS NULL
              AND (a.expires_at IS NULL OR a.expires_at > now())
              AND s.status = 'active'
        )",
    )
    .bind(hash_token(&token))
    .fetch_one(&state.database)
    .await
    .map_err(|_| ApiError::internal())?;
    if exists {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Subscription was not found.",
        ))
    }
}

fn snapshot_is_fresh(row: &EntitlementRow) -> bool {
    row.snapshot.is_some()
        && row
            .snapshot_fresh_until
            .is_some_and(|value| value > Utc::now())
}

fn profile_from_row(row: &EntitlementRow, key: &[u8; 32]) -> SubscriptionProfile {
    row.snapshot
        .as_ref()
        .and_then(|value| decrypt_app_secret(key, value).ok())
        .and_then(|value| serde_json::from_str::<SubscriptionProfile>(&value).ok())
        .unwrap_or_else(|| SubscriptionProfile {
            title: "VECTOR subscription".to_owned(),
            expires_at: row.expires_at.map(|value| value.timestamp()),
            traffic_used_bytes: row.traffic_used_bytes,
            traffic_limit_bytes: row.traffic_limit_bytes,
            support_url: None,
            nodes: Vec::new(),
        })
}

fn classify_client(headers: &HeaderMap) -> ClientCategory {
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if CLIENT_MARKERS
        .iter()
        .any(|marker| user_agent.contains(marker))
    {
        return ClientCategory::VpnClient;
    }
    let accepts_html = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html"));
    if accepts_html || user_agent.is_empty() || user_agent.contains("mozilla") {
        ClientCategory::Browser
    } else {
        ClientCategory::VpnClient
    }
}

fn browser_response(headers: &HeaderMap, token: &str, profile: &SubscriptionProfile) -> Response {
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("subscription.invalid");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("https");
    let link = format!("{scheme}://{host}/sub/{token}");
    let title = html_escape(&profile.title);
    let expiry = profile
        .expires_at
        .map_or_else(|| "unlimited".to_owned(), |timestamp| timestamp.to_string());
    let remaining = profile.traffic_limit_bytes.map_or_else(
        || "unlimited".to_owned(),
        |limit| limit.saturating_sub(profile.traffic_used_bytes).to_string(),
    );
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"referrer\" content=\"no-referrer\"><title>{title}</title><style>body{{background:#0b1018;color:#e9f0f2;font:16px system-ui;max-width:680px;margin:12vh auto;padding:24px}}main{{border:1px solid #31404a;border-radius:10px;padding:28px}}em{{color:#d6f27a;font-style:normal}}code{{display:block;word-break:break-all;background:#131e26;padding:14px;border-radius:6px}}small{{color:#9eafb5}}</style></head><body><main><small>VECTOR / PRIVATE ROUTE</small><h1>{title}</h1><p>Status: <em>active</em></p><p>Expires: {expiry}<br>Traffic remaining: {remaining}</p><code>{}</code><p>Use this link in Clash, sing-box, Xray or another compatible VPN client.</p></main></body></html>",
        html_escape(&link)
    );
    let mut response = Response::new(body.into());
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    apply_common_headers(response.headers_mut(), profile);
    response
}

fn config_response(
    profile: &SubscriptionProfile,
    format: SubscriptionFormat,
    body: String,
) -> Response {
    let content_type = match format {
        SubscriptionFormat::Clash => "text/yaml; charset=utf-8",
        SubscriptionFormat::SingBox | SubscriptionFormat::Xray => "application/json; charset=utf-8",
        SubscriptionFormat::Base64 => "text/plain; charset=utf-8",
    };
    let mut response = Response::new(body.into());
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    apply_common_headers(response.headers_mut(), profile);
    response
}

fn apply_common_headers(headers: &mut HeaderMap, profile: &SubscriptionProfile) {
    if let Ok(value) = HeaderValue::from_str(&profile.title) {
        headers.insert("Profile-Title", value);
    }
    headers.insert("announce", HeaderValue::from_static("true"));
    headers.insert("Profile-Update-Interval", HeaderValue::from_static("30"));
    let user_info = format!(
        "upload=0; download={}; total={}; expire={}",
        profile.traffic_used_bytes,
        profile.traffic_limit_bytes.unwrap_or(0),
        profile.expires_at.unwrap_or(0)
    );
    if let Ok(value) = HeaderValue::from_str(&user_info) {
        headers.insert("Subscription-Userinfo", value);
    }
    if let Some(support_url) = &profile.support_url {
        if let Ok(value) = HeaderValue::from_str(support_url) {
            headers.insert("Support-Url", value);
        }
    }
    headers.insert("providerid", HeaderValue::from_static("vector"));
    headers.insert("sub-info-color", HeaderValue::from_static("d6f27a"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

fn render(profile: &SubscriptionProfile, format: SubscriptionFormat) -> Result<String, ApiError> {
    match format {
        SubscriptionFormat::Base64 => Ok(STANDARD.encode(
            profile
                .nodes
                .iter()
                .map(|node| node.uri.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        )),
        SubscriptionFormat::Clash => render_clash(profile),
        SubscriptionFormat::SingBox => render_sing_box(profile),
        SubscriptionFormat::Xray => render_xray(profile),
    }
}

fn parsed_nodes(profile: &SubscriptionProfile) -> Vec<ParsedNode> {
    profile.nodes.iter().filter_map(parse_node).collect()
}

fn parse_node(node: &ProxyNode) -> Option<ParsedNode> {
    let url = Url::parse(&node.uri).ok()?;
    let scheme = url.scheme().to_owned();
    if !matches!(scheme.as_str(), "vless" | "hysteria2" | "hy2") {
        return None;
    }
    let host = url.host_str()?.to_owned();
    let port = url.port_or_known_default()?;
    let credential = url.username().to_owned();
    let query = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    Some(ParsedNode {
        name: node.name.clone(),
        scheme,
        host,
        port,
        credential,
        query,
        path: url.path().to_owned(),
    })
}

fn render_clash(profile: &SubscriptionProfile) -> Result<String, ApiError> {
    let mut output = String::from("proxies:\n");
    for node in parsed_nodes(profile) {
        output.push_str(&format!(
            "  - name: '{}'\n    type: {}\n    server: '{}'\n    port: {}\n",
            yaml(&node.name),
            if node.scheme == "vless" {
                "vless"
            } else {
                "hysteria2"
            },
            yaml(&node.host),
            node.port
        ));
        if node.scheme == "vless" {
            output.push_str(&format!(
                "    uuid: '{}'\n    tls: {}\n",
                yaml(&node.credential),
                node.query
                    .get("security")
                    .is_some_and(|value| value != "none")
            ));
            if let Some(flow) = node.query.get("flow") {
                output.push_str(&format!("    flow: '{}'\n", yaml(flow)));
            }
            if node.query.get("type").is_some_and(|value| value == "ws") {
                output.push_str("    network: ws\n    ws-opts:\n");
                output.push_str(&format!("      path: '{}'\n", yaml(&node.path)));
                if let Some(host) = node.query.get("host") {
                    output.push_str(&format!("      headers:\n        Host: '{}'\n", yaml(host)));
                }
            }
            if node
                .query
                .get("security")
                .is_some_and(|value| value == "reality")
            {
                output.push_str("    reality-opts:\n");
                if let Some(public_key) = node.query.get("pbk") {
                    output.push_str(&format!("      public-key: '{}'\n", yaml(public_key)));
                }
                if let Some(short_id) = node.query.get("sid") {
                    output.push_str(&format!("      short-id: '{}'\n", yaml(short_id)));
                }
            }
        } else {
            output.push_str(&format!("    password: '{}'\n", yaml(&node.credential)));
            if let Some(sni) = node.query.get("sni") {
                output.push_str(&format!("    sni: '{}'\n", yaml(sni)));
            }
        }
    }
    if output == "proxies:\n" {
        return Err(ApiError::unavailable(
            "No supported proxy nodes are available.",
        ));
    }
    output.push_str("proxy-groups:\n  - name: VECTOR\n    type: select\n    proxies:\n");
    for node in parsed_nodes(profile) {
        output.push_str(&format!("      - '{}'\n", yaml(&node.name)));
    }
    output.push_str("rules:\n  - MATCH, VECTOR\n");
    Ok(output)
}

fn render_sing_box(profile: &SubscriptionProfile) -> Result<String, ApiError> {
    let mut outbounds = Vec::new();
    for node in parsed_nodes(profile) {
        let mut item = serde_json::Map::new();
        let kind = if node.scheme == "vless" {
            "vless"
        } else {
            "hysteria2"
        };
        item.insert("type".to_owned(), Value::String(kind.to_owned()));
        item.insert("tag".to_owned(), Value::String(node.name));
        item.insert("server".to_owned(), Value::String(node.host));
        item.insert("server_port".to_owned(), json!(node.port));
        if node.scheme == "vless" {
            item.insert("uuid".to_owned(), Value::String(node.credential));
            if let Some(flow) = node.query.get("flow") {
                item.insert("flow".to_owned(), Value::String(flow.clone()));
            }
            if node
                .query
                .get("security")
                .is_some_and(|value| value != "none")
            {
                let mut tls = serde_json::Map::new();
                tls.insert("enabled".to_owned(), Value::Bool(true));
                if let Some(sni) = node.query.get("sni") {
                    tls.insert("server_name".to_owned(), Value::String(sni.clone()));
                }
                if node
                    .query
                    .get("security")
                    .is_some_and(|value| value == "reality")
                {
                    tls.insert("reality".to_owned(), json!({"enabled":true,"public_key":node.query.get("pbk"),"short_id":node.query.get("sid")}));
                }
                item.insert("tls".to_owned(), Value::Object(tls));
            }
            if node.query.get("type").is_some_and(|value| value == "ws") {
                item.insert(
                    "transport".to_owned(),
                    json!({"type":"ws","path":node.path,"headers":{"Host":node.query.get("host")}}),
                );
            }
        } else {
            item.insert("password".to_owned(), Value::String(node.credential));
            if let Some(sni) = node.query.get("sni") {
                item.insert("tls".to_owned(), json!({"enabled":true,"server_name":sni}));
            }
            if let Some(obfs) = node.query.get("obfs") {
                item.insert(
                    "obfs".to_owned(),
                    json!({"type":obfs,"password":node.query.get("obfs-password")}),
                );
            }
        }
        outbounds.push(Value::Object(item));
    }
    if outbounds.is_empty() {
        return Err(ApiError::unavailable(
            "No supported proxy nodes are available.",
        ));
    }
    serde_json::to_string_pretty(&json!({"log":{"level":"warn"},"outbounds":outbounds,"route":{"auto_detect_interface":true}})).map_err(|_| ApiError::internal())
}

fn render_xray(profile: &SubscriptionProfile) -> Result<String, ApiError> {
    let nodes = parsed_nodes(profile)
        .into_iter()
        .filter(|node| node.scheme == "vless")
        .collect::<Vec<_>>();
    if nodes.is_empty() {
        return Err(ApiError::unavailable(
            "No Xray-compatible proxy nodes are available.",
        ));
    }
    let outbounds = nodes.iter().map(|node| {
        let mut stream = serde_json::Map::new();
        if let Some(network) = node.query.get("type") { stream.insert("network".to_owned(), Value::String(network.clone())); }
        if let Some(security) = node.query.get("security") { stream.insert("security".to_owned(), Value::String(security.clone())); }
        if node.query.get("type").is_some_and(|value| value == "ws") { stream.insert("wsSettings".to_owned(), json!({"path":node.path,"headers":{"Host":node.query.get("host")}})); }
        if node.query.get("security").is_some_and(|value| value == "reality") { stream.insert("realitySettings".to_owned(), json!({"serverName":node.query.get("sni"),"publicKey":node.query.get("pbk"),"shortId":node.query.get("sid"),"fingerprint":node.query.get("fp")})); }
        json!({"protocol":"vless","tag":node.name,"settings":{"vnext":[{"address":node.host,"port":node.port,"users":[{"id":node.credential,"encryption":"none","flow":node.query.get("flow").cloned().unwrap_or_default()}]}]},"streamSettings":stream})
    }).collect::<Vec<_>>();
    serde_json::to_string_pretty(&json!({"log":{"loglevel":"warning"},"inbounds":[],"outbounds":outbounds,"routing":{"domainStrategy":"AsIs","rules":[]}})).map_err(|_| ApiError::internal())
}

fn yaml(value: &str) -> String {
    value.replace('\'', "''")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::{ClientCategory, EntitlementRow, classify_client, render, snapshot_is_fresh};
    use axum::http::{HeaderMap, HeaderValue, header};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use chrono::{Duration, Utc};
    use vpn_domain::{ProxyNode, SubscriptionFormat, SubscriptionProfile};

    fn profile() -> SubscriptionProfile {
        SubscriptionProfile {
            title: "VECTOR / ONE".to_owned(),
            expires_at: Some(1_800_000_000),
            traffic_used_bytes: 10,
            traffic_limit_bytes: Some(100),
            support_url: Some("https://support.example".to_owned()),
            nodes: vec![ProxyNode {
                name: "Amsterdam / 01".to_owned(),
                uri: "vless://00000000-0000-0000-0000-000000000001@edge.example:443?encryption=none&security=reality&sni=cdn.example&pbk=public-key&sid=abcd&type=ws&path=%2Fgateway#edge".to_owned(),
            }],
        }
    }

    #[test]
    fn explicit_renderers_produce_client_profiles() {
        let value = profile();
        let clash = render(&value, SubscriptionFormat::Clash).unwrap();
        assert!(clash.contains("type: vless"));
        assert!(clash.contains("reality-opts:"));
        let sing_box = render(&value, SubscriptionFormat::SingBox).unwrap();
        assert!(sing_box.contains("\"type\": \"vless\""));
        let xray = render(&value, SubscriptionFormat::Xray).unwrap();
        assert!(xray.contains("\"protocol\": \"vless\""));
        let base64 = render(&value, SubscriptionFormat::Base64).unwrap();
        assert_eq!(
            STANDARD.decode(base64).unwrap(),
            value.nodes[0].uri.as_bytes()
        );
    }

    #[test]
    fn snapshot_freshness_rejects_absent_and_expired_profiles() {
        let now = Utc::now();
        let fresh = EntitlementRow {
            status: "active".to_owned(),
            expires_at: None,
            traffic_used_bytes: 0,
            traffic_limit_bytes: None,
            snapshot: Some(vec![1]),
            snapshot_fresh_until: Some(now + Duration::seconds(1)),
        };
        assert!(snapshot_is_fresh(&fresh));

        let expired = EntitlementRow {
            snapshot_fresh_until: Some(now - Duration::seconds(1)),
            ..fresh
        };
        assert!(!snapshot_is_fresh(&expired));

        let absent = EntitlementRow {
            snapshot: None,
            snapshot_fresh_until: Some(now + Duration::seconds(1)),
            ..expired
        };
        assert!(!snapshot_is_fresh(&absent));
    }

    #[test]
    fn classifier_is_conservative() {
        let mut browser = HeaderMap::new();
        browser.insert(header::USER_AGENT, HeaderValue::from_static("Mozilla/5.0"));
        browser.insert(header::ACCEPT, HeaderValue::from_static("text/html"));
        assert!(matches!(classify_client(&browser), ClientCategory::Browser));

        let mut client = HeaderMap::new();
        client.insert(header::USER_AGENT, HeaderValue::from_static("Hiddify/1.0"));
        assert!(matches!(
            classify_client(&client),
            ClientCategory::VpnClient
        ));
    }
}
