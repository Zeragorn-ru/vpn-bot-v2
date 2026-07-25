# Architecture Decisions

## System Shape

v2 is a Rust workspace with stateless processes and PostgreSQL as the source of
truth. Redis is used only for rate limits, short-lived sessions, replay keys,
locks, and cache. PostgreSQL transactional outbox events coordinate workers;
workers do not make internal synchronous HTTP calls for business operations.

```text
Telegram Bot / Mini App / Admin SPA / Payment Provider
                    |
                    v
                api + bot
                    |
             PostgreSQL <--> Redis
                    |
   billing / provisioning / notification workers
                    |
     Payment providers / Remnawave / Telegram API
```

## Repository Layout

```text
apps/api
apps/telegram-bot
apps/billing-worker
apps/provisioning-worker
apps/notification-worker
crates/domain
crates/storage
crates/integrations
crates/api-contracts
crates/observability
web/admin
web/mini-app
```

## Decision Records

### ADR-001: Telegram update transport

Use Telegram webhooks in production and long polling only for local development.
Production webhook requests use a configured secret token, HTTPS, rate limits,
and a bounded request body. Webhook ingestion stores enough update identity to
make processing idempotent before sending a response.

### ADR-002: First payment provider

Implement providers behind a normalized `PaymentProvider` port and registry in
Stage 2. The initial adapter set is Crypto Pay, Anore, and Telegram Stars;
Platega is excluded. Each adapter implements invoice creation, status
reconciliation, payment-state normalization, and webhook verification where a
provider supplies a signed callback. Crypto Pay and Stars do not accept an
unverified public payment callback: Crypto Pay is reconciled against its API,
while Stars completion comes only from trusted Telegram successful-payment
updates. Anore requires `Anore-Signature` and `Anore-Event` before an event is
accepted. No provider event changes money until its identity, status, amount,
currency, and invoice mapping are verified transactionally.

Providers execute within `billing-worker` by default. This centralizes the
wallet ledger, replay protection, outbox, and reconciliation without internal
HTTP hops. Provider enablement is held in PostgreSQL; secrets remain in
environment-backed secret storage. Disabling a provider blocks new invoices
but reconciliation continues for existing pending invoices until a terminal
status. Additions require a new adapter module, fixture tests, database setting
registration, and an explicit webhook/reconciliation policy.

`telegram_stars` invoices are excluded from generic provider reconciliation:
their authoritative paid signal is Telegram's trusted `successful_payment`
update, which the Telegram update handler will submit to the same transactional
fulfillment use case. This prevents an unresolved Stars invoice from blocking
the worker's Crypto Pay/Anore reconciliation queue.

### ADR-003: Identity and sessions

Mini App identity derives exclusively from cryptographically verified
`Telegram.WebApp.initData`. The API enforces an `auth_date` TTL and replay key
in Redis, then issues a short-lived server session. A body-supplied Telegram ID
is never authoritative. Admin authorization uses the same verified Telegram
identity, PostgreSQL RBAC, reauthentication for critical operations, and
cookie protections when cookies are selected.

### ADR-004: Money and fulfillment

Money uses integer minor units and immutable wallet transactions. Invoice state
transitions and wallet credit occur in one PostgreSQL transaction. A completed
purchase creates an outbox event; the provisioning worker performs the
external operation with a persisted idempotency key. Failed provisioning is
observable and retryable without a second debit or second entitlement.

### ADR-005: VPN provider boundary

The domain crate owns subscriptions and lifecycle states. The integrations
crate implements a Remnawave adapter that translates only provider requests
and responses. Subscription URLs are encrypted at rest where feasible, redacted
from logs, and exposed only to their authenticated owner.

### ADR-006: Configuration boundary

Environment variables hold infrastructure endpoints, encryption keys, allowed
origins, logging settings, and integration secrets. PostgreSQL holds editable
brand content, tariffs, trial, promos/referrals, feature flags, required
channels, notification rules, subscription policies, client instructions, and
non-secret provider settings. Each editable setting has server-side validation,
an audited admin mutation, and a safe default. APIs never return a stored secret
after save.

### ADR-007: Notification automation

Automation rules are non-secret, typed PostgreSQL settings managed through the
admin API. A rule has an explicit trigger, delay, cooldown, localized message
template, and action. The worker evaluates only allowlisted server-side rule
types and enqueues notifications in the same database that owns delivery;
rules never execute administrator-provided SQL, code, or HTTP requests.

