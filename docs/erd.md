# Initial ERD

This is the logical model implemented by the application.

```text
users 1---1 user_profiles
users 1---1 wallets 1---* wallet_transactions
users 1---* invoices 1---* payment_attempts
invoices 1---* payment_webhook_events
users 1---* subscriptions 1---1 vpn_accounts
subscriptions 1---* subscription_events
tariffs 1---* tariff_prices
users 1---* promo_redemptions *---1 promo_codes
users 1---* referrals (referrer_id, referred_id)
referrals 1---* referral_rewards
users 1---* notifications
users 1---* audit_log (actor_id)
outbox_events -> aggregate entities
```

## Core Entities

| Group | Entities |
| --- | --- |
| Identity | `users`, `user_profiles`, `user_languages`, `user_settings`, `roles`, `user_roles`, `admin_sessions` |
| Brand | `brands`, `brand_locales`, `app_settings`, `media_assets`, `legal_documents`, `required_channels` |
| Commerce | `tariffs`, `tariff_prices`, `promo_codes`, `promo_redemptions`, `referrals`, `referral_rewards` |
| Money | `wallets`, `wallet_transactions`, `invoices`, `payment_attempts`, `payment_webhook_events` |
| VPN | `vpn_accounts`, `subscriptions`, `subscription_events`, `provider_sync_jobs`, `vpn_provider_credentials` |
| Delivery | `notifications`, `broadcasts`, `notification_deliveries`, `scheduled_reports` |
| Reliability | `outbox_events`, `idempotency_keys`, `audit_log` |

## Invariants

- `users.telegram_user_id` is unique.
- Wallet balance is derived from or atomically maintained with `wallet_transactions`.
- `(provider, provider_invoice_id)` is unique when a provider invoice ID exists.
- `(provider, provider_event_id)` is unique for webhook replay protection.
- A referral has unique `referred_id`; it cannot reference the same user as referrer.
- A balance promo has unique `(promo_code_id, user_id)` redemption.
- A trial entitlement has unique `(user_id, entitlement_type)` while the policy permits only one.
- Each external VPN provider ID is unique per provider.
- `outbox_events` include a stable event UUID and are never deleted before retention expires.
