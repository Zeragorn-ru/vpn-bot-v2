# P0 plan: Telegram VPN shop and subscription aggregator

## Context

Продукт — Telegram-бот для продажи VPN-доступа. Пользователь получает одну subscription-ссылку собственного агрегатора. При открытии в браузере ссылка показывает web-страницу подписки, а VPN-клиенту возвращает конфигурацию в подходящем формате. Главная цель переделки — убрать трудную для поддержки и монолитную legacy-архитектуру, сохранив коммерчески важный P0-функционал.

Legacy-поведение агрегатора исследовано в `/opt/vpn-bot/src/vpn_sales_bot/aggregator/`. Реализация не переносится буквально: сохраняются observable contract и нужные форматы, но устраняются предсказуемые user-id slug, смешение HTTP/бизнес-логики, raw provider payloads и небезопасное логирование.

## Installation and configuration model

### Минимум bootstrap secrets

`.env`/Compose должен содержать только то, без чего невозможно безопасно запустить инфраструктуру:

```env
POSTGRES_PASSWORD=<generated-random-value>
APPLICATION_ENCRYPTION_KEY=<generated-32-byte-key>
```

Пароль первого владельца, Telegram token, Remnawave token, payment secrets, S3 credentials, домены, public paths и рабочие настройки не являются обязательными env-параметрами. Installer генерирует bootstrap flow/одноразовый setup token или создаёт владельца через локальный setup command; пароль не хранится в compose.

Если конкретный deployment требует внешний secret manager, эти два значения могут приходить Docker secrets вместо `.env`. В Git остаётся только `.env.example` с placeholders.

### Один Compose-файл

`docker-compose.yml` описывает весь runtime:

```text
postgres
redis
api                 # REST, Mini App API, Admin API, aggregator, webhooks
telegram-bot
billing-worker
provisioning-worker
notification-worker
admin-web
mini-app-web
```

Сервисы не собираются на production host: CI публикует immutable images, Compose только запускает их. PostgreSQL, Redis и приложения находятся в одной private Docker network. Наружу публикуются только loopback-порты gateway/admin/mini-app/webhook, если они реально нужны host nginx.

### Что настраивает администратор

После входа в Admin UI настройки хранятся в PostgreSQL. Только секреты шифруются at rest; обычные URL, пути, домены и настройки не шифруются:

- Telegram bot token, webhook secret и polling/webhook mode;
- Remnawave endpoint/token, default squad и provider policies;
- payment provider credentials and enablement;
- тарифы, цены, trial, promo/referral rules;
- бренд, локали, rich-text templates и client instructions;
- required channels;
- aggregator host/path rules, browser page and supported formats;
- logical ports, reverse-proxy paths and public URLs;
- backup schedule, S3 endpoint/bucket/region/prefix and encrypted S3 credentials;
- notification schedules, retention, feature flags and operational limits;
- admin accounts, roles and permissions.

Secret values never return from API or appear in audit diffs/logs. Changing runtime routing/ports creates an audited pending change and an explicit **Apply configuration** operation. It must validate the new settings, write a generated runtime snapshot, restart/reload only affected containers, and verify readiness. A process cannot magically change its Docker host binding without a controlled Compose reload; this distinction must be visible in UI.

### Persistent data and backups

Все чувствительные к потере данные монтируются на disk volumes/bind mounts:

```text
./data/postgres   -> PostgreSQL data
./data/redis      -> Redis AOF/session data if retained
./data/backups    -> local encrypted backup staging
./data/uploads    -> only if a future feature needs uploads
```

Database schema, migrations and deployment metadata остаются в release files, не в volume. Backup worker, управляемый из Admin UI, по расписанию:

1. делает consistent PostgreSQL dump/custom backup;
2. сохраняет необходимые encrypted application settings and key metadata according to retention policy;
3. при необходимости архивирует Redis only for explicitly configured recoverable data (Redis is not source of truth);
4. шифрует архив before upload;
5. uploads to S3-compatible storage with checksum, object lock/retention where available;
6. records success/failure, size, checksum, release and schema version in backup history;
7. prunes local/S3 copies only after retention and successful verification.

S3 endpoint, bucket, schedule, retention and credentials are configured in Admin UI, but the encryption root key remains outside PostgreSQL (in Docker secret/env) so a database leak does not decrypt backups. A restore rehearsal to a disposable PostgreSQL database is a required operator action and CI/release acceptance check. Backups must never be silently treated as successful if S3 is unavailable.

