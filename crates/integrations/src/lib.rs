//! Typed boundaries for external payment and VPN providers.

use std::{collections::HashMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use http::HeaderMap;
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;
use vpn_domain::Money;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaymentProviderCode {
    CryptoPay,
    Anore,
    TelegramStars,
}

impl PaymentProviderCode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CryptoPay => "crypto_pay",
            Self::Anore => "anore",
            Self::TelegramStars => "telegram_stars",
        }
    }
}

impl std::str::FromStr for PaymentProviderCode {
    type Err = IntegrationError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "crypto_pay" => Ok(Self::CryptoPay),
            "anore" => Ok(Self::Anore),
            "telegram_stars" => Ok(Self::TelegramStars),
            _ => Err(IntegrationError::InvalidConfiguration),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PaymentInvoiceRequest {
    pub local_invoice_id: Uuid,
    pub amount: Money,
    pub currency_code: String,
    pub title: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderInvoice {
    pub provider_invoice_id: String,
    pub payment_url: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPaymentStatus {
    Pending,
    Paid,
    Expired,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPaymentEvent {
    pub provider_event_id: String,
    pub provider_invoice_id: String,
    pub provider_payment_id: Option<String>,
    pub status: ProviderPaymentStatus,
    pub amount_minor: i64,
    pub currency_code: String,
}

#[async_trait]
pub trait PaymentProvider: Send + Sync {
    #[must_use]
    fn code(&self) -> PaymentProviderCode;

    /// Creates a provider invoice for a normalized amount and local invoice ID.
    ///
    /// # Errors
    ///
    /// Returns an integration error when the remote provider rejects or cannot
    /// complete the request.
    async fn create_invoice(
        &self,
        request: &PaymentInvoiceRequest,
    ) -> Result<ProviderInvoice, IntegrationError>;

    /// Reconciles the current provider-side state of an invoice.
    ///
    /// # Errors
    ///
    /// Returns an integration error when the remote provider cannot be queried.
    async fn payment_status(
        &self,
        provider_invoice_id: &str,
    ) -> Result<ProviderPaymentStatus, IntegrationError>;

    /// Verifies and normalizes a provider callback.
    ///
    /// Providers without a signed callback return `UnsupportedWebhook`; their
    /// authoritative completion is ingested from a trusted Telegram update or
    /// reconciliation request instead.
    ///
    /// # Errors
    ///
    /// Returns an integration error if signature verification or parsing fails.
    fn verify_webhook(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
    ) -> Result<VerifiedPaymentEvent, IntegrationError>;
}

#[derive(Default)]
pub struct PaymentProviderRegistry {
    providers: HashMap<PaymentProviderCode, Arc<dyn PaymentProvider>>,
}

impl PaymentProviderRegistry {
    #[must_use]
    pub fn new(providers: impl IntoIterator<Item = Arc<dyn PaymentProvider>>) -> Self {
        let providers = providers
            .into_iter()
            .map(|provider| (provider.code(), provider))
            .collect();
        Self { providers }
    }

    #[must_use]
    pub fn enabled_codes(&self) -> Vec<PaymentProviderCode> {
        let mut codes = self.providers.keys().copied().collect::<Vec<_>>();
        codes.sort_unstable_by_key(|code| code.as_str());
        codes
    }

    #[must_use]
    pub fn get(&self, code: PaymentProviderCode) -> Option<&Arc<dyn PaymentProvider>> {
        self.providers.get(&code)
    }

    /// Builds a registry from the enabled provider configurations.
    ///
    /// # Errors
    ///
    /// Returns an error when an enabled adapter has incomplete configuration.
    pub fn from_settings(settings: PaymentProviderSettings) -> Result<Self, IntegrationError> {
        let mut providers: Vec<Arc<dyn PaymentProvider>> = Vec::new();
        if let Some(config) = settings.crypto_pay {
            providers.push(Arc::new(CryptoPayProvider::new(config)?));
        }
        if let Some(config) = settings.anore {
            providers.push(Arc::new(AnoreProvider::new(config)?));
        }
        if let Some(config) = settings.telegram_stars {
            providers.push(Arc::new(TelegramStarsProvider::new(config)?));
        }
        Ok(Self::new(providers))
    }
}

#[derive(Debug, Clone, Default)]
pub struct PaymentProviderSettings {
    pub crypto_pay: Option<CryptoPayConfig>,
    pub anore: Option<AnoreConfig>,
    pub telegram_stars: Option<TelegramStarsConfig>,
}

#[derive(Debug, Clone)]
pub struct CryptoPayConfig {
    pub api_token: String,
    pub base_url: String,
}

pub struct CryptoPayProvider {
    client: Client,
    config: CryptoPayConfig,
}

impl CryptoPayProvider {
    /// Creates a Crypto Pay adapter for fiat invoices.
    ///
    /// # Errors
    ///
    /// Returns an error when an HTTP client cannot be created.
    pub fn new(config: CryptoPayConfig) -> Result<Self, IntegrationError> {
        Ok(Self {
            client: payment_http_client()?,
            config,
        })
    }
}

#[async_trait]
impl PaymentProvider for CryptoPayProvider {
    fn code(&self) -> PaymentProviderCode {
        PaymentProviderCode::CryptoPay
    }

    async fn create_invoice(
        &self,
        request: &PaymentInvoiceRequest,
    ) -> Result<ProviderInvoice, IntegrationError> {
        require_rub(&request.currency_code)?;
        let body = serde_json::json!({
            "currency_type": "fiat",
            "fiat": "RUB",
            "amount": rub_decimal(request.amount),
            "description": request.description,
            "payload": request.local_invoice_id.to_string(),
            "allow_comments": false,
            "allow_anonymous": false,
        });
        let response = self
            .client
            .post(format!(
                "{}/createInvoice",
                self.config.base_url.trim_end_matches('/')
            ))
            .header("Crypto-Pay-API-Token", &self.config.api_token)
            .json(&body)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        let result: CryptoPayResponse = response
            .json()
            .await
            .map_err(|_| IntegrationError::InvalidResponse)?;
        let invoice = result.result.ok_or(IntegrationError::InvalidResponse)?;
        if !result.ok {
            return Err(IntegrationError::ProviderRequest);
        }
        Ok(ProviderInvoice {
            provider_invoice_id: invoice.invoice_id.to_string(),
            payment_url: invoice
                .mini_app_invoice_url
                .or(invoice.web_app_invoice_url)
                .or(invoice.bot_invoice_url)
                .ok_or(IntegrationError::InvalidResponse)?,
            expires_at: invoice.expiration_date,
        })
    }

    async fn payment_status(
        &self,
        provider_invoice_id: &str,
    ) -> Result<ProviderPaymentStatus, IntegrationError> {
        let response = self
            .client
            .post(format!(
                "{}/getInvoices",
                self.config.base_url.trim_end_matches('/')
            ))
            .header("Crypto-Pay-API-Token", &self.config.api_token)
            .json(&serde_json::json!({"invoice_ids": provider_invoice_id}))
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        let result: CryptoPayInvoicesResponse = response
            .json()
            .await
            .map_err(|_| IntegrationError::InvalidResponse)?;
        Ok(result
            .result
            .items
            .first()
            .map_or(ProviderPaymentStatus::Unknown, |invoice| {
                normalize_status(&invoice.status)
            }))
    }

    fn verify_webhook(
        &self,
        _headers: &HeaderMap,
        _raw_body: &[u8],
    ) -> Result<VerifiedPaymentEvent, IntegrationError> {
        Err(IntegrationError::UnsupportedWebhook)
    }
}

#[derive(Debug, Clone)]
pub struct AnoreConfig {
    pub api_key: String,
    pub signing_secret: String,
    pub base_url: String,
}

pub struct AnoreProvider {
    client: Client,
    config: AnoreConfig,
}

impl AnoreProvider {
    /// Creates an Anore adapter with mandatory request and webhook verification.
    ///
    /// # Errors
    ///
    /// Returns an error when the secret is absent or the HTTP client fails.
    pub fn new(config: AnoreConfig) -> Result<Self, IntegrationError> {
        if config.signing_secret.is_empty() {
            return Err(IntegrationError::InvalidConfiguration);
        }
        Ok(Self {
            client: payment_http_client()?,
            config,
        })
    }
}

#[async_trait]
impl PaymentProvider for AnoreProvider {
    fn code(&self) -> PaymentProviderCode {
        PaymentProviderCode::Anore
    }

    async fn create_invoice(
        &self,
        request: &PaymentInvoiceRequest,
    ) -> Result<ProviderInvoice, IntegrationError> {
        require_rub(&request.currency_code)?;
        let body = serde_json::to_vec(&serde_json::json!({
            "amount": rub_decimal(request.amount),
            "description": request.description,
            "orderId": request.local_invoice_id.to_string(),
        }))
        .map_err(|_| IntegrationError::InvalidResponse)?;
        let signature = hmac_sha256_hex(&self.config.signing_secret, &body);
        let response = self
            .client
            .post(format!(
                "{}/payments",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.api_key)
            .header("Anore-Signature", signature)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        let invoice: AnoreInvoice = response
            .json()
            .await
            .map_err(|_| IntegrationError::InvalidResponse)?;
        Ok(ProviderInvoice {
            provider_invoice_id: invoice.id,
            payment_url: invoice
                .payment_url
                .ok_or(IntegrationError::InvalidResponse)?,
            expires_at: None,
        })
    }

    async fn payment_status(
        &self,
        provider_invoice_id: &str,
    ) -> Result<ProviderPaymentStatus, IntegrationError> {
        let response = self
            .client
            .get(format!(
                "{}/payments/{provider_invoice_id}",
                self.config.base_url.trim_end_matches('/')
            ))
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        let payment: AnorePayment = response
            .json()
            .await
            .map_err(|_| IntegrationError::InvalidResponse)?;
        Ok(normalize_status(&payment.status))
    }

    fn verify_webhook(
        &self,
        headers: &HeaderMap,
        raw_body: &[u8],
    ) -> Result<VerifiedPaymentEvent, IntegrationError> {
        let signature = headers
            .get("Anore-Signature")
            .and_then(|value| value.to_str().ok())
            .ok_or(IntegrationError::InvalidWebhookSignature)?;
        let event_id = headers
            .get("Anore-Event")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or(IntegrationError::MissingWebhookEventId)?;
        if !constant_time_equal(
            signature.as_bytes(),
            hmac_sha256_hex(&self.config.signing_secret, raw_body).as_bytes(),
        ) {
            return Err(IntegrationError::InvalidWebhookSignature);
        }
        let event: AnoreWebhook =
            serde_json::from_slice(raw_body).map_err(|_| IntegrationError::InvalidResponse)?;
        Ok(VerifiedPaymentEvent {
            provider_event_id: event_id.to_owned(),
            provider_invoice_id: event.id.clone(),
            provider_payment_id: Some(event.id),
            status: normalize_status(&event.status),
            amount_minor: parse_rub_minor(&event.amount)?,
            currency_code: event.currency.unwrap_or_else(|| "RUB".to_owned()),
        })
    }
}

#[derive(Debug, Clone)]
pub struct TelegramStarsConfig {
    pub bot_token: String,
}

pub struct TelegramStarsProvider {
    client: Client,
    config: TelegramStarsConfig,
}

impl TelegramStarsProvider {
    /// Creates a Telegram Stars adapter that issues Bot API invoice links.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be created.
    pub fn new(config: TelegramStarsConfig) -> Result<Self, IntegrationError> {
        Ok(Self {
            client: payment_http_client()?,
            config,
        })
    }
}

#[async_trait]
impl PaymentProvider for TelegramStarsProvider {
    fn code(&self) -> PaymentProviderCode {
        PaymentProviderCode::TelegramStars
    }

    async fn create_invoice(
        &self,
        request: &PaymentInvoiceRequest,
    ) -> Result<ProviderInvoice, IntegrationError> {
        if request.currency_code != "XTR" {
            return Err(IntegrationError::UnsupportedCurrency);
        }
        let response = self
            .client
            .post(format!(
                "https://api.telegram.org/bot{}/createInvoiceLink",
                self.config.bot_token
            ))
            .json(&serde_json::json!({
                "title": request.title,
                "description": request.description,
                "payload": request.local_invoice_id.to_string(),
                "currency": "XTR",
                "prices": [{"label": request.title, "amount": request.amount.amount_minor}],
            }))
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        let document: TelegramBotResponse<String> = response
            .json()
            .await
            .map_err(|_| IntegrationError::InvalidResponse)?;
        if !document.ok {
            return Err(IntegrationError::ProviderRequest);
        }
        Ok(ProviderInvoice {
            provider_invoice_id: request.local_invoice_id.to_string(),
            payment_url: document.result.ok_or(IntegrationError::InvalidResponse)?,
            expires_at: None,
        })
    }

    async fn payment_status(
        &self,
        _provider_invoice_id: &str,
    ) -> Result<ProviderPaymentStatus, IntegrationError> {
        Ok(ProviderPaymentStatus::Unknown)
    }

    fn verify_webhook(
        &self,
        _headers: &HeaderMap,
        _raw_body: &[u8],
    ) -> Result<VerifiedPaymentEvent, IntegrationError> {
        Err(IntegrationError::UnsupportedWebhook)
    }
}

fn payment_http_client() -> Result<Client, IntegrationError> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| IntegrationError::ProviderRequest)
}

fn require_rub(currency_code: &str) -> Result<(), IntegrationError> {
    if currency_code == "RUB" {
        Ok(())
    } else {
        Err(IntegrationError::UnsupportedCurrency)
    }
}

fn rub_decimal(amount: Money) -> String {
    format!(
        "{}.{:02}",
        amount.amount_minor / 100,
        amount.amount_minor % 100
    )
}

fn parse_rub_minor(value: &str) -> Result<i64, IntegrationError> {
    let (whole, fractional) = value.split_once('.').unwrap_or((value, ""));
    let whole = whole
        .parse::<i64>()
        .map_err(|_| IntegrationError::InvalidResponse)?;
    let fractional = match fractional.len() {
        0 => 0,
        1 => {
            fractional
                .parse::<i64>()
                .map_err(|_| IntegrationError::InvalidResponse)?
                * 10
        }
        2 => fractional
            .parse::<i64>()
            .map_err(|_| IntegrationError::InvalidResponse)?,
        _ => return Err(IntegrationError::InvalidResponse),
    };
    whole
        .checked_mul(100)
        .and_then(|amount| amount.checked_add(fractional))
        .filter(|amount| *amount >= 0)
        .ok_or(IntegrationError::InvalidResponse)
}

fn normalize_status(status: &str) -> ProviderPaymentStatus {
    match status.to_ascii_lowercase().as_str() {
        "paid" | "completed" | "confirmed" | "succeeded" | "success" => ProviderPaymentStatus::Paid,
        "pending" | "active" | "new" => ProviderPaymentStatus::Pending,
        "expired" => ProviderPaymentStatus::Expired,
        "cancelled" | "canceled" => ProviderPaymentStatus::Cancelled,
        _ => ProviderPaymentStatus::Unknown,
    }
}

fn hmac_sha256_hex(secret: &str, payload: &[u8]) -> String {
    use hmac::Mac;
    use sha2::Sha256;

    let mut mac = hmac::Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("HMAC accepts arbitrary key lengths");
    mac.update(payload);
    hex::encode(mac.finalize().into_bytes())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |difference, (a, b)| difference | (a ^ b))
            == 0
}

#[derive(Debug, Deserialize)]
struct CryptoPayResponse {
    ok: bool,
    result: Option<CryptoPayInvoice>,
}

#[derive(Debug, Deserialize)]
struct CryptoPayInvoicesResponse {
    result: CryptoPayInvoices,
}

#[derive(Debug, Deserialize)]
struct CryptoPayInvoices {
    items: Vec<CryptoPayInvoice>,
}

#[derive(Debug, Deserialize)]
struct CryptoPayInvoice {
    invoice_id: i64,
    status: String,
    bot_invoice_url: Option<String>,
    mini_app_invoice_url: Option<String>,
    web_app_invoice_url: Option<String>,
    expiration_date: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize)]
struct AnoreInvoice {
    id: String,
    #[serde(
        alias = "paymentUrl",
        alias = "payment_url",
        alias = "pay_url",
        alias = "url"
    )]
    payment_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnorePayment {
    status: String,
}

