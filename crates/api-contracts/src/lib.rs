//! JSON contracts shared by API clients and handlers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub code: &'static str,
    pub message: &'static str,
    pub request_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SetupStatusResponse {
    pub setup_required: bool,
}

#[derive(Debug, Deserialize)]
pub struct SetupOwnerRequest {
    pub setup_token: String,
    pub login: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct SetupOwnerResponse {
    pub account_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub login: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub access_token: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct AdminMeResponse {
    pub account_id: Uuid,
    pub login: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicSettings {
    pub brand_name: Option<String>,
    pub mini_app_url: Option<String>,
    pub admin_url: Option<String>,
    pub subscription_url: Option<String>,
    pub support_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSettings {
    pub api_port: u16,
    pub admin_port: u16,
    pub mini_app_port: u16,
    pub webhook_port: u16,
}

#[derive(Debug, Deserialize)]
pub struct JsonValueRequest {
    pub value: Value,
}

#[derive(Debug, Serialize)]
pub struct SecretMetadata {
    pub key: String,
    pub is_set: bool,
}

#[derive(Debug, Serialize)]
pub struct SubscriptionAccessTokenResponse {
    pub subscription_id: Uuid,
    pub token: String,
    pub path: String,
}

#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub id: Uuid,
    pub actor_account_id: Option<Uuid>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<Uuid>,
    pub correlation_id: Uuid,
    pub created_at: DateTime<Utc>,
}
