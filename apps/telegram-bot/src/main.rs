use std::{env, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use sqlx::PgPool;
use tracing::{error, info, warn};
use url::Url;
use uuid::Uuid;
use vpn_integrations::{
    SendMessageRequest, TelegramClient, TelegramError, TelegramMessage, TelegramUpdate,
    rich_text::escape_html,
};
use vpn_observability::init_tracing;
use vpn_storage::{connect, decode_encryption_key, load_app_secret};

const API_BASE_URL: &str = "https://api.telegram.org/";

#[derive(Clone)]
struct BotState {
    database: PgPool,
    telegram: TelegramClient,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let encryption_key = decode_encryption_key(
        &env::var("APPLICATION_ENCRYPTION_KEY")
            .context("APPLICATION_ENCRYPTION_KEY is required")?,
    )?;
    let database = connect(&database_url)
        .await
        .context("database connection failed")?;
    let token = load_app_secret(&database, &encryption_key, "TELEGRAM_BOT_TOKEN")
        .await
        .context("Telegram secret lookup failed")?
        .filter(|value| !value.is_empty())
        .context("TELEGRAM_BOT_TOKEN is not configured in Admin UI")?;
    let api_base = env::var("TELEGRAM_API_BASE_URL").unwrap_or_else(|_| API_BASE_URL.to_owned());
    let telegram = TelegramClient::new(
        Arc::<str>::from(token),
        Url::parse(&api_base).context("TELEGRAM_API_BASE_URL is invalid")?,
    )?;
    let state = BotState { database, telegram };
    info!(
        component = "telegram-bot",
        "Telegram polling transport started"
    );
    run_polling(state).await
}

async fn run_polling(state: BotState) -> Result<()> {
    let mut offset = None;
    loop {
        let result = tokio::select! {
            updates = state.telegram.get_updates(offset) => Some(updates),
            signal = shutdown_signal() => { signal?; None },
        };
        let Some(result) = result else {
            return Ok(());
        };
        match result {
            Ok(updates) => {
                for update in updates {
                    offset = Some(update.update_id.saturating_add(1));
                    if let Err(error) = process_update(&state, &update).await {
                        error!(update_id = update.update_id, %error, "Telegram update processing failed");
                        mark_update_failed(&state.database, update.update_id, &error.to_string())
                            .await;
                    }
                }
            }
            Err(TelegramError::RetryAfter(seconds)) => {
                warn!(seconds, "Telegram requested a retry delay");
                tokio::time::sleep(Duration::from_secs(seconds.min(60))).await;
            }
            Err(error) if error.is_transient() => {
                warn!(%error, "Telegram transport failed; retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn process_update(state: &BotState, update: &TelegramUpdate) -> Result<()> {
    if !claim_update(&state.database, update.update_id).await? {
        return Ok(());
    }
    if let Some(message) = &update.message {
        if let Some(text) = &message.text {
            if text.split_whitespace().next() == Some("/start") {
                handle_start(state, message).await?;
            }
        }
    }
    sqlx::query("UPDATE telegram_updates SET processed_at = now() WHERE update_id = $1")
        .bind(update.update_id)
        .execute(&state.database)
        .await?;
    Ok(())
}

async fn handle_start(state: &BotState, message: &TelegramMessage) -> Result<()> {
    let Some(user) = &message.from else {
        return Ok(());
    };
    let user_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)
         ON CONFLICT (telegram_user_id) DO UPDATE SET updated_at = now()",
    )
    .bind(user_id)
    .bind(user.id)
    .execute(&state.database)
    .await?;
    let local_id =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE telegram_user_id = $1")
            .bind(user.id)
            .fetch_one(&state.database)
            .await?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, username, first_name, last_name, language_code)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id) DO UPDATE SET username = EXCLUDED.username,
           first_name = EXCLUDED.first_name, last_name = EXCLUDED.last_name,
           language_code = EXCLUDED.language_code, updated_at = now()",
    )
    .bind(local_id)
    .bind(&user.username)
    .bind(&user.first_name)
    .bind(&user.last_name)
    .bind(user.language_code.as_deref().unwrap_or("ru"))
    .execute(&state.database)
    .await?;
    sqlx::query(
        "INSERT INTO wallets (id, user_id) VALUES ($1, $2) ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(local_id)
    .execute(&state.database)
    .await?;

    let first_name = escape_html(&user.first_name);
    let text = format!(
        "<b>VECTOR</b>\nWelcome, {first_name}. Your private route is ready to configure.\n\nOpen the Mini App to choose a plan or connect an existing subscription.",
    );
    state
        .telegram
        .send_message(&SendMessageRequest {
            chat_id: message.chat.id,
            text: &text,
            parse_mode: "HTML",
        })
        .await
        .map(|_| ())
        .map_err(Into::into)
}

async fn claim_update(database: &PgPool, update_id: i64) -> Result<bool> {
    let result = sqlx::query(
        "INSERT INTO telegram_updates (update_id, claimed_at)
         VALUES ($1, now()) ON CONFLICT (update_id) DO NOTHING",
    )
    .bind(update_id)
    .execute(database)
    .await?;
    Ok(result.rows_affected() == 1)
}

async fn mark_update_failed(database: &PgPool, update_id: i64, error: &str) {
    let redacted = error.chars().take(500).collect::<String>();
    let _ = sqlx::query("UPDATE telegram_updates SET processing_error = $2 WHERE update_id = $1")
        .bind(update_id)
        .bind(redacted)
        .execute(database)
        .await;
}

async fn shutdown_signal() -> Result<()> {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let terminate = async {
            let mut signal =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .context("failed to install SIGTERM handler")?;
            signal.recv().await.context("SIGTERM stream ended")
        };
        tokio::select! {
            result = ctrl_c => result.context("failed to wait for Ctrl-C"),
            result = terminate => result,
        }
    }
    #[cfg(not(unix))]
    ctrl_c.await.context("failed to wait for Ctrl-C")
}
