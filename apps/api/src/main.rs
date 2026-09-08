#![allow(clippy::module_name_repetitions)]

use std::{env, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool, Postgres, Row, Transaction, postgres::PgRow};
use tower_http::{catch_panic::CatchPanicLayer, limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::{info, warn};
use uuid::Uuid;
use vpn_api_contracts::{
    AdminMeResponse, AuditEntry, ErrorResponse, HealthResponse, JsonValueRequest, LoginRequest,
    LoginResponse, PublicSettings, RuntimeSettings, SecretMetadata, SetupOwnerRequest,
    SetupOwnerResponse, SetupStatusResponse, SubscriptionAccessTokenResponse,
};
use vpn_observability::init_tracing;
use vpn_storage::{
    connect, decode_encryption_key, encrypt_app_secret, generate_token, hash_password, hash_token,
    is_ready, verify_password,
};

mod aggregator;

const SESSION_TTL_HOURS: i64 = 24;
const SETUP_TOKEN_FILE: &str = "setup-token";

#[derive(Clone)]
struct AppState {
    database: PgPool,
    redis: redis::Client,
    encryption_key: Arc<[u8; 32]>,
    bootstrap_dir: Arc<PathBuf>,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
    request_id: Uuid,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            request_id: Uuid::now_v7(),
        }
    }

    fn invalid(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }
    fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication is required.",
        )
    }
    fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "This action is not permitted.",
        )
    }
    fn conflict(message: &'static str) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    fn unavailable(message: &'static str) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }
    fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "An unexpected error occurred.",
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code,
                message: self.message,
                request_id: self.request_id,
            }),
        )
            .into_response()
    }
}

#[derive(Debug, Clone)]
struct Principal {
    account_id: Uuid,
    login: String,
    permissions: Vec<String>,
}

#[derive(Debug)]
struct SessionRow {
    account_id: Uuid,
    login: String,
}

impl<'row> FromRow<'row, PgRow> for SessionRow {
    fn from_row(row: &'row PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            account_id: row.try_get("account_id")?,
            login: row.try_get("login")?,
        })
    }
}

#[derive(Debug, Deserialize)]
struct SecretRequest {
    value: String,
}

