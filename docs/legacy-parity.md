# Legacy Parity Contract

## Scope

This document records observable behavior in `/opt/vpn-bot` as the v2 product
specification. v2 starts with an empty PostgreSQL database. It does not import
legacy SQLite, YAML, configuration, secrets, users, balances, subscriptions,
invoices, access URLs, or bot tokens.

## Roles

| Role | v2 responsibility |
| --- | --- |
| User | View subscription, top up wallet, buy or renew access, use a trial, redeem a promo code, invite referrals, and contact support. |
| Bootstrap administrator | Initial administrator identified by an environment allowlist until roles are configured in PostgreSQL. |
| Administrator | Manages users, subscriptions, payments, tariffs, promos, referrals, broadcasts, brand settings, integrations, and audit data. |

Mirror owners are not a v2 P0 role. Bot mirrors require accepting and storing
third-party Telegram bot credentials, so their product and security model must
be redesigned before any later release.

## P0 User Scenarios

| Scenario | Required observable result |
| --- | --- |
| `/start` | User exists, language defaults safely, referral is attached only once, and the user reaches the cabinet. |
| Required channels | Non-admin users who have not joined configured required channels cannot use commercial flows. |
| Language | User can choose Russian or English; all P0 messages use the selected locale with a safe fallback. |
| Tariff catalog | User sees active, ordered tariffs with price, currency, duration, traffic allowance, and availability rules. |
| Wallet top-up | User selects an enabled provider, receives one invoice, and can safely repeat a status check without duplicated credit. |
| Payment webhook | A valid provider event moves one invoice through its permitted state transition and credits one wallet transaction at most once. |
| Purchase and renewal | An authenticated user purchases an eligible tariff with wallet funds or a direct invoice; the order is created once and provisioning is requested asynchronously. |
| Trial | An eligible user activates one 72-hour, 10 GB trial. Eligibility is evaluated server-side and recorded transactionally. |
| Subscription cabinet | User sees status, expiry, available traffic when supplied by the provider, and a revocable opaque access link/instructions. |
| Referral | A referrer can be attached once by a start parameter. A confirmed wallet top-up awards the configured percentage exactly once. |
| Promo | A user can redeem a balance promo or apply a tariff discount promo according to product, tariff, per-user, and global limits. |
| Notifications | Payment result, provisioning outcome, and subscription expiry reminders are queued for delivery rather than sent in business transactions. |
| Account deletion | User can request deletion; v2 anonymizes personal data according to the adopted retention policy while retaining required financial/audit records. |

## P0 Admin Scenarios

| Scenario | Required observable result |
| --- | --- |
| Bootstrap and login | A configured bootstrap Telegram ID can create the first admin session. Further authorization is controlled by PostgreSQL roles and permissions. |
| Brand and operational settings | Administrator manages non-secret brand, contacts, locales, required channels, feature flags, and notification settings. |
| Tariffs | Administrator creates, orders, activates, edits, and retires tariffs. Historical purchases retain price and product snapshots. |
| Payments | Administrator filters invoices and payment attempts, sees normalized status, and can investigate webhook failures without viewing secrets. |
| Users and subscriptions | Administrator searches users, views wallet ledger and subscription history, performs audited adjustments, and requests supported provider actions. |
| Promotions and referrals | Administrator manages promo rules and inspects redemption/reward history. |
| Audit | All privileged mutations record actor, target, correlation ID, before/after diff, and timestamp. |

## Priority Classification

| Priority | Included behavior |
| --- | --- |
| P0 | Telegram enrollment, required channels, RU/EN, catalog, wallet, one selected payment provider, invoice reconciliation and signed webhooks, wallet purchase/renewal, trial, promo/referral rules, Remnawave provisioning, subscription cabinet, critical notifications, minimum admin management, audit, Mini App authentication. |
| P1 | Telegram Stars and additional payment providers, direct-pay checkout for every provider, broadcasts, enhanced analytics, traffic-only add-ons, advanced subscription controls, payment exports, all Remnawave synchronization tooling, account deletion UI, full administrator RBAC editor. |
| P2 | User-created bot mirrors, bypass-link pool, external subscription aggregation, Happ/INCY redirect formats, live configuration editing/reload, database downloads from the admin UI, legacy backup controls, unused payment allocation paths. |

