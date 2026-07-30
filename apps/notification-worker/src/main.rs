use std::{env, time::Duration};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info};
use uuid::Uuid;
use vpn_observability::init_tracing;
use vpn_storage::{connect, decode_encryption_key, load_app_secret};

#[derive(Debug, FromRow)]
struct PendingNotification {
    id: Uuid,
    telegram_user_id: i64,
    kind: String,
    locale: String,
    payload: Value,
}

async fn load_secret_from_db(
    database: &PgPool,
    encryption_key: &[u8; 32],
    key: &str,
) -> Result<String> {
    load_app_secret(database, encryption_key, key)
        .await?
        .filter(|value| !value.is_empty())
        .context(format!("secret {key} not found in app_secrets"))
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let pool = connect(&database_url)
        .await
        .context("database connection failed")?;
    let encryption_key = decode_encryption_key(&env::var("APPLICATION_ENCRYPTION_KEY")?)?;
    let telegram_bot_token = match env::var("TELEGRAM_BOT_TOKEN") {
        Ok(value) if !value.is_empty() => value,
        _ => load_secret_from_db(&pool, &encryption_key, "TELEGRAM_BOT_TOKEN")
            .await
            .context("TELEGRAM_BOT_TOKEN is required (set in env or app_secrets)")?,
    };
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .context("notification HTTP client initialization failed")?;

    info!("notification worker started");
    let mut ticker = interval(Duration::from_secs(3));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(error) = materialize_one_notification(&pool).await {
                    error!(%error, "notification outbox processing failed");
                }
                if let Err(error) = deliver_one_notification(&pool, &client, &telegram_bot_token).await {
                    error!(%error, "notification delivery failed");
                }
            }
            result = tokio::signal::ctrl_c() => {
                result.context("failed to wait for signal")?;
                break;
            }
        }
    }
    info!("notification worker stopped");
    Ok(())
}

async fn materialize_one_notification(pool: &PgPool) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let event = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, payload FROM outbox_events
         WHERE event_type = 'notification.requested' AND processed_at IS NULL AND available_at <= now()
         ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((event_id, payload)) = event else {
        transaction.commit().await?;
        return Ok(());
    };
    let user_id = payload
        .get("user_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
        .context("notification event has no user_id")?;
    let kind = payload
        .get("kind")
        .and_then(Value::as_str)
        .context("notification event has no kind")?
        .to_owned();
    let locale = sqlx::query_scalar::<_, String>(
        "SELECT language_code FROM user_profiles WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO notifications (id, source_event_id, user_id, kind, locale, payload)
          VALUES ($1, $2, $3, $4, $5, $6)
          ON CONFLICT (source_event_id) DO NOTHING",
    )
    .bind(Uuid::now_v7())
    .bind(event_id)
    .bind(user_id)
    .bind(kind)
    .bind(locale)
    .bind(payload)
    .execute(&mut *transaction)
    .await?;
    sqlx::query("UPDATE outbox_events SET processed_at = now() WHERE id = $1")
        .bind(event_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

async fn deliver_one_notification(
    pool: &PgPool,
    client: &reqwest::Client,
    telegram_bot_token: &str,
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let notification = sqlx::query_as::<_, PendingNotification>(
        "SELECT n.id, u.telegram_user_id, n.kind, n.locale, n.payload
         FROM notifications n JOIN users u ON u.id = n.user_id
         WHERE n.status = 'pending' AND n.available_at <= now()
         ORDER BY n.created_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(notification) = notification else {
        transaction.commit().await?;
        return Ok(());
    };
    sqlx::query(
        "UPDATE notifications SET attempts = attempts + 1, available_at = now() + interval '30 seconds' WHERE id = $1",
    )
        .bind(notification.id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    let text = render_notification(&notification);
    let result = client
        .post(format!(
            "https://api.telegram.org/bot{telegram_bot_token}/sendMessage"
        ))
        .json(&json!({"chat_id": notification.telegram_user_id, "text": text}))
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {
            sqlx::query("UPDATE notifications SET status = 'sent', sent_at = now() WHERE id = $1")
                .bind(notification.id)
                .execute(pool)
                .await?;
        }
        Ok(response)
            if response.status().is_client_error() && response.status().as_u16() != 429 =>
        {
            sqlx::query(
                "UPDATE notifications SET status = 'failed', last_error = $1 WHERE id = $2",
            )
            .bind(format!(
                "Telegram returned permanent HTTP status {}",
                response.status()
            ))
            .bind(notification.id)
            .execute(pool)
            .await?;
        }
        Ok(response) => {
            defer_notification(
                pool,
                notification.id,
                &format!("Telegram returned HTTP status {}", response.status()),
            )
            .await?;
        }
        Err(error) => {
            defer_notification(pool, notification.id, &error.to_string()).await?;
        }
    }
    Ok(())
}

async fn defer_notification(pool: &PgPool, notification_id: Uuid, error: &str) -> Result<()> {
    sqlx::query(
                "UPDATE notifications SET status = CASE WHEN attempts >= 10 THEN 'failed' ELSE 'pending' END,
                 available_at = now() + make_interval(secs => LEAST(3600, 30 * (2 ^ LEAST(attempts, 6)))),
                 last_error = $1 WHERE id = $2",
            )
            .bind(error)
            .bind(notification_id)
            .execute(pool)
            .await?;
    Ok(())
}

fn render_notification(notification: &PendingNotification) -> String {
    match (notification.kind.as_str(), notification.locale.as_str()) {
        ("subscription_active", "ru") => {
            "VPN-подписка активирована. Откройте Mini App, чтобы получить доступ.".to_owned()
        }
        ("subscription_active", _) => {
            "Your VPN subscription is active. Open the Mini App to get access.".to_owned()
        }
        ("subscription_expired", "ru") => {
            "VPN-подписка истекла. Откройте Mini App, чтобы продлить доступ.".to_owned()
        }
        ("subscription_expired", _) => {
            "Your VPN subscription has expired. Open the Mini App to renew access.".to_owned()
        }
        ("wallet_topped_up", "ru") => {
            "Кошелёк пополнен. Средства доступны для покупки VPN-подписки.".to_owned()
        }
        ("wallet_topped_up", _) => {
            "Your wallet was topped up. Funds are available for a VPN subscription purchase."
                .to_owned()
        }
        ("referral_reward", "ru") => {
            let amount = notification
                .payload
                .get("amount_minor")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            format!("Реферальное вознаграждение зачислено: {amount} мин. ед.")
        }
        ("referral_reward", _) => {
            let amount = notification
                .payload
                .get("amount_minor")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            format!("Referral reward credited: {amount} minor units.")
        }
        _ => format!("Notification: {}", notification.payload),
    }
}
