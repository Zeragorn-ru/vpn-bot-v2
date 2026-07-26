#![recursion_limit = "512"]

use std::{
    collections::{BTreeMap, HashSet},
    env,
    fmt::Write as _,
    net::SocketAddr,
    sync::Arc,
};

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use anyhow::{Context, Result};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE, RETRY_AFTER},
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{
    Engine,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use chrono::{DateTime, Duration as ChronoDuration, NaiveDate, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::{AllowOrigin, CorsLayer},
    trace::TraceLayer,
};
use tracing::info;
use uuid::Uuid;
use vpn_integrations::{
    AnoreConfig, AnoreProvider, CryptoPayConfig, CryptoPayProvider, PaymentInvoiceRequest,
    PaymentProvider, PaymentProviderCode, ProviderPaymentStatus, TelegramStarsConfig,
    TelegramStarsProvider,
};
use vpn_observability::init_tracing;
use vpn_storage::{InvoiceForFulfillment, connect, fulfill_paid_invoice, is_ready};

use vpn_domain::{DEFAULT_TRIAL_DURATION_SECONDS, DEFAULT_TRIAL_TRAFFIC_BYTES};

const INIT_DATA_MAX_AGE_SECONDS: i64 = 300;
const SESSION_TTL_HOURS: i64 = 24;
const RATE_LIMIT_WINDOW_SECONDS: i64 = 60;

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone)]
struct AppState {
    database: PgPool,
    redis: redis::Client,
    telegram_bot_token: Arc<str>,
    encryption_key: Arc<[u8; 32]>,
    bootstrap_admin_telegram_ids: Arc<HashSet<i64>>,
    payment_providers: Vec<Arc<dyn PaymentProvider>>,
    subscription_public_url: Arc<str>,
}

#[derive(Debug, Serialize)]
struct AdminSetupStatusResponse {
    setup_required: bool,
}

#[derive(Debug, Deserialize)]
struct AdminCredentialsRequest {
    login: String,
    password: String,
}

#[derive(Debug, Serialize)]
struct AdminLoginResponse {
    access_token: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PublicSettings {
    mini_app_url: String,
    admin_url: String,
    subscription_public_url: String,
    telegram_webhook_url: String,
    cors_origins: Vec<String>,
    support_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[allow(clippy::struct_field_names)]
struct RuntimeSettings {
    api_host_port: u16,
    mini_app_host_port: u16,
    admin_host_port: u16,
    telegram_webhook_host_port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TelegramTransportSettings {
    mode: String,
}

#[derive(Debug, Serialize)]
struct HealthResponse {
    status: &'static str,
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    code: &'static str,
    message: &'static str,
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: &'static str,
}

impl ApiError {
    const fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
        }
    }

    const fn invalid(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }

    const fn unauthorized() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Authentication is required.",
        )
    }

    const fn conflict(message: &'static str) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }

    const fn unavailable(message: &'static str) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, "unavailable", message)
    }

    const fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "forbidden",
            "Administrator access is required.",
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
            }),
        )
            .into_response()
    }
}