## Business Rules

| Rule | v2 decision |
| --- | --- |
| Money | Store amounts as integer minor units. A wallet is changed only by an immutable ledger transaction. |
| Invoice lifetime | Default 30 minutes; provider-specific expiration is stored with the invoice. |
| Payment idempotency | Provider event ID, provider invoice ID, and application idempotency key are unique in their applicable scopes. Repeated `paid` events do not produce another credit. |
| Trial | One per user, only before any paid or trial VPN entitlement. Default is 72 hours and 10 GB; values are administrative settings. |
| Referral | Referrer is set once and cannot be self. Default reward is 10 percent of a referred user's confirmed wallet top-up, rounded down to the currency minor unit. Reward is unique per source invoice. |
| Balance promo | One redemption per user; creates a wallet credit. |
| Discount promo | Percentage from 0 through 100; can be limited by tariff, product, per-user, and global activation count. It becomes redeemed only when the corresponding purchase completes. |
| Provisioning | A paid purchase enters `provisioning_pending`; worker retries safely. An external provider operation uses an idempotency key and records the external ID. |
| Access links | Use opaque, high-entropy, revocable secrets. Never derive them from Telegram IDs and never log them. |
| Required channels | Membership checks are cached only as an optimization. The authoritative decision is server-side and applies to every commercial entry point. |

## Legacy Interfaces And v2 Decisions

| Legacy interface | v2 decision |
| --- | --- |
| Telegram Bot API and Mini App `initData` | P0. Verify Telegram HMAC, enforce `auth_date` TTL and replay protection; never trust a client-provided user ID. |
| Remnawave | P0. Implement behind a provider port with create, extend, block, delete, and reconciliation operations. |
| Crypto Pay | First-release candidate, pending confirmation that it is the production provider to retain. |
| Telegram Stars | P1 unless selected as the first provider; retain the normalized provider interface. |
| Platega and Anore | P1. Do not expose webhooks until provider-specific signature verification, timestamp checking, and fixture tests exist. |
| Telegram Login Widget | Replace with verified Mini App authentication for P0 admin bootstrap/session issuance where possible; keep a separate admin auth design decision in architecture. |
| Mirrors and BotFather tokens | P2. Do not accept, store, or run third-party bot tokens in v2 P0. |
| Bypass pool and external source aggregation | P2. Excluded pending security, legal, and operational review. |

## Explicitly Excluded Legacy Data

The following legacy sources are not migrated and must not be copied into v2:

- SQLite tables, database backups, and admin login tokens.
- `config.yml`, `config.local.yml`, payment credentials, Telegram tokens, and Remnawave credentials.
- Existing users, balances, invoices, promo redemptions, referral bindings, VPN accounts, access URLs, and subscriptions.
- Mirror bot tokens, bypass pools, external subscription URLs, and audit payloads.

## Known Legacy Defects Not Carried Forward

- Deterministic subscription tokens derived from Telegram IDs.
- Payment fulfillment paths without a verified provider signature gate.
- Plaintext mirror tokens and configuration endpoints that can expose credentials.
- Traffic purchase prerequisite checks that differ by user interface path.
- Duplicated payment fulfillment logic across polling, webhooks, and bot callbacks.

## Acceptance Fixtures

Stage 2 must add synthetic fixtures and contract tests for:

- First start with and without a valid referral.
- Required-channel rejection and successful membership check.
- Trial eligibility and repeated trial rejection.
- Referral rounding in minor units and duplicate invoice replay.
- Promo product/tariff restrictions, global limit, and concurrent redemption.
- Duplicate webhook delivery, timeout/retry, and provider status reconciliation.
- Duplicate purchase request with the same idempotency key.
- Provisioning failure, retry, and no duplicate external account.
