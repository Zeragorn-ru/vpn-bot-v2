use std::{env, time::Duration};

use aes_gcm::{Aes256Gcm, KeyInit, Nonce, aead::Aead};
use anyhow::{Context, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use rand::RngCore;
use serde_json::{Value, json};
use sqlx::{FromRow, PgPool};
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info};
use uuid::Uuid;
use vpn_domain::DEFAULT_TRIAL_DURATION_SECONDS;
use vpn_integrations::{ProvisionRequest, RemnawaveConfig, RemnawaveProvider, VpnProvider};
use vpn_observability::init_tracing;
use vpn_storage::connect;

const OUTBOX_LOCK_LEASE_SECONDS: i64 = 300;

#[derive(Debug, FromRow)]
struct PendingSubscription {
    user_id: Uuid,
    telegram_user_id: i64,
    username: Option<String>,
    duration_seconds: Option<i64>,
    traffic_bytes: Option<i64>,
    current_expiry: Option<DateTime<Utc>>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let encryption_key = load_encryption_key()?;
    let provider = RemnawaveProvider::new(load_remnawave_config()?)?;
    let pool = connect(&database_url)
        .await
        .context("database connection failed")?;

    info!("provisioning worker started");
    let mut ticker = interval(Duration::from_secs(5));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(error) = process_one_subscription(&pool, &provider, &encryption_key).await {
                    error!(%error, "subscription provisioning attempt failed");
                }
                if let Err(error) = expire_one_subscription(&pool).await {
                    error!(%error, "subscription expiry processing failed");
                }
                if let Err(error) = process_one_block_request(&pool, &provider).await {
                    error!(%error, "subscription block attempt failed");
                }
                if let Err(error) = reconcile_one_provider_account(&pool, &provider).await {
                    error!(%error, "provider account reconciliation failed");
                }
            }
            result = tokio::signal::ctrl_c() => {
                result.context("failed to wait for signal")?;
                break;
            }
        }
    }
    info!("provisioning worker stopped");
    Ok(())
}

async fn expire_one_subscription(pool: &PgPool) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let subscription = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT id, user_id FROM subscriptions
         WHERE status = 'active' AND expires_at <= now()
         ORDER BY expires_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((subscription_id, user_id)) = subscription else {
        transaction.commit().await?;
        return Ok(());
    };
    sqlx::query(
        "UPDATE subscriptions SET status = 'expired', updated_at = now()
         WHERE id = $1 AND status = 'active'",
    )
    .bind(subscription_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO subscription_events (id, subscription_id, event_type, details)
         VALUES ($1, $2, 'expired', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"expired_at": Utc::now()}))
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
          VALUES ($1, 'subscription', $2, 'subscription.changed', $3),
                 ($4, 'notification', $2, 'notification.requested', $5),
                 ($6, 'subscription', $2, 'subscription.block_requested', $7)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"subscription_id": subscription_id, "status": "expired", "version": 1}))
    .bind(Uuid::now_v7())
    .bind(json!({"user_id": user_id, "kind": "subscription_expired", "subscription_id": subscription_id, "version": 1}))
    .bind(Uuid::now_v7())
    .bind(json!({"subscription_id": subscription_id, "version": 1}))
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    info!(%subscription_id, "subscription expired");
    Ok(())
}

async fn process_one_block_request(pool: &PgPool, provider: &RemnawaveProvider) -> Result<()> {
    let Some((event_id, payload)) =
        claim_next_outbox_event(pool, "subscription.block_requested").await?
    else {
        return Ok(());
    };
    let Some(subscription_id) = payload
        .get("subscription_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    else {
        defer_outbox_event(
            pool,
            event_id,
            "outbox event does not contain subscription_id",
        )
        .await?;
        anyhow::bail!("outbox event does not contain subscription_id");
    };

    let account = sqlx::query_scalar::<_, String>(
        "SELECT a.external_id FROM subscriptions s
         JOIN vpn_accounts a ON a.id = s.vpn_account_id
         WHERE s.id = $1 AND s.status = 'expired'",
    )
    .bind(subscription_id)
    .fetch_optional(pool)
    .await?;
    let Some(external_id) = account else {
        mark_outbox_event_processed(pool, event_id).await?;
        return Ok(());
    };
    if let Err(error) = provider.disable(&external_id).await {
        defer_outbox_event(pool, event_id, &error.to_string()).await?;
        return Err(anyhow::anyhow!(error));
    }
    let mut transaction = pool.begin().await?;
    sqlx::query(
        "INSERT INTO subscription_events (id, subscription_id, event_type, details)
         VALUES ($1, $2, 'provider_blocked', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"provider": "remnawave"}))
    .execute(&mut *transaction)
    .await?;
    mark_outbox_event_processed_in_transaction(&mut transaction, event_id).await?;
    transaction.commit().await?;
    info!(%subscription_id, "subscription blocked at provider");
    Ok(())
}

