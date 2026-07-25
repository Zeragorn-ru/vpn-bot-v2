use serde_json::json;
use sqlx::{FromRow, PgPool, Postgres, Transaction, postgres::PgPoolOptions};
use uuid::Uuid;

/// Opens the application's `PostgreSQL` pool.
///
/// # Errors
///
/// Returns a database error when the URL is invalid or a connection cannot be
/// established.
pub async fn connect(database_url: &str) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await
}

pub async fn is_ready(pool: &PgPool) -> bool {
    sqlx::query("SELECT 1").execute(pool).await.is_ok()
}

#[derive(Debug, FromRow)]
pub struct InvoiceForFulfillment {
    pub id: Uuid,
    pub user_id: Uuid,
    pub purpose: String,
    pub status: String,
    pub currency_code: String,
    pub amount_minor: i64,
    pub tariff_id: Option<Uuid>,
}

/// Applies the local side effects of a paid invoice in the caller's
/// transaction. The caller must lock the invoice and mark it paid first.
///
/// # Errors
///
/// Returns an error when the invoice references missing commerce records.
pub async fn fulfill_paid_invoice(
    transaction: &mut Transaction<'_, Postgres>,
    invoice: &InvoiceForFulfillment,
) -> Result<(), sqlx::Error> {
    match invoice.purpose.as_str() {
        "wallet_top_up" => {
            let wallet_id = sqlx::query_scalar::<_, Uuid>(
                "SELECT id FROM wallets WHERE user_id = $1 FOR UPDATE",
            )
            .bind(invoice.user_id)
            .fetch_one(&mut **transaction)
            .await?;
            let ledger_insert = sqlx::query(
                "INSERT INTO wallet_transactions
                 (id, wallet_id, amount_minor, currency_code, kind, reference_type, reference_id)
                 VALUES ($1, $2, $3, $4, 'payment_credit', 'invoice', $5)
                 ON CONFLICT (reference_type, reference_id, kind) DO NOTHING",
            )
            .bind(Uuid::now_v7())
            .bind(wallet_id)
            .bind(invoice.amount_minor)
            .bind(&invoice.currency_code)
            .bind(invoice.id)
            .execute(&mut **transaction)
            .await?;
            if ledger_insert.rows_affected() == 0 {
                return Ok(());
            }
            sqlx::query(
                "UPDATE wallets SET balance_minor = balance_minor + $1, updated_at = now() WHERE id = $2",
            )
            .bind(invoice.amount_minor)
            .bind(wallet_id)
            .execute(&mut **transaction)
            .await?;
            award_referral_reward(transaction, invoice).await?;
            sqlx::query(
                "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
                 VALUES ($1, 'invoice', $2, 'notification.requested', $3)",
            )
            .bind(Uuid::now_v7())
            .bind(invoice.id)
            .bind(json!({"user_id": invoice.user_id, "kind": "wallet_topped_up", "invoice_id": invoice.id, "version": 1}))
            .execute(&mut **transaction)
            .await?;
        }
        "direct_purchase" => {
            let tariff_id = invoice.tariff_id.ok_or_else(|| {
                sqlx::Error::Protocol("direct purchase invoice has no tariff".to_owned())
            })?;
            let subscription_id = Uuid::now_v7();
            let subscription_id = sqlx::query_scalar::<_, Uuid>(
                "INSERT INTO subscriptions (id, user_id, tariff_id, source_invoice_id, status)
                 VALUES ($1, $2, $3, $4, 'provisioning_pending')
                 ON CONFLICT (source_invoice_id) DO NOTHING
                 RETURNING id",
            )
            .bind(subscription_id)
            .bind(invoice.user_id)
            .bind(tariff_id)
            .bind(invoice.id)
            .fetch_optional(&mut **transaction)
            .await?;
            let Some(subscription_id) = subscription_id else {
                return Ok(());
            };
            sqlx::query(
                "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
                 VALUES ($1, 'subscription', $2, 'subscription.requested', $3)",
            )
            .bind(Uuid::now_v7())
            .bind(subscription_id)
            .bind(json!({"subscription_id": subscription_id, "user_id": invoice.user_id, "version": 1}))
            .execute(&mut **transaction)
            .await?;
        }
        _ => {
            return Err(sqlx::Error::Protocol(
                "invoice purpose is invalid".to_owned(),
            ));
        }
    }
    Ok(())
}