#[derive(Debug, Deserialize)]
struct AnoreWebhook {
    id: String,
    status: String,
    #[serde(deserialize_with = "deserialize_amount")]
    amount: String,
    #[serde(default)]
    currency: Option<String>,
}

fn deserialize_amount<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(value) => Ok(value),
        serde_json::Value::Number(value) => Ok(value.to_string()),
        _ => Err(serde::de::Error::custom(
            "amount must be a number or string",
        )),
    }
}

#[derive(Debug, Deserialize)]
struct TelegramBotResponse<T> {
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Clone)]
pub struct RemnawaveConfig {
    pub base_url: String,
    pub api_token: String,
    pub internal_squad_uuids: Vec<String>,
    pub external_squad_uuid: Option<String>,
    pub traffic_limit_strategy: String,
    pub user_tag: String,
    pub username_prefix: String,
}

#[derive(Debug, Clone)]
pub struct ProvisionRequest {
    pub user_id: Uuid,
    pub telegram_user_id: i64,
    pub username: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub traffic_limit_bytes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionedAccess {
    pub external_id: String,
    pub subscription_url: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAccessState {
    pub status: String,
    pub expires_at: DateTime<Utc>,
    pub used_traffic_bytes: i64,
}

#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("provider request failed")]
    ProviderRequest,
    #[error("provider returned an invalid response")]
    InvalidResponse,
    #[error("provider configuration is incomplete")]
    InvalidConfiguration,
    #[error("provider does not support this currency")]
    UnsupportedCurrency,
    #[error("provider does not support public webhooks")]
    UnsupportedWebhook,
    #[error("provider webhook signature is invalid")]
    InvalidWebhookSignature,
    #[error("provider webhook event ID is missing")]
    MissingWebhookEventId,
}

#[async_trait]
pub trait VpnProvider: Send + Sync {
    /// Creates or updates a VPN entitlement without exposing provider fields to
    /// the domain layer.
    ///
    /// # Errors
    ///
    /// Returns an integration error when provisioning cannot complete.
    async fn provision(
        &self,
        request: ProvisionRequest,
    ) -> Result<ProvisionedAccess, IntegrationError>;

