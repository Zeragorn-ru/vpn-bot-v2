//! Domain values shared by every runtime process.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const DEFAULT_TRIAL_DURATION_SECONDS: i64 = 72 * 60 * 60;
pub const DEFAULT_TRIAL_TRAFFIC_BYTES: i64 = 10_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Money {
    pub amount_minor: i64,
}

impl Money {
    /// Creates a non-negative minor-unit amount.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::NegativeMoney`] for negative amounts.
    pub const fn new(amount_minor: i64) -> Result<Self, DomainError> {
        if amount_minor < 0 {
            Err(DomainError::NegativeMoney)
        } else {
            Ok(Self { amount_minor })
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvoiceStatus {
    Pending,
    Paid,
    Expired,
    Cancelled,
}

impl InvoiceStatus {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Pending, Self::Paid | Self::Expired | Self::Cancelled)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionStatus {
    ProvisioningPending,
    Active,
    Suspended,
    Expired,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubscriptionFormat {
    Clash,
    SingBox,
    Xray,
    Base64,
}

impl SubscriptionFormat {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "clash" => Some(Self::Clash),
            "sing-box" | "singbox" => Some(Self::SingBox),
            "xray" => Some(Self::Xray),
            "base64" => Some(Self::Base64),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clash => "clash",
            Self::SingBox => "sing-box",
            Self::Xray => "xray",
            Self::Base64 => "base64",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyNode {
    pub name: String,
    pub uri: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscriptionProfile {
    pub title: String,
    pub expires_at: Option<i64>,
    pub traffic_used_bytes: i64,
    pub traffic_limit_bytes: Option<i64>,
    pub support_url: Option<String>,
    pub nodes: Vec<ProxyNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyKey(pub Uuid);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("money cannot be negative")]
    NegativeMoney,
}

#[cfg(test)]
mod tests {
    use super::{InvoiceStatus, Money, SubscriptionFormat};

    #[test]
    fn invoice_cannot_leave_a_terminal_state() {
        assert!(InvoiceStatus::Pending.can_transition_to(InvoiceStatus::Paid));
        assert!(!InvoiceStatus::Paid.can_transition_to(InvoiceStatus::Pending));
    }

    #[test]
    fn money_rejects_negative_values() {
        assert!(Money::new(-1).is_err());
        assert_eq!(Money::new(0).unwrap().amount_minor, 0);
    }

    #[test]
    fn format_parser_is_conservative() {
        assert_eq!(
            SubscriptionFormat::parse("sing-box"),
            Some(SubscriptionFormat::SingBox)
        );
        assert!(SubscriptionFormat::parse("unknown").is_none());
    }
}