async fn award_referral_reward(
    transaction: &mut Transaction<'_, Postgres>,
    invoice: &InvoiceForFulfillment,
) -> Result<(), sqlx::Error> {
    let referral = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT r.id, r.referrer_user_id FROM referrals r
         WHERE r.referred_user_id = $1 FOR UPDATE",
    )
    .bind(invoice.user_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let Some((referral_id, referrer_user_id)) = referral else {
        return Ok(());
    };
    let reward_percent = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE((value ->> 'percent')::BIGINT, 10)
         FROM app_settings WHERE key = 'referral_settings'",
    )
    .fetch_optional(&mut **transaction)
    .await?
    .unwrap_or(10);
    if !(1..=100).contains(&reward_percent) {
        return Err(sqlx::Error::Protocol(
            "referral reward percent is invalid".to_owned(),
        ));
    }
    let reward_amount = invoice
        .amount_minor
        .checked_mul(reward_percent)
        .and_then(|amount| amount.checked_div(100))
        .ok_or_else(|| sqlx::Error::Protocol("referral reward amount overflows".to_owned()))?;
    if reward_amount == 0 {
        return Ok(());
    }
    let referrer_wallet_id =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM wallets WHERE user_id = $1 FOR UPDATE")
            .bind(referrer_user_id)
            .fetch_one(&mut **transaction)
            .await?;
    let reward_transaction_id = Uuid::now_v7();
    let inserted = sqlx::query(
        "INSERT INTO wallet_transactions
         (id, wallet_id, amount_minor, currency_code, kind, reference_type, reference_id)
         VALUES ($1, $2, $3, $4, 'referral_reward', 'invoice', $5)
         ON CONFLICT (reference_type, reference_id, kind) DO NOTHING",
    )
    .bind(reward_transaction_id)
    .bind(referrer_wallet_id)
    .bind(reward_amount)
    .bind(&invoice.currency_code)
    .bind(invoice.id)
    .execute(&mut **transaction)
    .await?;
    if inserted.rows_affected() == 0 {
        return Ok(());
    }
    sqlx::query(
        "UPDATE wallets SET balance_minor = balance_minor + $1, updated_at = now() WHERE id = $2",
    )
    .bind(reward_amount)
    .bind(referrer_wallet_id)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO referral_rewards
         (id, referral_id, source_invoice_id, wallet_transaction_id, amount_minor)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::now_v7())
    .bind(referral_id)
    .bind(invoice.id)
    .bind(reward_transaction_id)
    .bind(reward_amount)
    .execute(&mut **transaction)
    .await?;
    sqlx::query(
        "INSERT INTO outbox_events (id, aggregate_type, aggregate_id, event_type, payload)
         VALUES ($1, 'referral', $2, 'notification.requested', $3)",
    )
    .bind(Uuid::now_v7())
    .bind(referral_id)
    .bind(json!({"user_id": referrer_user_id, "kind": "referral_reward", "amount_minor": reward_amount, "currency_code": invoice.currency_code, "version": 1}))
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[cfg(test)]
mod integration_tests {
    use std::env;

    use super::{InvoiceForFulfillment, fulfill_paid_invoice};
    use sqlx::PgPool;
    use uuid::Uuid;

    async fn test_pool() -> Option<PgPool> {
        let url = env::var("VPN_TEST_DATABASE_URL").ok()?;
        PgPool::connect(&url).await.ok()
    }

