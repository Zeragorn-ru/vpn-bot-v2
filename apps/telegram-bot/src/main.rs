use std::{env, net::SocketAddr, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use tokio::time::sleep;
use tracing::{error, info};
use uuid::Uuid;
use vpn_observability::init_tracing;
use vpn_storage::{InvoiceForFulfillment, connect, fulfill_paid_invoice};

const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";

#[derive(Clone)]
struct BotState {
    client: reqwest::Client,
    database: PgPool,
    token: Arc<str>,
    mini_app_url: Arc<str>,
    support_url: Option<Arc<str>>,
    webhook_secret: Option<Arc<str>>,
}

#[derive(Debug, Deserialize)]
struct TelegramResponse<T> {
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct TelegramUpdate {
    update_id: i64,
    message: Option<TelegramMessage>,
    callback_query: Option<TelegramCallbackQuery>,
}

#[derive(Debug, Deserialize)]
struct TelegramMessage {
    chat: TelegramChat,
    from: Option<TelegramUser>,
    text: Option<String>,
    successful_payment: Option<TelegramSuccessfulPayment>,
}

#[derive(Debug, Deserialize)]
struct TelegramCallbackQuery {
    id: String,
    from: TelegramUser,
    message: Option<TelegramMessage>,
    data: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramChat {
    id: i64,
}

#[derive(Debug, Deserialize)]
struct TelegramUser {
    id: i64,
    username: Option<String>,
    first_name: String,
    last_name: Option<String>,
    language_code: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TelegramSuccessfulPayment {
    currency: String,
    total_amount: i64,
    invoice_payload: String,
    telegram_payment_charge_id: String,
}

#[derive(Debug, FromRow)]
struct PendingStarsInvoice {
    id: Uuid,
    user_id: Uuid,
    purpose: String,
    currency_code: String,
    amount_minor: i64,
    tariff_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct TelegramUserInsert {
    id: Uuid,
    inserted: bool,
}

#[derive(Debug, FromRow)]
struct BotUser {
    id: Uuid,
    language_code: String,
}

#[derive(Debug, FromRow)]
struct RequiredChannel {
    telegram_chat_id: i64,
    title: String,
    public_url: Option<String>,
}

#[derive(Debug, FromRow)]
struct BotTariff {
    name: Value,
    duration_seconds: Option<i64>,
    traffic_bytes: Option<i64>,
    amount_minor: i64,
    currency_code: String,
}

#[derive(Debug, FromRow)]
struct BotSubscription {
    status: String,
    expires_at: Option<DateTime<Utc>>,
    access_available: bool,
}

#[derive(Debug, Deserialize)]
struct TrialSettings {
    duration_seconds: i64,
    traffic_bytes: i64,
}

#[derive(Debug, Deserialize, Serialize)]
struct TelegramChatMember {
    status: String,
    #[serde(default)]
    is_member: Option<bool>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let token = env::var("TELEGRAM_BOT_TOKEN").context("TELEGRAM_BOT_TOKEN is required")?;
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let database = connect(&database_url)
        .await
        .context("database connection failed")?;
    let mini_app_url =
        env::var("MINI_APP_PUBLIC_URL").context("MINI_APP_PUBLIC_URL is required")?;
    validate_mini_app_url(&mini_app_url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(35))
        .build()
        .context("Telegram client initialization failed")?;

    let support_url = env::var("SUPPORT_URL")
        .ok()
        .filter(|value| !value.is_empty());
    let state = BotState {
        client,
        database,
        token: Arc::from(token),
        mini_app_url: Arc::from(mini_app_url),
        support_url: support_url.map(Arc::from),
        webhook_secret: env::var("TELEGRAM_WEBHOOK_SECRET")
            .ok()
            .filter(|value| !value.is_empty())
            .map(Arc::from),
    };
    register_commands(&state).await?;
    info!("telegram bot process started");
    match load_transport_mode(&state.database).await?.as_str() {
        "polling" => {
            delete_webhook(&state).await?;
            run_polling_loop(&state).await?;
        }
        "webhook" => run_webhook_server(state).await?,
        _ => anyhow::bail!("telegram transport setting must be polling or webhook"),
    }
    info!("telegram bot process stopped");
    Ok(())
}

async fn load_transport_mode(database: &PgPool) -> Result<String> {
    let value = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM app_settings WHERE key = 'telegram_transport_settings'",
    )
    .fetch_optional(database)
    .await?
    .unwrap_or_else(|| json!({"mode": "polling"}));
    let mode = value
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("polling")
        .to_owned();
    if matches!(mode.as_str(), "polling" | "webhook") {
        Ok(mode)
    } else {
        anyhow::bail!("telegram transport setting must be polling or webhook")
    }
}

async fn should_restart(database: &PgPool) -> Result<bool> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM app_settings WHERE key = 'bot_restart')",
    )
    .fetch_one(database)
    .await?;
    Ok(exists)
}

async fn run_polling_loop(state: &BotState) -> Result<()> {
    let mut offset = None;
    let mut restart_check = tokio::time::interval(Duration::from_secs(10));
    restart_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            result = poll_updates(state, &mut offset) => {
                if let Err(error) = result {
                    error!(%error, "Telegram update processing failed");
                    sleep(Duration::from_secs(2)).await;
                }
            }
            _ = restart_check.tick() => {
                if should_restart(&state.database).await.unwrap_or(false) {
                    info!("restart flag detected, shutting down for Docker restart");
                    return Ok(());
                }
            }
            result = tokio::signal::ctrl_c() => {
                result.context("failed to wait for signal")?;
                return Ok(());
            }
        }
    }
}

