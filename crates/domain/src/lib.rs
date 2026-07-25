//! Business types deliberately independent of HTTP, Telegram, and SQL.

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
    /// Creates a non-negative amount represented in a currency's minor unit.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::NegativeMoney`] when `amount_minor` is negative.
    pub const fn new(amount_minor: i64) -> Result<Self, DomainError> {
        if amount_minor < 0 {
            return Err(DomainError::NegativeMoney);
        }
        Ok(Self { amount_minor })
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdempotencyKey(pub Uuid);

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("money cannot be negative")]
    NegativeMoney,
}

#[cfg(test)]
mod tests {
    use super::{InvoiceStatus, Money};

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
}