The initial trigger is `subscription_expired_without_renewal`: a subscription
expired at least N days ago and its owner has no newer active or provisioning
subscription. This is deliberately not represented as "VPN activity" until a
trusted Remnawave usage/session signal is synchronized. Delivery is idempotent
per rule, user, and expired subscription through a deterministic event key and
transaction-scoped lock.

Personal promo actions are deferred. The current commerce schema does not
enforce that a generated code belongs to exactly one recipient, so sending a
code would allow leakage and redemption by another user. That action requires
a server-enforced ownership model, expiry, redemption limit, and atomic issue
record before being exposed in the admin UI.

## Service Ownership

| Process | Owns |
| --- | --- |
| `api` | Versioned HTTP API, Mini App auth, admin sessions, webhook ingestion, OpenAPI. |
| `telegram-bot` | Telegram update handling, dialog state machine, message requests. |
| `billing-worker` | Invoice reconciliation, signed webhook processing continuation, wallet credits, referral rewards. |
| `provisioning-worker` | Subscription lifecycle, Remnawave operations, provider reconciliation. |
| `notification-worker` | Notification queue, scheduled reminders, broadcasts and reports. |

## Outbox Contracts

| Event | Producer | Consumer |
| --- | --- | --- |
| `payment.confirmed` | API/billing transaction | billing worker, notification worker |
| `subscription.requested` | Purchase transaction | provisioning worker |
| `subscription.changed` | Provisioning transaction | notification worker |
| `notification.requested` | Any business transaction | notification worker |
| `audit.recorded` | Privileged mutation | audit sink/analytics later |

Every outbox record includes event ID, aggregate type and ID, payload version,
attempt count, next-attempt time, and last error. Consumers lock records while
processing and are idempotent by event ID plus aggregate state.

## Initial HTTP Contract

All routes are under `/api/v1`, return structured JSON errors, and publish
OpenAPI before frontend implementation.

| Route | Purpose |
| --- | --- |
| `POST /auth/telegram` | Verify Mini App `initData` and issue a session. |
| `GET /me` | User profile, wallet summary, and subscription summary. |
| `GET /tariffs` | Active public tariff catalog. |
| `POST /invoices` | Create a top-up or direct-purchase invoice with an idempotency key. |
| `GET /invoices/{id}` | Read invoice status and resulting subscription visible only to its owner. |
| `POST /purchases` | Spend wallet funds to purchase or renew a tariff. |
| `POST /trials` | Activate one eligible trial. |
| `POST /promos/validate` | Validate a promo against an intended purchase. |
| `POST /webhooks/payments/{provider}` | Receive a provider event with provider-specific verification. |
| `GET /admin/dashboard` | RBAC-protected aggregate counts and confirmed RUB revenue for the operations overview. |
| `GET /admin/audit` | RBAC-protected recent privileged mutations for the audit-log screen. |
| `GET /admin/*` | RBAC-protected operational resources. |
| `POST /admin/*` | RBAC-protected, audited mutations. |

Operational lists use the common `limit` and `offset` query parameters and
return `{items, total, limit, offset}`. Users accept `q`; subscriptions accept
`q` and `status`; invoices accept `q`, `status`, and `provider`; audit accepts
`action`. Filtering and counting happen in PostgreSQL before pagination.
| `GET /healthz` | Liveness check. |
| `GET /readyz` | Dependency readiness check. |

## UX Information Architecture

### Mini App

1. Home: subscription status and primary next action.
2. Catalog: tariffs and eligibility-aware purchase flow.
3. Checkout: wallet, price breakdown, promo, and payment option.
4. Access: subscription URL, safe copy action, and client instructions.
5. Account: language, referrals, support, and deletion request.

### Admin

1. Dashboard: revenue, payments, subscriptions, users, and provisioning errors.
2. Operations: users, subscriptions, invoices, payments, and support context.
3. Commerce: tariffs, promos, referrals, and payment integrations.
4. Communication: notifications and broadcasts.
5. Configuration: brand, content, channels, roles, and audit log.

## Security Baseline

- Secret values are environment-only and redacted from logs, responses, audit diffs, and exports.
- Provider callbacks require HTTPS, provider-specific verification, rate limiting, and idempotency.
- The API applies Redis-backed fixed-window limits by direct peer address. Authentication,
  payment webhooks, and admin mutations use stricter buckets; if Redis is unavailable,
  protected API requests fail closed. Browser CORS is an explicit allowlist from
  `MINI_APP_ALLOWED_ORIGINS`; wildcard origins are not permitted.
- Access links are opaque, random, revocable, and never derived from user IDs.
- API validates schema, limits request sizes, returns stable error codes, and uses correlation IDs.
- Admin actions require RBAC, audit records, double-submit protection, and reauthentication for destructive actions.
