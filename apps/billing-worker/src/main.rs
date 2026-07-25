use std::{env, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use sqlx::{FromRow, PgPool};
use tokio::time::{MissedTickBehavior, interval};
use tracing::{error, info};
use uuid::Uuid;
use vpn_integrations::{
    AnoreConfig, CryptoPayConfig, PaymentProviderCode, PaymentProviderRegistry,
    PaymentProviderSettings, ProviderPaymentStatus, TelegramStarsConfig,
};
use vpn_observability::init_tracing;
use vpn_storage::{InvoiceForFulfillment, connect};

#[derive(Debug, FromRow)]
struct PendingInvoice {
    id: Uuid,
    user_id: Uuid,
    provider: String,
    provider_invoice_id: String,
    purpose: String,
    currency_code: String,
    amount_minor: i64,
    tariff_id: Option<Uuid>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL is required")?;
    let pool = connect(&database_url)
        .await
        .context("database connection failed")?;
    let providers = Arc::new(load_configured_providers()?);
    info!(providers = ?providers.enabled_codes(), "billing provider registry initialized");

    info!("billing worker started");
    let mut ticker = interval(Duration::from_secs(15));
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(error) = reconcile_one_invoice(&pool, &providers).await {
                    error!(%error, "payment reconciliation failed");
                }
            }
            result = tokio::signal::ctrl_c() => {
                result.context("failed to wait for signal")?;
                break;
            }
        }
    }
    info!("billing worker stopped");
    Ok(())
}

async fn reconcile_one_invoice(pool: &PgPool, providers: &PaymentProviderRegistry) -> Result<()> {
    let mut transaction = pool.begin().await?;
    let invoice = sqlx::query_as::<_, PendingInvoice>(
        "SELECT id, user_id, provider, provider_invoice_id, purpose, currency_code, amount_minor, tariff_id
         FROM invoices WHERE status = 'pending' AND expires_at > now() AND provider_invoice_id IS NOT NULL
           AND provider <> 'telegram_stars'
         ORDER BY created_at FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .fetch_optional(&mut *transaction)
    .await?;
    let Some(invoice) = invoice else {
        transaction.commit().await?;
        return Ok(());
    };
    let provider_code = invoice.provider.parse::<PaymentProviderCode>()?;
    let provider = providers
        .get(provider_code)
        .context("pending invoice provider is no longer configured")?;
    let status = provider
        .payment_status(&invoice.provider_invoice_id)
        .await?;
    match status {
        ProviderPaymentStatus::Paid => settle_paid_invoice(&mut transaction, &invoice).await?,
        ProviderPaymentStatus::Expired | ProviderPaymentStatus::Cancelled => {
            let status = if status == ProviderPaymentStatus::Expired {
                "expired"
            } else {
                "cancelled"
            };
            sqlx::query("UPDATE invoices SET status = $1, updated_at = now() WHERE id = $2")
                .bind(status)
                .bind(invoice.id)
                .execute(&mut *transaction)
                .await?;
        }
        ProviderPaymentStatus::Pending | ProviderPaymentStatus::Unknown => {}
    }
    transaction.commit().await?;
    Ok(())
}

async fn settle_paid_invoice(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    invoice: &PendingInvoice,
) -> Result<()> {
    sqlx::query(
        "UPDATE invoices SET status = 'paid', paid_at = now(), updated_at = now() WHERE id = $1",
    )
    .bind(invoice.id)
    .execute(&mut **transaction)
    .await?;
    vpn_storage::fulfill_paid_invoice(
        transaction,
        &InvoiceForFulfillment {
            id: invoice.id,
            user_id: invoice.user_id,
            purpose: invoice.purpose.clone(),
            status: "paid".to_owned(),
            currency_code: invoice.currency_code.clone(),
            amount_minor: invoice.amount_minor,
            tariff_id: invoice.tariff_id,
        },
    )
    .await?;
    Ok(())
}

fn load_configured_providers() -> Result<PaymentProviderRegistry> {
    let crypto_pay = env::var("CRYPTO_PAY_TOKEN")
        .ok()
        .filter(|token| !token.is_empty() && token != "replace-me")
        .map(|api_token| CryptoPayConfig {
            api_token,
            base_url: env::var("CRYPTO_PAY_BASE_URL")
                .unwrap_or_else(|_| "https://pay.crypt.bot/api".to_owned()),
        });
    let anore = match (env::var("ANORE_API_KEY"), env::var("ANORE_SIGNING_SECRET")) {
        (Ok(api_key), Ok(signing_secret)) if !api_key.is_empty() && !signing_secret.is_empty() => {
            Some(AnoreConfig {
                api_key,
                signing_secret,
                base_url: env::var("ANORE_BASE_URL")
                    .unwrap_or_else(|_| "https://api.anore.cc/v1".to_owned()),
            })
        }
        (Err(_), Err(_)) => None,
        _ => anyhow::bail!("ANORE_API_KEY and ANORE_SIGNING_SECRET must be configured together"),
    };
    let telegram_stars = env::var("TELEGRAM_BOT_TOKEN")
        .ok()
        .filter(|token| !token.is_empty() && token != "replace-me")
        .map(|bot_token| TelegramStarsConfig { bot_token });
    PaymentProviderRegistry::from_settings(PaymentProviderSettings {
        crypto_pay,
        anore,
        telegram_stars,
    })
    .context("payment provider configuration is invalid")
}