    /// Disables access while retaining the provider account and its history.
    ///
    /// # Errors
    ///
    /// Returns an integration error when the provider cannot confirm the
    /// disabled state.
    async fn disable(&self, external_id: &str) -> Result<(), IntegrationError>;

    /// Permanently removes a provider account.
    ///
    /// # Errors
    ///
    /// Returns an integration error when the provider cannot confirm the
    /// account is absent.
    async fn delete(&self, external_id: &str) -> Result<(), IntegrationError>;

    /// Reads the provider's current account state for reconciliation.
    ///
    /// `Ok(None)` means the provider no longer has this account.
    ///
    /// # Errors
    ///
    /// Returns an integration error when the provider cannot be queried.
    async fn access_state(
        &self,
        external_id: &str,
    ) -> Result<Option<ProviderAccessState>, IntegrationError>;

    /// Performs a lightweight provider availability check.
    ///
    /// # Errors
    ///
    /// Returns an integration error when the provider cannot be reached.
    async fn health_check(&self) -> Result<(), IntegrationError>;
}

pub struct RemnawaveProvider {
    client: Client,
    config: RemnawaveConfig,
}

impl RemnawaveProvider {
    /// Builds a provider adapter using the configured base URL and bearer token.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration cannot form a safe HTTP client.
    pub fn new(config: RemnawaveConfig) -> Result<Self, IntegrationError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        Ok(Self { client, config })
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/{}", self.config.base_url.trim_end_matches('/'), path)
    }