#[derive(Debug, Serialize)]
struct ReadyResponse {
    status: &'static str,
    postgres: bool,
    redis: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let redis_url = env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".to_owned());
    let encryption_key = decode_encryption_key(
        &env::var("APPLICATION_ENCRYPTION_KEY")
            .context("APPLICATION_ENCRYPTION_KEY is required")?,
    )?;
    let database = connect(&database_url)
        .await
        .context("database connection failed")?;
    let redis = redis::Client::open(redis_url).context("redis client initialization failed")?;
    let bootstrap_dir = PathBuf::from(
        env::var("BOOTSTRAP_DIR").unwrap_or_else(|_| "/var/lib/vpn/bootstrap".to_owned()),
    );
    let state = AppState {
        database,
        redis,
        encryption_key: Arc::new(encryption_key),
        bootstrap_dir: Arc::new(bootstrap_dir),
    };
    let bind_addr: SocketAddr = env::var("BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()
        .context("BIND_ADDR is invalid")?;

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/setup/status", get(setup_status))
        .route("/setup/owner", post(create_owner))
        .route("/sub/{token}", get(aggregator::get).head(aggregator::head))
        .route("/admin/auth/login", post(login))
        .route("/admin/auth/logout", post(logout))
        .route("/admin/me", get(me))
        .route(
            "/admin/settings/public",
            get(get_public_settings).put(put_public_settings),
        )
        .route(
            "/admin/settings/runtime",
            get(get_runtime_settings).put(put_runtime_settings),
        )
        .route("/admin/secrets", get(list_secrets))
        .route("/admin/secrets/{key}", put(put_secret))
        .route("/admin/audit", get(list_audit))
        .route(
            "/admin/subscriptions/{subscription_id}/token",
            post(issue_access_token),
        )
        .route(
            "/admin/subscriptions/{subscription_id}/token/revoke",
            post(revoke_access_tokens),
        )
        .with_state(state)
        .layer(middleware::from_fn(request_context))
        .layer(RequestBodyLimitLayer::new(64 * 1024))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("API bind failed")?;
    info!(%bind_addr, "api listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("API server failed")?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! { () = ctrl_c => {}, () = terminate => {} }
}

async fn request_context(request: axum::extract::Request, next: Next) -> Response<Body> {
    next.run(request).await
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn ready(State(state): State<AppState>) -> Result<Json<ReadyResponse>, ApiError> {
    let postgres = is_ready(&state.database).await;
    let redis = redis_ready(&state.redis).await;
    let response = ReadyResponse {
        status: if postgres && redis {
            "ready"
        } else {
            "not_ready"
        },
        postgres,
        redis,
    };
    if postgres && redis {
        Ok(Json(response))
    } else {
        Err(ApiError::unavailable(
            "Required dependencies are not ready.",
        ))
    }
}

async fn redis_ready(client: &redis::Client) -> bool {
    let Ok(mut connection) = client.get_multiplexed_async_connection().await else {
        return false;
    };
    redis::cmd("PING")
        .query_async::<String>(&mut connection)
        .await
        .is_ok()
}

async fn setup_status(
    State(state): State<AppState>,
) -> Result<Json<SetupStatusResponse>, ApiError> {
    let setup_required = owner_count(&state.database)
        .await
        .map_err(|_| ApiError::internal())?
        == 0;
    Ok(Json(SetupStatusResponse { setup_required }))
}

async fn create_owner(
    State(state): State<AppState>,
    Json(request): Json<SetupOwnerRequest>,
) -> Result<Json<SetupOwnerResponse>, ApiError> {
    validate_login(&request.login)?;
    let password_hash = hash_password(&request.password)
        .map_err(|_| ApiError::invalid("Password does not meet the minimum policy."))?;
    let token_path = state.bootstrap_dir.join(SETUP_TOKEN_FILE);
    let expected = tokio::fs::read_to_string(&token_path)
        .await
        .map_err(|_| ApiError::unavailable("Initial setup is not armed."))?;
    if hash_token(expected.trim()) != hash_token(request.setup_token.trim()) {
        return Err(ApiError::unauthorized());
    }

    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(7_381_i64)
        .execute(&mut *transaction)
        .await
        .map_err(|_| ApiError::internal())?;
    if owner_count_tx(&mut transaction)
        .await
        .map_err(|_| ApiError::internal())?
        > 0
    {
        return Err(ApiError::conflict("An owner account already exists."));
    }
    let user_id = Uuid::now_v7();
    let account_id = Uuid::now_v7();
    sqlx::query("INSERT INTO users (id) VALUES ($1)")
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| ApiError::internal())?;
    sqlx::query("INSERT INTO user_profiles (user_id, language_code) VALUES ($1, 'ru')")
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| ApiError::internal())?;
    sqlx::query("INSERT INTO wallets (id, user_id) VALUES ($1, $2)")
        .bind(Uuid::now_v7())
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(|_| ApiError::internal())?;
    sqlx::query(
        "INSERT INTO admin_accounts (id, user_id, login, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(account_id)
    .bind(user_id)
    .bind(&request.login)
    .bind(password_hash)
    .execute(&mut *transaction)
    .await
    .map_err(|_| ApiError::internal())?;
    sqlx::query("INSERT INTO admin_account_roles (admin_account_id, role_id) SELECT $1, id FROM roles WHERE code = 'owner'").bind(account_id).execute(&mut *transaction).await.map_err(|_| ApiError::internal())?;
    write_audit(
        &mut transaction,
        Some(account_id),
        "owner.created",
        "admin_account",
        Some(account_id),
        Uuid::now_v7(),
    )
    .await
    .map_err(|_| ApiError::internal())?;
    transaction
        .commit()
        .await
        .map_err(|_| ApiError::internal())?;
    if let Err(error) = tokio::fs::remove_file(token_path).await {
        warn!(%error, "setup token could not be removed after owner creation");
    }
    Ok(Json(SetupOwnerResponse { account_id }))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, ApiError> {
    let row = sqlx::query_as::<_, (Uuid, Uuid, String, String)>(
        "SELECT id, user_id, login, password_hash FROM admin_accounts WHERE login = $1",
    )
    .bind(&request.login)
    .fetch_optional(&state.database)
    .await
    .map_err(|_| ApiError::internal())?
    .ok_or_else(ApiError::unauthorized)?;
    if !verify_password(&request.password, &row.3) {
        return Err(ApiError::unauthorized());
    }
    let token = generate_token();
    let expires_at = Utc::now() + chrono::Duration::hours(SESSION_TTL_HOURS);
    sqlx::query("INSERT INTO admin_sessions (id, admin_account_id, session_hash, expires_at) VALUES ($1, $2, $3, $4)").bind(Uuid::now_v7()).bind(row.0).bind(hash_token(&token)).bind(expires_at).execute(&state.database).await.map_err(|_| ApiError::internal())?;
    sqlx::query(
        "UPDATE admin_accounts SET last_login_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(row.0)
    .execute(&state.database)
    .await
    .map_err(|_| ApiError::internal())?;
    Ok(Json(LoginResponse {
        access_token: token,
        expires_at,
    }))
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<StatusCode, ApiError> {
    let Some(token) = bearer_token(&headers) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    sqlx::query("UPDATE admin_sessions SET revoked_at = now() WHERE session_hash = $1 AND revoked_at IS NULL").bind(hash_token(token)).execute(&state.database).await.map_err(|_| ApiError::internal())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AdminMeResponse>, ApiError> {
    let principal = authenticate(&state, &headers).await?;
    Ok(Json(AdminMeResponse {
        account_id: principal.account_id,
        login: principal.login,
        permissions: principal.permissions,
    }))
}

async fn get_public_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let principal = require_permission(&state, &headers, "system.manage").await?;
    let _ = principal;
    Ok(Json(
        load_setting(&state.database, "public_settings")
            .await
            .map_err(|_| ApiError::internal())?
            .unwrap_or_else(|| {
                json!(PublicSettings {
                    brand_name: None,
                    mini_app_url: None,
                    admin_url: None,
                    subscription_url: None,
                    support_url: None
                })
            }),
    ))
}

async fn put_public_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<JsonValueRequest>,
) -> Result<Json<Value>, ApiError> {
    let principal = require_permission(&state, &headers, "system.manage").await?;
    validate_public_settings(&request.value)?;
    upsert_setting(
        &state.database,
        &principal,
        "public_settings",
        request.value.clone(),
    )
    .await?;
    Ok(Json(request.value))
}

async fn get_runtime_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let _ = require_permission(&state, &headers, "system.manage").await?;
    Ok(Json(
        load_setting(&state.database, "runtime_settings")
            .await
            .map_err(|_| ApiError::internal())?
            .unwrap_or_else(|| {
                json!(RuntimeSettings {
                    api_port: 18080,
                    admin_port: 18082,
                    mini_app_port: 18081,
                    webhook_port: 18083
                })
            }),
    ))
}

async fn put_runtime_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<JsonValueRequest>,
) -> Result<Json<Value>, ApiError> {
    let principal = require_permission(&state, &headers, "system.manage").await?;
    validate_runtime_settings(&request.value)?;
    upsert_setting(
        &state.database,
        &principal,
        "runtime_settings",
        request.value.clone(),
    )
    .await?;
    Ok(Json(request.value))
}

async fn list_secrets(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<SecretMetadata>>, ApiError> {
    let _ = require_permission(&state, &headers, "secrets.manage").await?;
    let rows = sqlx::query_as::<_, (String,)>("SELECT key FROM app_secrets ORDER BY key")
        .fetch_all(&state.database)
        .await
        .map_err(|_| ApiError::internal())?;
    Ok(Json(
        rows.into_iter()
            .map(|(key,)| SecretMetadata { key, is_set: true })
            .collect(),
    ))
}

async fn put_secret(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(key): Path<String>,
    Json(request): Json<SecretRequest>,
) -> Result<StatusCode, ApiError> {
    let principal = require_permission(&state, &headers, "secrets.manage").await?;
    if !valid_secret_key(&key) || request.value.is_empty() || request.value.len() > 16_384 {
        return Err(ApiError::invalid("Secret key or value is invalid."));
    }
    let encrypted = encrypt_app_secret(&state.encryption_key, &request.value)
        .map_err(|_| ApiError::internal())?;
    sqlx::query("INSERT INTO app_secrets (key, value, updated_by_admin_id) VALUES ($1, $2, $3) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by_admin_id = EXCLUDED.updated_by_admin_id, updated_at = now()").bind(&key).bind(encrypted).bind(principal.account_id).execute(&state.database).await.map_err(|_| ApiError::internal())?;
    let correlation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO admin_audit_log (id, actor_admin_id, action, target_type, target_id, before_value, after_value, correlation_id) VALUES ($1, $2, 'secret.updated', 'app_secret', NULL, NULL, '{\"redacted\":true}', $3)").bind(Uuid::now_v7()).bind(principal.account_id).bind(correlation_id).execute(&state.database).await.map_err(|_| ApiError::internal())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_audit(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<AuditEntry>>, ApiError> {
    let _ = require_permission(&state, &headers, "audit.read").await?;
    let rows = sqlx::query_as::<_, (Uuid, Option<Uuid>, String, String, Option<Uuid>, Uuid, DateTime<Utc>)>("SELECT id, actor_admin_id, action, target_type, target_id, correlation_id, created_at FROM admin_audit_log ORDER BY created_at DESC LIMIT 100").fetch_all(&state.database).await.map_err(|_| ApiError::internal())?;
    Ok(Json(
        rows.into_iter()
            .map(|row| AuditEntry {
                id: row.0,
                actor_account_id: row.1,
                action: row.2,
                target_type: row.3,
                target_id: row.4,
                correlation_id: row.5,
                created_at: row.6,
            })
            .collect(),
    ))
}

async fn issue_access_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subscription_id): Path<Uuid>,
) -> Result<Json<SubscriptionAccessTokenResponse>, ApiError> {
    let principal = require_permission(&state, &headers, "operations.manage").await?;
    let mut transaction = state
        .database
        .begin()
        .await
        .map_err(|_| ApiError::internal())?;
    let (_, expires_at) = sqlx::query_as::<_, (Uuid, Option<DateTime<Utc>>)>(
        "SELECT id, expires_at FROM subscriptions WHERE id = $1 FOR UPDATE",
    )
    .bind(subscription_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|_| ApiError::internal())?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Subscription was not found.",
        )
    })?;
    sqlx::query(
        "UPDATE subscription_access_tokens SET revoked_at = now()
         WHERE subscription_id = $1 AND revoked_at IS NULL",
    )
    .bind(subscription_id)
    .execute(&mut *transaction)
    .await
    .map_err(|_| ApiError::internal())?;
    let token = generate_token();
    sqlx::query(
        "INSERT INTO subscription_access_tokens
         (id, subscription_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(hash_token(&token))
    .bind(expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(|_| ApiError::internal())?;
    write_audit(
        &mut transaction,
        Some(principal.account_id),
        "subscription.access_token.rotated",
        "subscription",
        Some(subscription_id),
        Uuid::now_v7(),
    )
    .await
    .map_err(|_| ApiError::internal())?;
    transaction
        .commit()
        .await
        .map_err(|_| ApiError::internal())?;
    let path = public_subscription_path(&state.database, &token).await?;
    Ok(Json(SubscriptionAccessTokenResponse {
        subscription_id,
        token,
        path,
    }))
}

async fn revoke_access_tokens(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(subscription_id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let principal = require_permission(&state, &headers, "operations.manage").await?;
    let result = sqlx::query(
        "UPDATE subscription_access_tokens SET revoked_at = now()
         WHERE subscription_id = $1 AND revoked_at IS NULL",
    )
    .bind(subscription_id)
    .execute(&state.database)
    .await
    .map_err(|_| ApiError::internal())?;
    if result.rows_affected() == 0 {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Active access token was not found.",
        ));
    }
    let correlation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO admin_audit_log (id, actor_admin_id, action, target_type, target_id, before_value, after_value, correlation_id) VALUES ($1, $2, 'subscription.access_token.revoked', 'subscription', $3, NULL, '{\"redacted\":true}', $4)")
        .bind(Uuid::now_v7())
        .bind(principal.account_id)
        .bind(subscription_id)
        .bind(correlation_id)
        .execute(&state.database)
        .await
        .map_err(|_| ApiError::internal())?;
    Ok(StatusCode::NO_CONTENT)
}

async fn public_subscription_path(pool: &PgPool, token: &str) -> Result<String, ApiError> {
    let settings = load_setting(pool, "public_settings")
        .await
        .map_err(|_| ApiError::internal())?;
    let configured = settings
        .as_ref()
        .and_then(|value| value.get("subscription_url"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty());
    Ok(configured.map_or_else(
        || format!("/sub/{token}"),
        |base| {
            if base.contains("{token}") {
                base.replace("{token}", token)
            } else {
                format!("{}/sub/{token}", base.trim_end_matches('/'))
            }
        },
    ))
}

async fn owner_count(pool: &PgPool) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM admin_account_roles aar JOIN roles r ON r.id = aar.role_id WHERE r.code = 'owner'").fetch_one(pool).await
}

async fn owner_count_tx(transaction: &mut Transaction<'_, Postgres>) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM admin_account_roles aar JOIN roles r ON r.id = aar.role_id WHERE r.code = 'owner'").fetch_one(&mut **transaction).await
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<Principal, ApiError> {
    let token = bearer_token(headers).ok_or_else(ApiError::unauthorized)?;
    let row = sqlx::query_as::<_, SessionRow>("SELECT a.id, a.login FROM admin_sessions s JOIN admin_accounts a ON a.id = s.admin_account_id WHERE s.session_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > now()").bind(hash_token(token)).fetch_optional(&state.database).await.map_err(|_| ApiError::internal())?.ok_or_else(ApiError::unauthorized)?;
    let permissions = sqlx::query_scalar::<_, String>("SELECT DISTINCT p.code FROM admin_account_roles aar JOIN role_permissions rp ON rp.role_id = aar.role_id JOIN permissions p ON p.id = rp.permission_id WHERE aar.admin_account_id = $1 ORDER BY p.code").bind(row.account_id).fetch_all(&state.database).await.map_err(|_| ApiError::internal())?;
    sqlx::query("UPDATE admin_sessions SET last_seen_at = now() WHERE session_hash = $1")
        .bind(hash_token(token))
        .execute(&state.database)
        .await
        .map_err(|_| ApiError::internal())?;
    Ok(Principal {
        account_id: row.account_id,
        login: row.login,
        permissions,
    })
}

async fn require_permission(
    state: &AppState,
    headers: &HeaderMap,
    permission: &str,
) -> Result<Principal, ApiError> {
    let principal = authenticate(state, headers).await?;
    if principal.permissions.iter().any(|item| item == permission) {
        Ok(principal)
    } else {
        Err(ApiError::forbidden())
    }
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn validate_login(login: &str) -> Result<(), ApiError> {
    if !(3..=128).contains(&login.chars().count()) || login.chars().any(char::is_whitespace) {
        Err(ApiError::invalid("Login is invalid."))
    } else {
        Ok(())
    }
}

fn validate_public_settings(value: &Value) -> Result<(), ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::invalid("Public settings must be an object."))?;
    for key in [
        "mini_app_url",
        "admin_url",
        "subscription_url",
        "support_url",
    ] {
        if let Some(item) = object.get(key)
            && !item.is_null()
            && !item.is_string()
        {
            return Err(ApiError::invalid("Public URL settings must be strings."));
        }
    }
    Ok(())
}

fn validate_runtime_settings(value: &Value) -> Result<(), ApiError> {
    let object = value
        .as_object()
        .ok_or_else(|| ApiError::invalid("Runtime settings must be an object."))?;
    for key in ["api_port", "admin_port", "mini_app_port", "webhook_port"] {
        let Some(port) = object.get(key).and_then(Value::as_u64) else {
            return Err(ApiError::invalid("Every runtime port is required."));
        };
        if !(1024..=65_535).contains(&port) {
            return Err(ApiError::invalid(
                "Runtime ports must be between 1024 and 65535.",
            ));
        }
    }
    Ok(())
}

fn valid_secret_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

async fn load_setting(pool: &PgPool, key: &str) -> Result<Option<Value>, sqlx::Error> {
    sqlx::query_scalar::<_, Value>("SELECT value FROM app_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await
}

async fn upsert_setting(
    pool: &PgPool,
    principal: &Principal,
    key: &str,
    value: Value,
) -> Result<(), ApiError> {
    let before = load_setting(pool, key)
        .await
        .map_err(|_| ApiError::internal())?;
    sqlx::query("INSERT INTO app_settings (key, value, updated_by_admin_id) VALUES ($1, $2, $3) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_by_admin_id = EXCLUDED.updated_by_admin_id, updated_at = now()").bind(key).bind(&value).bind(principal.account_id).execute(pool).await.map_err(|_| ApiError::internal())?;
    let after = if key.contains("secret") {
        json!({"redacted": true})
    } else {
        value
    };
    let correlation_id = Uuid::now_v7();
    sqlx::query("INSERT INTO admin_audit_log (id, actor_admin_id, action, target_type, target_id, before_value, after_value, correlation_id) VALUES ($1, $2, 'settings.updated', 'app_setting', NULL, $3, $4, $5)").bind(Uuid::now_v7()).bind(principal.account_id).bind(before.unwrap_or(Value::Null)).bind(after).bind(correlation_id).execute(pool).await.map_err(|_| ApiError::internal())?;
    Ok(())
}

async fn write_audit(
    transaction: &mut Transaction<'_, Postgres>,
    actor: Option<Uuid>,
    action: &str,
    target_type: &str,
    target_id: Option<Uuid>,
    correlation_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO admin_audit_log (id, actor_admin_id, action, target_type, target_id, before_value, after_value, correlation_id) VALUES ($1, $2, $3, $4, $5, NULL, NULL, $6)").bind(Uuid::now_v7()).bind(actor).bind(action).bind(target_type).bind(target_id).bind(correlation_id).execute(&mut **transaction).await.map(|_| ())
}