async fn reconcile_one_provider_account(pool: &PgPool, provider: &RemnawaveProvider) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let account = sqlx::query_as::<_, (Uuid, Uuid, String, DateTime<Utc>)>(
        "SELECT s.id, a.id, a.external_id, s.expires_at
         FROM subscriptions s
         JOIN vpn_accounts a ON a.id = s.vpn_account_id
         WHERE s.status = 'active' AND a.provider = 'remnawave'
           AND a.updated_at <= now() - interval '15 minutes'
         ORDER BY a.updated_at FOR UPDATE OF a SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((subscription_id, account_id, external_id, local_expires_at)) = account else {
        transaction.commit().await?;
        return Ok(());
    };
    // Claim the account before the remote call, limiting each account to one sync per interval.
    sqlx::query("UPDATE vpn_accounts SET updated_at = now() WHERE id = $1")
        .bind(account_id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;

    let state = provider.access_state(&external_id).await?;
    let details = match state {
        Some(state) => json!({
            "provider": "remnawave",
            "provider_status": state.status,
            "provider_expires_at": state.expires_at,
            "local_expires_at": local_expires_at,
            "used_traffic_bytes": state.used_traffic_bytes,
            "expiry_matches": state.expires_at == local_expires_at,
        }),
        None => json!({"provider": "remnawave", "provider_account_missing": true}),
    };
    sqlx::query(
        "INSERT INTO subscription_events (id, subscription_id, event_type, details)
         VALUES ($1, $2, 'provider_reconciled', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(details)
    .execute(pool)
    .await?;
    info!(%subscription_id, "provider account reconciled");
    Ok(())
}

async fn process_one_subscription(
    pool: &PgPool,
    provider: &RemnawaveProvider,
    encryption_key: &[u8; 32],
) -> Result<()> {
    let Some((event_id, payload)) = claim_next_outbox_event(pool, "subscription.requested").await?
    else {
        return Ok(());
    };
    let Some(subscription_id) = payload
        .get("subscription_id")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<Uuid>().ok())
    else {
        defer_outbox_event(
            pool,
            event_id,
            "outbox event does not contain subscription_id",
        )
        .await?;
        anyhow::bail!("outbox event does not contain subscription_id");
    };

    let subscription = load_subscription(pool, subscription_id).await?;
    let duration_seconds = payload
        .get("duration_seconds")
        .and_then(Value::as_i64)
        .or(subscription.duration_seconds)
        .unwrap_or(DEFAULT_TRIAL_DURATION_SECONDS);
    let expires_at = subscription
        .current_expiry
        .filter(|expiry| *expiry > Utc::now())
        .unwrap_or_else(Utc::now)
        + ChronoDuration::seconds(duration_seconds);
    let provisioned = provider
        .provision(ProvisionRequest {
            user_id: subscription.user_id,
            telegram_user_id: subscription.telegram_user_id,
            username: subscription.username.clone(),
            expires_at,
            traffic_limit_bytes: subscription.traffic_bytes.unwrap_or(0),
        })
        .await;
    let provisioned = match provisioned {
        Ok(provisioned) => provisioned,
        Err(error) => {
            defer_outbox_event(pool, event_id, &error.to_string()).await?;
            return Err(anyhow::anyhow!(error));
        }
    };
    let encrypted_url = encrypt_access_url(encryption_key, &provisioned.subscription_url)?;
    complete_provisioning(
        pool,
        event_id,
        subscription_id,
        &subscription,
        provisioned.external_id,
        encrypted_url,
        provisioned.expires_at,
    )
    .await
}

async fn claim_next_outbox_event(pool: &PgPool, event_type: &str) -> Result<Option<(Uuid, Value)>> {
    let mut transaction = pool.begin().await?;
    let event = sqlx::query_as::<_, (Uuid, Value)>(
        "SELECT id, payload FROM outbox_events
         WHERE event_type = $1 AND processed_at IS NULL AND available_at <= now()
           AND (locked_at IS NULL OR locked_at <= now() - make_interval(secs => $2))
         ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .bind(event_type)
    .bind(OUTBOX_LOCK_LEASE_SECONDS)
    .fetch_optional(&mut *transaction)
    .await?;
    let Some((event_id, payload)) = event else {
        transaction.commit().await?;
        return Ok(None);
    };
    sqlx::query(
        "UPDATE outbox_events SET locked_at = now(), lock_owner = 'provisioning-worker', attempts = attempts + 1
         WHERE id = $1",
    )
    .bind(event_id)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(Some((event_id, payload)))
}

async fn defer_outbox_event(pool: &PgPool, event_id: Uuid, reason: &str) -> Result<()> {
    sqlx::query(
        "UPDATE outbox_events SET available_at = now() + interval '30 seconds', locked_at = NULL,
         lock_owner = NULL, last_error = $1 WHERE id = $2",
    )
    .bind(reason)
    .bind(event_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn mark_outbox_event_processed(pool: &PgPool, event_id: Uuid) -> Result<()> {
    sqlx::query("UPDATE outbox_events SET processed_at = now(), locked_at = NULL, lock_owner = NULL WHERE id = $1")
        .bind(event_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn mark_outbox_event_processed_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event_id: Uuid,
) -> Result<()> {
    sqlx::query("UPDATE outbox_events SET processed_at = now(), locked_at = NULL, lock_owner = NULL WHERE id = $1")
        .bind(event_id)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn load_subscription(pool: &PgPool, subscription_id: Uuid) -> Result<PendingSubscription> {
    sqlx::query_as(
        "SELECT s.user_id, u.telegram_user_id, p.username, t.duration_seconds,
                COALESCE(s.traffic_bytes, t.traffic_bytes) AS traffic_bytes,
                 (SELECT max(expires_at) FROM subscriptions WHERE user_id = s.user_id AND status = 'active') AS current_expiry
          FROM subscriptions s
          JOIN users u ON u.id = s.user_id
          JOIN user_profiles p ON p.user_id = s.user_id
          LEFT JOIN tariffs t ON t.id = s.tariff_id
         WHERE s.id = $1 AND s.status = 'provisioning_pending'",
    )
    .bind(subscription_id)
    .fetch_one(pool)
    .await
    .context("pending subscription not found")
}

async fn complete_provisioning(
    pool: &PgPool,
    event_id: Uuid,
    subscription_id: Uuid,
    subscription: &PendingSubscription,
    external_id: String,
    encrypted_url: Vec<u8>,
    expires_at: DateTime<Utc>,
) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let vpn_account_id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO vpn_accounts (id, user_id, provider, external_id, encrypted_access_url)
         VALUES ($1, $2, 'remnawave', $3, $4)
         ON CONFLICT (user_id, provider) DO UPDATE SET
           external_id = EXCLUDED.external_id, encrypted_access_url = EXCLUDED.encrypted_access_url, updated_at = now()
         RETURNING id",
    )
    .bind(Uuid::now_v7())
    .bind(subscription.user_id)
    .bind(external_id)
    .bind(encrypted_url)
    .fetch_one(&mut *transaction)
    .await?;
    sqlx::query(
        "UPDATE subscriptions SET status = 'active', vpn_account_id = $1, starts_at = COALESCE(starts_at, now()),
         expires_at = $2, updated_at = now() WHERE id = $3",
    )
    .bind(vpn_account_id)
    .bind(expires_at)
    .bind(subscription_id)
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO subscription_events (id, subscription_id, event_type, details)
         VALUES ($1, $2, 'provisioned', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"expires_at": expires_at}))
    .execute(&mut *transaction)
    .await?;
    sqlx::query(
        "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
         VALUES ($1, 'subscription', $2, 'subscription.changed', $3),
                ($4, 'notification', $2, 'notification.requested', $5)",
    )
    .bind(Uuid::now_v7())
    .bind(subscription_id)
    .bind(json!({"subscription_id": subscription_id, "status": "active", "version": 1}))
    .bind(Uuid::now_v7())
    .bind(json!({"user_id": subscription.user_id, "kind": "subscription_active", "subscription_id": subscription_id, "version": 1}))
    .execute(&mut *transaction)
    .await?;
    mark_outbox_event_processed_in_transaction(&mut transaction, event_id).await?;
    transaction.commit().await?;
    info!(%subscription_id, "subscription provisioned");
    Ok(())
}

fn load_remnawave_config() -> Result<RemnawaveConfig> {
    Ok(RemnawaveConfig {
        base_url: env::var("REMNAWAVE_BASE_URL").context("REMNAWAVE_BASE_URL is required")?,
        api_token: env::var("REMNAWAVE_API_TOKEN").context("REMNAWAVE_API_TOKEN is required")?,
        internal_squad_uuids: env::var("REMNAWAVE_INTERNAL_SQUAD_UUIDS")
            .unwrap_or_default()
            .split(',')
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
            .collect(),
        external_squad_uuid: env::var("REMNAWAVE_EXTERNAL_SQUAD_UUID")
            .ok()
            .filter(|value| !value.is_empty()),
        traffic_limit_strategy: env::var("REMNAWAVE_TRAFFIC_LIMIT_STRATEGY")
            .unwrap_or_else(|_| "NO_RESET".to_owned()),
        user_tag: env::var("REMNAWAVE_USER_TAG").unwrap_or_else(|_| "PAID".to_owned()),
        username_prefix: env::var("REMNAWAVE_USERNAME_PREFIX").unwrap_or_else(|_| "vpn".to_owned()),
    })
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

fn encrypt_access_url(key: &[u8; 32], access_url: &str) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("AES-256 uses a 32-byte key");
    let mut nonce = [0_u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), access_url.as_bytes())
        .map_err(|_| anyhow::anyhow!("access URL encryption failed"))?;
    Ok([nonce.as_slice(), ciphertext.as_slice()].concat())
}

#[cfg(test)]
mod tests {
    use std::{env, sync::Arc};

    use axum::{
        Json, Router,
        extract::State,
        http::StatusCode,
        routing::{get, post},
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use serde_json::json;
    use sqlx::PgPool;
    use uuid::Uuid;
    use vpn_integrations::{RemnawaveConfig, RemnawaveProvider};

    use super::{
        OUTBOX_LOCK_LEASE_SECONDS, claim_next_outbox_event, defer_outbox_event,
        process_one_subscription,
    };

    async fn test_pool() -> Option<PgPool> {
        let url = env::var("VPN_TEST_DATABASE_URL").ok()?;
        PgPool::connect(&url).await.ok()
    }

    #[derive(Default)]
    struct MockRemnawaveState {
        created_users: tokio::sync::Mutex<Vec<serde_json::Value>>,
    }

    async fn missing_mock_user() -> StatusCode {
        StatusCode::NOT_FOUND
    }

    async fn create_mock_user(
        State(state): State<Arc<MockRemnawaveState>>,
        Json(payload): Json<serde_json::Value>,
    ) -> Json<serde_json::Value> {
        state.created_users.lock().await.push(payload);
        Json(json!({"response": {
            "uuid": "mock-remnawave-user",
            "username": "vpn_123",
            "subscriptionUrl": "https://subscriptions.example.invalid/mock",
            "expireAt": (Utc::now() + ChronoDuration::days(30)).to_rfc3339(),
            "status": "ACTIVE",
            "userTraffic": {"usedTrafficBytes": 0}
        }}))
    }

    async fn mock_remnawave_server()
    -> (String, Arc<MockRemnawaveState>, tokio::task::JoinHandle<()>) {
        let state = Arc::new(MockRemnawaveState::default());
        let app = Router::new()
            .route("/api/users/by-telegram-id/{id}", get(missing_mock_user))
            .route("/api/users", post(create_mock_user))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), state, task)
    }

    #[tokio::test]
    async fn outbox_claim_respects_active_lease_and_recovers_stale_claims() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let event_id = Uuid::now_v7();
        let aggregate_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload, available_at)
             VALUES ($1, 'subscription', $2, 'subscription.requested', $3, now() - interval '1 day')",
        )
        .bind(event_id)
        .bind(aggregate_id)
        .bind(json!({"subscription_id": aggregate_id, "version": 1}))
        .execute(&pool)
        .await
        .unwrap();

        let first_claim = claim_next_outbox_event(&pool, "subscription.requested")
            .await
            .unwrap();
        assert_eq!(first_claim.map(|event| event.0), Some(event_id));
        let still_locked = sqlx::query_scalar::<_, bool>(
            "SELECT locked_at IS NOT NULL AND lock_owner = 'provisioning-worker'
             FROM outbox_events WHERE id = $1",
        )
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(still_locked);

        sqlx::query(
            "UPDATE outbox_events SET locked_at = now() - make_interval(secs => $1) WHERE id = $2",
        )
        .bind(OUTBOX_LOCK_LEASE_SECONDS + 1)
        .bind(event_id)
        .execute(&pool)
        .await
        .unwrap();
        let recovered_claim = claim_next_outbox_event(&pool, "subscription.requested")
            .await
            .unwrap();
        assert_eq!(recovered_claim.map(|event| event.0), Some(event_id));

        defer_outbox_event(&pool, event_id, "provider timeout")
            .await
            .unwrap();
        let (attempts, unlocked, retry_state) = sqlx::query_as::<_, (i32, bool, bool)>(
            "SELECT attempts, locked_at IS NULL AND lock_owner IS NULL,
                    last_error = 'provider timeout', available_at > now()
             FROM outbox_events WHERE id = $1",
        )
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(attempts, 2);
        assert!(unlocked && retry_state);

        sqlx::query("DELETE FROM outbox_events WHERE id = $1")
            .bind(event_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn mock_remnawave_provisioning_activates_subscription_and_queues_notification() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let (base_url, mock, server) = mock_remnawave_server().await;
        let user_id = Uuid::now_v7();
        let tariff_id = Uuid::now_v7();
        let subscription_id = Uuid::now_v7();
        let event_id = Uuid::now_v7();
        let telegram_user_id =
            i64::from_be_bytes(Uuid::now_v7().as_bytes()[8..].try_into().unwrap()) & i64::MAX;
        sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(telegram_user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO user_profiles (user_id, first_name, language_code) VALUES ($1, 'Test', 'en')")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO tariffs (id, code, name, duration_seconds) VALUES ($1, $2, '{\"en\":\"Test\"}', 2592000)")
            .bind(tariff_id)
            .bind(format!("test-{tariff_id}"))
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO subscriptions (id, user_id, tariff_id, status) VALUES ($1, $2, $3, 'provisioning_pending')")
            .bind(subscription_id)
            .bind(user_id)
            .bind(tariff_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload, available_at) VALUES ($1, 'subscription', $2, 'subscription.requested', $3, now() - interval '1 day')")
            .bind(event_id)
            .bind(subscription_id)
            .bind(json!({"subscription_id": subscription_id, "user_id": user_id, "version": 1}))
            .execute(&pool)
            .await
            .unwrap();
        let provider = RemnawaveProvider::new(RemnawaveConfig {
            base_url,
            api_token: "test-token".to_owned(),
            internal_squad_uuids: vec!["test-squad".to_owned()],
            external_squad_uuid: None,
            traffic_limit_strategy: "NO_RESET".to_owned(),
            user_tag: "PAID".to_owned(),
            username_prefix: "vpn".to_owned(),
        })
        .unwrap();
        process_one_subscription(&pool, &provider, &[7; 32])
            .await
            .unwrap();
        let (status, account_id) = sqlx::query_as::<_, (String, Option<Uuid>)>(
            "SELECT status, vpn_account_id FROM subscriptions WHERE id = $1",
        )
        .bind(subscription_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let processed = sqlx::query_scalar::<_, bool>(
            "SELECT processed_at IS NOT NULL FROM outbox_events WHERE id = $1",
        )
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let activation_notifications = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM outbox_events WHERE aggregate_id = $1 AND event_type = 'notification.requested'")
            .bind(subscription_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(status, "active");
        assert!(account_id.is_some() && processed);
        assert_eq!(activation_notifications, 1);
        assert_eq!(mock.created_users.lock().await.len(), 1);

        sqlx::query("DELETE FROM outbox_events WHERE aggregate_id = $1 OR id = $1")
            .bind(subscription_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM subscription_events WHERE subscription_id = $1")
            .bind(subscription_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM subscriptions WHERE id = $1")
            .bind(subscription_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM vpn_accounts WHERE user_id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM tariffs WHERE id = $1")
            .bind(tariff_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM user_profiles WHERE user_id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        server.abort();
    }
}