async fn poll_updates(state: &BotState, offset: &mut Option<i64>) -> Result<()> {
    poll_updates_from_base(state, offset, TELEGRAM_API_BASE_URL).await
}

async fn poll_updates_from_base(
    state: &BotState,
    offset: &mut Option<i64>,
    telegram_api_base_url: &str,
) -> Result<()> {
    let response = state
        .client
        .get(format!(
            "{telegram_api_base_url}/bot{}/getUpdates",
            state.token
        ))
        .query(&[
            ("timeout", "30"),
            ("allowed_updates", "[\"message\",\"callback_query\"]"),
            (
                "offset",
                &offset.map_or_else(String::new, |value| value.to_string()),
            ),
        ])
        .send()
        .await
        .context("Telegram getUpdates request failed")?
        .error_for_status()
        .context("Telegram getUpdates returned an error")?;
    let document: TelegramResponse<Vec<TelegramUpdate>> = response
        .json()
        .await
        .context("Telegram getUpdates response is invalid")?;
    if !document.ok {
        anyhow::bail!("Telegram getUpdates was rejected");
    }
    for update in document.result.unwrap_or_default() {
        let update_id = update.update_id;
        process_update(state, update).await?;
        *offset = Some(update_id + 1);
    }
    Ok(())
}

async fn process_update(state: &BotState, update: TelegramUpdate) -> Result<()> {
    if let Some(message) = update.message {
        let TelegramMessage {
            chat,
            from,
            text,
            successful_payment,
        } = message;
        if let Some(payment) = successful_payment {
            fulfill_telegram_stars_payment(&state.database, update.update_id, payment)
                .await
                .with_context(|| {
                    format!(
                        "Telegram Stars fulfillment failed for update {}",
                        update.update_id
                    )
                })?;
        }
        if let Some(user) = from {
            let created_user = upsert_telegram_user(&state.database, &user).await?;
            if text
                .as_deref()
                .is_some_and(|text| text == "/start" || text.starts_with("/start "))
                && created_user.inserted
                && let Some(source_telegram_id) = start_referrer_telegram_id(text.as_deref())
            {
                attach_referral(
                    &state.database,
                    created_user.id,
                    user.id,
                    source_telegram_id,
                )
                .await?;
            }
            if let Some(text) = text.as_deref() {
                handle_command(
                    &state.client,
                    &state.database,
                    &state.token,
                    chat.id,
                    &user,
                    text,
                    &state.mini_app_url,
                    state.support_url.as_deref(),
                )
                .await?;
            }
        }
    }
    if let Some(callback) = update.callback_query {
        let chat_id = callback.message.as_ref().map(|message| message.chat.id);
        let bot_user = upsert_telegram_user(&state.database, &callback.from).await?;
        answer_callback(&state.client, &state.token, &callback.id).await?;
        if let Some(chat_id) = chat_id {
            handle_callback(
                &state.client,
                &state.database,
                &state.token,
                chat_id,
                &callback.from,
                bot_user.id,
                callback.data.as_deref().unwrap_or_default(),
                &state.mini_app_url,
                state.support_url.as_deref(),
            )
            .await?;
        }
    }
    Ok(())
}

async fn run_webhook_server(state: BotState) -> Result<()> {
    let secret = state
        .webhook_secret
        .as_deref()
        .context("TELEGRAM_WEBHOOK_SECRET is required for webhook transport")?;
    let public_url = env::var("TELEGRAM_WEBHOOK_PUBLIC_URL")
        .context("TELEGRAM_WEBHOOK_PUBLIC_URL is required for webhook transport")?;
    let parsed = url::Url::parse(&public_url).context("TELEGRAM_WEBHOOK_PUBLIC_URL is invalid")?;
    if parsed.scheme() != "https" || parsed.path() != "/telegram/webhook" {
        anyhow::bail!("TELEGRAM_WEBHOOK_PUBLIC_URL must be an HTTPS /telegram/webhook URL");
    }
    register_webhook(&state, &public_url, secret).await?;
    let bind_addr: SocketAddr = env::var("TELEGRAM_WEBHOOK_BIND_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8083".to_owned())
        .parse()
        .context("TELEGRAM_WEBHOOK_BIND_ADDR must be a socket address")?;
    let app = Router::new()
        .route("/healthz", get(|| async { StatusCode::OK }))
        .route("/telegram/webhook", post(telegram_webhook))
        .with_state(state.clone());
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .context("Telegram webhook bind failed")?;
    info!(%bind_addr, "Telegram webhook server started");
    let restart_state = state;
    let shutdown_signal = async move {
        let mut restart_check = tokio::time::interval(Duration::from_secs(10));
        restart_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = restart_check.tick() => {
                    if should_restart(&restart_state.database).await.unwrap_or(false) {
                        info!("restart flag detected, shutting down for Docker restart");
                        return;
                    }
                }
                result = tokio::signal::ctrl_c() => {
                    result.expect("failed to install Ctrl+C handler");
                    return;
                }
            }
        }
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal)
        .await
        .context("Telegram webhook server failed")
}

