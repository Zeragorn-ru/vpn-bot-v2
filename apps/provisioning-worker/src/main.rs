use std::{env, sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use sqlx::{FromRow, PgPool, Row, postgres::PgRow};
use tracing::{error, info};
use url::Url;
use uuid::Uuid;
use vpn_integrations::provider::RemnawaveClient;
use vpn_observability::init_tracing;
use vpn_storage::{connect, decode_encryption_key, load_app_secret, store_provider_snapshot};

#[derive(Clone)]
struct WorkerState {
    database: PgPool,
    encryption_key: Arc<[u8; 32]>,
}

#[derive(Debug)]
struct OutboxItem {
    id: Uuid,
    aggregate_id: Uuid,
    event_type: String,
}

impl<'row> FromRow<'row, PgRow> for OutboxItem {
    fn from_row(row: &'row PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            aggregate_id: row.try_get("aggregate_id")?,
            event_type: row.try_get("event_type")?,
        })
    }
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
    let state = WorkerState {
        database,
        encryption_key: Arc::new(encryption_key),
    };
    info!(
        component = "provisioning-worker",
        "provisioning worker started"
    );
    run(state).await
}

async fn run(state: WorkerState) -> Result<()> {
    let worker_id = format!("provisioning-{}", Uuid::now_v7());
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        tokio::select! {
            _ = interval.tick() => {
                while let Some(item) = claim_event(&state.database, &worker_id).await? {
                    if let Err(error) = process_event(&state, &item).await {
                        error!(event_id = %item.id, subscription_id = %item.aggregate_id, %error, "provisioning event failed");
                        fail_event(&state.database, item.id, &error.to_string()).await;
                    }
                }
            }
            result = shutdown_signal() => { result?; return Ok(()); }
        }
    }
}

async fn claim_event(database: &PgPool, worker_id: &str) -> Result<Option<OutboxItem>> {
    let item = sqlx::query_as::<_, OutboxItem>(
        "UPDATE outbox_events SET locked_at = now(), lock_owner = $1, attempts = attempts + 1
         WHERE id = (
           SELECT id FROM outbox_events
           WHERE processed_at IS NULL AND available_at <= now()
             AND event_type = 'subscription.requested'
             AND (locked_at IS NULL OR locked_at < now() - interval '5 minutes')
           ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1
         ) RETURNING id, aggregate_id, event_type",
    )
    .bind(worker_id)
    .fetch_optional(database)
    .await?;
    Ok(item)
}

async fn process_event(state: &WorkerState, item: &OutboxItem) -> Result<()> {
    if item.event_type != "subscription.requested" {
        return Err(anyhow!("unsupported provisioning event"));
    }
    let telegram_user_id = sqlx::query_scalar::<_, i64>(
        "SELECT u.telegram_user_id FROM subscriptions s JOIN users u ON u.id = s.user_id WHERE s.id = $1",
    )
    .bind(item.aggregate_id)
    .fetch_optional(&state.database)
    .await?
    .context("subscription user is missing")?;
    let endpoint = load_app_secret(&state.database, &state.encryption_key, "REMNAWAVE_ENDPOINT")
        .await?
        .context("Remnawave endpoint is not configured")?;
    let token = load_app_secret(&state.database, &state.encryption_key, "REMNAWAVE_TOKEN")
        .await?
        .context("Remnawave token is not configured")?;
    let client = RemnawaveClient::new(
        Url::parse(&endpoint).context("Remnawave endpoint is invalid")?,
        token,
    )
    .map_err(|_| anyhow!("provider client initialization failed"))?;
    let profile = client
        .fetch_profile(telegram_user_id)
        .await
        .map_err(|_| anyhow!("provider snapshot fetch failed"))?;
    let fresh_until = Utc::now() + ChronoDuration::seconds(90);
    store_provider_snapshot(
        &state.database,
        &state.encryption_key,
        item.aggregate_id,
        &profile,
        None,
        fresh_until,
    )
    .await?;
    let expires_at = profile
        .expires_at
        .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0));
    sqlx::query(
        "UPDATE subscriptions SET status = 'active', starts_at = COALESCE(starts_at, now()),
         expires_at = COALESCE($2, expires_at), traffic_used_bytes = $3,
         traffic_limit_bytes = $4, updated_at = now() WHERE id = $1",
    )
    .bind(item.aggregate_id)
    .bind(expires_at)
    .bind(profile.traffic_used_bytes)
    .bind(profile.traffic_limit_bytes)
    .execute(&state.database)
    .await?;
    sqlx::query("UPDATE outbox_events SET processed_at = now(), locked_at = NULL, lock_owner = NULL WHERE id = $1")
        .bind(item.id)
        .execute(&state.database)
        .await?;
    Ok(())
}

async fn fail_event(database: &PgPool, event_id: Uuid, error: &str) {
    let message = error.chars().take(500).collect::<String>();
    let _ = sqlx::query(
        "UPDATE outbox_events SET last_error = $2, locked_at = NULL, lock_owner = NULL,
         available_at = now() + LEAST(interval '15 minutes', interval '2 seconds' * power(2, LEAST(attempts, 9)))
         WHERE id = $1",
    )
    .bind(event_id)
    .bind(message)
    .execute(database)
    .await;
}

async fn shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c()
        .await
        .context("failed to wait for Ctrl-C")
}