    async fn get_user(
        &self,
        telegram_user_id: i64,
    ) -> Result<Option<RemnawaveUser>, IntegrationError> {
        let response = self
            .client
            .get(self.endpoint(&format!("api/users/by-telegram-id/{telegram_user_id}")))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        parse_response(response).await.map(Some)
    }

    async fn default_squad_uuid(&self) -> Result<String, IntegrationError> {
        let response = self
            .client
            .get(self.endpoint("api/internal-squads"))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?
            .error_for_status()
            .map_err(|_| IntegrationError::ProviderRequest)?;
        let document: RemnawaveEnvelope<InternalSquadsResponse> = response
            .json()
            .await
            .map_err(|_| IntegrationError::InvalidResponse)?;
        document
            .response
            .internal_squads
            .into_iter()
            .find(|squad| squad.name == "Default-Squad")
            .map(|squad| squad.uuid)
            .ok_or(IntegrationError::InvalidResponse)
    }
}

#[async_trait]
impl VpnProvider for RemnawaveProvider {
    async fn provision(
        &self,
        request: ProvisionRequest,
    ) -> Result<ProvisionedAccess, IntegrationError> {
        let existing = self.get_user(request.telegram_user_id).await?;
        let squads = if self.config.internal_squad_uuids.is_empty() {
            vec![self.default_squad_uuid().await?]
        } else {
            self.config.internal_squad_uuids.clone()
        };
        let payload = RemnawaveUserPayload {
            username: existing.as_ref().map_or_else(
                || {
                    format!(
                        "{}_{}",
                        self.config.username_prefix, request.telegram_user_id
                    )
                },
                |user| user.username.clone(),
            ),
            expire_at: request.expires_at,
            status: "ACTIVE",
            traffic_limit_bytes: request.traffic_limit_bytes.max(0),
            traffic_limit_strategy: self.config.traffic_limit_strategy.clone(),
            active_internal_squads: squads,
            external_squad_uuid: self.config.external_squad_uuid.clone(),
            tag: self.config.user_tag.clone(),
            telegram_id: request.telegram_user_id,
            description: request
                .username
                .unwrap_or_else(|| request.telegram_user_id.to_string()),
        };
        let response = match existing {
            Some(user) => self
                .client
                .patch(self.endpoint("api/users"))
                .bearer_auth(&self.config.api_token)
                .json(&RemnawaveUpdatePayload {
                    uuid: user.uuid,
                    user: payload,
                })
                .send()
                .await
                .map_err(|_| IntegrationError::ProviderRequest)?,
            None => self
                .client
                .post(self.endpoint("api/users"))
                .bearer_auth(&self.config.api_token)
                .json(&payload)
                .send()
                .await
                .map_err(|_| IntegrationError::ProviderRequest)?,
        }
        .error_for_status()
        .map_err(|_| IntegrationError::ProviderRequest)?;
        let user = parse_response(response).await?;
        Ok(ProvisionedAccess {
            external_id: user.uuid,
            subscription_url: user
                .subscription_url
                .ok_or(IntegrationError::InvalidResponse)?,
            expires_at: user.expire_at,
        })
    }