async fn register_webhook(state: &BotState, public_url: &str, secret: &str) -> Result<()> {
    let response = state
        .client
        .post(format!(
            "{TELEGRAM_API_BASE_URL}/bot{}/setWebhook",
            state.token
        ))
        .json(&json!({
            "url": public_url,
            "secret_token": secret,
            "allowed_updates": ["message", "callback_query"],
            "drop_pending_updates": false,
        }))
        .send()
        .await
        .context("Telegram setWebhook request failed")?
        .error_for_status()
        .context("Telegram setWebhook was rejected")?;
    let document: TelegramResponse<Value> = response
        .json()
        .await
        .context("Telegram setWebhook response is invalid")?;
    if !document.ok {
        anyhow::bail!("Telegram setWebhook was rejected");
    }
    Ok(())
}

async fn delete_webhook(state: &BotState) -> Result<()> {
    let response = state
        .client
        .post(format!(
            "{TELEGRAM_API_BASE_URL}/bot{}/deleteWebhook",
            state.token
        ))
        .json(&json!({"drop_pending_updates":false}))
        .send()
        .await
        .context("Telegram deleteWebhook request failed")?
        .error_for_status()
        .context("Telegram deleteWebhook was rejected")?;
    let document: TelegramResponse<Value> = response
        .json()
        .await
        .context("Telegram deleteWebhook response is invalid")?;
    if !document.ok {
        anyhow::bail!("Telegram deleteWebhook was rejected");
    }
    Ok(())
}

async fn register_commands(state: &BotState) -> Result<()> {
    register_commands_at_base(state, TELEGRAM_API_BASE_URL).await
}

async fn register_commands_at_base(state: &BotState, telegram_api_base_url: &str) -> Result<()> {
    let response = state
        .client
        .post(format!(
            "{telegram_api_base_url}/bot{}/setMyCommands",
            state.token
        ))
        .json(&json!({
            "commands": [
                {"command":"menu","description":"Open VPN menu"},
                {"command":"help","description":"Show help"},
                {"command":"plans","description":"View plans"},
                {"command":"trial","description":"Start trial"},
                {"command":"access","description":"View access"},
                {"command":"language","description":"Switch language"},
                {"command":"support","description":"Contact support"}
            ]
        }))
        .send()
        .await
        .context("Telegram setMyCommands request failed")?
        .error_for_status()
        .context("Telegram setMyCommands was rejected")?;
    let document: TelegramResponse<Value> = response
        .json()
        .await
        .context("Telegram setMyCommands response is invalid")?;
    if !document.ok {
        anyhow::bail!("Telegram setMyCommands was rejected");
    }
    Ok(())
}