## P0 roles and access

Use permission checks, not hard-coded role names:

```text
owner:      system, secrets, admins, billing, provider, destructive actions
admin:      users, tariffs, subscriptions, payments, operations, settings
support:    users.read, subscriptions.read, support notes; no secrets or money mutation
billing:    payments.read, wallet.adjust, refunds; no provider/Telegram secrets
operator:   provisioning, revoke/block, provider sync; no payment secrets
viewer:     dashboard/read-only/audit.read
```

Tables: `admin_accounts`, `roles`, `permissions`, `role_permissions`, `admin_account_roles`, `admin_sessions`, `admin_audit_log`. Sensitive mutations require reauthentication/confirmation, correlation ID, before/after diff with redaction, and idempotency protection.

## Subscription aggregator contract from legacy

### Legacy observed flow

The old `AggregatorHandlers.handle_unified` does the following:

1. Reads `user_id` from the URL and a `token` query parameter.
2. Validates a deterministic SHA-256 slug derived from `user_id`.
3. Classifies browser vs VPN client using User-Agent/Accept. Known client markers include Karing, Happ, Hiddify, Clash/Mihomo, Shadowrocket, sing-box, Streisand, Loon, Nekobox/Nekoray, v2ray and similar clients.
4. For a browser, serves the SPA or subscription HTML page with expiry, traffic, copy link and instructions.
5. For a VPN client, fetches the Remnawave `subscriptionUrl`, with a local access-key fallback, then adds configured external sources and static VLESS.
6. Fetches HTTP(S) sources, extracts nodes, labels them by source, deduplicates exact nodes, and renders based on client profile.
7. Returns client metadata headers and logs client, format, HWID, User-Agent, address, node count and timestamp.

The legacy renderer dispatches:

```text
xray_json   -> Xray JSON
singbox_json -> sing-box JSON
clash_yaml  -> Clash/Mihomo YAML
default     -> generic base64 subscription
```

Legacy response headers include `Profile-Title`, `announce`, `Profile-Update-Interval`, `Subscription-Userinfo`, `Support-Url`, `Profile-Web-Page-Url`, `providerid`, `sub-info-color`, optional `color-profile`, and `Content-Disposition`. These are compatibility features, not permissions to expose provider internals.

### New P0 URL contract

The canonical path is configured by the administrator, defaulting to `/sub/{token}`. Ordinary subscription paths, provider subscription URLs and aggregator routing settings are stored as regular PostgreSQL text and do not need database encryption. The access token itself is high-entropy, opaque and revocable; only its hash is stored in PostgreSQL. It is never derived from Telegram ID or user ID. The token is not a secret value returned by the database: it is issued once to the owner, and rotation/revocation is controlled by the access-token table.

```text
GET /sub/{token}                         # content negotiation
GET /sub/{token}?format=clash            # Clash/Mihomo YAML
GET /sub/{token}?format=sing-box         # sing-box JSON
GET /sub/{token}?format=xray             # Xray JSON
GET /sub/{token}?format=base64           # generic base64 URI list
```

`format` is normalized to an enum and validated against configured enabled formats. Explicit format wins over User-Agent. Without `format`, browser detection uses a conservative allowlist of known VPN-client markers plus `Accept`; unknown clients receive the safe subscription format, never the browser secret page by accident. `HEAD` never fetches provider data.

Compatibility redirects are P1/P2 and only added when a real client requires them:

```text
/happ/{token}
/incy/{token}
```

They must validate the same opaque token, issue no permanent cache, and never accept a user ID as authorization.

### Response pipeline

```text
validate token hash + revoked/expired status
  -> classify browser/client or honor explicit format
  -> read entitlement and cached provider snapshot
  -> fetch/update provider snapshot through provider adapter
  -> normalize to SubscriptionProfile
  -> deduplicate nodes by canonical fingerprint
  -> optional safe source merge (P1 external sources)
  -> apply renderer and format-specific headers
  -> return response with no-store/private cache policy
  -> write privacy-safe access log
```

`SubscriptionProfile` is provider-independent:

```text
profile title, expiry, traffic usage/limit, support URL, normalized proxy nodes
```