#[derive(Debug, Deserialize)]
struct TelegramAuthRequest {
    init_data: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    access_token: String,
    expires_at: DateTime<Utc>,
    user_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct TelegramWebAppUser {
    id: i64,
    username: Option<String>,
    first_name: String,
    last_name: Option<String>,
    language_code: Option<String>,
}

#[derive(Debug, FromRow, Serialize)]
struct TariffResponse {
    id: Uuid,
    code: String,
    name: Value,
    description: Value,
    duration_seconds: Option<i64>,
    traffic_bytes: Option<i64>,
    amount_minor: i64,
    currency_code: String,
}

#[derive(Debug, FromRow, Serialize)]
struct MeResponse {
    telegram_user_id: i64,
    language_code: String,
    currency_code: String,
    balance_minor: i64,
}

#[derive(Debug, Deserialize)]
struct UpdateLanguageRequest {
    language_code: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct PurchaseRequest {
    tariff_id: Uuid,
    promo_code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PurchaseResponse {
    subscription_id: Uuid,
    status: String,
}

#[derive(Debug, Deserialize)]
struct CreateInvoiceRequest {
    provider: PaymentProviderCode,
    amount_minor: Option<i64>,
    currency_code: String,
    tariff_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct InvoiceResponse {
    id: Uuid,
    provider: PaymentProviderCode,
    payment_url: String,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize)]
struct InvoiceDetailResponse {
    id: Uuid,
    provider: String,
    purpose: String,
    status: String,
    currency_code: String,
    amount_minor: i64,
    expires_at: DateTime<Utc>,
    paid_at: Option<DateTime<Utc>>,
    subscription_id: Option<Uuid>,
}

#[derive(Debug, Serialize)]
struct PaymentProviderResponse {
    code: PaymentProviderCode,
    supported_currency_codes: Vec<&'static str>,
}

#[derive(Debug, FromRow, Serialize)]
struct CurrentSubscriptionResponse {
    id: Uuid,
    status: String,
    starts_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    traffic_bytes: Option<i64>,
    access_available: bool,
}

#[derive(Debug, Serialize)]
struct AccessLinkResponse {
    subscription_id: Uuid,
    access_url: String,
}

#[derive(Debug, Deserialize)]
struct SubscriptionGatewayQuery {
    token: String,
}

#[derive(Debug, FromRow)]
struct SubscriptionGatewayRecord {
    encrypted_access_url: Vec<u8>,
    expires_at: DateTime<Utc>,
    traffic_bytes: Option<i64>,
    language_code: String,
}

#[derive(Debug, Serialize)]
struct TrialResponse {
    subscription_id: Uuid,
    status: &'static str,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct TrialSettings {
    duration_seconds: i64,
    traffic_bytes: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ReferralSettings {
    percent: i64,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminPaymentProviderResponse {
    provider_code: String,
    is_enabled: bool,
    is_configured: bool,
}

#[derive(Debug, Deserialize)]
struct UpdatePaymentProviderRequest {
    is_enabled: bool,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminTariffResponse {
    id: Uuid,
    code: String,
    name: Value,
    description: Value,
    duration_seconds: Option<i64>,
    traffic_bytes: Option<i64>,
    position: i32,
    is_active: bool,
    amount_minor: i64,
    currency_code: String,
}

#[derive(Debug, Deserialize)]
struct UpsertTariffRequest {
    code: String,
    name: Value,
    description: Value,
    duration_seconds: Option<i64>,
    traffic_bytes: Option<i64>,
    position: i32,
    is_active: bool,
    amount_minor: i64,
    currency_code: String,
}

#[derive(Debug, FromRow, Serialize)]
struct RequiredChannelResponse {
    id: Uuid,
    telegram_chat_id: i64,
    title: String,
    public_url: Option<String>,
    is_active: bool,
}

#[derive(Debug, Deserialize)]
struct UpsertRequiredChannelRequest {
    telegram_chat_id: i64,
    title: String,
    public_url: Option<String>,
    is_active: bool,
}

#[derive(Debug, Deserialize)]
struct TelegramBotResponse<T> {
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct TelegramChatMember {
    status: String,
    #[serde(default)]
    is_member: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ListQuery {
    #[serde(default)]
    limit: Option<i64>,
    #[serde(default)]
    offset: Option<i64>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    action: Option<String>,
}

#[derive(Debug, Serialize)]
struct PageResponse<T> {
    items: Vec<T>,
    total: i64,
    limit: i64,
    offset: i64,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminUserResponse {
    id: Uuid,
    telegram_user_id: i64,
    username: Option<String>,
    first_name: String,
    language_code: String,
    balance_minor: i64,
    currency_code: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
struct CreateAdminUserRequest {
    telegram_user_id: i64,
    username: Option<String>,
    first_name: String,
    language_code: String,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminDailyMetric {
    day: NaiveDate,
    value: i64,
}

#[derive(Debug, Serialize)]
struct AdminAnalyticsResponse {
    registrations: Vec<AdminDailyMetric>,
    revenue_rub_minor: Vec<AdminDailyMetric>,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminSubscriptionResponse {
    id: Uuid,
    telegram_user_id: i64,
    username: Option<String>,
    tariff_code: Option<String>,
    status: String,
    starts_at: Option<DateTime<Utc>>,
    expires_at: Option<DateTime<Utc>>,
    traffic_bytes: Option<i64>,
    is_trial: bool,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminInvoiceResponse {
    id: Uuid,
    telegram_user_id: i64,
    username: Option<String>,
    provider: String,
    purpose: String,
    status: String,
    currency_code: String,
    amount_minor: i64,
    created_at: DateTime<Utc>,
    paid_at: Option<DateTime<Utc>>,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminDashboardResponse {
    registered_users: i64,
    active_subscriptions: i64,
    paid_invoices: i64,
    paid_revenue_rub_minor: i64,
    pending_invoices: i64,
    provisioning_pending_subscriptions: i64,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminAuditResponse {
    id: Uuid,
    actor_user_id: Option<Uuid>,
    action: String,
    target_type: String,
    target_id: Option<Uuid>,
    created_at: DateTime<Utc>,
}

#[derive(Debug, FromRow, Serialize)]
struct AdminPromoResponse {
    id: Uuid,
    code: String,
    kind: String,
    amount_minor: Option<i64>,
    discount_percent: Option<i16>,
    maximum_redemptions: Option<i32>,
    redeemed_count: i32,
    is_active: bool,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct UpsertPromoRequest {
    code: String,
    kind: String,
    amount_minor: Option<i64>,
    discount_percent: Option<i16>,
    maximum_redemptions: Option<i32>,
    is_active: bool,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct RedeemPromoRequest {
    code: String,
}

#[derive(Debug, Serialize)]
struct RedeemPromoResponse {
    code: String,
    credited_amount_minor: i64,
    currency_code: &'static str,
}

#[derive(Debug, Serialize)]
struct PromoPreviewResponse {
    code: String,
    kind: String,
    amount_minor: Option<i64>,
    discount_percent: Option<i16>,
}

#[derive(Clone, Copy)]
enum RateLimitKind {
    Standard,
    Authentication,
    Webhook,
    AdminMutation,
}

impl RateLimitKind {
    const fn key(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Authentication => "authentication",
            Self::Webhook => "webhook",
            Self::AdminMutation => "admin_mutation",
        }
    }

    const fn limit(self) -> i64 {
        match self {
            Self::Standard => 120,
            Self::Authentication => 10,
            Self::Webhook => 60,
            Self::AdminMutation => 30,
        }
    }
}

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let redis_url = env::var("REDIS_URL").context("REDIS_URL is required")?;
    let telegram_bot_token = env::var("TELEGRAM_BOT_TOKEN").unwrap_or_default();
    let encryption_key = load_encryption_key()?;
    let database = connect(&database_url)
        .await
        .context("database connection failed")?;
    ensure_trial_settings(&database)
        .await
        .context("trial settings initialization failed")?;
    ensure_referral_settings(&database)
        .await
        .context("referral settings initialization failed")?;
    let bind_addr: SocketAddr = env::var("API_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_owned())
        .parse()
        .context("API_BIND_ADDR must be a socket address")?;
    let cors = load_cors_layer()?;
    let subscription_public_url = env::var("SUBSCRIPTION_PUBLIC_URL")
        .or_else(|_| env::var("MINI_APP_PUBLIC_URL"))
        .unwrap_or_else(|_| "http://127.0.0.1:18081".to_owned());

    let state = Arc::new(AppState {
        database,
        redis: redis::Client::open(redis_url).context("REDIS_URL is invalid")?,
        telegram_bot_token: Arc::from(telegram_bot_token),
        encryption_key: Arc::new(encryption_key),
        bootstrap_admin_telegram_ids: Arc::new(load_bootstrap_admin_telegram_ids()?),
        payment_providers: build_payment_providers()?,
        subscription_public_url: Arc::from(subscription_public_url),
    });

    let app = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/sub/{id}", get(subscription_gateway))
        .route("/api/v1/openapi.json", get(openapi_document))
        .route("/api/v1/admin/setup-status", get(admin_setup_status))
        .route("/api/v1/admin/setup", post(create_root_admin))
        .route("/api/v1/admin/login", post(admin_login))
        .route("/api/v1/auth/telegram", post(authenticate_telegram))
        .route("/api/v1/me", get(me).put(update_language))
        .route("/api/v1/subscriptions/current", get(current_subscription))
        .route(
            "/api/v1/subscriptions/{id}/access",
            get(subscription_access),
        )
        .route(
            "/api/v1/admin/trial-settings",
            get(admin_trial_settings).put(update_admin_trial_settings),
        )
        .route(
            "/api/v1/admin/referral-settings",
            get(admin_referral_settings).put(update_admin_referral_settings),
        )
        .route(
            "/api/v1/admin/public-settings",
            get(admin_public_settings).put(update_admin_public_settings),
        )
        .route(
            "/api/v1/admin/runtime-settings",
            get(admin_runtime_settings).put(update_admin_runtime_settings),
        )
        .route(
            "/api/v1/admin/telegram-transport",
            get(admin_telegram_transport).put(update_admin_telegram_transport),
        )
        .route(
            "/api/v1/admin/payment-providers",
            get(admin_payment_providers),
        )
        .route(
            "/api/v1/admin/payment-providers/{provider}",
            put(update_admin_payment_provider),
        )
        .route(
            "/api/v1/admin/tariffs",
            get(admin_tariffs).post(create_admin_tariff),
        )
        .route("/api/v1/admin/tariffs/{id}", put(update_admin_tariff))
        .route(
            "/api/v1/admin/required-channels",
            get(admin_required_channels).post(create_admin_required_channel),
        )
        .route(
            "/api/v1/admin/required-channels/{id}",
            put(update_admin_required_channel),
        )
        .route(
            "/api/v1/admin/users",
            get(admin_users).post(create_admin_user),
        )
        .route("/api/v1/admin/dashboard", get(admin_dashboard))
        .route("/api/v1/admin/analytics", get(admin_analytics))
        .route("/api/v1/admin/audit", get(admin_audit))
        .route("/api/v1/admin/subscriptions", get(admin_subscriptions))
        .route("/api/v1/admin/invoices", get(admin_invoices))
        .route(
            "/api/v1/admin/promos",
            get(admin_promos).post(create_admin_promo),
        )
        .route("/api/v1/admin/promos/{id}", put(update_admin_promo))
        .route("/api/v1/tariffs", get(tariffs))
        .route("/api/v1/payment-providers", get(payment_providers))
        .route("/api/v1/purchases", post(purchase))
        .route("/api/v1/trials", post(activate_trial))
        .route("/api/v1/promos/redeem", post(redeem_promo))
        .route("/api/v1/promos/{code}/preview", get(preview_promo))
        .route("/api/v1/invoices", post(create_invoice))
        .route("/api/v1/invoices/{id}", get(invoice_detail))
        .route(
            "/api/v1/webhooks/payments/{provider}",
            post(payment_webhook),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, rate_limit))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new());

    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("API bind failed")?;
    info!(%bind_addr, "api started");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("API server failed")
}

fn load_cors_layer() -> Result<CorsLayer> {
    let origins = env::var("MINI_APP_ALLOWED_ORIGINS")
        .unwrap_or_else(|_| "http://127.0.0.1:18081,http://127.0.0.1:18082".to_owned())
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(|origin| {
            origin
                .parse::<HeaderValue>()
                .context("MINI_APP_ALLOWED_ORIGINS contains an invalid origin")
        })
        .collect::<Result<Vec<_>>>()?;
    if origins.is_empty() {
        anyhow::bail!("MINI_APP_ALLOWED_ORIGINS must contain at least one origin");
    }
    Ok(CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST, Method::PUT])
        .allow_headers([AUTHORIZATION, CONTENT_TYPE, "idempotency-key".parse()?]))
}

async fn rate_limit(
    State(state): State<Arc<AppState>>,
    ConnectInfo(peer_address): ConnectInfo<SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let Some(kind) = rate_limit_kind(request.method(), request.uri().path()) else {
        return next.run(request).await;
    };
    let window = Utc::now().timestamp().div_euclid(RATE_LIMIT_WINDOW_SECONDS);
    let key = format!("rate-limit:{}:{}:{}", kind.key(), peer_address.ip(), window);
    let mut connection = match state.redis.get_multiplexed_async_connection().await {
        Ok(connection) => connection,
        Err(error) => {
            tracing::error!(%error, "rate limit Redis connection failed");
            return ApiError::unavailable("Request protection is temporarily unavailable.")
                .into_response();
        }
    };
    let count = match connection.incr::<_, _, i64>(&key, 1).await {
        Ok(count) => count,
        Err(error) => {
            tracing::error!(%error, "rate limit increment failed");
            return ApiError::unavailable("Request protection is temporarily unavailable.")
                .into_response();
        }
    };
    if count == 1
        && connection
            .expire::<_, bool>(&key, RATE_LIMIT_WINDOW_SECONDS)
            .await
            .is_err()
    {
        return ApiError::unavailable("Request protection is temporarily unavailable.")
            .into_response();
    }
    if count > kind.limit() {
        let mut response = ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "Too many requests. Try again shortly.",
        )
        .into_response();
        response
            .headers_mut()
            .insert(RETRY_AFTER, HeaderValue::from_static("60"));
        return response;
    }
    next.run(request).await
}

fn rate_limit_kind(method: &Method, path: &str) -> Option<RateLimitKind> {
    if matches!(method, &Method::OPTIONS) || matches!(path, "/healthz" | "/readyz") {
        return None;
    }
    if path == "/api/v1/auth/telegram" {
        return Some(RateLimitKind::Authentication);
    }
    if path.starts_with("/api/v1/webhooks/") {
        return Some(RateLimitKind::Webhook);
    }
    if path.starts_with("/api/v1/admin/") && !matches!(method, &Method::GET) {
        return Some(RateLimitKind::AdminMutation);
    }
    Some(RateLimitKind::Standard)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

#[allow(clippy::too_many_lines)]
async fn openapi_document() -> Json<Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "VPN Bot v2 API",
            "version": "1.0.0",
            "description": "REST contract for the Telegram Mini App, administrator SPA, and payment webhooks. Monetary amounts are integer minor units."
        },
        "servers": [{"url": "/"}],
        "security": [{"bearerAuth": []}],
        "paths": {
            "/api/v1/auth/telegram": {"post": {
                "tags": ["Authentication"], "summary": "Verify Telegram Mini App init data and create a session", "security": [],
                "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/TelegramAuthRequest"}}}},
                "responses": {"200": {"description": "Session created", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AuthResponse"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "409": {"$ref": "#/components/responses/Conflict"}}
            }},
            "/api/v1/me": {
                "get": {"tags": ["Mini App"], "summary": "Get authenticated user profile", "responses": {"200": {"$ref": "#/components/responses/Me"}, "401": {"$ref": "#/components/responses/Unauthorized"}}},
                "put": {"tags": ["Mini App"], "summary": "Update language", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpdateLanguageRequest"}}}}, "responses": {"200": {"$ref": "#/components/responses/Me"}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "401": {"$ref": "#/components/responses/Unauthorized"}}}
            },
            "/api/v1/tariffs": {"get": {"tags": ["Mini App"], "summary": "List active tariffs", "responses": {"200": {"description": "Active tariffs", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/Tariff"}}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}}}},
            "/api/v1/payment-providers": {"get": {"tags": ["Payments"], "summary": "List enabled payment providers", "responses": {"200": {"description": "Enabled providers", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/PaymentProvider"}}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}}}},
            "/api/v1/subscriptions/current": {"get": {"tags": ["Subscriptions"], "summary": "Get current subscription", "responses": {"200": {"description": "Current subscription or null", "content": {"application/json": {"schema": {"oneOf": [{"$ref": "#/components/schemas/CurrentSubscription"}, {"type": "null"}]}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}}}},
            "/api/v1/subscriptions/{id}/access": {"get": {"tags": ["Subscriptions"], "summary": "Get protected subscription gateway URL", "parameters": [{"$ref": "#/components/parameters/UuidPath"}], "responses": {"200": {"description": "Gateway URL", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AccessLink"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "404": {"$ref": "#/components/responses/NotFound"}}}},
            "/api/v1/purchases": {"post": {"tags": ["Payments"], "summary": "Purchase a tariff from wallet", "parameters": [{"$ref": "#/components/parameters/IdempotencyKey"}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/PurchaseRequest"}}}}, "responses": {"200": {"description": "Purchase result", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/PurchaseResponse"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "401": {"$ref": "#/components/responses/Unauthorized"}, "409": {"$ref": "#/components/responses/Conflict"}}}},
            "/api/v1/trials": {"post": {"tags": ["Subscriptions"], "summary": "Activate an eligible trial", "responses": {"201": {"description": "Trial activated", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/TrialResponse"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "409": {"$ref": "#/components/responses/Conflict"}}}},
            "/api/v1/promos/{code}/preview": {"get": {"tags": ["Promos"], "summary": "Preview a promo", "parameters": [{"name": "code", "in": "path", "required": true, "schema": {"type": "string"}}], "responses": {"200": {"description": "Promo preview", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/PromoPreview"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "404": {"$ref": "#/components/responses/NotFound"}}}},
            "/api/v1/promos/redeem": {"post": {"tags": ["Promos"], "summary": "Redeem a balance promo", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/RedeemPromoRequest"}}}}, "responses": {"200": {"description": "Promo redeemed", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/RedeemPromoResponse"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "409": {"$ref": "#/components/responses/Conflict"}}}},
            "/api/v1/invoices": {"post": {"tags": ["Payments"], "summary": "Create a payment invoice", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/CreateInvoiceRequest"}}}}, "responses": {"201": {"description": "Invoice created", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Invoice"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "401": {"$ref": "#/components/responses/Unauthorized"}, "409": {"$ref": "#/components/responses/Conflict"}, "503": {"$ref": "#/components/responses/Unavailable"}}}},
            "/api/v1/invoices/{id}": {"get": {"tags": ["Payments"], "summary": "Get an owner-scoped invoice", "parameters": [{"$ref": "#/components/parameters/UuidPath"}], "responses": {"200": {"description": "Invoice", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/InvoiceDetail"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "404": {"$ref": "#/components/responses/NotFound"}}}},
            "/api/v1/webhooks/payments/{provider}": {"post": {"tags": ["Webhooks"], "summary": "Ingest a provider-verified payment webhook", "description": "Provider-specific signature validation is required. This endpoint does not use bearer sessions.", "security": [], "parameters": [{"name": "provider", "in": "path", "required": true, "schema": {"type": "string", "enum": ["crypto_pay", "anore", "telegram_stars"]}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"type": "object", "additionalProperties": true}}}}, "responses": {"200": {"description": "Webhook accepted or previously processed"}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "401": {"$ref": "#/components/responses/Unauthorized"}}}},
            "/api/v1/admin/dashboard": {"get": {"tags": ["Admin"], "summary": "Get admin operations aggregates", "responses": {"200": {"description": "Dashboard aggregates", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminDashboard"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/users": {"get": {"tags": ["Admin"], "summary": "List users with pagination and search", "parameters": [{"$ref": "#/components/parameters/ListLimit"}, {"$ref": "#/components/parameters/ListOffset"}, {"$ref": "#/components/parameters/SearchQuery"}], "responses": {"200": {"description": "User page", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UserPage"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/subscriptions": {"get": {"tags": ["Admin"], "summary": "List subscriptions with pagination and filters", "parameters": [{"$ref": "#/components/parameters/ListLimit"}, {"$ref": "#/components/parameters/ListOffset"}, {"$ref": "#/components/parameters/SearchQuery"}, {"$ref": "#/components/parameters/StatusQuery"}], "responses": {"200": {"description": "Subscription page", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/SubscriptionPage"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/invoices": {"get": {"tags": ["Admin"], "summary": "List invoices with pagination and filters", "parameters": [{"$ref": "#/components/parameters/ListLimit"}, {"$ref": "#/components/parameters/ListOffset"}, {"$ref": "#/components/parameters/SearchQuery"}, {"$ref": "#/components/parameters/StatusQuery"}, {"$ref": "#/components/parameters/ProviderQuery"}], "responses": {"200": {"description": "Invoice page", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminInvoicePage"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/audit": {"get": {"tags": ["Admin"], "summary": "List audit records with pagination and action filter", "parameters": [{"$ref": "#/components/parameters/ListLimit"}, {"$ref": "#/components/parameters/ListOffset"}, {"$ref": "#/components/parameters/ActionQuery"}], "responses": {"200": {"description": "Audit page", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AuditPage"}}}}, "401": {"$ref": "#/components/responses/Unauthorized"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/trial-settings": {"get": {"tags": ["Admin"], "summary": "Get trial settings", "responses": {"200": {"$ref": "#/components/responses/TrialSettings"}, "403": {"$ref": "#/components/responses/Forbidden"}}}, "put": {"tags": ["Admin"], "summary": "Update trial settings", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/TrialSettings"}}}}, "responses": {"200": {"$ref": "#/components/responses/TrialSettings"}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/referral-settings": {"get": {"tags": ["Admin"], "summary": "Get referral settings", "responses": {"200": {"$ref": "#/components/responses/ReferralSettings"}, "403": {"$ref": "#/components/responses/Forbidden"}}}, "put": {"tags": ["Admin"], "summary": "Update referral settings", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ReferralSettings"}}}}, "responses": {"200": {"$ref": "#/components/responses/ReferralSettings"}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/payment-providers": {"get": {"tags": ["Admin"], "summary": "List payment provider settings", "responses": {"200": {"description": "Provider settings", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/AdminPaymentProvider"}}}}}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/payment-providers/{provider}": {"put": {"tags": ["Admin"], "summary": "Enable or disable a configured payment provider", "parameters": [{"name": "provider", "in": "path", "required": true, "schema": {"type": "string"}}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpdatePaymentProviderRequest"}}}}, "responses": {"200": {"description": "Updated provider", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminPaymentProvider"}}}}, "403": {"$ref": "#/components/responses/Forbidden"}, "404": {"$ref": "#/components/responses/NotFound"}}}},
            "/api/v1/admin/tariffs": {"get": {"tags": ["Admin"], "summary": "List tariffs", "responses": {"200": {"description": "Tariffs", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/AdminTariff"}}}}}, "403": {"$ref": "#/components/responses/Forbidden"}}}, "post": {"tags": ["Admin"], "summary": "Create a tariff", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpsertTariffRequest"}}}}, "responses": {"201": {"description": "Tariff created", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminTariff"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/tariffs/{id}": {"put": {"tags": ["Admin"], "summary": "Update a tariff", "parameters": [{"$ref": "#/components/parameters/UuidPath"}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpsertTariffRequest"}}}}, "responses": {"200": {"description": "Tariff updated", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminTariff"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}, "404": {"$ref": "#/components/responses/NotFound"}}}},
            "/api/v1/admin/promos": {"get": {"tags": ["Admin"], "summary": "List promos", "responses": {"200": {"description": "Promos", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/AdminPromo"}}}}}, "403": {"$ref": "#/components/responses/Forbidden"}}}, "post": {"tags": ["Admin"], "summary": "Create a promo", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpsertPromoRequest"}}}}, "responses": {"201": {"description": "Promo created", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminPromo"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/promos/{id}": {"put": {"tags": ["Admin"], "summary": "Update a promo", "parameters": [{"$ref": "#/components/parameters/UuidPath"}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpsertPromoRequest"}}}}, "responses": {"200": {"description": "Promo updated", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/AdminPromo"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}, "404": {"$ref": "#/components/responses/NotFound"}}}},
            "/api/v1/admin/required-channels": {"get": {"tags": ["Admin"], "summary": "List required channels", "responses": {"200": {"description": "Required channels", "content": {"application/json": {"schema": {"type": "array", "items": {"$ref": "#/components/schemas/RequiredChannel"}}}}}, "403": {"$ref": "#/components/responses/Forbidden"}}}, "post": {"tags": ["Admin"], "summary": "Create a required channel", "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpsertRequiredChannelRequest"}}}}, "responses": {"201": {"description": "Channel created", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/RequiredChannel"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}}}},
            "/api/v1/admin/required-channels/{id}": {"put": {"tags": ["Admin"], "summary": "Update a required channel", "parameters": [{"$ref": "#/components/parameters/UuidPath"}], "requestBody": {"required": true, "content": {"application/json": {"schema": {"$ref": "#/components/schemas/UpsertRequiredChannelRequest"}}}}, "responses": {"200": {"description": "Channel updated", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/RequiredChannel"}}}}, "400": {"$ref": "#/components/responses/InvalidRequest"}, "403": {"$ref": "#/components/responses/Forbidden"}, "404": {"$ref": "#/components/responses/NotFound"}}}}
        },
        "components": {
            "securitySchemes": {"bearerAuth": {"type": "http", "scheme": "bearer", "bearerFormat": "opaque session token"}},
            "parameters": {
                "UuidPath": {"name": "id", "in": "path", "required": true, "schema": {"type": "string", "format": "uuid"}},
                "IdempotencyKey": {"name": "Idempotency-Key", "in": "header", "required": true, "schema": {"type": "string", "minLength": 1, "maxLength": 255}},
                "ListLimit": {"name": "limit", "in": "query", "schema": {"type": "integer", "minimum": 1, "maximum": 100, "default": 50}},
                "ListOffset": {"name": "offset", "in": "query", "schema": {"type": "integer", "minimum": 0, "default": 0}},
                "SearchQuery": {"name": "q", "in": "query", "schema": {"type": "string"}},
                "StatusQuery": {"name": "status", "in": "query", "schema": {"type": "string"}},
                "ProviderQuery": {"name": "provider", "in": "query", "schema": {"type": "string"}},
                "ActionQuery": {"name": "action", "in": "query", "schema": {"type": "string"}}
            },
            "responses": {
                "InvalidRequest": {"description": "Request validation failed", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}, "example": {"code": "invalid_request", "message": "Language must be ru or en."}}}},
                "Unauthorized": {"description": "Authentication is required or invalid", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}, "example": {"code": "unauthorized", "message": "Authentication is required."}}}},
                "Forbidden": {"description": "Administrator access is required", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}, "example": {"code": "forbidden", "message": "Administrator access is required."}}}},
                "NotFound": {"description": "Resource was not found", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}, "example": {"code": "not_found", "message": "Resource was not found."}}}},
                "Conflict": {"description": "Operation conflicts with current state", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}, "example": {"code": "conflict", "message": "The operation cannot be completed."}}}},
                "Unavailable": {"description": "A required dependency is temporarily unavailable", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Error"}, "example": {"code": "unavailable", "message": "The service is temporarily unavailable."}}}},
                "Me": {"description": "Authenticated user", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Me"}}}},
                "TrialSettings": {"description": "Trial settings", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/TrialSettings"}}}},
                "ReferralSettings": {"description": "Referral settings", "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ReferralSettings"}}}}
            },
            "schemas": {
                "Error": {"type": "object", "additionalProperties": false, "required": ["code", "message"], "properties": {"code": {"type": "string"}, "message": {"type": "string"}}},
                "TelegramAuthRequest": {"type": "object", "additionalProperties": false, "required": ["init_data"], "properties": {"init_data": {"type": "string", "minLength": 1}}},
                "AuthResponse": {"type": "object", "required": ["access_token", "expires_at", "user_id"], "properties": {"access_token": {"type": "string"}, "expires_at": {"type": "string", "format": "date-time"}, "user_id": {"type": "string", "format": "uuid"}}, "example": {"access_token": "session-token", "expires_at": "2026-07-25T12:00:00Z", "user_id": "018f6f7a-0e8f-7f72-8e16-5be37b8942f9"}},
                "Me": {"type": "object", "required": ["telegram_user_id", "language_code", "currency_code", "balance_minor"], "properties": {"telegram_user_id": {"type": "integer", "format": "int64"}, "language_code": {"type": "string", "enum": ["ru", "en"]}, "currency_code": {"type": "string"}, "balance_minor": {"type": "integer", "format": "int64"}}},
                "UpdateLanguageRequest": {"type": "object", "additionalProperties": false, "required": ["language_code"], "properties": {"language_code": {"type": "string", "enum": ["ru", "en"]}}},
                "Tariff": {"type": "object", "required": ["id", "code", "name", "description", "amount_minor", "currency_code"], "properties": {"id": {"type": "string", "format": "uuid"}, "code": {"type": "string"}, "name": {"type": "object", "additionalProperties": {"type": "string"}}, "description": {"type": "object", "additionalProperties": {"type": "string"}}, "duration_seconds": {"type": ["integer", "null"], "format": "int64"}, "traffic_bytes": {"type": ["integer", "null"], "format": "int64"}, "amount_minor": {"type": "integer", "format": "int64", "minimum": 1}, "currency_code": {"type": "string"}}},
                "PaymentProvider": {"type": "object", "required": ["code", "supported_currency_codes"], "properties": {"code": {"type": "string", "enum": ["crypto_pay", "anore", "telegram_stars"]}, "supported_currency_codes": {"type": "array", "items": {"type": "string"}}}},
                "CurrentSubscription": {"type": "object", "required": ["id", "status", "access_available"], "properties": {"id": {"type": "string", "format": "uuid"}, "status": {"type": "string"}, "starts_at": {"type": ["string", "null"], "format": "date-time"}, "expires_at": {"type": ["string", "null"], "format": "date-time"}, "traffic_bytes": {"type": ["integer", "null"], "format": "int64"}, "access_available": {"type": "boolean"}}},
                "AccessLink": {"type": "object", "required": ["subscription_id", "access_url"], "properties": {"subscription_id": {"type": "string", "format": "uuid"}, "access_url": {"type": "string", "format": "uri"}}},
                "PurchaseRequest": {"type": "object", "additionalProperties": false, "required": ["tariff_id"], "properties": {"tariff_id": {"type": "string", "format": "uuid"}, "promo_code": {"type": ["string", "null"]}}},
                "PurchaseResponse": {"type": "object", "required": ["subscription_id", "status"], "properties": {"subscription_id": {"type": "string", "format": "uuid"}, "status": {"type": "string"}}},
                "TrialResponse": {"type": "object", "required": ["subscription_id", "status"], "properties": {"subscription_id": {"type": "string", "format": "uuid"}, "status": {"type": "string", "const": "pending_provisioning"}}},
                "CreateInvoiceRequest": {"type": "object", "additionalProperties": false, "required": ["provider", "currency_code"], "properties": {"provider": {"type": "string", "enum": ["crypto_pay", "anore", "telegram_stars"]}, "amount_minor": {"type": ["integer", "null"], "format": "int64", "minimum": 1}, "currency_code": {"type": "string"}, "tariff_id": {"type": ["string", "null"], "format": "uuid"}}, "example": {"provider": "crypto_pay", "amount_minor": 19900, "currency_code": "RUB"}},
                "Invoice": {"type": "object", "required": ["id", "provider", "payment_url", "expires_at"], "properties": {"id": {"type": "string", "format": "uuid"}, "provider": {"type": "string"}, "payment_url": {"type": "string", "format": "uri"}, "expires_at": {"type": "string", "format": "date-time"}}},
                "InvoiceDetail": {"type": "object", "required": ["id", "provider", "purpose", "status", "currency_code", "amount_minor", "expires_at"], "properties": {"id": {"type": "string", "format": "uuid"}, "provider": {"type": "string"}, "purpose": {"type": "string"}, "status": {"type": "string"}, "currency_code": {"type": "string"}, "amount_minor": {"type": "integer", "format": "int64"}, "expires_at": {"type": "string", "format": "date-time"}, "paid_at": {"type": ["string", "null"], "format": "date-time"}, "subscription_id": {"type": ["string", "null"], "format": "uuid"}}},
                "RedeemPromoRequest": {"type": "object", "additionalProperties": false, "required": ["code"], "properties": {"code": {"type": "string", "minLength": 1}}},
                "RedeemPromoResponse": {"type": "object", "required": ["code", "credited_amount_minor", "currency_code"], "properties": {"code": {"type": "string"}, "credited_amount_minor": {"type": "integer", "format": "int64"}, "currency_code": {"type": "string", "const": "RUB"}}},
                "PromoPreview": {"type": "object", "required": ["code", "kind"], "properties": {"code": {"type": "string"}, "kind": {"type": "string"}, "amount_minor": {"type": ["integer", "null"], "format": "int64"}, "discount_percent": {"type": ["integer", "null"], "format": "int16", "minimum": 1, "maximum": 100}}},
                "TrialSettings": {"type": "object", "additionalProperties": false, "required": ["duration_seconds", "traffic_bytes"], "properties": {"duration_seconds": {"type": "integer", "minimum": 3_600, "maximum": 2_678_400}, "traffic_bytes": {"type": "integer", "minimum": 1_000_000, "maximum": 1_000_000_000_000_i64}}},
                "ReferralSettings": {"type": "object", "additionalProperties": false, "required": ["percent"], "properties": {"percent": {"type": "integer", "minimum": 1, "maximum": 100}}},
                "AdminPaymentProvider": {"type": "object", "required": ["provider_code", "is_enabled", "is_configured"], "properties": {"provider_code": {"type": "string"}, "is_enabled": {"type": "boolean"}, "is_configured": {"type": "boolean"}}},
                "UpdatePaymentProviderRequest": {"type": "object", "additionalProperties": false, "required": ["is_enabled"], "properties": {"is_enabled": {"type": "boolean"}}},
                "AdminTariff": {"allOf": [{"$ref": "#/components/schemas/Tariff"}, {"type": "object", "required": ["position", "is_active"], "properties": {"position": {"type": "integer", "format": "int32"}, "is_active": {"type": "boolean"}}}]},
                "UpsertTariffRequest": {"type": "object", "additionalProperties": false, "required": ["code", "name", "description", "position", "is_active", "amount_minor", "currency_code"], "properties": {"code": {"type": "string"}, "name": {"type": "object", "additionalProperties": {"type": "string"}}, "description": {"type": "object", "additionalProperties": {"type": "string"}}, "duration_seconds": {"type": ["integer", "null"], "format": "int64"}, "traffic_bytes": {"type": ["integer", "null"], "format": "int64"}, "position": {"type": "integer", "format": "int32"}, "is_active": {"type": "boolean"}, "amount_minor": {"type": "integer", "format": "int64", "minimum": 1}, "currency_code": {"type": "string"}}},
                "RequiredChannel": {"type": "object", "required": ["id", "telegram_chat_id", "title", "is_active"], "properties": {"id": {"type": "string", "format": "uuid"}, "telegram_chat_id": {"type": "integer", "format": "int64"}, "title": {"type": "string"}, "public_url": {"type": ["string", "null"], "format": "uri"}, "is_active": {"type": "boolean"}}},
                "UpsertRequiredChannelRequest": {"type": "object", "additionalProperties": false, "required": ["telegram_chat_id", "title", "is_active"], "properties": {"telegram_chat_id": {"type": "integer", "format": "int64"}, "title": {"type": "string", "minLength": 1}, "public_url": {"type": ["string", "null"], "format": "uri"}, "is_active": {"type": "boolean"}}},
                "AdminPromo": {"type": "object", "required": ["id", "code", "kind", "redeemed_count", "is_active"], "properties": {"id": {"type": "string", "format": "uuid"}, "code": {"type": "string"}, "kind": {"type": "string"}, "amount_minor": {"type": ["integer", "null"], "format": "int64"}, "discount_percent": {"type": ["integer", "null"], "format": "int16"}, "maximum_redemptions": {"type": ["integer", "null"], "format": "int32"}, "redeemed_count": {"type": "integer", "format": "int32"}, "is_active": {"type": "boolean"}, "starts_at": {"type": ["string", "null"], "format": "date-time"}, "ends_at": {"type": ["string", "null"], "format": "date-time"}}},
                "UpsertPromoRequest": {"type": "object", "additionalProperties": false, "required": ["code", "kind", "is_active"], "properties": {"code": {"type": "string"}, "kind": {"type": "string"}, "amount_minor": {"type": ["integer", "null"], "format": "int64"}, "discount_percent": {"type": ["integer", "null"], "format": "int16"}, "maximum_redemptions": {"type": ["integer", "null"], "format": "int32"}, "is_active": {"type": "boolean"}, "starts_at": {"type": ["string", "null"], "format": "date-time"}, "ends_at": {"type": ["string", "null"], "format": "date-time"}}},
                "AdminDashboard": {"type": "object", "required": ["registered_users", "active_subscriptions", "paid_invoices", "paid_revenue_rub_minor", "pending_invoices", "provisioning_pending_subscriptions"], "properties": {"registered_users": {"type": "integer", "format": "int64"}, "active_subscriptions": {"type": "integer", "format": "int64"}, "paid_invoices": {"type": "integer", "format": "int64"}, "paid_revenue_rub_minor": {"type": "integer", "format": "int64"}, "pending_invoices": {"type": "integer", "format": "int64"}, "provisioning_pending_subscriptions": {"type": "integer", "format": "int64"}}},
                "AdminUser": {"type": "object", "required": ["id", "telegram_user_id", "first_name", "language_code", "balance_minor", "currency_code", "created_at"], "properties": {"id": {"type": "string", "format": "uuid"}, "telegram_user_id": {"type": "integer", "format": "int64"}, "username": {"type": ["string", "null"]}, "first_name": {"type": "string"}, "language_code": {"type": "string"}, "balance_minor": {"type": "integer", "format": "int64"}, "currency_code": {"type": "string"}, "created_at": {"type": "string", "format": "date-time"}}},
                "AdminSubscription": {"type": "object", "required": ["id", "telegram_user_id", "status", "is_trial", "created_at"], "properties": {"id": {"type": "string", "format": "uuid"}, "telegram_user_id": {"type": "integer", "format": "int64"}, "username": {"type": ["string", "null"]}, "tariff_code": {"type": ["string", "null"]}, "status": {"type": "string"}, "starts_at": {"type": ["string", "null"], "format": "date-time"}, "expires_at": {"type": ["string", "null"], "format": "date-time"}, "traffic_bytes": {"type": ["integer", "null"], "format": "int64"}, "is_trial": {"type": "boolean"}, "created_at": {"type": "string", "format": "date-time"}}},
                "AdminInvoice": {"type": "object", "required": ["id", "telegram_user_id", "provider", "purpose", "status", "currency_code", "amount_minor", "created_at"], "properties": {"id": {"type": "string", "format": "uuid"}, "telegram_user_id": {"type": "integer", "format": "int64"}, "username": {"type": ["string", "null"]}, "provider": {"type": "string"}, "purpose": {"type": "string"}, "status": {"type": "string"}, "currency_code": {"type": "string"}, "amount_minor": {"type": "integer", "format": "int64"}, "created_at": {"type": "string", "format": "date-time"}, "paid_at": {"type": ["string", "null"], "format": "date-time"}}},
                "AuditRecord": {"type": "object", "required": ["id", "action", "target_type", "created_at"], "properties": {"id": {"type": "string", "format": "uuid"}, "actor_user_id": {"type": ["string", "null"], "format": "uuid"}, "action": {"type": "string"}, "target_type": {"type": "string"}, "target_id": {"type": ["string", "null"], "format": "uuid"}, "created_at": {"type": "string", "format": "date-time"}}},
                "Page": {"type": "object", "required": ["items", "total", "limit", "offset"], "properties": {"items": {"type": "array"}, "total": {"type": "integer", "format": "int64"}, "limit": {"type": "integer", "format": "int64"}, "offset": {"type": "integer", "format": "int64"}}},
                "UserPage": {"allOf": [{"$ref": "#/components/schemas/Page"}, {"type": "object", "properties": {"items": {"type": "array", "items": {"$ref": "#/components/schemas/AdminUser"}}}}]},
                "SubscriptionPage": {"allOf": [{"$ref": "#/components/schemas/Page"}, {"type": "object", "properties": {"items": {"type": "array", "items": {"$ref": "#/components/schemas/AdminSubscription"}}}}]},
                "AdminInvoicePage": {"allOf": [{"$ref": "#/components/schemas/Page"}, {"type": "object", "properties": {"items": {"type": "array", "items": {"$ref": "#/components/schemas/AdminInvoice"}}}}]},
                "AuditPage": {"allOf": [{"$ref": "#/components/schemas/Page"}, {"type": "object", "properties": {"items": {"type": "array", "items": {"$ref": "#/components/schemas/AuditRecord"}}}}]}
            }
        }
    }))
}

async fn ready(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    if is_ready(&state.database).await && redis_is_ready(&state.redis).await {
        (StatusCode::OK, Json(HealthResponse { status: "ready" }))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(HealthResponse {
                status: "not_ready",
            }),
        )
    }
}

async fn authenticate_telegram(
    State(state): State<Arc<AppState>>,
    Json(request): Json<TelegramAuthRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let telegram_user = verify_telegram_init_data(&request.init_data, &state.telegram_bot_token)?;
    reject_telegram_replay(&state.redis, &request.init_data).await?;

    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let user_id = upsert_user(&mut transaction, &telegram_user).await?;
    let (access_token, token_hash) = create_session_token();
    let expires_at = Utc::now() + ChronoDuration::hours(SESSION_TTL_HOURS);

    sqlx::query(
        "INSERT INTO user_sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;

    Ok(Json(AuthResponse {
        access_token,
        expires_at,
        user_id,
    }))
}

async fn admin_setup_status(
    State(state): State<Arc<AppState>>,
) -> Result<Json<AdminSetupStatusResponse>, ApiError> {
    let setup_required =
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM admin_accounts)")
            .fetch_one(&state.database)
            .await
            .map_err(database_error)?;
    Ok(Json(AdminSetupStatusResponse { setup_required }))
}

async fn create_root_admin(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AdminCredentialsRequest>,
) -> Result<(StatusCode, Json<AdminLoginResponse>), ApiError> {
    validate_admin_credentials(&request)?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let exists = sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM admin_accounts)")
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
    if exists {
        return Err(ApiError::conflict("The root administrator already exists."));
    }
    let user_id = Uuid::now_v7();
    let password_hash = hash_password(&request.password)?;
    sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(-1_i64)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, first_name, language_code) VALUES ($1, 'Root', 'ru')",
    )
    .bind(user_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query("INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')")
        .bind(Uuid::now_v7())
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    let role_id = Uuid::now_v7();
    let role_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO roles (id, code, name) VALUES ($1, 'super_admin', 'Super administrator')
         ON CONFLICT (code) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .bind(role_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query("INSERT INTO user_roles (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    sqlx::query("INSERT INTO admin_accounts (user_id, login, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(normalize_login(&request.login))
        .bind(password_hash)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    let response = create_admin_session(&mut transaction, user_id).await?;
    insert_audit_record(
        &mut transaction,
        user_id,
        "admin.root_created",
        "admin_account",
        user_id,
        None,
        json!({"login": normalize_login(&request.login)}),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn admin_login(
    State(state): State<Arc<AppState>>,
    Json(request): Json<AdminCredentialsRequest>,
) -> Result<Json<AdminLoginResponse>, ApiError> {
    let login = normalize_login(&request.login);
    let account = sqlx::query_as::<_, (Uuid, String)>(
        "SELECT user_id, password_hash FROM admin_accounts WHERE login = $1",
    )
    .bind(login)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(ApiError::unauthorized)?;
    verify_password(&request.password, &account.1)?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    sqlx::query("UPDATE admin_accounts SET last_login_at = now() WHERE user_id = $1")
        .bind(account.0)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    let response = create_admin_session(&mut transaction, account.0).await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(response))
}

async fn create_admin_session(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
) -> Result<AdminLoginResponse, ApiError> {
    let (access_token, token_hash) = create_session_token();
    let expires_at = Utc::now() + ChronoDuration::hours(SESSION_TTL_HOURS);
    sqlx::query(
        "INSERT INTO user_sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(user_id)
    .bind(token_hash)
    .bind(expires_at)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(AdminLoginResponse {
        access_token,
        expires_at,
    })
}

async fn admin_public_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<PublicSettings>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    Ok(Json(load_public_settings(&state.database).await?))
}

async fn update_admin_public_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<PublicSettings>,
) -> Result<Json<PublicSettings>, ApiError> {
    validate_public_settings(&settings)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    save_admin_setting(&state.database, actor_user_id, "public_settings", &settings).await?;
    Ok(Json(settings))
}

async fn admin_runtime_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<RuntimeSettings>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    Ok(Json(load_runtime_settings(&state.database).await?))
}

async fn update_admin_runtime_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<RuntimeSettings>,
) -> Result<Json<RuntimeSettings>, ApiError> {
    validate_runtime_settings(&settings)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    save_admin_setting(
        &state.database,
        actor_user_id,
        "runtime_settings",
        &settings,
    )
    .await?;
    Ok(Json(settings))
}

async fn admin_telegram_transport(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<TelegramTransportSettings>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    Ok(Json(
        load_telegram_transport_settings(&state.database).await?,
    ))
}

async fn update_admin_telegram_transport(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<TelegramTransportSettings>,
) -> Result<Json<TelegramTransportSettings>, ApiError> {
    validate_telegram_transport_settings(&settings)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    save_admin_setting(
        &state.database,
        actor_user_id,
        "telegram_transport_settings",
        &settings,
    )
    .await?;
    Ok(Json(settings))
}

async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    let user = sqlx::query_as::<_, MeResponse>(
        "SELECT u.telegram_user_id, p.language_code, w.currency_code, w.balance_minor
         FROM users u
         JOIN user_profiles p ON p.user_id = u.id
         JOIN wallets w ON w.user_id = u.id
         WHERE u.id = $1 AND u.deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(ApiError::unauthorized)?;
    Ok(Json(user))
}

async fn update_language(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpdateLanguageRequest>,
) -> Result<Json<MeResponse>, ApiError> {
    if !matches!(request.language_code.as_str(), "ru" | "en") {
        return Err(ApiError::invalid("Language must be ru or en."));
    }
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    sqlx::query(
        "UPDATE user_profiles SET language_code = $1, updated_at = now() WHERE user_id = $2",
    )
    .bind(&request.language_code)
    .bind(user_id)
    .execute(&state.database)
    .await
    .map_err(database_error)?;
    me(State(state), headers).await
}

async fn current_subscription(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Option<CurrentSubscriptionResponse>>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    let subscription = sqlx::query_as::<_, CurrentSubscriptionResponse>(
        "SELECT s.id, s.status, s.starts_at, s.expires_at, s.traffic_bytes,
                (a.encrypted_access_url IS NOT NULL) AS access_available
         FROM subscriptions s
         LEFT JOIN vpn_accounts a ON a.id = s.vpn_account_id
         WHERE s.user_id = $1
         ORDER BY CASE s.status WHEN 'active' THEN 0 WHEN 'provisioning_pending' THEN 1 ELSE 2 END,
                  s.created_at DESC
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(subscription))
}

async fn subscription_access(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(subscription_id): Path<Uuid>,
) -> Result<Json<AccessLinkResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    let access_available = sqlx::query_scalar::<_, bool>(
        "SELECT a.encrypted_access_url IS NOT NULL
         FROM subscriptions s
         JOIN vpn_accounts a ON a.id = s.vpn_account_id
         WHERE s.id = $1 AND s.user_id = $2 AND s.status = 'active'
            AND a.encrypted_access_url IS NOT NULL",
    )
    .bind(subscription_id)
    .bind(user_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Subscription access is unavailable."))?;
    if !access_available {
        return Err(ApiError::invalid("Subscription access is unavailable."));
    }
    Ok(Json(AccessLinkResponse {
        subscription_id,
        access_url: subscription_gateway_url(&state, subscription_id).await,
    }))
}

async fn subscription_gateway(
    State(state): State<Arc<AppState>>,
    Path(subscription_id): Path<Uuid>,
    Query(query): Query<SubscriptionGatewayQuery>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !constant_time_equal(
        query.token.as_bytes(),
        subscription_gateway_token(&state.encryption_key, subscription_id).as_bytes(),
    ) {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Subscription was not found.",
        ));
    }
    let subscription = sqlx::query_as::<_, SubscriptionGatewayRecord>(
        "SELECT a.encrypted_access_url, s.expires_at, s.traffic_bytes, p.language_code
         FROM subscriptions s
         JOIN vpn_accounts a ON a.id = s.vpn_account_id
         JOIN user_profiles p ON p.user_id = s.user_id
         WHERE s.id = $1 AND s.status = 'active' AND s.expires_at > now()
           AND a.encrypted_access_url IS NOT NULL",
    )
    .bind(subscription_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        ApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "Subscription was not found.",
        )
    })?;
    let public_url = subscription_gateway_url(&state, subscription_id).await;
    if is_browser_subscription_request(&headers) {
        return Ok((
            [
                ("cache-control", "no-store"),
                (CONTENT_TYPE.as_str(), "text/html; charset=utf-8"),
            ],
            Html(subscription_landing_page(
                &public_url,
                subscription.expires_at,
                subscription.traffic_bytes,
                &subscription.language_code,
            )),
        )
            .into_response());
    }
    let upstream_url =
        decrypt_access_url(&state.encryption_key, &subscription.encrypted_access_url).map_err(
            |_| ApiError::unavailable("Subscription access is temporarily unavailable."),
        )?;
    let payload = fetch_informative_subscription_payload(&upstream_url)
        .await
        .map_err(|_| ApiError::unavailable("Subscription access is temporarily unavailable."))?;
    let (payload, content_type) = render_subscription_for_client(&payload, &headers)
        .ok_or_else(|| ApiError::unavailable("Subscription access is temporarily unavailable."))?;
    Ok((
        [
            ("cache-control", "no-store"),
            (CONTENT_TYPE.as_str(), content_type),
        ],
        payload,
    )
        .into_response())
}

async fn tariffs(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<TariffResponse>>, ApiError> {
    let rows = sqlx::query_as::<_, TariffResponse>(
        "SELECT t.id, t.code, t.name, t.description, t.duration_seconds, t.traffic_bytes,
                p.amount_minor, p.currency_code
         FROM tariffs t
         JOIN tariff_prices p ON p.tariff_id = t.id AND p.is_active
         WHERE t.is_active
         ORDER BY t.position, t.created_at",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(rows))
}

async fn payment_providers(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PaymentProviderResponse>>, ApiError> {
    let enabled = sqlx::query_scalar::<_, String>(
        "SELECT provider_code FROM payment_provider_settings WHERE is_enabled ORDER BY provider_code",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    let providers = enabled
        .into_iter()
        .filter_map(|code| code.parse::<PaymentProviderCode>().ok())
        .filter(|code| {
            state
                .payment_providers
                .iter()
                .any(|provider| provider.code() == *code)
        })
        .map(|code| PaymentProviderResponse {
            code,
            supported_currency_codes: match code {
                PaymentProviderCode::TelegramStars => vec!["XTR"],
                PaymentProviderCode::CryptoPay | PaymentProviderCode::Anore => vec!["RUB"],
            },
        })
        .collect();
    Ok(Json(providers))
}

async fn purchase(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<PurchaseRequest>,
) -> Result<Json<PurchaseResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    ensure_required_channels(&state, user_id).await?;
    let idempotency_key = request_idempotency_key(&headers)?;
    let request_hash = sha256_bytes(
        serde_json::to_string(&request)
            .expect("purchase request is serializable")
            .as_bytes(),
    );
    let mut transaction = state.database.begin().await.map_err(database_error)?;

    if let Some(saved) =
        load_idempotent_purchase(&mut transaction, idempotency_key, &request_hash).await?
    {
        transaction.commit().await.map_err(database_error)?;
        return Ok(Json(saved));
    }

    let purchase = create_wallet_purchase(
        &mut transaction,
        user_id,
        request.tariff_id,
        request.promo_code.as_deref(),
    )
    .await?;
    store_idempotent_purchase(&mut transaction, idempotency_key, request_hash, &purchase).await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(purchase))
}

async fn activate_trial(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<TrialResponse>), ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    ensure_required_channels(&state, user_id).await?;
    let settings = load_trial_settings(&state.database).await?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
    let already_entitled = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM subscriptions WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    if already_entitled {
        return Err(ApiError::conflict("Trial is no longer available."));
    }
    let subscription_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO subscriptions (id, user_id, status, traffic_bytes, is_trial)
         VALUES ($1, $2, 'provisioning_pending', $3, true)",
    )
    .bind(subscription_id)
    .bind(user_id)
    .bind(settings.traffic_bytes)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
         VALUES ($1, 'subscription', $2, 'subscription.requested', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({
        "subscription_id": subscription_id,
        "user_id": user_id,
        "duration_seconds": settings.duration_seconds,
        "version": 1,
    }))
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(TrialResponse {
            subscription_id,
            status: "provisioning_pending",
        }),
    ))
}

async fn admin_trial_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<TrialSettings>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    Ok(Json(load_trial_settings(&state.database).await?))
}

async fn update_admin_trial_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<TrialSettings>,
) -> Result<Json<TrialSettings>, ApiError> {
    validate_trial_settings(&settings)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let previous = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM app_settings WHERE key = 'trial_settings' FOR UPDATE",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .unwrap_or_else(|| {
        serde_json::to_value(default_trial_settings()).expect("trial settings serialize")
    });
    let next = serde_json::to_value(&settings).expect("trial settings serialize");
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_by_user_id)
         VALUES ('trial_settings', $1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
           updated_by_user_id = EXCLUDED.updated_by_user_id, updated_at = now()",
    )
    .bind(&next)
    .bind(actor_user_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO audit_log
         (id, actor_user_id, action, target_type, before_value, after_value, correlation_id)
         VALUES ($1, $2, 'trial_settings.updated', 'app_setting', $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(actor_user_id)
    .bind(previous)
    .bind(next)
    .bind(Uuid::now_v7())
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(settings))
}

async fn admin_referral_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<ReferralSettings>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    Ok(Json(load_referral_settings(&state.database).await?))
}

async fn update_admin_referral_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(settings): Json<ReferralSettings>,
) -> Result<Json<ReferralSettings>, ApiError> {
    validate_referral_settings(&settings)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let previous = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM app_settings WHERE key = 'referral_settings' FOR UPDATE",
    )
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .unwrap_or_else(|| {
        serde_json::to_value(default_referral_settings()).expect("referral settings serialize")
    });
    let next = serde_json::to_value(&settings).expect("referral settings serialize");
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_by_user_id)
         VALUES ('referral_settings', $1, $2)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
           updated_by_user_id = EXCLUDED.updated_by_user_id, updated_at = now()",
    )
    .bind(&next)
    .bind(actor_user_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "referral_settings.updated",
        "app_setting",
        Uuid::nil(),
        Some(previous),
        next,
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(settings))
}

async fn admin_payment_providers(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminPaymentProviderResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let configured = state
        .payment_providers
        .iter()
        .map(|provider| provider.code().as_str())
        .collect::<HashSet<_>>();
    let mut providers = sqlx::query_as::<_, (String, bool)>(
        "SELECT provider_code, is_enabled FROM payment_provider_settings ORDER BY provider_code",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(|(provider_code, is_enabled)| AdminPaymentProviderResponse {
        is_configured: configured.contains(provider_code.as_str()),
        provider_code,
        is_enabled,
    })
    .collect::<Vec<_>>();
    providers.sort_unstable_by(|left, right| left.provider_code.cmp(&right.provider_code));
    Ok(Json(providers))
}

async fn update_admin_payment_provider(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Json(request): Json<UpdatePaymentProviderRequest>,
) -> Result<Json<AdminPaymentProviderResponse>, ApiError> {
    let provider_code = provider
        .parse::<PaymentProviderCode>()
        .map_err(|_| ApiError::invalid("Unknown payment provider."))?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let is_configured = state
        .payment_providers
        .iter()
        .any(|provider| provider.code() == provider_code);
    if request.is_enabled && !is_configured {
        return Err(ApiError::conflict(
            "Payment provider is not configured with its required secrets.",
        ));
    }
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let previous = sqlx::query_scalar::<_, Value>(
        "SELECT jsonb_build_object('is_enabled', is_enabled)
         FROM payment_provider_settings WHERE provider_code = $1 FOR UPDATE",
    )
    .bind(provider_code.as_str())
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Payment provider is not registered."))?;
    sqlx::query(
        "UPDATE payment_provider_settings SET is_enabled = $1, updated_by_user_id = $2, updated_at = now()
         WHERE provider_code = $3",
    )
    .bind(request.is_enabled)
    .bind(actor_user_id)
    .bind(provider_code.as_str())
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    let response = AdminPaymentProviderResponse {
        provider_code: provider_code.as_str().to_owned(),
        is_enabled: request.is_enabled,
        is_configured,
    };
    sqlx::query(
        "INSERT INTO audit_log
         (id, actor_user_id, action, target_type, before_value, after_value, correlation_id)
         VALUES ($1, $2, 'payment_provider.updated', 'payment_provider', $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(actor_user_id)
    .bind(previous)
    .bind(serde_json::to_value(&response).expect("provider response is serializable"))
    .bind(Uuid::now_v7())
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(response))
}

async fn admin_tariffs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminTariffResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let tariffs = sqlx::query_as::<_, AdminTariffResponse>(
        "SELECT t.id, t.code, t.name, t.description, t.duration_seconds, t.traffic_bytes,
                t.position, t.is_active, p.amount_minor, p.currency_code
         FROM tariffs t
         JOIN tariff_prices p ON p.tariff_id = t.id AND p.is_active
         ORDER BY t.position, t.created_at",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(tariffs))
}

async fn create_admin_tariff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpsertTariffRequest>,
) -> Result<(StatusCode, Json<AdminTariffResponse>), ApiError> {
    validate_tariff_request(&request)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let tariff_id = Uuid::now_v7();
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    sqlx::query(
        "INSERT INTO tariffs
         (id, code, name, description, duration_seconds, traffic_bytes, position, is_active)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(tariff_id)
    .bind(&request.code)
    .bind(&request.name)
    .bind(&request.description)
    .bind(request.duration_seconds)
    .bind(request.traffic_bytes)
    .bind(request.position)
    .bind(request.is_active)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO tariff_prices (id, tariff_id, currency_code, amount_minor)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(tariff_id)
    .bind(&request.currency_code)
    .bind(request.amount_minor)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    let response = tariff_response(tariff_id, &request);
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "tariff.created",
        "tariff",
        tariff_id,
        None,
        serde_json::to_value(&response).expect("tariff response is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn update_admin_tariff(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(tariff_id): Path<Uuid>,
    Json(request): Json<UpsertTariffRequest>,
) -> Result<Json<AdminTariffResponse>, ApiError> {
    validate_tariff_request(&request)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let previous = sqlx::query_as::<_, AdminTariffResponse>(
        "SELECT t.id, t.code, t.name, t.description, t.duration_seconds, t.traffic_bytes,
                t.position, t.is_active, p.amount_minor, p.currency_code
         FROM tariffs t JOIN tariff_prices p ON p.tariff_id = t.id AND p.is_active
         WHERE t.id = $1 FOR UPDATE",
    )
    .bind(tariff_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Tariff was not found."))?;
    sqlx::query(
        "UPDATE tariffs SET code = $1, name = $2, description = $3, duration_seconds = $4,
         traffic_bytes = $5, position = $6, is_active = $7, updated_at = now() WHERE id = $8",
    )
    .bind(&request.code)
    .bind(&request.name)
    .bind(&request.description)
    .bind(request.duration_seconds)
    .bind(request.traffic_bytes)
    .bind(request.position)
    .bind(request.is_active)
    .bind(tariff_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query("UPDATE tariff_prices SET is_active = false WHERE tariff_id = $1 AND is_active")
        .bind(tariff_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO tariff_prices (id, tariff_id, currency_code, amount_minor)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::now_v7())
    .bind(tariff_id)
    .bind(&request.currency_code)
    .bind(request.amount_minor)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    let response = tariff_response(tariff_id, &request);
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "tariff.updated",
        "tariff",
        tariff_id,
        Some(serde_json::to_value(previous).expect("tariff response is serializable")),
        serde_json::to_value(&response).expect("tariff response is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(response))
}

fn validate_tariff_request(request: &UpsertTariffRequest) -> Result<(), ApiError> {
    if request.code.is_empty()
        || request.code.len() > 64
        || !request.code.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(ApiError::invalid(
            "Tariff code must use lowercase letters, digits, and underscores.",
        ));
    }
    if request.amount_minor < 0 || request.currency_code.len() != 3 {
        return Err(ApiError::invalid("Tariff price is invalid."));
    }
    if request.duration_seconds.is_some_and(|value| value <= 0)
        || request.traffic_bytes.is_some_and(|value| value < 0)
    {
        return Err(ApiError::invalid(
            "Tariff duration or traffic limit is invalid.",
        ));
    }
    for locale in ["ru", "en"] {
        if request
            .name
            .get(locale)
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(ApiError::invalid(
                "Tariff name must contain RU and EN text.",
            ));
        }
    }
    if !request.description.is_object() {
        return Err(ApiError::invalid(
            "Tariff description must be localized text.",
        ));
    }
    Ok(())
}

fn tariff_response(tariff_id: Uuid, request: &UpsertTariffRequest) -> AdminTariffResponse {
    AdminTariffResponse {
        id: tariff_id,
        code: request.code.clone(),
        name: request.name.clone(),
        description: request.description.clone(),
        duration_seconds: request.duration_seconds,
        traffic_bytes: request.traffic_bytes,
        position: request.position,
        is_active: request.is_active,
        amount_minor: request.amount_minor,
        currency_code: request.currency_code.clone(),
    }
}

async fn insert_audit_record(
    transaction: &mut Transaction<'_, Postgres>,
    actor_user_id: Uuid,
    action: &str,
    target_type: &str,
    target_id: Uuid,
    before_value: Option<Value>,
    after_value: Value,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO audit_log
         (id, actor_user_id, action, target_type, target_id, before_value, after_value, correlation_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(Uuid::now_v7())
    .bind(actor_user_id)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(before_value)
    .bind(after_value)
    .bind(Uuid::now_v7())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn admin_required_channels(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<RequiredChannelResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let channels = sqlx::query_as::<_, RequiredChannelResponse>(
        "SELECT id, telegram_chat_id, title, public_url, is_active
         FROM required_channels ORDER BY created_at",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(channels))
}

async fn admin_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageResponse<AdminUserResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let (limit, offset) = list_pagination(&query);
    let search = search_pattern(query.q.as_deref());
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM users u JOIN user_profiles p ON p.user_id = u.id
         WHERE u.deleted_at IS NULL
           AND ($1::text IS NULL OR p.username ILIKE $1 OR p.first_name ILIKE $1
                OR u.telegram_user_id::text ILIKE $1)",
    )
    .bind(&search)
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    let users = sqlx::query_as::<_, AdminUserResponse>(
        "SELECT u.id, u.telegram_user_id, p.username, p.first_name, p.language_code,
                w.balance_minor, w.currency_code, u.created_at
          FROM users u JOIN user_profiles p ON p.user_id = u.id
          JOIN wallets w ON w.user_id = u.id
          WHERE u.deleted_at IS NULL
            AND ($1::text IS NULL OR p.username ILIKE $1 OR p.first_name ILIKE $1
                 OR u.telegram_user_id::text ILIKE $1)
          ORDER BY u.created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(search)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(PageResponse {
        items: users,
        total,
        limit,
        offset,
    }))
}

async fn create_admin_user(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateAdminUserRequest>,
) -> Result<(StatusCode, Json<AdminUserResponse>), ApiError> {
    if request.telegram_user_id <= 0
        || request.first_name.trim().is_empty()
        || request.first_name.chars().count() > 128
        || !matches!(request.language_code.as_str(), "ru" | "en")
        || request.username.as_ref().is_some_and(|username| {
            username.is_empty()
                || username.len() > 64
                || !username
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '_')
        })
    {
        return Err(ApiError::invalid("Client details are invalid."));
    }
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let user_id = Uuid::now_v7();
    let username = request
        .username
        .as_deref()
        .map(|value| value.trim_start_matches('@').to_owned())
        .filter(|value| !value.is_empty());
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let existing = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM users WHERE telegram_user_id = $1 AND deleted_at IS NULL)",
    )
    .bind(request.telegram_user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    if existing {
        return Err(ApiError::conflict(
            "A client with this Telegram ID already exists.",
        ));
    }
    sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(request.telegram_user_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, username, first_name, language_code)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(&username)
    .bind(request.first_name.trim())
    .bind(&request.language_code)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query("INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')")
        .bind(Uuid::now_v7())
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    let response = AdminUserResponse {
        id: user_id,
        telegram_user_id: request.telegram_user_id,
        username,
        first_name: request.first_name.trim().to_owned(),
        language_code: request.language_code,
        balance_minor: 0,
        currency_code: "RUB".to_owned(),
        created_at: Utc::now(),
    };
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "client.created",
        "client",
        user_id,
        None,
        serde_json::to_value(&response).expect("client response is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn admin_dashboard(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminDashboardResponse>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let dashboard = sqlx::query_as::<_, AdminDashboardResponse>(
        "SELECT
             (SELECT COUNT(*) FROM users WHERE deleted_at IS NULL) AS registered_users,
             (SELECT COUNT(*) FROM subscriptions WHERE status = 'active') AS active_subscriptions,
             (SELECT COUNT(*) FROM invoices WHERE status = 'paid') AS paid_invoices,
             (SELECT COALESCE(SUM(amount_minor), 0)::BIGINT FROM invoices
               WHERE status = 'paid' AND currency_code = 'RUB') AS paid_revenue_rub_minor,
             (SELECT COUNT(*) FROM invoices WHERE status = 'pending') AS pending_invoices,
             (SELECT COUNT(*) FROM subscriptions WHERE status = 'provisioning_pending')
               AS provisioning_pending_subscriptions",
    )
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(dashboard))
}

async fn admin_analytics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminAnalyticsResponse>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let registrations = sqlx::query_as::<_, AdminDailyMetric>(
        "SELECT day::date AS day,
                COALESCE((SELECT COUNT(*) FROM users u WHERE u.created_at >= day AND u.created_at < day + interval '1 day'), 0)::BIGINT AS value
         FROM generate_series(current_date - 13, current_date, interval '1 day') AS day
         ORDER BY day",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    let revenue_rub_minor = sqlx::query_as::<_, AdminDailyMetric>(
        "SELECT day::date AS day,
                COALESCE((SELECT SUM(i.amount_minor) FROM invoices i
                  WHERE i.status = 'paid' AND i.currency_code = 'RUB'
                    AND i.paid_at >= day AND i.paid_at < day + interval '1 day'), 0)::BIGINT AS value
         FROM generate_series(current_date - 13, current_date, interval '1 day') AS day
         ORDER BY day",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(AdminAnalyticsResponse {
        registrations,
        revenue_rub_minor,
    }))
}

async fn admin_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageResponse<AdminAuditResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let (limit, offset) = list_pagination(&query);
    let action = search_pattern(query.action.as_deref());
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audit_log WHERE $1::text IS NULL OR action ILIKE $1",
    )
    .bind(&action)
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    let entries = sqlx::query_as::<_, AdminAuditResponse>(
        "SELECT id, actor_user_id, action, target_type, target_id, created_at
         FROM audit_log WHERE $1::text IS NULL OR action ILIKE $1
         ORDER BY created_at DESC LIMIT $2 OFFSET $3",
    )
    .bind(action)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(PageResponse {
        items: entries,
        total,
        limit,
        offset,
    }))
}

async fn admin_subscriptions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageResponse<AdminSubscriptionResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let (limit, offset) = list_pagination(&query);
    let status = query.status.filter(|status| !status.trim().is_empty());
    let search = search_pattern(query.q.as_deref());
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM subscriptions s JOIN users u ON u.id = s.user_id
         JOIN user_profiles p ON p.user_id = u.id LEFT JOIN tariffs t ON t.id = s.tariff_id
         WHERE ($1::text IS NULL OR s.status = $1)
           AND ($2::text IS NULL OR p.username ILIKE $2 OR t.code ILIKE $2
                OR u.telegram_user_id::text ILIKE $2)",
    )
    .bind(&status)
    .bind(&search)
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    let subscriptions = sqlx::query_as::<_, AdminSubscriptionResponse>(
        "SELECT s.id, u.telegram_user_id, p.username, t.code AS tariff_code, s.status,
                s.starts_at, s.expires_at, s.traffic_bytes, s.is_trial, s.created_at
          FROM subscriptions s JOIN users u ON u.id = s.user_id
          JOIN user_profiles p ON p.user_id = u.id LEFT JOIN tariffs t ON t.id = s.tariff_id
          WHERE ($1::text IS NULL OR s.status = $1)
            AND ($2::text IS NULL OR p.username ILIKE $2 OR t.code ILIKE $2
                 OR u.telegram_user_id::text ILIKE $2)
          ORDER BY s.created_at DESC LIMIT $3 OFFSET $4",
    )
    .bind(status)
    .bind(search)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(PageResponse {
        items: subscriptions,
        total,
        limit,
        offset,
    }))
}

async fn admin_invoices(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<ListQuery>,
) -> Result<Json<PageResponse<AdminInvoiceResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let (limit, offset) = list_pagination(&query);
    let status = query.status.filter(|status| !status.trim().is_empty());
    let provider = query
        .provider
        .filter(|provider| !provider.trim().is_empty());
    let search = search_pattern(query.q.as_deref());
    let total = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)
         FROM invoices i JOIN users u ON u.id = i.user_id
         JOIN user_profiles p ON p.user_id = u.id
         WHERE ($1::text IS NULL OR i.status = $1)
           AND ($2::text IS NULL OR i.provider = $2)
           AND ($3::text IS NULL OR i.id::text ILIKE $3 OR p.username ILIKE $3
                OR u.telegram_user_id::text ILIKE $3)",
    )
    .bind(&status)
    .bind(&provider)
    .bind(&search)
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    let invoices = sqlx::query_as::<_, AdminInvoiceResponse>(
        "SELECT i.id, u.telegram_user_id, p.username, i.provider, i.purpose, i.status,
                i.currency_code, i.amount_minor, i.created_at, i.paid_at
          FROM invoices i JOIN users u ON u.id = i.user_id
          JOIN user_profiles p ON p.user_id = u.id
          WHERE ($1::text IS NULL OR i.status = $1)
            AND ($2::text IS NULL OR i.provider = $2)
            AND ($3::text IS NULL OR i.id::text ILIKE $3 OR p.username ILIKE $3
                 OR u.telegram_user_id::text ILIKE $3)
          ORDER BY i.created_at DESC LIMIT $4 OFFSET $5",
    )
    .bind(status)
    .bind(provider)
    .bind(search)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(PageResponse {
        items: invoices,
        total,
        limit,
        offset,
    }))
}

async fn admin_promos(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<AdminPromoResponse>>, ApiError> {
    authenticated_admin_id(&state, &headers).await?;
    let promos = sqlx::query_as::<_, AdminPromoResponse>(
        "SELECT id, code, kind, amount_minor, discount_percent, maximum_redemptions,
                redeemed_count, is_active, starts_at, ends_at
         FROM promo_codes ORDER BY created_at DESC",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    Ok(Json(promos))
}

async fn create_admin_promo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpsertPromoRequest>,
) -> Result<(StatusCode, Json<AdminPromoResponse>), ApiError> {
    validate_promo_request(&request)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let response = promo_response(Uuid::now_v7(), &request, 0);
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    sqlx::query(
        "INSERT INTO promo_codes
         (id, code, kind, amount_minor, discount_percent, maximum_redemptions, is_active, starts_at, ends_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(response.id)
    .bind(&response.code)
    .bind(&response.kind)
    .bind(response.amount_minor)
    .bind(response.discount_percent)
    .bind(response.maximum_redemptions)
    .bind(response.is_active)
    .bind(response.starts_at)
    .bind(response.ends_at)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "promo.created",
        "promo",
        response.id,
        None,
        serde_json::to_value(&response).expect("promo is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn update_admin_promo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(promo_id): Path<Uuid>,
    Json(request): Json<UpsertPromoRequest>,
) -> Result<Json<AdminPromoResponse>, ApiError> {
    validate_promo_request(&request)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let previous = sqlx::query_as::<_, AdminPromoResponse>(
        "SELECT id, code, kind, amount_minor, discount_percent, maximum_redemptions,
                redeemed_count, is_active, starts_at, ends_at
         FROM promo_codes WHERE id = $1 FOR UPDATE",
    )
    .bind(promo_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Promo code was not found."))?;
    let response = promo_response(promo_id, &request, previous.redeemed_count);
    sqlx::query(
        "UPDATE promo_codes SET code = $1, kind = $2, amount_minor = $3, discount_percent = $4,
         maximum_redemptions = $5, is_active = $6, starts_at = $7, ends_at = $8 WHERE id = $9",
    )
    .bind(&response.code)
    .bind(&response.kind)
    .bind(response.amount_minor)
    .bind(response.discount_percent)
    .bind(response.maximum_redemptions)
    .bind(response.is_active)
    .bind(response.starts_at)
    .bind(response.ends_at)
    .bind(promo_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "promo.updated",
        "promo",
        promo_id,
        Some(serde_json::to_value(previous).expect("promo is serializable")),
        serde_json::to_value(&response).expect("promo is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(response))
}

async fn redeem_promo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<RedeemPromoRequest>,
) -> Result<Json<RedeemPromoResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    let code = normalize_promo_code(&request.code)?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let promo = sqlx::query_as::<_, AdminPromoResponse>(
        "SELECT id, code, kind, amount_minor, discount_percent, maximum_redemptions,
                redeemed_count, is_active, starts_at, ends_at
         FROM promo_codes WHERE code = $1 FOR UPDATE",
    )
    .bind(&code)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Promo code is invalid."))?;
    if promo.kind != "balance" {
        return Err(ApiError::conflict(
            "This promo code must be applied during checkout.",
        ));
    }
    if !promo.is_active
        || promo
            .starts_at
            .is_some_and(|starts_at| starts_at > Utc::now())
        || promo.ends_at.is_some_and(|ends_at| ends_at <= Utc::now())
        || promo
            .maximum_redemptions
            .is_some_and(|maximum| promo.redeemed_count >= maximum)
    {
        return Err(ApiError::conflict("Promo code is unavailable."));
    }
    let amount_minor = promo
        .amount_minor
        .ok_or_else(|| ApiError::invalid("Promo code is invalid."))?;
    let wallet_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM wallets WHERE user_id = $1 AND currency_code = 'RUB' FOR UPDATE",
    )
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    let redemption_id = Uuid::now_v7();
    let wallet_transaction_id = Uuid::now_v7();
    let already_redeemed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM promo_redemptions WHERE promo_code_id = $1 AND user_id = $2)",
    )
    .bind(promo.id)
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(database_error)?;
    if already_redeemed {
        return Err(ApiError::conflict("Promo code was already redeemed."));
    }
    sqlx::query(
        "UPDATE wallets SET balance_minor = balance_minor + $1, updated_at = now() WHERE id = $2",
    )
    .bind(amount_minor)
    .bind(wallet_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO promo_redemptions (id, promo_code_id, user_id, wallet_transaction_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(redemption_id)
    .bind(promo.id)
    .bind(user_id)
    .bind(wallet_transaction_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO wallet_transactions
         (id, wallet_id, amount_minor, currency_code, kind, reference_type, reference_id)
         VALUES ($1, $2, $3, 'RUB', 'promo_credit', 'promo_redemption', $4)",
    )
    .bind(wallet_transaction_id)
    .bind(wallet_id)
    .bind(amount_minor)
    .bind(redemption_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query("UPDATE promo_codes SET redeemed_count = redeemed_count + 1 WHERE id = $1")
        .bind(promo.id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(RedeemPromoResponse {
        code: promo.code,
        credited_amount_minor: amount_minor,
        currency_code: "RUB",
    }))
}

async fn preview_promo(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<PromoPreviewResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    let code = normalize_promo_code(&code)?;
    let promo = sqlx::query_as::<_, AdminPromoResponse>(
        "SELECT id, code, kind, amount_minor, discount_percent, maximum_redemptions,
                redeemed_count, is_active, starts_at, ends_at
         FROM promo_codes WHERE code = $1",
    )
    .bind(&code)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Promo code is invalid."))?;
    if !promo.is_active
        || promo
            .starts_at
            .is_some_and(|starts_at| starts_at > Utc::now())
        || promo.ends_at.is_some_and(|ends_at| ends_at <= Utc::now())
        || promo
            .maximum_redemptions
            .is_some_and(|maximum| promo.redeemed_count >= maximum)
    {
        return Err(ApiError::conflict("Promo code is unavailable."));
    }
    let already_redeemed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM promo_redemptions WHERE promo_code_id = $1 AND user_id = $2)",
    )
    .bind(promo.id)
    .bind(user_id)
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    if already_redeemed {
        return Err(ApiError::conflict("Promo code was already redeemed."));
    }
    Ok(Json(PromoPreviewResponse {
        code: promo.code,
        kind: promo.kind,
        amount_minor: promo.amount_minor,
        discount_percent: promo.discount_percent,
    }))
}

fn validate_promo_request(request: &UpsertPromoRequest) -> Result<(), ApiError> {
    if normalize_promo_code(&request.code)? != request.code {
        return Err(ApiError::invalid(
            "Promo code must be uppercase letters, digits, or underscores.",
        ));
    }
    if request
        .starts_at
        .zip(request.ends_at)
        .is_some_and(|(starts, ends)| starts >= ends)
    {
        return Err(ApiError::invalid(
            "Promo end time must be after its start time.",
        ));
    }
    if request.maximum_redemptions.is_some_and(|value| value <= 0) {
        return Err(ApiError::invalid("Promo redemption limit is invalid."));
    }
    match request.kind.as_str() {
        "balance"
            if request.amount_minor.is_some_and(|amount| amount > 0)
                && request.discount_percent.is_none() =>
        {
            Ok(())
        }
        "discount"
            if request.amount_minor.is_none()
                && request
                    .discount_percent
                    .is_some_and(|percent| (0..=100).contains(&percent)) =>
        {
            Ok(())
        }
        _ => Err(ApiError::invalid("Promo configuration is invalid.")),
    }
}

fn normalize_promo_code(value: &str) -> Result<String, ApiError> {
    let code = value.trim().to_ascii_uppercase();
    if code.is_empty()
        || code.len() > 64
        || !code.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
    {
        return Err(ApiError::invalid("Promo code is invalid."));
    }
    Ok(code)
}

fn promo_response(
    id: Uuid,
    request: &UpsertPromoRequest,
    redeemed_count: i32,
) -> AdminPromoResponse {
    AdminPromoResponse {
        id,
        code: request.code.clone(),
        kind: request.kind.clone(),
        amount_minor: request.amount_minor,
        discount_percent: request.discount_percent,
        maximum_redemptions: request.maximum_redemptions,
        redeemed_count,
        is_active: request.is_active,
        starts_at: request.starts_at,
        ends_at: request.ends_at,
    }
}

fn list_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 100)
}

fn list_pagination(query: &ListQuery) -> (i64, i64) {
    (list_limit(query.limit), query.offset.unwrap_or(0).max(0))
}

fn search_pattern(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("%{value}%"))
}

async fn create_admin_required_channel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpsertRequiredChannelRequest>,
) -> Result<(StatusCode, Json<RequiredChannelResponse>), ApiError> {
    validate_required_channel_request(&request)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let response = RequiredChannelResponse {
        id: Uuid::now_v7(),
        telegram_chat_id: request.telegram_chat_id,
        title: request.title,
        public_url: request.public_url,
        is_active: request.is_active,
    };
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    sqlx::query(
        "INSERT INTO required_channels (id, telegram_chat_id, title, public_url, is_active)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(response.id)
    .bind(response.telegram_chat_id)
    .bind(&response.title)
    .bind(&response.public_url)
    .bind(response.is_active)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "required_channel.created",
        "required_channel",
        response.id,
        None,
        serde_json::to_value(&response).expect("required channel is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok((StatusCode::CREATED, Json(response)))
}

async fn update_admin_required_channel(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(channel_id): Path<Uuid>,
    Json(request): Json<UpsertRequiredChannelRequest>,
) -> Result<Json<RequiredChannelResponse>, ApiError> {
    validate_required_channel_request(&request)?;
    let actor_user_id = authenticated_admin_id(&state, &headers).await?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let previous = sqlx::query_as::<_, RequiredChannelResponse>(
        "SELECT id, telegram_chat_id, title, public_url, is_active
         FROM required_channels WHERE id = $1 FOR UPDATE",
    )
    .bind(channel_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Required channel was not found."))?;
    let response = RequiredChannelResponse {
        id: channel_id,
        telegram_chat_id: request.telegram_chat_id,
        title: request.title,
        public_url: request.public_url,
        is_active: request.is_active,
    };
    sqlx::query(
        "UPDATE required_channels SET telegram_chat_id = $1, title = $2, public_url = $3,
         is_active = $4 WHERE id = $5",
    )
    .bind(response.telegram_chat_id)
    .bind(&response.title)
    .bind(&response.public_url)
    .bind(response.is_active)
    .bind(channel_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "required_channel.updated",
        "required_channel",
        channel_id,
        Some(serde_json::to_value(previous).expect("required channel is serializable")),
        serde_json::to_value(&response).expect("required channel is serializable"),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(Json(response))
}

fn validate_required_channel_request(
    request: &UpsertRequiredChannelRequest,
) -> Result<(), ApiError> {
    if request.title.trim().is_empty() || request.title.chars().count() > 128 {
        return Err(ApiError::invalid("Required channel title is invalid."));
    }
    if let Some(public_url) = &request.public_url {
        let parsed = url::Url::parse(public_url)
            .map_err(|_| ApiError::invalid("Required channel URL is invalid."))?;
        if parsed.scheme() != "https" || parsed.host_str() != Some("t.me") {
            return Err(ApiError::invalid(
                "Required channel URL must be an HTTPS t.me link.",
            ));
        }
    }
    Ok(())
}

async fn ensure_required_channels(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    if is_admin_user(state, user_id).await? {
        return Ok(());
    }
    let channels = sqlx::query_as::<_, RequiredChannelResponse>(
        "SELECT id, telegram_chat_id, title, public_url, is_active
         FROM required_channels WHERE is_active ORDER BY created_at",
    )
    .fetch_all(&state.database)
    .await
    .map_err(database_error)?;
    if channels.is_empty() {
        return Ok(());
    }
    let telegram_user_id = sqlx::query_scalar::<_, i64>(
        "SELECT telegram_user_id FROM users WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(ApiError::unauthorized)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|_| ApiError::unavailable("Required channel check is unavailable."))?;
    for channel in channels {
        let response = client
            .get(format!(
                "https://api.telegram.org/bot{}/getChatMember",
                state.telegram_bot_token
            ))
            .query(&[
                ("chat_id", channel.telegram_chat_id),
                ("user_id", telegram_user_id),
            ])
            .send()
            .await
            .map_err(|_| ApiError::unavailable("Required channel check is unavailable."))?
            .error_for_status()
            .map_err(|_| ApiError::unavailable("Required channel check is unavailable."))?;
        let document: TelegramBotResponse<TelegramChatMember> = response
            .json()
            .await
            .map_err(|_| ApiError::unavailable("Required channel check is unavailable."))?;
        let member = document
            .result
            .filter(|_| document.ok)
            .ok_or_else(|| ApiError::unavailable("Required channel check is unavailable."))?;
        let is_member = matches!(
            member.status.as_str(),
            "creator" | "administrator" | "member"
        ) || (member.status == "restricted" && member.is_member == Some(true));
        if !is_member {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "required_channels",
                "Join the required channels before continuing.",
            ));
        }
    }
    Ok(())
}

async fn create_invoice(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Json(request): Json<CreateInvoiceRequest>,
) -> Result<(StatusCode, Json<InvoiceResponse>), ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    ensure_required_channels(&state, user_id).await?;
    let enabled: bool = sqlx::query_scalar(
        "SELECT is_enabled FROM payment_provider_settings WHERE provider_code = $1",
    )
    .bind(request.provider.as_str())
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .unwrap_or(false);
    if !enabled {
        return Err(ApiError::conflict("Payment provider is unavailable."));
    }
    let provider = state
        .payment_providers
        .iter()
        .find(|provider| provider.code() == request.provider)
        .ok_or_else(|| ApiError::unavailable("Payment provider is not configured."))?;
    let (purpose, amount_minor) = invoice_amount_and_purpose(&state.database, &request).await?;
    let amount = vpn_domain::Money::new(amount_minor)
        .map_err(|_| ApiError::invalid("Invoice amount is invalid."))?;
    let invoice_id = Uuid::now_v7();
    let provider_invoice = provider
        .create_invoice(&PaymentInvoiceRequest {
            local_invoice_id: invoice_id,
            amount,
            currency_code: request.currency_code.clone(),
            title: "VPN Bot".to_owned(),
            description: if purpose == "wallet_top_up" {
                "Wallet top-up".to_owned()
            } else {
                "VPN subscription".to_owned()
            },
        })
        .await
        .map_err(|_| ApiError::unavailable("Payment provider could not create an invoice."))?;
    let expires_at = provider_invoice
        .expires_at
        .unwrap_or_else(|| Utc::now() + ChronoDuration::minutes(30));
    sqlx::query(
        "INSERT INTO invoices
         (id, user_id, provider, provider_invoice_id, purpose, status, currency_code, amount_minor, expires_at)
         VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7, $8, $9)",
    )
    .bind(invoice_id)
    .bind(user_id)
    .bind(request.provider.as_str())
    .bind(&provider_invoice.provider_invoice_id)
    .bind(purpose)
    .bind(&request.currency_code)
    .bind(amount_minor)
    .bind(request.tariff_id)
    .bind(expires_at)
    .execute(&state.database)
    .await
    .map_err(database_error)?;
    Ok((
        StatusCode::CREATED,
        Json(InvoiceResponse {
            id: invoice_id,
            provider: request.provider,
            payment_url: provider_invoice.payment_url,
            expires_at,
        }),
    ))
}

async fn invoice_detail(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(invoice_id): Path<Uuid>,
) -> Result<Json<InvoiceDetailResponse>, ApiError> {
    let user_id = authenticated_user_id(&state.database, &headers).await?;
    let invoice = sqlx::query_as::<_, InvoiceDetailResponse>(
        "SELECT i.id, i.provider, i.purpose, i.status, i.currency_code, i.amount_minor,
                i.expires_at, i.paid_at, s.id AS subscription_id
         FROM invoices i
         LEFT JOIN subscriptions s ON s.source_invoice_id = i.id
         WHERE i.id = $1 AND i.user_id = $2",
    )
    .bind(invoice_id)
    .bind(user_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", "Invoice was not found."))?;
    Ok(Json(invoice))
}

#[allow(clippy::too_many_lines)]
async fn payment_webhook(
    State(state): State<Arc<AppState>>,
    Path(provider_code): Path<String>,
    headers: HeaderMap,
    raw_body: Bytes,
) -> Result<StatusCode, ApiError> {
    let provider_code = provider_code
        .parse::<PaymentProviderCode>()
        .map_err(|_| ApiError::invalid("Unknown payment provider."))?;
    let provider = state
        .payment_providers
        .iter()
        .find(|provider| provider.code() == provider_code)
        .ok_or_else(|| ApiError::unavailable("Payment provider is not configured."))?;
    let event = provider
        .verify_webhook(&headers, &raw_body)
        .map_err(|_| ApiError::unauthorized())?;
    if event.currency_code != "RUB" {
        return Err(ApiError::invalid("Webhook currency is unsupported."));
    }

    let payload: Value = serde_json::from_slice(&raw_body)
        .map_err(|_| ApiError::invalid("Webhook payload is invalid."))?;
    let mut transaction = state.database.begin().await.map_err(database_error)?;
    let webhook_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_webhook_events (id, provider, provider_event_id, payload)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (provider, provider_event_id) DO NOTHING
         RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(provider_code.as_str())
    .bind(&event.provider_event_id)
    .bind(payload)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?;
    let Some(webhook_id) = webhook_id else {
        transaction.commit().await.map_err(database_error)?;
        return Ok(StatusCode::OK);
    };

    let invoice = sqlx::query_as::<_, InvoiceForFulfillment>(
        "SELECT id, user_id, purpose, status, currency_code, amount_minor, tariff_id
         FROM invoices WHERE provider = $1 AND provider_invoice_id = $2 FOR UPDATE",
    )
    .bind(provider_code.as_str())
    .bind(&event.provider_invoice_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Webhook invoice is unknown."))?;
    if invoice.currency_code != event.currency_code || invoice.amount_minor != event.amount_minor {
        return Err(ApiError::invalid("Webhook amount does not match invoice."));
    }
    if invoice.status != "pending" {
        sqlx::query("UPDATE payment_webhook_events SET processed_at = now() WHERE id = $1")
            .bind(webhook_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        return Ok(StatusCode::OK);
    }
    if event.status != ProviderPaymentStatus::Paid {
        let status = match event.status {
            ProviderPaymentStatus::Expired => "expired",
            ProviderPaymentStatus::Cancelled => "cancelled",
            ProviderPaymentStatus::Pending
            | ProviderPaymentStatus::Unknown
            | ProviderPaymentStatus::Paid => "pending",
        };
        sqlx::query("UPDATE invoices SET status = $1, updated_at = now() WHERE id = $2")
            .bind(status)
            .bind(invoice.id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        sqlx::query("UPDATE payment_webhook_events SET processed_at = now() WHERE id = $1")
            .bind(webhook_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        return Ok(StatusCode::OK);
    }

    sqlx::query(
        "UPDATE invoices SET status = 'paid', paid_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(invoice.id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO payment_attempts
         (id, invoice_id, provider, provider_payment_id, status)
         VALUES ($1, $2, $3, $4, 'paid')
         ON CONFLICT (provider, provider_payment_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(invoice.id)
    .bind(provider_code.as_str())
    .bind(&event.provider_payment_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    fulfill_paid_invoice(&mut transaction, &invoice)
        .await
        .map_err(database_error)?;
    sqlx::query("UPDATE payment_webhook_events SET processed_at = now() WHERE id = $1")
        .bind(webhook_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
    transaction.commit().await.map_err(database_error)?;
    Ok(StatusCode::OK)
}

async fn invoice_amount_and_purpose(
    database: &PgPool,
    request: &CreateInvoiceRequest,
) -> Result<(&'static str, i64), ApiError> {
    if let Some(tariff_id) = request.tariff_id {
        if request.amount_minor.is_some() {
            return Err(ApiError::invalid(
                "Tariff invoices must not include a client-controlled amount.",
            ));
        }
        let amount = sqlx::query_scalar::<_, i64>(
            "SELECT p.amount_minor FROM tariffs t
             JOIN tariff_prices p ON p.tariff_id = t.id AND p.is_active
             WHERE t.id = $1 AND t.is_active AND p.currency_code = $2",
        )
        .bind(tariff_id)
        .bind(&request.currency_code)
        .fetch_optional(database)
        .await
        .map_err(database_error)?
        .ok_or_else(|| ApiError::invalid("Tariff is unavailable for this currency."))?;
        if amount <= 0 {
            return Err(ApiError::invalid("Tariff amount must be positive."));
        }
        Ok(("direct_purchase", amount))
    } else {
        let amount = request
            .amount_minor
            .filter(|amount| *amount > 0)
            .ok_or_else(|| ApiError::invalid("Invoice amount must be positive."))?;
        Ok(("wallet_top_up", amount))
    }
}

fn build_payment_providers() -> Result<Vec<Arc<dyn PaymentProvider>>> {
    let mut providers: Vec<Arc<dyn PaymentProvider>> = Vec::new();
    if let Ok(token) = env::var("CRYPTO_PAY_TOKEN")
        && !token.is_empty()
        && token != "replace-me"
    {
        providers.push(Arc::new(CryptoPayProvider::new(CryptoPayConfig {
            api_token: token,
            base_url: env::var("CRYPTO_PAY_BASE_URL")
                .unwrap_or_else(|_| "https://pay.crypt.bot/api".to_owned()),
        })?));
    }
    if let (Ok(api_key), Ok(signing_secret)) =
        (env::var("ANORE_API_KEY"), env::var("ANORE_SIGNING_SECRET"))
        && !api_key.is_empty()
        && !signing_secret.is_empty()
        && api_key != "replace-me"
    {
        providers.push(Arc::new(AnoreProvider::new(AnoreConfig {
            api_key,
            signing_secret,
            base_url: env::var("ANORE_BASE_URL")
                .unwrap_or_else(|_| "https://api.anore.cc/v1".to_owned()),
        })?));
    }
    if let Ok(token) = env::var("TELEGRAM_BOT_TOKEN")
        && !token.is_empty()
        && token != "replace-me"
    {
        providers.push(Arc::new(TelegramStarsProvider::new(TelegramStarsConfig {
            bot_token: token,
        })?));
    }
    Ok(providers)
}

async fn redis_is_ready(client: &redis::Client) -> bool {
    match client.get_multiplexed_async_connection().await {
        Ok(mut connection) => redis::cmd("PING")
            .query_async::<String>(&mut connection)
            .await
            .is_ok(),
        Err(_) => false,
    }
}

async fn reject_telegram_replay(client: &redis::Client, init_data: &str) -> Result<(), ApiError> {
    let key = format!(
        "telegram-init-data:{}",
        hex::encode(sha256_bytes(init_data.as_bytes()))
    );
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .map_err(|_| ApiError::unavailable("Authentication replay protection is unavailable."))?;
    let created: Option<String> = redis::cmd("SET")
        .arg(&key)
        .arg("1")
        .arg("NX")
        .arg("EX")
        .arg(INIT_DATA_MAX_AGE_SECONDS)
        .query_async(&mut connection)
        .await
        .map_err(|_| ApiError::unavailable("Authentication replay protection is unavailable."))?;
    if created.is_none() {
        return Err(ApiError::conflict("Telegram init data was already used."));
    }
    Ok(())
}

fn verify_telegram_init_data(
    init_data: &str,
    bot_token: &str,
) -> Result<TelegramWebAppUser, ApiError> {
    let mut fields = url::form_urlencoded::parse(init_data.as_bytes())
        .into_owned()
        .collect::<Vec<_>>();
    let hash_position = fields
        .iter()
        .position(|(key, _)| key == "hash")
        .ok_or_else(|| ApiError::invalid("Telegram init data does not contain a signature."))?;
    let received_hash = fields.remove(hash_position).1;
    fields.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    let data_check_string = fields
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut secret = <HmacSha256 as Mac>::new_from_slice(b"WebAppData")
        .expect("HMAC accepts a fixed-length key");
    secret.update(bot_token.as_bytes());
    let secret_key = secret.finalize().into_bytes();
    let mut signature = <HmacSha256 as Mac>::new_from_slice(&secret_key)
        .expect("HMAC accepts a SHA-256 output key");
    signature.update(data_check_string.as_bytes());
    let expected_hash = hex::encode(signature.finalize().into_bytes());
    if !constant_time_equal(expected_hash.as_bytes(), received_hash.as_bytes()) {
        return Err(ApiError::unauthorized());
    }

    let auth_date = fields
        .iter()
        .find(|(key, _)| key == "auth_date")
        .and_then(|(_, value)| value.parse::<i64>().ok())
        .ok_or_else(|| ApiError::invalid("Telegram init data does not contain auth_date."))?;
    let age = Utc::now().timestamp() - auth_date;
    if !(0..=INIT_DATA_MAX_AGE_SECONDS).contains(&age) {
        return Err(ApiError::unauthorized());
    }

    let user = fields
        .iter()
        .find(|(key, _)| key == "user")
        .map(|(_, value)| value)
        .ok_or_else(|| ApiError::invalid("Telegram init data does not contain a user."))?;
    serde_json::from_str(user).map_err(|_| ApiError::invalid("Telegram user data is invalid."))
}

fn create_session_token() -> (String, Vec<u8>) {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token = URL_SAFE_NO_PAD.encode(bytes);
    (token.clone(), sha256_bytes(token.as_bytes()))
}

fn sha256_bytes(value: &[u8]) -> Vec<u8> {
    Sha256::digest(value).to_vec()
}

fn load_encryption_key() -> Result<[u8; 32]> {
    let encoded =
        env::var("APPLICATION_ENCRYPTION_KEY").context("APPLICATION_ENCRYPTION_KEY is required")?;
    let bytes = STANDARD
        .decode(encoded)
        .context("APPLICATION_ENCRYPTION_KEY must be base64")?;
    bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("APPLICATION_ENCRYPTION_KEY must decode to 32 bytes"))
}

fn decrypt_access_url(key: &[u8; 32], encrypted_url: &[u8]) -> Result<String> {
    let (nonce, ciphertext) = encrypted_url
        .split_at_checked(12)
        .context("encrypted access URL is truncated")?;
    let cipher = Aes256Gcm::new_from_slice(key).expect("AES-256 uses a 32-byte key");
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| anyhow::anyhow!("access URL decryption failed"))?;
    String::from_utf8(plaintext).context("access URL is not UTF-8")
}

async fn subscription_gateway_url(state: &AppState, subscription_id: Uuid) -> String {
    let public_url = load_public_settings(&state.database)
        .await
        .ok()
        .filter(|settings| !settings.subscription_public_url.is_empty())
        .map_or_else(
            || state.subscription_public_url.to_string(),
            |settings| settings.subscription_public_url,
        );
    format!(
        "{}/sub/{subscription_id}?token={}",
        public_url.trim_end_matches('/'),
        subscription_gateway_token(&state.encryption_key, subscription_id),
    )
}

fn subscription_gateway_token(key: &[u8; 32], subscription_id: Uuid) -> String {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC accepts fixed encryption key");
    mac.update(subscription_id.as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

fn is_browser_subscription_request(headers: &HeaderMap) -> bool {
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let accept = headers
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let subscription_clients = [
        "happ/",
        "v2rayng",
        "streisand",
        "karing",
        "clash",
        "mihomo",
        "stash",
        "sing-box",
        "singbox",
        "v2rayn",
        "nekobox",
        "nekoray",
        "hiddify",
        "shadowrocket",
        "loon",
        "incy",
    ];
    !subscription_clients
        .iter()
        .any(|marker| user_agent.contains(marker))
        && accept.contains("text/html")
}

async fn fetch_informative_subscription_payload(url: &str) -> Result<String> {
    let parsed = url::Url::parse(url).context("subscription URL is invalid")?;
    if parsed.scheme() != "https" {
        anyhow::bail!("subscription URL must use HTTPS");
    }
    let response = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(15))
        .build()?
        .get(parsed)
        // Happ requests the rich Xray-style representation from Remnawave.
        .header("user-agent", "Happ/4.7.4/ios/2604141220584")
        .header("accept", "text/plain,*/*")
        .send()
        .await?
        .error_for_status()?;
    let bytes = response.bytes().await?;
    if bytes.len() > 2 * 1024 * 1024 {
        anyhow::bail!("subscription payload exceeds the size limit");
    }
    String::from_utf8(bytes.to_vec()).context("subscription payload is not UTF-8")
}

fn subscription_landing_page(
    gateway_url: &str,
    expires_at: DateTime<Utc>,
    traffic_bytes: Option<i64>,
    language_code: &str,
) -> String {
    let ru = language_code == "ru";
    let title = if ru {
        "Подписка VPN"
    } else {
        "VPN subscription"
    };
    let expires_label = if ru {
        "Доступ активен до"
    } else {
        "Access active until"
    };
    let guide_title = if ru {
        "Как подключиться"
    } else {
        "How to connect"
    };
    let copy_label = if ru {
        "Скопировать ссылку"
    } else {
        "Copy link"
    };
    let copied_label = if ru {
        "Ссылка скопирована"
    } else {
        "Link copied"
    };
    let traffic = traffic_bytes.map_or_else(String::new, |bytes| {
        let gigabytes = bytes / 1_000_000_000;
        if ru {
            format!("<p>Лимит трафика: {gigabytes} GB</p>")
        } else {
            format!("<p>Traffic limit: {gigabytes} GB</p>")
        }
    });
    format!(
        "<!doctype html><html lang=\"{}\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"theme-color\" content=\"#080d1f\"><title>{title}</title><style>:root{{color-scheme:dark;font-family:Inter,ui-sans-serif,system-ui,sans-serif;color:#edf4ff;background:#080d1f}}*{{box-sizing:border-box}}body{{min-width:320px;margin:0;background:radial-gradient(circle at 90% -10%,#193c64 0,transparent 40%),#080d1f}}main{{width:min(100%,620px);min-height:100vh;margin:auto;padding:calc(18px + env(safe-area-inset-top)) 18px calc(28px + env(safe-area-inset-bottom))}}.topbar{{display:flex;align-items:center;justify-content:space-between;margin-bottom:28px}}.wordmark{{margin:0;font-size:13px;font-weight:900;letter-spacing:.15em}}.mark{{display:inline-grid;place-items:center;width:25px;height:25px;margin-right:7px;background:#b8ff60;color:#07101d;font-style:normal;clip-path:polygon(50% 0,100% 20%,100% 80%,50% 100%,0 80%,0 20%)}}.eyebrow{{margin:0 0 10px;color:#9fb2d6;font-size:11px;font-weight:800;letter-spacing:.13em}}h1{{margin:0;font-size:clamp(39px,11vw,58px);line-height:.92;letter-spacing:-.07em}}h2{{margin:0;font-size:24px;letter-spacing:-.04em}}.route-card,.guide-card{{position:relative;overflow:hidden;border:1px solid #273653;border-radius:24px;background:rgba(15,24,47,.88)}}.route-card{{min-height:365px;margin-top:20px;padding:29px}}.grid{{position:absolute;inset:0;opacity:.24;background-image:linear-gradient(#385070 1px,transparent 1px),linear-gradient(90deg,#385070 1px,transparent 1px);background-size:30px 30px;mask-image:linear-gradient(to bottom,black,transparent 75%)}}.route-card>*:not(.grid){{position:relative}}.status{{display:flex;align-items:center;gap:9px;margin:45px 0 8px;color:#c4d0e4;font-size:14px}}.status i{{width:9px;height:9px;border-radius:50%;background:#60ddab;box-shadow:0 0 0 5px rgba(96,221,171,.12)}}.expiry{{display:block;font-size:clamp(25px,7vw,34px);letter-spacing:-.055em}}.traffic{{margin:10px 0 0;color:#9fb2d6;font-size:14px}}code{{display:block;max-height:78px;margin-top:28px;overflow:auto;overflow-wrap:anywhere;padding:13px 14px;border:1px solid #2b3b59;border-radius:13px;background:#091125;color:#c4d0e4;font:12px ui-monospace,SFMono-Regular,monospace}}button{{width:100%;margin-top:14px;border:0;border-radius:14px;padding:15px 18px;background:#b8ff60;color:#07101d;font:800 14px inherit;cursor:pointer}}button:active{{transform:scale(.98)}}.guide-card{{margin-top:16px;padding:24px}}.guide-card ol{{margin:18px 0 0;padding-left:20px;color:#c4d0e4;line-height:1.5}}.guide-card li+li{{margin-top:10px}}@media (min-width:640px){{main{{padding-top:30px}}}}</style><main><header class=\"topbar\"><p class=\"wordmark\"><i class=\"mark\">V</i>VECTOR</p><p class=\"eyebrow\">SECURE ROUTE / 04</p></header><p class=\"eyebrow\">ACCESS STATUS</p><h1>{title}</h1><section class=\"route-card\"><div class=\"grid\"></div><p class=\"eyebrow\">ACTIVE CONNECTION</p><div class=\"status\"><i></i>{expires_label}</div><strong class=\"expiry\">{}</strong>{traffic}<code id=\"link\">{gateway_url}</code><button onclick=\"navigator.clipboard.writeText(document.querySelector('#link').textContent).then(()=>this.textContent='{copied_label}')\">{copy_label}</button></section><section class=\"guide-card\"><p class=\"eyebrow\">01 / 02 / 03</p><h2>{guide_title}</h2><ol><li>{}</li><li>{}</li><li>{}</li></ol></section></main></html>",
        if ru { "ru" } else { "en" },
        expires_at.format("%Y-%m-%d %H:%M UTC"),
        if ru {
            "Установите Happ, v2rayNG, Karing или Clash."
        } else {
            "Install Happ, v2rayNG, Karing, or Clash."
        },
        if ru {
            "Откройте эту ссылку в выбранном клиенте."
        } else {
            "Open this link in the selected client."
        },
        if ru {
            "Клиент автоматически получит совместимую конфигурацию."
        } else {
            "The client will receive a compatible configuration automatically."
        },
    )
}

#[derive(Clone, Copy)]
enum SubscriptionOutput {
    Base64,
    XrayJson,
    SingboxJson,
    ClashYaml,
}

#[derive(Debug, Clone)]
struct SubscriptionNode {
    raw: String,
    scheme: String,
    credential: String,
    host: String,
    port: Option<u16>,
    params: BTreeMap<String, String>,
    remark: String,
}

fn render_subscription_for_client(
    payload: &str,
    headers: &HeaderMap,
) -> Option<(String, &'static str)> {
    let nodes = extract_subscription_nodes(payload);
    if nodes.is_empty() {
        return None;
    }
    match subscription_output_for_headers(headers) {
        SubscriptionOutput::Base64 => Some((
            render_base64_subscription(&nodes),
            "text/plain; charset=utf-8",
        )),
        SubscriptionOutput::XrayJson => Some((
            render_xray_subscription(&nodes),
            "application/json; charset=utf-8",
        )),
        SubscriptionOutput::SingboxJson => Some((
            render_singbox_subscription(&nodes),
            "application/json; charset=utf-8",
        )),
        SubscriptionOutput::ClashYaml => Some((
            render_clash_subscription(&nodes),
            "text/yaml; charset=utf-8",
        )),
    }
}

fn subscription_output_for_headers(headers: &HeaderMap) -> SubscriptionOutput {
    let user_agent = headers
        .get("user-agent")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let accept = headers
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if ["clash", "mihomo", "stash"]
        .iter()
        .any(|marker| user_agent.contains(marker))
        || accept.contains("text/yaml")
        || accept.contains("application/yaml")
    {
        return SubscriptionOutput::ClashYaml;
    }
    if user_agent.contains("karing") {
        return SubscriptionOutput::SingboxJson;
    }
    if ["happ/", "v2rayng", "streisand", "incy"]
        .iter()
        .any(|marker| user_agent.contains(marker))
        || accept.contains("application/json")
    {
        return SubscriptionOutput::XrayJson;
    }
    SubscriptionOutput::Base64
}

fn extract_subscription_nodes(payload: &str) -> Vec<SubscriptionNode> {
    let decoded = decode_subscription_payload(payload);
    let raw_nodes = extract_nodes_from_json(&decoded).unwrap_or_else(|| {
        decoded
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(str::to_owned)
            .collect()
    });
    let mut seen = HashSet::new();
    raw_nodes
        .into_iter()
        .filter_map(parse_subscription_node)
        .filter(|node| seen.insert(node.raw.clone()))
        .collect()
}

fn decode_subscription_payload(payload: &str) -> String {
    let text = payload.trim();
    if text.is_empty()
        || text.starts_with('[')
        || text.starts_with('{')
        || text.contains("://")
        || text.contains('\n')
    {
        return text.to_owned();
    }
    let compact = text.split_whitespace().collect::<String>();
    let padded = format!("{}{}", compact, "=".repeat((4 - compact.len() % 4) % 4));
    URL_SAFE_NO_PAD
        .decode(compact.as_bytes())
        .or_else(|_| STANDARD.decode(padded.as_bytes()))
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .filter(|decoded| !decoded.trim().is_empty())
        .unwrap_or_else(|| text.to_owned())
}

fn parse_subscription_node(raw: String) -> Option<SubscriptionNode> {
    let parsed = url::Url::parse(&raw).ok()?;
    let scheme = parsed.scheme().to_ascii_lowercase();
    if !matches!(scheme.as_str(), "vless" | "hysteria2" | "hysteria") {
        return None;
    }
    let host = parsed.host_str()?.to_owned();
    let credential = parsed.username().to_owned();
    if credential.is_empty() {
        return None;
    }
    Some(SubscriptionNode {
        raw,
        scheme,
        credential,
        host,
        port: parsed.port(),
        params: parsed.query_pairs().into_owned().collect(),
        remark: parsed.fragment().unwrap_or_default().to_owned(),
    })
}

fn extract_nodes_from_json(payload: &str) -> Option<Vec<String>> {
    let configs: Vec<Value> = match serde_json::from_str::<Value>(payload).ok()? {
        Value::Array(configs) => configs,
        config @ Value::Object(_) => vec![config],
        _ => return None,
    };
    let mut nodes = Vec::new();
    for config in configs {
        let remark = config
            .get("remarks")
            .or_else(|| config.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("VPN");
        let Some(outbounds) = config.get("outbounds").and_then(Value::as_array) else {
            continue;
        };
        for outbound in outbounds {
            if let Some(node) = xray_outbound_to_uri(outbound, remark) {
                nodes.push(node);
            }
        }
    }
    Some(nodes)
}

#[allow(clippy::too_many_lines)]
fn xray_outbound_to_uri(outbound: &Value, remark: &str) -> Option<String> {
    let protocol = outbound.get("protocol")?.as_str()?;
    if protocol == "vless" {
        let target = outbound
            .get("settings")?
            .get("vnext")?
            .as_array()?
            .first()?;
        let user = target.get("users")?.as_array()?.first()?;
        let credential = user.get("id")?.as_str()?;
        let host = target.get("address")?.as_str()?;
        let port = target.get("port")?.as_i64()?;
        let stream = outbound.get("streamSettings").unwrap_or(&Value::Null);
        let mut query = vec![
            (
                "encryption".to_owned(),
                user.get("encryption")
                    .and_then(Value::as_str)
                    .unwrap_or("none")
                    .to_owned(),
            ),
            (
                "type".to_owned(),
                stream
                    .get("network")
                    .and_then(Value::as_str)
                    .unwrap_or("tcp")
                    .to_owned(),
            ),
        ];
        if let Some(flow) = user
            .get("flow")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            query.push(("flow".to_owned(), flow.to_owned()));
        }
        if let Some(security) = stream
            .get("security")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            query.push(("security".to_owned(), security.to_owned()));
            let settings = if security == "reality" {
                stream.get("realitySettings")
            } else if security == "tls" {
                stream.get("tlsSettings")
            } else {
                None
            };
            if let Some(settings) = settings {
                for (source, target) in [
                    ("serverName", "sni"),
                    ("fingerprint", "fp"),
                    ("publicKey", "pbk"),
                    ("shortId", "sid"),
                ] {
                    if let Some(value) = settings.get(source).and_then(Value::as_str) {
                        query.push((target.to_owned(), value.to_owned()));
                    }
                }
                if let Some(alpn) = settings.get("alpn").and_then(Value::as_array) {
                    let value = alpn
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(",");
                    if !value.is_empty() {
                        query.push(("alpn".to_owned(), value));
                    }
                }
            }
        }
        let network = stream
            .get("network")
            .and_then(Value::as_str)
            .unwrap_or("tcp");
        let transport = match network {
            "ws" => stream.get("wsSettings"),
            "grpc" => stream.get("grpcSettings"),
            "xhttp" => stream.get("xhttpSettings"),
            "splithttp" => stream.get("splithttpSettings"),
            _ => None,
        };
        if let Some(transport) = transport {
            for (source, target) in [
                ("path", "path"),
                ("host", "host"),
                ("mode", "mode"),
                ("serviceName", "serviceName"),
            ] {
                if let Some(value) = transport.get(source).and_then(Value::as_str) {
                    query.push((target.to_owned(), value.to_owned()));
                }
            }
            if let Some(host) = transport
                .get("headers")
                .and_then(|headers| headers.get("Host"))
                .and_then(Value::as_str)
            {
                query.push(("host".to_owned(), host.to_owned()));
            }
        }
        return Some(format!(
            "vless://{credential}@{host}:{port}?{}#{}",
            url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(query)
                .finish(),
            url::form_urlencoded::byte_serialize(remark.as_bytes()).collect::<String>(),
        ));
    }
    None
}

fn render_base64_subscription(nodes: &[SubscriptionNode]) -> String {
    STANDARD.encode(format!(
        "{}\n",
        nodes
            .iter()
            .map(|node| node.raw.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    ))
}

fn render_xray_subscription(nodes: &[SubscriptionNode]) -> String {
    let configs = nodes
        .iter()
        .filter_map(xray_config_for_node)
        .collect::<Vec<_>>();
    serde_json::to_string(&configs).unwrap_or_else(|_| "[]".to_owned())
}

fn xray_config_for_node(node: &SubscriptionNode) -> Option<Value> {
    let outbound = if node.scheme == "vless" {
        let network = node.params.get("type").map_or("tcp", String::as_str);
        let mut stream = json!({"network": network});
        if let Some(security) = node
            .params
            .get("security")
            .filter(|value| value.as_str() != "none")
        {
            stream["security"] = Value::String(security.clone());
            if security == "reality" {
                stream["realitySettings"] = json!({
                    "serverName": node.params.get("sni").cloned().unwrap_or_default(),
                    "fingerprint": node.params.get("fp").cloned().unwrap_or_default(),
                    "publicKey": node.params.get("pbk").cloned().unwrap_or_default(),
                    "shortId": node.params.get("sid").cloned().unwrap_or_default(),
                });
            } else if security == "tls" {
                stream["tlsSettings"] = json!({
                    "serverName": node.params.get("sni").cloned().unwrap_or_default(),
                    "fingerprint": node.params.get("fp").cloned().unwrap_or_default(),
                    "alpn": comma_separated_param(node, "alpn"),
                });
            }
        }
        if matches!(network, "ws" | "xhttp" | "splithttp") {
            let settings = json!({
                "path": node.params.get("path").cloned().unwrap_or_default(),
                "host": node.params.get("host").cloned().unwrap_or_default(),
                "mode": node.params.get("mode").cloned().unwrap_or_default(),
            });
            let field = if network == "ws" {
                "wsSettings"
            } else if network == "xhttp" {
                "xhttpSettings"
            } else {
                "splithttpSettings"
            };
            stream[field] = settings;
        } else if network == "grpc" {
            stream["grpcSettings"] = json!({
                "serviceName": node.params.get("serviceName").cloned().unwrap_or_default(),
            });
        }
        json!({"tag":"proxy","protocol":"vless","settings":{"vnext":[{"address":node.host,"port":node.port.unwrap_or(443),"users":[{"id":node.credential,"encryption":node.params.get("encryption").map_or("none", String::as_str),"flow":node.params.get("flow").cloned().unwrap_or_default()}]}]},"streamSettings":stream})
    } else if matches!(node.scheme.as_str(), "hysteria" | "hysteria2") {
        json!({"tag":"proxy","protocol":"hysteria","settings":{"address":node.host,"port":node.port.unwrap_or(443),"version":2},"streamSettings":{"network":"hysteria","security":"tls","hysteriaSettings":{"version":2,"auth":node.credential},"tlsSettings":{"serverName":node.params.get("sni").cloned().unwrap_or_default(),"fingerprint":node.params.get("fp").cloned().unwrap_or_default(),"alpn":comma_separated_param(node,"alpn")},"finalmask":{"quicParams":{"congestion":node.params.get("congestion").cloned().unwrap_or_default()}}}})
    } else {
        return None;
    };
    Some(
        json!({"log":{"loglevel":"warning"},"inbounds":[],"outbounds":[outbound],"remarks":node.remark}),
    )
}

fn render_singbox_subscription(nodes: &[SubscriptionNode]) -> String {
    let outbounds = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| singbox_outbound_for_node(node, index))
        .collect::<Vec<_>>();
    serde_json::to_string(&json!({"outbounds":outbounds})).unwrap_or_else(|_| "{}".to_owned())
}

fn singbox_outbound_for_node(node: &SubscriptionNode, index: usize) -> Option<Value> {
    let tag = if node.remark.is_empty() {
        format!("VPN {index}")
    } else {
        node.remark.clone()
    };
    if node.scheme == "vless" {
        let mut outbound = json!({"type":"vless","tag":tag,"server":node.host,"server_port":node.port.unwrap_or(443),"uuid":node.credential});
        if let Some(flow) = node.params.get("flow").filter(|value| !value.is_empty()) {
            outbound["flow"] = Value::String(flow.clone());
        }
        let network = node.params.get("type").map_or("tcp", String::as_str);
        if network != "tcp" {
            let mut transport = json!({"type": network});
            match network {
                "ws" => {
                    transport["path"] = json!(node.params.get("path").cloned().unwrap_or_default());
                    if let Some(host) = node.params.get("host") {
                        transport["headers"] = json!({"Host": host});
                    }
                }
                "grpc" => {
                    transport["service_name"] =
                        json!(node.params.get("serviceName").cloned().unwrap_or_default());
                }
                "xhttp" | "splithttp" => {
                    transport["path"] = json!(node.params.get("path").cloned().unwrap_or_default());
                    transport["host"] = json!(node.params.get("host").cloned().unwrap_or_default());
                    transport["mode"] = json!(node.params.get("mode").cloned().unwrap_or_default());
                }
                _ => {}
            }
            outbound["transport"] = transport;
        }
        if let Some(security) = node
            .params
            .get("security")
            .filter(|value| value.as_str() != "none")
        {
            let mut tls = json!({
                "enabled": true,
                "server_name": node.params.get("sni").cloned().unwrap_or_default(),
                "alpn": comma_separated_param(node, "alpn"),
            });
            if let Some(fingerprint) = node.params.get("fp") {
                tls["utls"] = json!({"enabled":true,"fingerprint":fingerprint});
            }
            if security == "reality" {
                tls["reality"] = json!({
                    "enabled":true,
                    "public_key":node.params.get("pbk").cloned().unwrap_or_default(),
                    "short_id":node.params.get("sid").cloned().unwrap_or_default(),
                });
            }
            outbound["tls"] = tls;
        }
        return Some(outbound);
    }
    if matches!(node.scheme.as_str(), "hysteria" | "hysteria2") {
        let mut outbound = json!({"type":"hysteria2","tag":tag,"server":node.host,"server_port":node.port.unwrap_or(443),"password":node.credential,"tls":{"enabled":true,"server_name":node.params.get("sni").cloned().unwrap_or_default(),"alpn":comma_separated_param(node,"alpn")}});
        if let Some(fingerprint) = node.params.get("fp") {
            outbound["tls"]["utls"] = json!({"enabled":true,"fingerprint":fingerprint});
        }
        if let Some(obfs) = node.params.get("obfs") {
            outbound["obfs"] = json!({"type":obfs,"password":node.params.get("obfs-password").cloned().unwrap_or_default()});
        }
        if let Some(congestion) = node.params.get("congestion") {
            outbound["congestion_control"] = json!(congestion);
        }
        return Some(outbound);
    }
    None
}

fn render_clash_subscription(nodes: &[SubscriptionNode]) -> String {
    let mut output = String::from("mixed-port: 7890\nmode: global\nproxies:\n");
    for (index, node) in nodes.iter().enumerate() {
        let name = if node.remark.is_empty() {
            format!("VPN {index}")
        } else {
            node.remark.clone()
        };
        let kind = if node.scheme == "vless" {
            "vless"
        } else {
            "hysteria2"
        };
        writeln!(
            output,
            "  - name: {}\n    type: {kind}\n    server: {}\n    port: {}\n    {}: {}",
            yaml_scalar(&name),
            yaml_scalar(&node.host),
            node.port.unwrap_or(443),
            if kind == "vless" { "uuid" } else { "password" },
            yaml_scalar(&node.credential),
        )
        .expect("writing into a String cannot fail");
        if node.scheme == "vless" {
            writeln!(
                output,
                "    network: {}\n    udp: true",
                yaml_scalar(node.params.get("type").map_or("tcp", String::as_str)),
            )
            .expect("writing into a String cannot fail");
            if node
                .params
                .get("security")
                .is_some_and(|value| value != "none")
            {
                output.push_str("    tls: true\n");
            }
            for (source, target) in [("sni", "servername"), ("fp", "client-fingerprint")] {
                if let Some(value) = node.params.get(source) {
                    writeln!(output, "    {target}: {}", yaml_scalar(value))
                        .expect("writing into a String cannot fail");
                }
            }
            if let Some(alpn) = node.params.get("alpn") {
                writeln!(output, "    alpn: {}", yaml_scalar(alpn))
                    .expect("writing into a String cannot fail");
            }
            if node
                .params
                .get("security")
                .is_some_and(|value| value == "reality")
            {
                output.push_str("    reality-opts:\n");
                for (source, target) in [("pbk", "public-key"), ("sid", "short-id")] {
                    if let Some(value) = node.params.get(source) {
                        writeln!(output, "      {target}: {}", yaml_scalar(value))
                            .expect("writing into a String cannot fail");
                    }
                }
            }
            let network = node.params.get("type").map_or("tcp", String::as_str);
            if matches!(network, "ws" | "xhttp" | "splithttp") {
                output.push_str(if network == "ws" {
                    "    ws-opts:\n"
                } else {
                    "    xhttp-opts:\n"
                });
                if let Some(path) = node.params.get("path") {
                    writeln!(output, "      path: {}", yaml_scalar(path))
                        .expect("writing into a String cannot fail");
                }
                if let Some(host) = node.params.get("host") {
                    writeln!(output, "      headers: {{Host: {}}}", yaml_scalar(host))
                        .expect("writing into a String cannot fail");
                }
            } else if network == "grpc" {
                if let Some(service_name) = node.params.get("serviceName") {
                    writeln!(
                        output,
                        "    grpc-opts: {{grpc-service-name: {}}}",
                        yaml_scalar(service_name),
                    )
                    .expect("writing into a String cannot fail");
                }
            }
        } else {
            for (source, target) in [
                ("sni", "sni"),
                ("fp", "fingerprint"),
                ("obfs", "obfs"),
                ("obfs-password", "obfs-password"),
            ] {
                if let Some(value) = node.params.get(source) {
                    writeln!(output, "    {target}: {}", yaml_scalar(value))
                        .expect("writing into a String cannot fail");
                }
            }
        }
    }
    output
}

fn comma_separated_param(node: &SubscriptionNode, key: &str) -> Vec<String> {
    node.params.get(key).map_or_else(Vec::new, |value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
        == 0
}

async fn upsert_user(
    transaction: &mut Transaction<'_, Postgres>,
    telegram_user: &TelegramWebAppUser,
) -> Result<Uuid, ApiError> {
    let user_id = Uuid::now_v7();
    let language = telegram_user
        .language_code
        .as_deref()
        .filter(|language| matches!(*language, "ru" | "en"))
        .unwrap_or("ru");
    let created_id: Uuid = sqlx::query_scalar(
        "INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)
         ON CONFLICT (telegram_user_id) DO UPDATE SET updated_at = now()
         RETURNING id",
    )
    .bind(user_id)
    .bind(telegram_user.id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, username, first_name, last_name, language_code)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id) DO UPDATE SET
           username = EXCLUDED.username, first_name = EXCLUDED.first_name,
           last_name = EXCLUDED.last_name, language_code = EXCLUDED.language_code,
           updated_at = now()",
    )
    .bind(created_id)
    .bind(&telegram_user.username)
    .bind(&telegram_user.first_name)
    .bind(&telegram_user.last_name)
    .bind(language)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(created_id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(created_id)
}

async fn authenticated_user_id(database: &PgPool, headers: &HeaderMap) -> Result<Uuid, ApiError> {
    let token = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(ApiError::unauthorized)?;
    let user_id = sqlx::query_scalar(
        "SELECT user_id FROM user_sessions
         WHERE token_hash = $1 AND expires_at > now() AND revoked_at IS NULL",
    )
    .bind(sha256_bytes(token.as_bytes()))
    .fetch_optional(database)
    .await
    .map_err(database_error)?
    .ok_or_else(ApiError::unauthorized)?;
    Ok(user_id)
}

async fn authenticated_admin_id(state: &AppState, headers: &HeaderMap) -> Result<Uuid, ApiError> {
    let user_id = authenticated_user_id(&state.database, headers).await?;
    if is_admin_user(state, user_id).await? {
        Ok(user_id)
    } else {
        Err(ApiError::forbidden())
    }
}

async fn is_admin_user(state: &AppState, user_id: Uuid) -> Result<bool, ApiError> {
    let telegram_user_id = sqlx::query_scalar::<_, i64>(
        "SELECT telegram_user_id FROM users WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(user_id)
    .fetch_optional(&state.database)
    .await
    .map_err(database_error)?
    .ok_or_else(ApiError::unauthorized)?;
    if state
        .bootstrap_admin_telegram_ids
        .contains(&telegram_user_id)
    {
        return Ok(true);
    }
    let has_role = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
           SELECT 1 FROM user_roles ur JOIN roles r ON r.id = ur.role_id
           WHERE ur.user_id = $1 AND r.code IN ('admin', 'super_admin')
         )",
    )
    .bind(user_id)
    .fetch_one(&state.database)
    .await
    .map_err(database_error)?;
    if has_role { Ok(true) } else { Ok(false) }
}

fn default_trial_settings() -> TrialSettings {
    TrialSettings {
        duration_seconds: DEFAULT_TRIAL_DURATION_SECONDS,
        traffic_bytes: DEFAULT_TRIAL_TRAFFIC_BYTES,
    }
}

fn default_referral_settings() -> ReferralSettings {
    ReferralSettings { percent: 10 }
}

fn default_public_settings() -> PublicSettings {
    PublicSettings {
        mini_app_url: String::new(),
        admin_url: String::new(),
        subscription_public_url: String::new(),
        telegram_webhook_url: String::new(),
        cors_origins: Vec::new(),
        support_url: None,
    }
}

fn default_runtime_settings() -> RuntimeSettings {
    RuntimeSettings {
        api_host_port: 18_080,
        mini_app_host_port: 18_081,
        admin_host_port: 18_082,
        telegram_webhook_host_port: 18_083,
    }
}

fn default_telegram_transport_settings() -> TelegramTransportSettings {
    TelegramTransportSettings {
        mode: "polling".to_owned(),
    }
}

async fn ensure_trial_settings(database: &PgPool) -> Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value)
         VALUES ('trial_settings', $1)
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(serde_json::to_value(default_trial_settings()).expect("trial settings serialize"))
    .execute(database)
    .await
    .context("could not initialize trial settings")?;
    Ok(())
}

async fn ensure_referral_settings(database: &PgPool) -> Result<()> {
    sqlx::query(
        "INSERT INTO app_settings (key, value)
         VALUES ('referral_settings', $1)
         ON CONFLICT (key) DO NOTHING",
    )
    .bind(serde_json::to_value(default_referral_settings()).expect("referral settings serialize"))
    .execute(database)
    .await
    .context("could not initialize referral settings")?;
    Ok(())
}

async fn load_admin_setting<T>(database: &PgPool, key: &str, default: T) -> Result<T, ApiError>
where
    T: for<'a> Deserialize<'a> + Serialize,
{
    let value = sqlx::query_scalar::<_, Value>("SELECT value FROM app_settings WHERE key = $1")
        .bind(key)
        .fetch_optional(database)
        .await
        .map_err(database_error)?
        .unwrap_or_else(|| serde_json::to_value(default).expect("setting serializes"));
    serde_json::from_value(value).map_err(|_| ApiError::unavailable("A system setting is invalid."))
}

async fn load_public_settings(database: &PgPool) -> Result<PublicSettings, ApiError> {
    load_admin_setting(database, "public_settings", default_public_settings()).await
}

async fn load_runtime_settings(database: &PgPool) -> Result<RuntimeSettings, ApiError> {
    load_admin_setting(database, "runtime_settings", default_runtime_settings()).await
}

async fn load_telegram_transport_settings(
    database: &PgPool,
) -> Result<TelegramTransportSettings, ApiError> {
    load_admin_setting(
        database,
        "telegram_transport_settings",
        default_telegram_transport_settings(),
    )
    .await
}

async fn save_admin_setting<T>(
    database: &PgPool,
    actor_user_id: Uuid,
    key: &str,
    value: &T,
) -> Result<(), ApiError>
where
    T: Serialize,
{
    let next = serde_json::to_value(value).expect("setting serializes");
    let mut transaction = database.begin().await.map_err(database_error)?;
    let previous =
        sqlx::query_scalar::<_, Value>("SELECT value FROM app_settings WHERE key = $1 FOR UPDATE")
            .bind(key)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO app_settings (key, value, updated_by_user_id) VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
           updated_by_user_id = EXCLUDED.updated_by_user_id, updated_at = now()",
    )
    .bind(key)
    .bind(&next)
    .bind(actor_user_id)
    .execute(&mut *transaction)
    .await
    .map_err(database_error)?;
    insert_audit_record(
        &mut transaction,
        actor_user_id,
        "system_setting.updated",
        "app_setting",
        Uuid::nil(),
        previous,
        json!({"key": key, "value": next}),
    )
    .await?;
    transaction.commit().await.map_err(database_error)?;
    Ok(())
}

fn normalize_login(login: &str) -> String {
    login.trim().to_ascii_lowercase()
}

fn validate_admin_credentials(request: &AdminCredentialsRequest) -> Result<(), ApiError> {
    let login = normalize_login(&request.login);
    if !(3..=64).contains(&login.len())
        || !login.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
    {
        return Err(ApiError::invalid(
            "Login must be 3-64 characters: lowercase letters, digits, dot, dash, or underscore.",
        ));
    }
    if !(12..=256).contains(&request.password.len()) {
        return Err(ApiError::invalid("Password must be 12-256 characters."));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| ApiError::unavailable("Password storage is unavailable."))
}

fn verify_password(password: &str, password_hash: &str) -> Result<(), ApiError> {
    let parsed = PasswordHash::new(password_hash).map_err(|_| ApiError::unauthorized())?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| ApiError::unauthorized())
}

fn validate_public_settings(settings: &PublicSettings) -> Result<(), ApiError> {
    for url in [
        &settings.mini_app_url,
        &settings.admin_url,
        &settings.subscription_public_url,
        &settings.telegram_webhook_url,
    ] {
        if url.is_empty() {
            continue;
        }
        let parsed =
            url::Url::parse(url).map_err(|_| ApiError::invalid("A public URL is invalid."))?;
        if parsed.scheme() != "https" || parsed.host_str().is_none() {
            return Err(ApiError::invalid(
                "Public URLs must use HTTPS and a hostname.",
            ));
        }
    }
    if !settings.telegram_webhook_url.is_empty()
        && !settings.telegram_webhook_url.ends_with("/telegram/webhook")
    {
        return Err(ApiError::invalid(
            "Telegram webhook URL must end with /telegram/webhook.",
        ));
    }
    for origin in &settings.cors_origins {
        let parsed =
            url::Url::parse(origin).map_err(|_| ApiError::invalid("A CORS origin is invalid."))?;
        if parsed.scheme() != "https" || parsed.host_str().is_none() || parsed.path() != "/" {
            return Err(ApiError::invalid(
                "CORS origins must be HTTPS origins without a path.",
            ));
        }
    }
    if let Some(url) = &settings.support_url {
        url::Url::parse(url).map_err(|_| ApiError::invalid("Support URL is invalid."))?;
    }
    Ok(())
}

fn validate_runtime_settings(settings: &RuntimeSettings) -> Result<(), ApiError> {
    let ports = [
        settings.api_host_port,
        settings.mini_app_host_port,
        settings.admin_host_port,
        settings.telegram_webhook_host_port,
    ];
    if ports.iter().any(|port| *port < 1024)
        || ports.iter().collect::<HashSet<_>>().len() != ports.len()
    {
        return Err(ApiError::invalid(
            "Ports must be unique non-privileged values. Compose restart is required after changing them.",
        ));
    }
    Ok(())
}

fn validate_telegram_transport_settings(
    settings: &TelegramTransportSettings,
) -> Result<(), ApiError> {
    if matches!(settings.mode.as_str(), "polling" | "webhook") {
        Ok(())
    } else {
        Err(ApiError::invalid(
            "Telegram mode must be polling or webhook.",
        ))
    }
}

async fn load_trial_settings(database: &PgPool) -> Result<TrialSettings, ApiError> {
    let value = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM app_settings WHERE key = 'trial_settings'",
    )
    .fetch_optional(database)
    .await
    .map_err(database_error)?
    .unwrap_or_else(|| {
        serde_json::to_value(default_trial_settings()).expect("trial settings serialize")
    });
    let settings = serde_json::from_value(value)
        .map_err(|_| ApiError::unavailable("Trial settings are invalid."))?;
    validate_trial_settings(&settings)?;
    Ok(settings)
}

fn validate_trial_settings(settings: &TrialSettings) -> Result<(), ApiError> {
    if !(60 * 60..=31 * 24 * 60 * 60).contains(&settings.duration_seconds) {
        return Err(ApiError::invalid(
            "Trial duration must be from 1 hour to 31 days.",
        ));
    }
    if !(1_000_000..=1_000_000_000_000).contains(&settings.traffic_bytes) {
        return Err(ApiError::invalid(
            "Trial traffic must be from 1 MB to 1 TB.",
        ));
    }
    Ok(())
}

async fn load_referral_settings(database: &PgPool) -> Result<ReferralSettings, ApiError> {
    let value = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM app_settings WHERE key = 'referral_settings'",
    )
    .fetch_optional(database)
    .await
    .map_err(database_error)?
    .unwrap_or_else(|| {
        serde_json::to_value(default_referral_settings()).expect("referral settings serialize")
    });
    let settings = serde_json::from_value(value)
        .map_err(|_| ApiError::unavailable("Referral settings are invalid."))?;
    validate_referral_settings(&settings)?;
    Ok(settings)
}

fn validate_referral_settings(settings: &ReferralSettings) -> Result<(), ApiError> {
    if !(1..=100).contains(&settings.percent) {
        return Err(ApiError::invalid(
            "Referral reward percentage must be from 1 to 100.",
        ));
    }
    Ok(())
}

fn load_bootstrap_admin_telegram_ids() -> Result<HashSet<i64>> {
    env::var("BOOTSTRAP_ADMIN_TELEGRAM_IDS")
        .unwrap_or_default()
        .split(',')
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .trim()
                .parse::<i64>()
                .context("BOOTSTRAP_ADMIN_TELEGRAM_IDS must contain comma-separated Telegram IDs")
        })
        .collect()
}

fn request_idempotency_key(headers: &HeaderMap) -> Result<Uuid, ApiError> {
    headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| ApiError::invalid("A UUID Idempotency-Key header is required."))
}

async fn load_idempotent_purchase(
    transaction: &mut Transaction<'_, Postgres>,
    key: Uuid,
    request_hash: &[u8],
) -> Result<Option<PurchaseResponse>, ApiError> {
    let saved = sqlx::query_scalar::<_, Value>(
        "SELECT response_body FROM idempotency_keys
         WHERE key = $1 AND scope = 'purchase' AND expires_at > now()",
    )
    .bind(key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    match saved {
        Some(value) => {
            let stored_hash = sqlx::query_scalar::<_, Vec<u8>>(
                "SELECT request_hash FROM idempotency_keys WHERE key = $1",
            )
            .bind(key)
            .fetch_one(&mut **transaction)
            .await
            .map_err(database_error)?;
            if stored_hash != request_hash {
                return Err(ApiError::conflict(
                    "Idempotency key was used for another request.",
                ));
            }
            serde_json::from_value(value).map(Some).map_err(|_| {
                ApiError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "Stored response is invalid.",
                )
            })
        }
        None => Ok(None),
    }
}

async fn create_wallet_purchase(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    tariff_id: Uuid,
    promo_code: Option<&str>,
) -> Result<PurchaseResponse, ApiError> {
    let tariff = sqlx::query_as::<_, (i64, String)>(
        "SELECT p.amount_minor, p.currency_code FROM tariffs t
         JOIN tariff_prices p ON p.tariff_id = t.id AND p.is_active
         WHERE t.id = $1 AND t.is_active FOR UPDATE",
    )
    .bind(tariff_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Tariff is unavailable."))?;
    let (amount_minor, promo) = match promo_code {
        Some(code) => apply_discount_promo(transaction, user_id, code, tariff.0).await?,
        None => (tariff.0, None),
    };
    let wallet = sqlx::query_as::<_, (Uuid, i64, String)>(
        "SELECT id, balance_minor, currency_code FROM wallets WHERE user_id = $1 FOR UPDATE",
    )
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if wallet.2 != tariff.1 || wallet.1 < amount_minor {
        return Err(ApiError::conflict("Insufficient wallet balance."));
    }
    let subscription_id = Uuid::now_v7();
    let debit_id = Uuid::now_v7();
    sqlx::query(
        "UPDATE wallets SET balance_minor = balance_minor - $1, updated_at = now() WHERE id = $2",
    )
    .bind(amount_minor)
    .bind(wallet.0)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO wallet_transactions
         (id, wallet_id, amount_minor, currency_code, kind, reference_type, reference_id)
         VALUES ($1, $2, $3, $4, 'purchase_debit', 'subscription', $5)",
    )
    .bind(debit_id)
    .bind(wallet.0)
    .bind(-amount_minor)
    .bind(&tariff.1)
    .bind(subscription_id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if let Some(promo) = promo {
        sqlx::query(
            "INSERT INTO promo_redemptions (id, promo_code_id, user_id, wallet_transaction_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::now_v7())
        .bind(promo.id)
        .bind(user_id)
        .bind(debit_id)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        sqlx::query("UPDATE promo_codes SET redeemed_count = redeemed_count + 1 WHERE id = $1")
            .bind(promo.id)
            .execute(&mut **transaction)
            .await
            .map_err(database_error)?;
    }
    sqlx::query(
        "INSERT INTO subscriptions (id, user_id, tariff_id, status)
         VALUES ($1, $2, $3, 'provisioning_pending')",
    )
    .bind(subscription_id)
    .bind(user_id)
    .bind(tariff_id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    sqlx::query(
        "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
         VALUES ($1, 'subscription', $2, 'subscription.requested', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"subscription_id": subscription_id, "user_id": user_id, "version": 1}))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(PurchaseResponse {
        subscription_id,
        status: "provisioning_pending".to_owned(),
    })
}

#[derive(Debug, FromRow)]
struct DiscountPromo {
    id: Uuid,
    discount_percent: i16,
    redeemed_count: i32,
    maximum_redemptions: Option<i32>,
    is_active: bool,
    starts_at: Option<DateTime<Utc>>,
    ends_at: Option<DateTime<Utc>>,
}

async fn apply_discount_promo(
    transaction: &mut Transaction<'_, Postgres>,
    user_id: Uuid,
    code: &str,
    amount_minor: i64,
) -> Result<(i64, Option<DiscountPromo>), ApiError> {
    let code = normalize_promo_code(code)?;
    let promo = sqlx::query_as::<_, DiscountPromo>(
        "SELECT id, discount_percent, redeemed_count, maximum_redemptions, is_active, starts_at, ends_at
         FROM promo_codes WHERE code = $1 AND kind = 'discount' FOR UPDATE",
    )
    .bind(code)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| ApiError::invalid("Discount promo code is invalid."))?;
    if !promo.is_active
        || promo
            .starts_at
            .is_some_and(|starts_at| starts_at > Utc::now())
        || promo.ends_at.is_some_and(|ends_at| ends_at <= Utc::now())
        || promo
            .maximum_redemptions
            .is_some_and(|maximum| promo.redeemed_count >= maximum)
    {
        return Err(ApiError::conflict("Discount promo code is unavailable."));
    }
    let already_redeemed = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM promo_redemptions WHERE promo_code_id = $1 AND user_id = $2)",
    )
    .bind(promo.id)
    .bind(user_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if already_redeemed {
        return Err(ApiError::conflict(
            "Discount promo code was already redeemed.",
        ));
    }
    let discounted_amount = amount_minor
        .checked_mul(i64::from(100 - promo.discount_percent))
        .and_then(|amount| amount.checked_div(100))
        .ok_or_else(|| ApiError::invalid("Discount promo amount is invalid."))?;
    Ok((discounted_amount, Some(promo)))
}

async fn store_idempotent_purchase(
    transaction: &mut Transaction<'_, Postgres>,
    key: Uuid,
    request_hash: Vec<u8>,
    response: &PurchaseResponse,
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO idempotency_keys
         (key, scope, request_hash, response_status, response_body, expires_at)
         VALUES ($1, 'purchase', $2, 201, $3, now() + interval '24 hours')",
    )
    .bind(key)
    .bind(request_hash)
    .bind(serde_json::to_value(response).expect("purchase response is serializable"))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn database_error(error: sqlx::Error) -> ApiError {
    tracing::error!(%error, "database operation failed");
    ApiError::new(
        StatusCode::INTERNAL_SERVER_ERROR,
        "internal_error",
        "An unexpected error occurred.",
    )
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl+C handler");
    info!("shutdown signal received");
}

#[cfg(test)]
mod tests {
    use super::{
        AnoreConfig, AnoreProvider, AppState, HmacSha256, INIT_DATA_MAX_AGE_SECONDS,
        PaymentProvider, RateLimitKind, extract_subscription_nodes, openapi_document,
        payment_webhook, rate_limit_kind, reject_telegram_replay, render_clash_subscription,
        render_singbox_subscription, render_subscription_for_client, render_xray_subscription,
        sha256_bytes, verify_telegram_init_data,
    };
    use axum::{
        body::Bytes,
        extract::{Path, State},
        http::{HeaderMap, HeaderValue, Method, StatusCode},
    };
    use chrono::Utc;
    use hmac::Mac;
    use redis::AsyncCommands;
    use sqlx::PgPool;
    use std::{collections::HashSet, env, sync::Arc};
    use uuid::Uuid;

    fn signed_init_data(bot_token: &str, auth_date: i64) -> String {
        let user = r#"{"id":123456,"first_name":"Test","language_code":"en"}"#;
        let data_check = format!("auth_date={auth_date}\nuser={user}");
        let mut secret = HmacSha256::new_from_slice(b"WebAppData").unwrap();
        secret.update(bot_token.as_bytes());
        let secret_key = secret.finalize().into_bytes();
        let mut signature = HmacSha256::new_from_slice(&secret_key).unwrap();
        signature.update(data_check.as_bytes());
        let hash = hex::encode(signature.finalize().into_bytes());
        url::form_urlencoded::Serializer::new(String::new())
            .append_pair("user", user)
            .append_pair("auth_date", &auth_date.to_string())
            .append_pair("hash", &hash)
            .finish()
    }

    async fn postgres_test_pool() -> Option<PgPool> {
        let url = env::var("VPN_TEST_DATABASE_URL").ok()?;
        PgPool::connect(&url).await.ok()
    }

    fn test_telegram_user_id() -> i64 {
        i64::from_be_bytes(Uuid::now_v7().as_bytes()[8..].try_into().unwrap()) & i64::MAX
    }

    #[test]
    fn verifies_fresh_telegram_init_data() {
        let init_data = signed_init_data("bot-token", Utc::now().timestamp());
        let user = verify_telegram_init_data(&init_data, "bot-token").unwrap();
        assert_eq!(user.id, 123_456);
        assert_eq!(user.language_code.as_deref(), Some("en"));
    }

    #[test]
    fn rejects_expired_telegram_init_data() {
        let init_data = signed_init_data(
            "bot-token",
            Utc::now().timestamp() - INIT_DATA_MAX_AGE_SECONDS - 1,
        );
        assert!(verify_telegram_init_data(&init_data, "bot-token").is_err());
    }

    #[test]
    fn rejects_tampered_telegram_init_data() {
        let init_data = signed_init_data("bot-token", Utc::now().timestamp());
        assert!(verify_telegram_init_data(&init_data, "another-token").is_err());
    }

    #[test]
    fn classifies_sensitive_rate_limit_routes() {
        assert!(matches!(
            rate_limit_kind(&Method::POST, "/api/v1/auth/telegram"),
            Some(RateLimitKind::Authentication)
        ));
        assert!(matches!(
            rate_limit_kind(&Method::POST, "/api/v1/webhooks/payments/anore"),
            Some(RateLimitKind::Webhook)
        ));
        assert!(matches!(
            rate_limit_kind(&Method::PUT, "/api/v1/admin/tariffs/id"),
            Some(RateLimitKind::AdminMutation)
        ));
        assert!(matches!(
            rate_limit_kind(&Method::GET, "/api/v1/admin/tariffs"),
            Some(RateLimitKind::Standard)
        ));
    }

    #[test]
    fn excludes_health_and_preflight_from_rate_limits() {
        assert!(rate_limit_kind(&Method::GET, "/healthz").is_none());
        assert!(rate_limit_kind(&Method::OPTIONS, "/api/v1/purchases").is_none());
    }

    #[tokio::test]
    async fn openapi_contract_covers_p0_routes() {
        let document = openapi_document().await.0;
        assert_eq!(document["openapi"], "3.1.0");
        for (path, method) in [
            ("/api/v1/auth/telegram", "post"),
            ("/api/v1/me", "get"),
            ("/api/v1/me", "put"),
            ("/api/v1/subscriptions/{id}/access", "get"),
            ("/api/v1/purchases", "post"),
            ("/api/v1/invoices", "post"),
            ("/api/v1/webhooks/payments/{provider}", "post"),
            ("/api/v1/admin/users", "get"),
            ("/api/v1/admin/tariffs", "post"),
            ("/api/v1/admin/required-channels/{id}", "put"),
        ] {
            let operation = &document["paths"][path][method];
            assert!(operation.is_object(), "missing {method} {path}");
            assert!(
                operation["responses"].is_object(),
                "missing responses for {method} {path}"
            );
        }
        assert_eq!(document["paths"].as_object().unwrap().len(), 28);
        assert_eq!(
            document["paths"]["/api/v1/auth/telegram"]["post"]["security"],
            serde_json::json!([])
        );
        assert_eq!(
            document["paths"]["/api/v1/webhooks/payments/{provider}"]["post"]["security"],
            serde_json::json!([])
        );
        assert_eq!(
            document["components"]["schemas"]["UpsertTariffRequest"]["required"],
            serde_json::json!([
                "code",
                "name",
                "description",
                "position",
                "is_active",
                "amount_minor",
                "currency_code"
            ])
        );
        assert!(document["components"]["schemas"].get("Error").is_some());
        assert!(
            document["components"]["responses"]
                .get("Unauthorized")
                .is_some()
        );
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn payment_webhook_replay_creates_one_direct_purchase() {
        let Some(pool) = postgres_test_pool().await else {
            return;
        };
        let user_id = Uuid::now_v7();
        let tariff_id = Uuid::now_v7();
        let invoice_id = Uuid::now_v7();
        let provider_invoice_id = format!("anore-{invoice_id}");
        sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(test_telegram_user_id())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tariffs (id, code, name) VALUES ($1, $2, '{\"en\":\"Test\"}')")
            .bind(tariff_id)
            .bind(format!("test-{tariff_id}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO invoices
             (id, user_id, provider, provider_invoice_id, purpose, status, currency_code, amount_minor, tariff_id, expires_at)
             VALUES ($1, $2, 'anore', $3, 'direct_purchase', 'pending', 'RUB', 50000, $4, now() + interval '1 hour')",
        )
        .bind(invoice_id)
        .bind(user_id)
        .bind(&provider_invoice_id)
        .bind(tariff_id)
        .execute(&pool)
        .await
        .unwrap();

        let signing_secret = "test-webhook-secret";
        let body = Bytes::from(format!(
            "{{\"id\":\"{provider_invoice_id}\",\"status\":\"paid\",\"amount\":\"500.00\",\"currency\":\"RUB\"}}"
        ));
        let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes()).unwrap();
        mac.update(&body);
        let signature = hex::encode(mac.finalize().into_bytes());
        let mut headers = HeaderMap::new();
        headers.insert(
            "Anore-Signature",
            HeaderValue::from_str(&signature).unwrap(),
        );
        headers.insert("Anore-Event", HeaderValue::from_static("event-replay-test"));
        let provider = AnoreProvider::new(AnoreConfig {
            api_key: "test-key".to_owned(),
            signing_secret: signing_secret.to_owned(),
            base_url: "https://example.invalid".to_owned(),
        })
        .unwrap();
        let state = Arc::new(AppState {
            database: pool.clone(),
            redis: redis::Client::open("redis://127.0.0.1:6379").unwrap(),
            telegram_bot_token: Arc::from("test-bot-token"),
            encryption_key: Arc::new([0; 32]),
            bootstrap_admin_telegram_ids: Arc::new(HashSet::default()),
            payment_providers: vec![Arc::new(provider) as Arc<dyn PaymentProvider>],
            subscription_public_url: Arc::from("https://example.invalid"),
        });
        for _ in 0..2 {
            let status = payment_webhook(
                State(state.clone()),
                Path("anore".to_owned()),
                headers.clone(),
                body.clone(),
            )
            .await
            .unwrap();
            assert_eq!(status, StatusCode::OK);
        }
        let subscriptions = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM subscriptions WHERE source_invoice_id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let webhook_events = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM payment_webhook_events WHERE provider = 'anore' AND provider_event_id = 'event-replay-test'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let payment_attempts = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM payment_attempts WHERE invoice_id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((subscriptions, webhook_events, payment_attempts), (1, 1, 1));

        sqlx::query("DELETE FROM outbox_events WHERE aggregate_id IN (SELECT id FROM subscriptions WHERE source_invoice_id = $1)")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM subscriptions WHERE source_invoice_id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM payment_attempts WHERE invoice_id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM payment_webhook_events WHERE provider = 'anore' AND provider_event_id = 'event-replay-test'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM invoices WHERE id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM tariffs WHERE id = $1")
            .bind(tariff_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[test]
    fn preserves_reality_and_websocket_fields_from_xray_payload() {
        let payload = r#"[{"remarks":"Reality WS","outbounds":[{"protocol":"vless","settings":{"vnext":[{"address":"edge.example","port":443,"users":[{"id":"00000000-0000-0000-0000-000000000001","encryption":"none","flow":"xtls-rprx-vision"}]}]},"streamSettings":{"network":"ws","security":"reality","realitySettings":{"serverName":"cdn.example","fingerprint":"chrome","publicKey":"public-key","shortId":"abcd"},"wsSettings":{"path":"/gateway","headers":{"Host":"host.example"}}}}]}]"#;
        let nodes = extract_subscription_nodes(payload);
        assert_eq!(nodes.len(), 1);
        let xray: Vec<serde_json::Value> =
            serde_json::from_str(&render_xray_subscription(&nodes)).unwrap();
        let stream = &xray[0]["outbounds"][0]["streamSettings"];
        assert_eq!(stream["realitySettings"]["publicKey"], "public-key");
        assert_eq!(stream["wsSettings"]["path"], "/gateway");
        let singbox: serde_json::Value =
            serde_json::from_str(&render_singbox_subscription(&nodes)).unwrap();
        assert_eq!(
            singbox["outbounds"][0]["tls"]["reality"]["short_id"],
            "abcd"
        );
        assert_eq!(
            singbox["outbounds"][0]["transport"]["headers"]["Host"],
            "host.example"
        );
        let clash = render_clash_subscription(&nodes);
        assert!(clash.contains("reality-opts:"));
        assert!(clash.contains("ws-opts:"));
    }

    #[test]
    fn preserves_hysteria2_tls_obfs_and_congestion_fields() {
        let nodes = extract_subscription_nodes(
            "hysteria2://secret@hy.example:8443?sni=cdn.example&alpn=h3,h2&fp=chrome&obfs=salamander&obfs-password=mask&congestion=bbr#HY2",
        );
        let singbox: serde_json::Value =
            serde_json::from_str(&render_singbox_subscription(&nodes)).unwrap();
        let outbound = &singbox["outbounds"][0];
        assert_eq!(outbound["tls"]["server_name"], "cdn.example");
        assert_eq!(outbound["obfs"]["password"], "mask");
        assert_eq!(outbound["congestion_control"], "bbr");
        let clash = render_clash_subscription(&nodes);
        assert!(clash.contains("obfs-password"));
    }

    #[test]
    fn client_import_contracts_select_the_expected_format() {
        let payload = include_str!("../tests/fixtures/reality-hysteria2.txt");
        let nodes = extract_subscription_nodes(payload);
        assert_eq!(nodes.len(), 2);
        for (user_agent, expected_content_type, assertion) in [
            ("Happ/1.0", "application/json; charset=utf-8", "outbounds"),
            (
                "v2rayNG/1.9",
                "application/json; charset=utf-8",
                "outbounds",
            ),
            ("Karing/1.0", "application/json; charset=utf-8", "outbounds"),
            ("Mihomo/1.18", "text/yaml; charset=utf-8", "proxies:"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert("user-agent", HeaderValue::from_static(user_agent));
            let (rendered, content_type) =
                render_subscription_for_client(payload, &headers).unwrap();
            assert_eq!(content_type, expected_content_type, "{user_agent}");
            assert!(rendered.contains(assertion), "{user_agent}");
        }
    }

    #[tokio::test]
    async fn redis_replay_guard_rejects_reused_init_data_and_sets_ttl() {
        let Some(redis_url) = env::var("VPN_TEST_REDIS_URL").ok() else {
            return;
        };
        let client = redis::Client::open(redis_url).unwrap();
        let init_data = format!("integration-test-{}", uuid::Uuid::now_v7());
        let key = format!(
            "telegram-init-data:{}",
            hex::encode(sha256_bytes(init_data.as_bytes()))
        );
        reject_telegram_replay(&client, &init_data).await.unwrap();
        let repeated = reject_telegram_replay(&client, &init_data)
            .await
            .unwrap_err();
        assert_eq!(repeated.code, "conflict");
        let mut connection = client.get_multiplexed_async_connection().await.unwrap();
        let ttl = connection.ttl::<_, i64>(&key).await.unwrap();
        assert!(ttl > 0 && ttl <= INIT_DATA_MAX_AGE_SECONDS);
        let _: usize = connection.del(&key).await.unwrap();
    }
}