    fn test_telegram_user_id() -> i64 {
        i64::from_be_bytes(Uuid::now_v7().as_bytes()[8..].try_into().unwrap()) & i64::MAX
    }

    #[tokio::test]
    async fn wallet_top_up_fulfillment_is_idempotent() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let user_id = Uuid::now_v7();
        let wallet_id = Uuid::now_v7();
        let invoice_id = Uuid::now_v7();
        sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(test_telegram_user_id())
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO user_profiles (user_id, first_name, language_code) VALUES ($1, 'Test', 'en')",
        )
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')")
            .bind(wallet_id)
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO invoices (id, user_id, provider, purpose, status, currency_code, amount_minor, expires_at)
             VALUES ($1, $2, 'test', 'wallet_top_up', 'paid', 'RUB', 50000, now() + interval '1 hour')",
        )
        .bind(invoice_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
        let invoice = InvoiceForFulfillment {
            id: invoice_id,
            user_id,
            purpose: "wallet_top_up".to_owned(),
            status: "paid".to_owned(),
            currency_code: "RUB".to_owned(),
            amount_minor: 50_000,
            tariff_id: None,
        };
        for _ in 0..2 {
            let mut transaction = pool.begin().await.unwrap();
            fulfill_paid_invoice(&mut transaction, &invoice)
                .await
                .unwrap();
            transaction.commit().await.unwrap();
        }
        let balance =
            sqlx::query_scalar::<_, i64>("SELECT balance_minor FROM wallets WHERE id = $1")
                .bind(wallet_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let ledger_entries = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM wallet_transactions WHERE reference_id = $1 AND kind = 'payment_credit'",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(balance, 50_000);
        assert_eq!(ledger_entries, 1);
        sqlx::query("DELETE FROM outbox_events WHERE aggregate_id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wallet_transactions WHERE wallet_id = $1")
            .bind(wallet_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM invoices WHERE id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wallets WHERE id = $1")
            .bind(wallet_id)
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
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn direct_purchase_fulfillment_is_idempotent() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let user_id = Uuid::now_v7();
        let wallet_id = Uuid::now_v7();
        let tariff_id = Uuid::now_v7();
        let invoice_id = Uuid::now_v7();
        let telegram_user_id = test_telegram_user_id();
        sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
            .bind(user_id)
            .bind(telegram_user_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO user_profiles (user_id, first_name, language_code) VALUES ($1, 'Test', 'en')",
        )
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')")
            .bind(wallet_id)
            .bind(user_id)
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
            "INSERT INTO invoices (id, user_id, provider, purpose, status, currency_code, amount_minor, tariff_id, expires_at)
             VALUES ($1, $2, 'test', 'direct_purchase', 'paid', 'RUB', 50000, $3, now() + interval '1 hour')",
        )
        .bind(invoice_id)
        .bind(user_id)
        .bind(tariff_id)
        .execute(&pool)
        .await
        .unwrap();
        let invoice = InvoiceForFulfillment {
            id: invoice_id,
            user_id,
            purpose: "direct_purchase".to_owned(),
            status: "paid".to_owned(),
            currency_code: "RUB".to_owned(),
            amount_minor: 50_000,
            tariff_id: Some(tariff_id),
        };
        for _ in 0..2 {
            let mut transaction = pool.begin().await.unwrap();
            fulfill_paid_invoice(&mut transaction, &invoice)
                .await
                .unwrap();
            transaction.commit().await.unwrap();
        }
        let subscriptions = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM subscriptions WHERE source_invoice_id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let provisioning_events = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM outbox_events WHERE aggregate_id IN
             (SELECT id FROM subscriptions WHERE source_invoice_id = $1) AND event_type = 'subscription.requested'",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(subscriptions, 1);
        assert_eq!(provisioning_events, 1);
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
        sqlx::query("DELETE FROM wallets WHERE id = $1")
            .bind(wallet_id)
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
    }

    #[tokio::test]
    #[allow(clippy::similar_names, clippy::too_many_lines)]
    async fn referral_reward_is_not_replayed_with_payment_fulfillment() {
        let Some(pool) = test_pool().await else {
            return;
        };
        let referrer_id = Uuid::now_v7();
        let referred_id = Uuid::now_v7();
        let referrer_wallet_id = Uuid::now_v7();
        let referred_wallet_id = Uuid::now_v7();
        let referral_id = Uuid::now_v7();
        let invoice_id = Uuid::now_v7();
        for (user_id, telegram_user_id) in [
            (referrer_id, test_telegram_user_id()),
            (referred_id, test_telegram_user_id()),
        ] {
            sqlx::query("INSERT INTO users (id, telegram_user_id) VALUES ($1, $2)")
                .bind(user_id)
                .bind(telegram_user_id)
                .execute(&pool)
                .await
                .unwrap();
            sqlx::query(
                "INSERT INTO user_profiles (user_id, first_name, language_code) VALUES ($1, 'Test', 'en')",
            )
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
        }
        for (wallet_id, user_id) in [
            (referrer_wallet_id, referrer_id),
            (referred_wallet_id, referred_id),
        ] {
            sqlx::query("INSERT INTO wallets (id, user_id, currency_code) VALUES ($1, $2, 'RUB')")
                .bind(wallet_id)
                .bind(user_id)
                .execute(&pool)
                .await
                .unwrap();
        }
        sqlx::query(
            "INSERT INTO referrals (id, referrer_user_id, referred_user_id) VALUES ($1, $2, $3)",
        )
        .bind(referral_id)
        .bind(referrer_id)
        .bind(referred_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO invoices (id, user_id, provider, purpose, status, currency_code, amount_minor, expires_at)
             VALUES ($1, $2, 'test', 'wallet_top_up', 'paid', 'RUB', 50000, now() + interval '1 hour')",
        )
        .bind(invoice_id)
        .bind(referred_id)
        .execute(&pool)
        .await
        .unwrap();
        let reward_percent = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE((value ->> 'percent')::BIGINT, 10) FROM app_settings WHERE key = 'referral_settings'",
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
        .unwrap_or(10);
        let invoice = InvoiceForFulfillment {
            id: invoice_id,
            user_id: referred_id,
            purpose: "wallet_top_up".to_owned(),
            status: "paid".to_owned(),
            currency_code: "RUB".to_owned(),
            amount_minor: 50_000,
            tariff_id: None,
        };
        for _ in 0..2 {
            let mut transaction = pool.begin().await.unwrap();
            fulfill_paid_invoice(&mut transaction, &invoice)
                .await
                .unwrap();
            transaction.commit().await.unwrap();
        }
        let reward_balance =
            sqlx::query_scalar::<_, i64>("SELECT balance_minor FROM wallets WHERE id = $1")
                .bind(referrer_wallet_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let rewards = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM referral_rewards WHERE source_invoice_id = $1",
        )
        .bind(invoice_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(reward_balance, 50_000 * reward_percent / 100);
        assert_eq!(rewards, 1);
        sqlx::query("DELETE FROM outbox_events WHERE aggregate_id IN ($1, $2)")
            .bind(invoice_id)
            .bind(referral_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM referral_rewards WHERE source_invoice_id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM wallet_transactions WHERE reference_id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM invoices WHERE id = $1")
            .bind(invoice_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM referrals WHERE id = $1")
            .bind(referral_id)
            .execute(&pool)
            .await
            .unwrap();
        for wallet_id in [referrer_wallet_id, referred_wallet_id] {
            sqlx::query("DELETE FROM wallets WHERE id = $1")
                .bind(wallet_id)
                .execute(&pool)
                .await
                .unwrap();
        }
        for user_id in [referrer_id, referred_id] {
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
        }
    }
}