Provider IDs, internal UUIDs and provider credentials are not returned. The configured public subscription URL/path is intentionally ordinary configuration and may be stored and returned where needed; only the opaque access token authorizes the profile. Browser HTML shows status, expiry, traffic, copy action, supported client instructions and support contact. It uses `Referrer-Policy: no-referrer`, CSP, escaped configured text and no secret-bearing analytics.

### Caching and consistency

Provider data is refreshed by provisioning/reconciliation workers and cached for a short configurable interval (default 30–120 seconds). Aggregator reads the cache for normal traffic. Revoke/rotate immediately invalidates token and profile cache. Provider failure returns a controlled error or last-known profile only within an explicit freshness bound; it must not silently serve an indefinitely stale active subscription.

Do not log the full `/sub/{token}` URL. Store token hash, format, client category, status, node count, coarse network metadata and timestamp. Never log raw subscription payloads or access keys.

## P0 implementation boundaries

- `aggregator` is a module inside `api`, not another container initially.
- HTTP handlers only validate requests and map responses.
- Application service owns token access, entitlement checks, provider snapshot and render selection.
- Provider adapter owns Remnawave API and external source fetching.
- Renderers consume normalized nodes and cannot make network calls.
- Browser page and VPN config are separate response paths with shared token authorization.
- Wallet/order/payment/provisioning use one idempotent application flow and transactional outbox.

## Full P0 implementation contract

### Runtime responsibilities

```text
admin-web       static Admin UI
mini-app-web    static Telegram Mini App
api             REST API + auth + Admin API + Mini App API + aggregator + webhooks
telegram-bot    Telegram updates, menus, callbacks, Stars pre-checkout/payment events
billing-worker  invoice polling/reconciliation and wallet/order transitions
provisioning-worker Remnawave lifecycle and subscription snapshots
notification-worker outbox delivery, reminders and scheduled backup jobs
postgres        source of truth
redis           cache, locks, rate limits, replay keys; rebuildable
```

The first release is a modular monolith in one Rust workspace, not a collection of independently evolving microservices. Each process has a narrow runtime responsibility, while domain/application modules and database transaction rules are shared. No handler directly performs arbitrary SQL or calls Remnawave/Telegram as part of a money transaction.

### Bootstrap lifecycle

1. Installer creates the data directories, generates `POSTGRES_PASSWORD` and `APPLICATION_ENCRYPTION_KEY`, writes a minimal `.env`, validates Docker/Compose and starts the single Compose stack.
2. The API exposes a local setup route only while no owner exists. Setup is bound to loopback and protected by a one-time setup token or an interactive local setup command; no permanent admin password is placed in `.env`.
3. The first owner creates an admin session, sets a password hash and can configure Telegram, Remnawave, one payment provider, public routing and the first tariff.
4. Admin UI validates integrations with safe `getMe`/health/test calls, stores only encrypted secrets and shows readiness blockers.
5. Bot/webhook and commercial flows stay disabled until required configuration is valid. Applying a routing/port change creates a pending release, reloads Compose under controlled supervision and verifies every health/readiness check.

### Configuration classification

| Stored in PostgreSQL | Stored as secret |
|---|---|
| ports, hostnames, public URLs and `/sub/{token}` path template | Telegram bot token/webhook secret |
| tariffs, trial, promo/referral rules, locales and templates | Remnawave/API credentials |
| required channels, provider selection and non-secret settings | payment credentials |
| aggregator formats, headers, instructions and cache TTL | S3 access key/secret |
| backup schedule, retention, bucket and operational limits | encryption root key remains outside DB |
| admin roles, permissions and feature flags | database password remains outside DB |

Public URL/path fields are not encrypted. They are configuration, may be displayed in the UI and are audited normally. Only credential/secret fields use application encryption. Subscription access tokens are a separate security mechanism: generate high entropy, store only a hash, issue once, rotate/revoke, and never log the raw token.

### Minimum database model

Core tables:

```text
users, user_profiles, wallets, wallet_transactions
catalog_tariffs, orders, invoices, payment_attempts, payment_webhook_events
subscriptions, provider_accounts, provider_snapshots
subscription_access_tokens, subscription_request_logs
promo_codes, promo_redemptions, referrals, referral_rewards
outbox_events, notifications, notification_deliveries
app_settings, app_secrets, backup_jobs, backup_runs
admin_accounts, roles, permissions, role_permissions, admin_account_roles
admin_sessions, admin_audit_log
```