async fn telegram_webhook(
    State(state): State<BotState>,
    headers: HeaderMap,
    Json(update): Json<TelegramUpdate>,
) -> StatusCode {
    let provided_secret = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    let expected_secret = state.webhook_secret.as_deref().unwrap_or_default();
    if !constant_time_equal(provided_secret.as_bytes(), expected_secret.as_bytes()) {
        return StatusCode::UNAUTHORIZED;
    }
    match process_update(&state, update).await {
        Ok(()) => StatusCode::OK,
        Err(error) => {
            error!(%error, "Telegram webhook update processing failed");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

async fn upsert_telegram_user(
    database: &PgPool,
    user: &TelegramUser,
) -> Result<TelegramUserInsert> {
    let mut transaction = database.begin().await?;
    let created_user = sqlx::query_as::<_, TelegramUserInsert>(
        "INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)
         ON CONFLICT (telegram_user_id) DO UPDATE SET updated_at = now()
         RETURNING id, (xmax = 0) AS inserted",
    )
    .bind(Uuid::now_v7())
    .bind(user.id)
    .fetch_one(&mut *transaction)
    .await?;
    let language = user
        .language_code
        .as_deref()
        .filter(|language| matches!(*language, "ru" | "en"))
        .unwrap_or("ru");
    sqlx::query(
        "INSERT INTO user_profiles (user_id, username, first_name, last_name, language_code)
          VALUES ($1, $2, $3, $4, $5)
          ON CONFLICT (user_id) DO UPDATE SET username = EXCLUDED.username,
            first_name = EXCLUDED.first_name, last_name = EXCLUDED.last_name,
            updated_at = now()",
    )
    .bind(created_user.id)
    .bind(&user.username)
    .bind(&user.first_name)
    .bind(&user.last_name)
    .bind(language)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(created_user.id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(created_user)
}

fn start_referrer_telegram_id(text: Option<&str>) -> Option<i64> {
    text?
        .strip_prefix("/start ")?
        .strip_prefix("ref_")?
        .parse()
        .ok()
        .filter(|telegram_user_id: &i64| *telegram_user_id > 0)
}

async fn attach_referral(
    database: &PgPool,
    new_user_id: Uuid,
    new_user_telegram_id: i64,
    source_telegram_id: i64,
) -> Result<()> {
    if new_user_telegram_id == source_telegram_id {
        return Ok(());
    }
    let mut transaction = database.begin().await?;
    let source_user = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM users WHERE telegram_user_id = $1 AND deleted_at IS NULL",
    )
    .bind(source_telegram_id)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(source_user) = source_user else {
        transaction.commit().await?;
        return Ok(());
    };
    sqlx::query(
        "INSERT INTO referrals (id, referrer_user_id, referred_user_id)
         VALUES ($1, $2, $3)
         ON CONFLICT (referred_user_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(source_user)
    .bind(new_user_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_command(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    text: &str,
    mini_app_url: &str,
    support_url: Option<&str>,
) -> Result<()> {
    if text.starts_with("/start plan") || text.starts_with("/start buy") {
        return show_plans(client, database, token, chat_id, user, mini_app_url).await;
    }
    if text.starts_with("/start access") {
        return show_access(client, database, token, chat_id, user, mini_app_url).await;
    }
    match text.split_whitespace().next().unwrap_or_default() {
        "/help" => show_help(client, token, chat_id, user).await,
        "/plans" => show_plans(client, database, token, chat_id, user, mini_app_url).await,
        "/trial" => {
            activate_trial_from_bot(client, database, token, chat_id, user, mini_app_url).await
        }
        "/access" => show_access(client, database, token, chat_id, user, mini_app_url).await,
        "/language" => {
            toggle_language(
                client,
                database,
                token,
                chat_id,
                user,
                mini_app_url,
                support_url,
            )
            .await
        }
        "/support" => show_support(client, token, chat_id, user, support_url).await,
        _ => {
            show_menu(
                client,
                database,
                token,
                chat_id,
                user,
                mini_app_url,
                support_url,
            )
            .await
        }
    }
}

async fn show_help(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
) -> Result<()> {
    let english = user.language_code.as_deref() == Some("en");
    send_message(
        client,
        token,
        chat_id,
        if english {
            "Commands:\n/menu - VPN menu\n/plans - plans\n/trial - trial access\n/access - access status\n/language - switch language\n/support - contact support"
        } else {
            "Команды:\n/menu - VPN-меню\n/plans - тарифы\n/trial - пробный доступ\n/access - статус доступа\n/language - сменить язык\n/support - поддержка"
        },
        None,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn handle_callback(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    user_id: Uuid,
    data: &str,
    mini_app_url: &str,
    support_url: Option<&str>,
) -> Result<()> {
    match data {
        "gate:verify" | "menu:home" => {
            show_menu(
                client,
                database,
                token,
                chat_id,
                user,
                mini_app_url,
                support_url,
            )
            .await
        }
        "menu:plans" => show_plans(client, database, token, chat_id, user, mini_app_url).await,
        "menu:trial" => {
            activate_trial_for_user(
                client,
                database,
                token,
                chat_id,
                user_id,
                language_for_user(database, user_id).await?,
                mini_app_url,
            )
            .await
        }
        "menu:access" => show_access(client, database, token, chat_id, user, mini_app_url).await,
        "menu:language" => {
            toggle_language(
                client,
                database,
                token,
                chat_id,
                user,
                mini_app_url,
                support_url,
            )
            .await
        }
        "menu:support" => show_support(client, token, chat_id, user, support_url).await,
        _ => Ok(()),
    }
}

async fn show_menu(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    mini_app_url: &str,
    support_url: Option<&str>,
) -> Result<()> {
    let bot_user = load_bot_user(database, user.id).await?;
    if !has_required_channels(client, database, token, user.id).await? {
        return show_required_channels(client, database, token, chat_id, &bot_user.language_code)
            .await;
    }
    let english = bot_user.language_code == "en";
    let text = if english {
        "VPN cabinet\n\nChoose an action. Payments and plan selection open securely in the Mini App."
    } else {
        "VPN-кабинет\n\nВыберите действие. Оплата и выбор тарифа безопасно откроются в Mini App."
    };
    let support_button = support_url
        .map(|url| json!({"text": if english {"Support"} else {"Поддержка"}, "url": url}));
    let mut keyboard = vec![
        vec![
            json!({"text": if english {"Open Mini App"} else {"Открыть Mini App"}, "web_app":{"url":mini_app_url}}),
        ],
        vec![
            json!({"text": if english {"Plans"} else {"Тарифы"}, "callback_data":"menu:plans"}),
            json!({"text": if english {"My access"} else {"Мой доступ"}, "callback_data":"menu:access"}),
        ],
        vec![
            json!({"text": if english {"Trial"} else {"Пробный доступ"}, "callback_data":"menu:trial"}),
            json!({"text":"RU / EN", "callback_data":"menu:language"}),
        ],
    ];
    if let Some(button) = support_button {
        keyboard.push(vec![button]);
    }
    send_message(
        client,
        token,
        chat_id,
        text,
        Some(json!({"inline_keyboard":keyboard})),
    )
    .await
}

async fn show_required_channels(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    language: &str,
) -> Result<()> {
    let channels = sqlx::query_as::<_, RequiredChannel>(
        "SELECT telegram_chat_id, title, public_url FROM required_channels WHERE is_active ORDER BY created_at",
    )
    .fetch_all(database)
    .await?;
    let english = language == "en";
    let mut keyboard = channels
        .into_iter()
        .filter_map(|channel| {
            channel
                .public_url
                .map(|url| vec![json!({"text":channel.title,"url":url})])
        })
        .collect::<Vec<_>>();
    keyboard.push(vec![json!({"text":if english {"Check subscription"} else {"Проверить подписку"},"callback_data":"gate:verify"})]);
    send_message(
        client,
        token,
        chat_id,
        if english {
            "Join the required channels, then check again."
        } else {
            "Подпишитесь на обязательные каналы, затем проверьте подписку."
        },
        Some(json!({"inline_keyboard":keyboard})),
    )
    .await
}

async fn show_plans(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    mini_app_url: &str,
) -> Result<()> {
    let language = language_for_telegram_user(database, user.id).await?;
    if !has_required_channels(client, database, token, user.id).await? {
        return show_required_channels(client, database, token, chat_id, &language).await;
    }
    let tariffs = sqlx::query_as::<_, BotTariff>(
        "SELECT t.name, t.duration_seconds, t.traffic_bytes, p.amount_minor, p.currency_code
         FROM tariffs t JOIN tariff_prices p ON p.tariff_id = t.id AND p.is_active
         WHERE t.is_active ORDER BY t.position, t.created_at",
    )
    .fetch_all(database)
    .await?;
    let english = language == "en";
    let lines = tariffs
        .into_iter()
        .map(|tariff| {
            let name = tariff
                .name
                .get(&language)
                .or_else(|| tariff.name.get("ru"))
                .or_else(|| tariff.name.get("en"))
                .and_then(Value::as_str)
                .unwrap_or("VPN");
            let days = tariff.duration_seconds.map_or(String::new(), |seconds| {
                format!(
                    " · {} {}",
                    seconds / 86_400,
                    if english { "days" } else { "дней" }
                )
            });
            let traffic = tariff.traffic_bytes.map_or(String::new(), |bytes| {
                format!(" · {} GB", bytes / 1_000_000_000)
            });
            format!(
                "{name}: {} {}{days}{traffic}",
                tariff.amount_minor / 100,
                tariff.currency_code
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    send_message(
        client,
        token,
        chat_id,
        &format!("{}\n\n{}", if english {"Available plans:"} else {"Доступные тарифы:"}, lines),
        Some(json!({"inline_keyboard":[[json!({"text":if english {"Choose and pay"} else {"Выбрать и оплатить"},"web_app":{"url":mini_app_url}})],[json!({"text":if english {"Back"} else {"Назад"},"callback_data":"menu:home"})]]})),
    )
    .await
}

async fn activate_trial_from_bot(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    mini_app_url: &str,
) -> Result<()> {
    let bot_user = load_bot_user(database, user.id).await?;
    activate_trial_for_user(
        client,
        database,
        token,
        chat_id,
        bot_user.id,
        bot_user.language_code,
        mini_app_url,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
async fn activate_trial_for_user(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user_id: Uuid,
    language: String,
    mini_app_url: &str,
) -> Result<()> {
    let telegram_user_id =
        sqlx::query_scalar::<_, i64>("SELECT telegram_user_id FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_one(database)
            .await?;
    if !has_required_channels(client, database, token, telegram_user_id).await? {
        return show_required_channels(client, database, token, chat_id, &language).await;
    }
    let settings = load_trial_settings(database).await?;
    let mut transaction = database.begin().await?;
    sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(user_id)
        .fetch_one(&mut *transaction)
        .await?;
    let already_entitled = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM subscriptions WHERE user_id = $1)",
    )
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await?;
    if already_entitled {
        transaction.commit().await?;
        return send_message(
            client,
            token,
            chat_id,
            if language == "en" {
                "Trial is no longer available for this account."
            } else {
                "Пробный доступ для этого аккаунта уже недоступен."
            },
            Some(open_app_keyboard(&language, mini_app_url)),
        )
        .await;
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
    .await?;
    sqlx::query(
        "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
         VALUES ($1, 'subscription', $2, 'subscription.requested', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"subscription_id": subscription_id, "user_id": user_id, "duration_seconds": settings.duration_seconds, "version": 1}))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    send_message(
        client,
        token,
        chat_id,
        if language == "en" {
            "Your trial is being prepared. We will notify you when access is ready."
        } else {
            "Пробный доступ создаётся. Мы уведомим вас, когда он будет готов."
        },
        Some(open_app_keyboard(&language, mini_app_url)),
    )
    .await
}

async fn show_access(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    mini_app_url: &str,
) -> Result<()> {
    let bot_user = load_bot_user(database, user.id).await?;
    if !has_required_channels(client, database, token, user.id).await? {
        return show_required_channels(client, database, token, chat_id, &bot_user.language_code)
            .await;
    }
    let subscription = sqlx::query_as::<_, BotSubscription>(
        "SELECT s.status, s.expires_at, (a.encrypted_access_url IS NOT NULL) AS access_available
         FROM subscriptions s LEFT JOIN vpn_accounts a ON a.id = s.vpn_account_id
         WHERE s.user_id = $1
         ORDER BY CASE s.status WHEN 'active' THEN 0 WHEN 'provisioning_pending' THEN 1 ELSE 2 END, s.created_at DESC LIMIT 1",
    )
    .bind(bot_user.id)
    .fetch_optional(database)
    .await?;
    let text = match subscription {
        Some(subscription) if subscription.status == "active" && subscription.access_available => {
            let expiry = subscription.expires_at.map_or_else(
                || "-".to_owned(),
                |value| value.format("%Y-%m-%d %H:%M UTC").to_string(),
            );
            if bot_user.language_code == "en" {
                format!(
                    "Your access is active until {expiry}. Open the Mini App to copy the protected subscription link and follow connection guides."
                )
            } else {
                format!(
                    "Ваш доступ активен до {expiry}. Откройте Mini App, чтобы скопировать защищённую ссылку подписки и посмотреть инструкции."
                )
            }
        }
        Some(subscription) if subscription.status == "provisioning_pending" => {
            if bot_user.language_code == "en" {
                "Your access is still being prepared.".to_owned()
            } else {
                "Ваш доступ ещё создаётся.".to_owned()
            }
        }
        _ => {
            if bot_user.language_code == "en" {
                "No active access yet. Choose a plan or start a trial.".to_owned()
            } else {
                "Активного доступа пока нет. Выберите тариф или начните пробный период.".to_owned()
            }
        }
    };
    send_message(
        client,
        token,
        chat_id,
        &text,
        Some(open_app_keyboard(&bot_user.language_code, mini_app_url)),
    )
    .await
}

async fn toggle_language(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    mini_app_url: &str,
    support_url: Option<&str>,
) -> Result<()> {
    let bot_user = load_bot_user(database, user.id).await?;
    let language = if bot_user.language_code == "ru" {
        "en"
    } else {
        "ru"
    };
    sqlx::query(
        "UPDATE user_profiles SET language_code = $1, updated_at = now() WHERE user_id = $2",
    )
    .bind(language)
    .bind(bot_user.id)
    .execute(database)
    .await?;
    let updated_user = TelegramUser {
        id: user.id,
        username: user.username.clone(),
        first_name: user.first_name.clone(),
        last_name: user.last_name.clone(),
        language_code: Some(language.to_owned()),
    };
    show_menu(
        client,
        database,
        token,
        chat_id,
        &updated_user,
        mini_app_url,
        support_url,
    )
    .await
}

async fn show_support(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    user: &TelegramUser,
    support_url: Option<&str>,
) -> Result<()> {
    let english = user.language_code.as_deref() == Some("en");
    let markup = support_url.map(|url| json!({"inline_keyboard":[[{"text":if english {"Open support"} else {"Открыть поддержку"},"url":url}]]}));
    send_message(
        client,
        token,
        chat_id,
        if english {
            "Contact support using the button below."
        } else {
            "Свяжитесь с поддержкой по кнопке ниже."
        },
        markup,
    )
    .await
}

async fn load_bot_user(database: &PgPool, telegram_user_id: i64) -> Result<BotUser> {
    sqlx::query_as::<_, BotUser>(
        "SELECT u.id, p.language_code FROM users u JOIN user_profiles p ON p.user_id = u.id
         WHERE u.telegram_user_id = $1 AND u.deleted_at IS NULL",
    )
    .bind(telegram_user_id)
    .fetch_one(database)
    .await
    .context("Telegram user is missing")
}

async fn language_for_telegram_user(database: &PgPool, telegram_user_id: i64) -> Result<String> {
    Ok(load_bot_user(database, telegram_user_id)
        .await?
        .language_code)
}

async fn language_for_user(database: &PgPool, user_id: Uuid) -> Result<String> {
    sqlx::query_scalar("SELECT language_code FROM user_profiles WHERE user_id = $1")
        .bind(user_id)
        .fetch_one(database)
        .await
        .context("user language is missing")
}

async fn load_trial_settings(database: &PgPool) -> Result<TrialSettings> {
    let settings = sqlx::query_scalar::<_, Value>(
        "SELECT value FROM app_settings WHERE key = 'trial_settings'",
    )
    .fetch_optional(database)
    .await?
    .unwrap_or_else(|| json!({"duration_seconds":259_200,"traffic_bytes":10_000_000_000_i64}));
    serde_json::from_value(settings).context("trial settings are invalid")
}

async fn has_required_channels(
    client: &reqwest::Client,
    database: &PgPool,
    token: &str,
    telegram_user_id: i64,
) -> Result<bool> {
    let channels = sqlx::query_as::<_, RequiredChannel>(
        "SELECT telegram_chat_id, title, public_url FROM required_channels WHERE is_active ORDER BY created_at",
    )
    .fetch_all(database)
    .await?;
    for channel in channels {
        let response = client
            .get(format!("{TELEGRAM_API_BASE_URL}/bot{token}/getChatMember"))
            .query(&[
                ("chat_id", channel.telegram_chat_id),
                ("user_id", telegram_user_id),
            ])
            .send()
            .await?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let document: TelegramResponse<TelegramChatMember> = response.json().await?;
        let member = document.result.filter(|_| document.ok);
        let is_member = member.is_some_and(|member| {
            matches!(
                member.status.as_str(),
                "creator" | "administrator" | "member"
            ) || (member.status == "restricted" && member.is_member == Some(true))
        });
        if !is_member {
            return Ok(false);
        }
    }
    Ok(true)
}

async fn answer_callback(
    client: &reqwest::Client,
    token: &str,
    callback_query_id: &str,
) -> Result<()> {
    client
        .post(format!(
            "{TELEGRAM_API_BASE_URL}/bot{token}/answerCallbackQuery"
        ))
        .json(&json!({"callback_query_id":callback_query_id}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn open_app_keyboard(language: &str, mini_app_url: &str) -> Value {
    json!({"inline_keyboard":[[json!({"text":if language == "en" {"Open Mini App"} else {"Открыть Mini App"},"web_app":{"url":mini_app_url}})],[json!({"text":if language == "en" {"Back"} else {"Назад"},"callback_data":"menu:home"})]]})
}

async fn send_message(
    client: &reqwest::Client,
    token: &str,
    chat_id: i64,
    text: &str,
    reply_markup: Option<Value>,
) -> Result<()> {
    let mut payload = json!({"chat_id":chat_id,"text":text});
    if let Some(reply_markup) = reply_markup {
        payload["reply_markup"] = reply_markup;
    }
    client
        .post(format!("{TELEGRAM_API_BASE_URL}/bot{token}/sendMessage"))
        .json(&payload)
        .send()
        .await
        .context("Telegram sendMessage failed")?
        .error_for_status()
        .context("Telegram sendMessage was rejected")?;
    Ok(())
}

fn validate_mini_app_url(value: &str) -> Result<()> {
    let parsed = url::Url::parse(value).context("MINI_APP_PUBLIC_URL is invalid")?;
    if parsed.scheme() != "https"
        && !(parsed.scheme() == "http"
            && matches!(parsed.host_str(), Some("localhost" | "127.0.0.1")))
    {
        anyhow::bail!("MINI_APP_PUBLIC_URL must use HTTPS outside local development");
    }
    Ok(())
}

async fn fulfill_telegram_stars_payment(
    database: &PgPool,
    update_id: i64,
    payment: TelegramSuccessfulPayment,
) -> Result<()> {
    if payment.currency != "XTR" || payment.telegram_payment_charge_id.is_empty() {
        anyhow::bail!("Telegram Stars payment has invalid currency or charge ID");
    }
    let invoice_id = payment
        .invoice_payload
        .parse::<Uuid>()
        .context("Telegram Stars payload is not a local invoice UUID")?;
    let mut transaction = database.begin().await?;
    let event_inserted = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO payment_webhook_events (id, provider, provider_event_id, payload)
         VALUES ($1, 'telegram_stars', $2, $3)
         ON CONFLICT (provider, provider_event_id) DO NOTHING
         RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(update_id.to_string())
    .bind(serde_json::json!({
        "invoice_payload": payment.invoice_payload,
        "currency": payment.currency,
        "total_amount": payment.total_amount,
        "telegram_payment_charge_id": payment.telegram_payment_charge_id,
    }))
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(event_id) = event_inserted else {
        transaction.commit().await?;
        return Ok(());
    };
    let invoice = sqlx::query_as::<_, PendingStarsInvoice>(
        "SELECT id, user_id, purpose, currency_code, amount_minor, tariff_id
         FROM invoices WHERE id = $1 AND provider = 'telegram_stars' FOR UPDATE",
    )
    .bind(invoice_id)
    .fetch_optional(&mut *transaction)
    .await?
    .context("Telegram Stars invoice is unknown")?;
    if invoice.currency_code != payment.currency || invoice.amount_minor != payment.total_amount {
        anyhow::bail!("Telegram Stars amount does not match the local invoice");
    }
    let updated = sqlx::query(
        "UPDATE invoices SET status = 'paid', paid_at = now(), updated_at = now()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(invoice.id)
    .execute(&mut *transaction)
    .await?;
    if updated.rows_affected() > 0 {
        let payment_attempt = sqlx::query(
            "INSERT INTO payment_attempts (id, invoice_id, provider, provider_payment_id, status)
             VALUES ($1, $2, 'telegram_stars', $3, 'paid')
             ON CONFLICT (provider, provider_payment_id) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(invoice.id)
        .bind(&payment.telegram_payment_charge_id)
        .execute(&mut *transaction)
        .await?;
        if payment_attempt.rows_affected() == 0 {
            anyhow::bail!("Telegram Stars charge ID was already used");
        }
        fulfill_paid_invoice(
            &mut transaction,
            &InvoiceForFulfillment {
                id: invoice.id,
                user_id: invoice.user_id,
                purpose: invoice.purpose,
                status: "paid".to_owned(),
                currency_code: invoice.currency_code,
                amount_minor: invoice.amount_minor,
                tariff_id: invoice.tariff_id,
            },
        )
        .await?;
    }
    sqlx::query("UPDATE payment_webhook_events SET processed_at = now() WHERE id = $1")
        .bind(event_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        routing::{get, post},
    };
    use serde_json::{Value, json};
    use sqlx::{PgPool, postgres::PgPoolOptions};
    use uuid::Uuid;

    use super::{
        BotState, TelegramSuccessfulPayment, fulfill_telegram_stars_payment,
        poll_updates_from_base, register_commands_at_base,
    };

    #[derive(Clone)]
    struct MockTelegramState {
        get_updates_calls: Arc<AtomicUsize>,
        commands_payload: Arc<Mutex<Option<Value>>>,
    }

    async fn mock_get_updates(State(state): State<MockTelegramState>) -> (StatusCode, Json<Value>) {
        if state.get_updates_calls.fetch_add(1, Ordering::SeqCst) == 0 {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"ok": false})),
            );
        }
        (StatusCode::OK, Json(json!({"ok": true, "result": []})))
    }

    async fn mock_set_commands(
        State(state): State<MockTelegramState>,
        Json(payload): Json<Value>,
    ) -> Json<Value> {
        *state.commands_payload.lock().unwrap() = Some(payload);
        Json(json!({"ok": true, "result": true}))
    }

    async fn mock_telegram_server() -> (String, MockTelegramState, tokio::task::JoinHandle<()>) {
        let state = MockTelegramState {
            get_updates_calls: Arc::new(AtomicUsize::new(0)),
            commands_payload: Arc::new(Mutex::new(None)),
        };
        let app = Router::new()
            .route("/bottest-token/getUpdates", get(mock_get_updates))
            .route("/bottest-token/setMyCommands", post(mock_set_commands))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), state, task)
    }

    fn test_bot_state() -> BotState {
        BotState {
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            database: PgPoolOptions::new()
                .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
                .unwrap(),
            token: Arc::from("test-token"),
            mini_app_url: Arc::from("https://example.invalid"),
            support_url: None,
            webhook_secret: None,
        }
    }

    async fn postgres_test_pool() -> Option<PgPool> {
        let url = std::env::var("VPN_TEST_DATABASE_URL").ok()?;
        PgPool::connect(&url).await.ok()
    }

    fn test_telegram_user_id() -> i64 {
        i64::from_be_bytes(Uuid::now_v7().as_bytes()[8..].try_into().unwrap()) & i64::MAX
    }

    #[tokio::test]
    async fn mock_telegram_transport_recovers_after_poll_failure_and_registers_commands() {
        let (base_url, mock_state, server) = mock_telegram_server().await;
        let state = test_bot_state();
        let mut offset = None;

        assert!(
            poll_updates_from_base(&state, &mut offset, &base_url)
                .await
                .is_err()
        );
        poll_updates_from_base(&state, &mut offset, &base_url)
            .await
            .unwrap();
        register_commands_at_base(&state, &base_url).await.unwrap();

        assert_eq!(mock_state.get_updates_calls.load(Ordering::SeqCst), 2);
        let payload = mock_state.commands_payload.lock().unwrap().clone().unwrap();
        assert_eq!(payload["commands"][0]["command"], "menu");
        assert_eq!(payload["commands"][6]["command"], "support");
        server.abort();
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn telegram_stars_payment_replay_creates_one_direct_purchase() {
        let Some(pool) = postgres_test_pool().await else {
            return;
        };
        let user_id = Uuid::now_v7();
        let tariff_id = Uuid::now_v7();
        let invoice_id = Uuid::now_v7();
        let update_id = 9_000_000_000_i64 + i64::from(Uuid::now_v7().as_bytes()[0]);
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
             (id, user_id, provider, purpose, status, currency_code, amount_minor, tariff_id, expires_at)
             VALUES ($1, $2, 'telegram_stars', 'direct_purchase', 'pending', 'XTR', 500, $3, now() + interval '1 hour')",
        )
        .bind(invoice_id)
        .bind(user_id)
        .bind(tariff_id)
        .execute(&pool)
        .await
        .unwrap();
        let payment = TelegramSuccessfulPayment {
            currency: "XTR".to_owned(),
            total_amount: 500,
            invoice_payload: invoice_id.to_string(),
            telegram_payment_charge_id: format!("charge-{invoice_id}"),
        };
        fulfill_telegram_stars_payment(&pool, update_id, payment)
            .await
            .unwrap();
        fulfill_telegram_stars_payment(
            &pool,
            update_id,
            TelegramSuccessfulPayment {
                currency: "XTR".to_owned(),
                total_amount: 500,
                invoice_payload: invoice_id.to_string(),
                telegram_payment_charge_id: format!("charge-{invoice_id}"),
            },
        )
        .await
        .unwrap();

        let subscriptions = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM subscriptions WHERE source_invoice_id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let webhook_events = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM payment_webhook_events WHERE provider = 'telegram_stars' AND provider_event_id = $1",
        )
        .bind(update_id.to_string())
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
        sqlx::query("DELETE FROM payment_webhook_events WHERE provider = 'telegram_stars' AND provider_event_id = $1")
            .bind(update_id.to_string())
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
}