    async fn disable(&self, external_id: &str) -> Result<(), IntegrationError> {
        let response = self
            .client
            .post(self.endpoint(&format!("api/users/{external_id}/actions/disable")))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        response
            .error_for_status()
            .map(|_| ())
            .map_err(|_| IntegrationError::ProviderRequest)
    }

    async fn delete(&self, external_id: &str) -> Result<(), IntegrationError> {
        let response = self
            .client
            .delete(self.endpoint(&format!("api/users/{external_id}")))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        response
            .error_for_status()
            .map(|_| ())
            .map_err(|_| IntegrationError::ProviderRequest)
    }

    async fn access_state(
        &self,
        external_id: &str,
    ) -> Result<Option<ProviderAccessState>, IntegrationError> {
        let response = self
            .client
            .get(self.endpoint(&format!("api/users/{external_id}")))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let user = parse_response(
            response
                .error_for_status()
                .map_err(|_| IntegrationError::ProviderRequest)?,
        )
        .await?;
        Ok(Some(ProviderAccessState {
            status: user.status,
            expires_at: user.expire_at,
            used_traffic_bytes: user.user_traffic.used_traffic_bytes.max(0),
        }))
    }

    async fn health_check(&self) -> Result<(), IntegrationError> {
        let response = self
            .client
            .get(self.endpoint("api/internal-squads"))
            .bearer_auth(&self.config.api_token)
            .send()
            .await
            .map_err(|_| IntegrationError::ProviderRequest)?;
        response
            .error_for_status()
            .map(|_| ())
            .map_err(|_| IntegrationError::ProviderRequest)
    }
}

