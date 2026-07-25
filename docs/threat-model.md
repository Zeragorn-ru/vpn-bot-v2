# Threat Model

## Assets

- Wallet ledger, invoices, subscription entitlements, and provider mappings.
- Telegram identities, Mini App sessions, administrator sessions, and roles.
- Payment-provider credentials, Remnawave credentials, encryption keys, and
  opaque subscription access links.
- Audit records and operational settings.

## Trust Boundaries

| Boundary | Threats | Required controls |
| --- | --- | --- |
| Telegram client to API | Forged identity, stale init data, replay | Verify HMAC, enforce `auth_date` TTL, store replay key in Redis, issue short session. |
| Provider to webhook route | Forged, replayed, or malformed payment event | HTTPS, provider-specific signature and timestamp verification, payload size limit, unique event IDs, rate limit. |
| API to PostgreSQL | Duplicate commercial action and inconsistent state | Database transaction, unique constraints, idempotency keys, append-only ledger, outbox. |
| Worker to Remnawave | Timeouts and repeated entitlement creation | Persisted external idempotency key, provider state machine, reconciliation job, retry policy. |
| Browser to admin API | Session theft, CSRF, privilege escalation | Secure HttpOnly SameSite cookies, CSRF token, CORS allowlist, RBAC, reauthentication, audit. |
| Runtime to logs/metrics | Secret or access-link disclosure | Structured redaction, sensitive-field filtering, no raw credentials/access URLs, retention policy. |
| Deployment artifacts | Committed or copied secret | Environment-only secrets, `.env.example` placeholders, secret scan, immutable images, least-privilege CI. |

## Abuse Cases

| Abuse case | Mitigation |
| --- | --- |
| Replaying a successful webhook to credit balance twice | Unique provider event and invoice transition in one transaction. |
| Sending a `paid` event for another invoice | Bind verified provider event, amount, currency, and provider invoice ID to the local invoice. |
| Creating multiple purchases by retrying a mobile request | Require idempotency key and return the original result. |
| Guessing another user's subscription link | Use random opaque revocable access tokens, authorization checks, and no ID-derived links. |
| Forging a Telegram identity | Only trust verified `initData`; user IDs in JSON are ignored for authentication. |
| Admin changes tariff or wallet without trace | Permission check, confirmation, audit diff, correlation ID, and duplicate-submit protection. |
| Remnawave timeout after a debit | Durable pending state, retried idempotent provisioning, operator-visible failure, compensation policy. |
| Credential leak via config or log | Do not store secrets in PostgreSQL in plaintext; redact all secret-bearing fields and scan source/history before publication. |

## Security Acceptance Checks

- Tests reject invalid, expired, and replayed Telegram `initData`.
- Tests reject unsigned, invalidly signed, stale, unmatched, and repeated webhooks.
- Tests prove no repeated wallet credit, purchase, referral reward, or provider entitlement.
- A log inspection test proves access URLs and credentials are redacted.
- RBAC and CSRF tests cover every privileged mutation.