Money uses integer minor units and append-only wallet transactions. Orders and invoices have explicit state transitions and unique idempotency keys. Provider snapshots keep aggregator requests off the critical path and retain `fetched_at`, `expires_at`, provider revision and normalized status. Financial/audit records survive account anonymization.

### Admin UI P0 sections

- **Setup/readiness:** missing integrations, webhook status, migration version, worker/outbox health.
- **Dashboard:** users, active subscriptions, revenue, pending invoices, provisioning failures, backup status and last provider sync.
- **Users/subscriptions:** search, entitlement details, revoke/rotate link, support notes; no raw provider credentials.
- **Commerce:** tariffs, trial, promo/referral rules and the selected payment provider.
- **Aggregator:** public URL/path, enabled formats, browser page, client instructions, headers, cache freshness and renderer options.
- **Infrastructure:** logical ports, reverse-proxy routes, Telegram transport, backup/S3 schedule, retention and restore rehearsal.
- **Access control:** accounts, roles, permissions, forced session revocation and audit history.

Every privileged mutation checks a permission, records actor/target/correlation/before-after (redacted), and uses a confirmation/re-authentication step for secrets, refunds, revoke/delete, routing reload and restore.

### Backup and restore contract

The scheduled backup worker is configured from Admin UI but runs with durable job state. It creates a consistent PostgreSQL backup, encrypts the archive, computes a checksum, uploads it to S3-compatible storage, verifies the object, records release/schema/checksum/size and applies retention. Local staging is mounted under `./data/backups`; PostgreSQL data is under `./data/postgres`; Redis data under `./data/redis` is optional to back up because it is rebuildable. S3 outage is an explicit failed run, not a success with a warning. Restore rehearsal always targets a disposable database first and reports its result in Admin UI.

### Aggregator acceptance contract

The aggregator is an `api` module with independent tests. It accepts `/sub/{token}` and optional `format=clash|sing-box|xray|base64`. Explicit format wins over User-Agent. Without it, the classifier recognizes legacy client markers (Happ, Karing, Hiddify, Clash/Mihomo, Shadowrocket, sing-box, Streisand, Loon, Nekobox/Nekoray, v2ray family) and browser `Accept` headers. Unknown clients receive a safe generic config, never a browser page containing the access URL.

After token/entitlement validation, the application reads a fresh provider snapshot, normalizes nodes, deduplicates by canonical fingerprint and selects a renderer. P0 uses the own Remnawave subscription; external source merging, static VLESS and bypass pools are deferred until the provider-independent source contract is reviewed. Browser responses show status/expiry/traffic/instructions only. VPN responses may include compatibility headers (`Profile-Title`, `announce`, `Profile-Update-Interval`, `Subscription-Userinfo`, `Support-Url`, `Profile-Web-Page-Url`, `providerid`, `sub-info-color`) but never provider credentials or internal IDs.

Responses use private/no-store policy where tokens are present, `Referrer-Policy: no-referrer`, CSP and escaped configured text. Request logs store token hash, format, client category, status, node count, coarse address and timestamp, never the raw URL or payload. Provider failure may serve a cached snapshot only within a configured freshness bound.

### Delivery order

1. Compose/bootstrap/data directories and PostgreSQL migrations.
2. Admin authentication, RBAC, settings/secrets separation and audit.
3. Telegram auth/bot transport and rich-text client boundary.
4. Catalog, wallet ledger, order/invoice idempotency and one payment provider.
5. Remnawave adapter, provisioning/outbox and subscription snapshots.
6. Opaque token issue/revoke and aggregator browser/client renderers.
7. Notifications, scheduled backup/S3 and operational dashboard.
8. CI migration tests, image provenance, smoke deploy, backup restore rehearsal and rollback runbook.

## Open decisions to settle before implementation

1. Whether the single public gateway is preferred over separate hostnames for API, admin, Mini App and aggregator. Separate hostnames simplify CSP/CORS; one gateway simplifies installation.
2. Whether Redis AOF is worth backing up. PostgreSQL is authoritative; sessions/cache can normally be rebuilt.
3. Whether provider data should be refreshed synchronously on an expired cache (better freshness, higher tail latency) or only by worker (better resilience, possibly stale data).
4. Which one payment provider is P0.
5. Whether admin-configured rich text uses HTML templates with strict tags or structured blocks. Structured blocks are safer; HTML is more flexible.
6. Whether routing/port changes are applied automatically after validation or require an explicit restart confirmation.