async fn parse_response(response: reqwest::Response) -> Result<RemnawaveUser, IntegrationError> {
    let document: RemnawaveEnvelope<RemnawaveUser> = response
        .json()
        .await
        .map_err(|_| IntegrationError::InvalidResponse)?;
    Ok(document.response)
}

#[derive(Debug, Deserialize)]
struct RemnawaveEnvelope<T> {
    response: T,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemnawaveUser {
    uuid: String,
    username: String,
    subscription_url: Option<String>,
    expire_at: DateTime<Utc>,
    #[serde(default)]
    status: String,
    #[serde(default)]
    user_traffic: RemnawaveUserTraffic,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RemnawaveUserTraffic {
    #[serde(default)]
    used_traffic_bytes: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemnawaveUserPayload {
    username: String,
    expire_at: DateTime<Utc>,
    status: &'static str,
    traffic_limit_bytes: i64,
    traffic_limit_strategy: String,
    active_internal_squads: Vec<String>,
    external_squad_uuid: Option<String>,
    tag: String,
    telegram_id: i64,
    description: String,
}

#[derive(Debug, Serialize)]
struct RemnawaveUpdatePayload {
    uuid: String,
    #[serde(flatten)]
    user: RemnawaveUserPayload,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct InternalSquadsResponse {
    internal_squads: Vec<InternalSquad>,
}

#[derive(Debug, Deserialize)]
struct InternalSquad {
    uuid: String,
    name: String,
}

#[cfg(test)]
mod tests {
    use super::{
        AnoreConfig, AnoreProvider, PaymentProvider, PaymentProviderCode, PaymentProviderRegistry,
        ProviderPaymentStatus, hmac_sha256_hex,
    };
    use http::{HeaderMap, HeaderValue};

    #[test]
    fn registry_reports_enabled_provider_codes_in_a_stable_order() {
        let registry = PaymentProviderRegistry::from_settings(super::PaymentProviderSettings {
            crypto_pay: None,
            anore: Some(AnoreConfig {
                api_key: "key".to_owned(),
                signing_secret: "secret".to_owned(),
                base_url: "https://example.invalid".to_owned(),
            }),
            telegram_stars: None,
        })
        .unwrap();
        assert_eq!(registry.enabled_codes(), vec![PaymentProviderCode::Anore]);
        assert!(registry.get(PaymentProviderCode::Anore).is_some());
    }

    #[test]
    fn anore_rejects_an_unsigned_webhook() {
        let provider = AnoreProvider::new(AnoreConfig {
            api_key: "key".to_owned(),
            signing_secret: "secret".to_owned(),
            base_url: "https://example.invalid".to_owned(),
        })
        .unwrap();
        assert!(
            provider
                .verify_webhook(&HeaderMap::new(), br#"{"id":"payment-1","status":"paid"}"#)
                .is_err()
        );
    }

    #[test]
    fn anore_verifies_a_signed_webhook_with_a_provider_event_id() {
        let provider = AnoreProvider::new(AnoreConfig {
            api_key: "key".to_owned(),
            signing_secret: "secret".to_owned(),
            base_url: "https://example.invalid".to_owned(),
        })
        .unwrap();
        let body = br#"{"id":"payment-1","status":"paid","amount":"299.00","currency":"RUB"}"#;
        let mut headers = HeaderMap::new();
        headers.insert(
            "Anore-Signature",
            HeaderValue::from_str(&hmac_sha256_hex("secret", body)).unwrap(),
        );
        headers.insert("Anore-Event", HeaderValue::from_static("event-1"));
        let event = provider.verify_webhook(&headers, body).unwrap();
        assert_eq!(event.provider_event_id, "event-1");
        assert_eq!(event.provider_invoice_id, "payment-1");
        assert_eq!(event.status, ProviderPaymentStatus::Paid);
        assert_eq!(event.amount_minor, 29_900);
    }
}
