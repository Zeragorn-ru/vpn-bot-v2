# VECTOR VPN bot v2

VECTOR is a Rust-based Telegram VPN shop with a Mini App, Admin UI and an opaque-token subscription aggregator.

## Current implementation slice

- PostgreSQL baseline with RBAC, encrypted app secrets, sessions, audit log, subscriptions and outbox.
- Axum API: health/readiness, one-time owner bootstrap, admin login/logout, settings and secret management.
- Opaque access-token issue/rotation/revocation. PostgreSQL stores only token hashes.
- Aggregator routes:
  - `/sub/{token}` browser/client negotiation;
  - `?format=clash`, `?format=sing-box`, `?format=xray`, `?format=base64`.
- Typed Telegram Bot API boundary with safe HTML rich text and Bot API 10.4-compatible updates.
- Telegram polling `/start` enrollment with durable update de-duplication.
- Remnawave adapter and provisioning worker with encrypted normalized provider snapshots.
- Strict Rust and migration tests.

The old Python data, credentials, SQLite database, deterministic subscription slugs and legacy provider payloads are not imported.

## Clean installation

Requirements: Docker Compose v2, OpenSSL and a host reverse proxy for TLS.

```sh
./setup.sh /opt/vector
# The installer prints the one-time setup token exactly once.
# CI/CD then deploys a full immutable Git SHA to this runtime directory.
```

The only manually managed `.env` values are:

```env
POSTGRES_PASSWORD=...
APPLICATION_ENCRYPTION_KEY=...
```

CI/CD deploys only published, SHA-tagged images; the server never builds the application source. PostgreSQL and Redis have no host listener. API, Admin and Mini App bind only to `127.0.0.1` (`18080`, `18082`, `18081`) for an optional host reverse proxy or SSH tunnel. Open Admin through that trusted path and create the first owner with the one-time token. Telegram polling stays disabled until an encrypted `TELEGRAM_BOT_TOKEN` is written in Admin UI. Telegram, Remnawave, public URLs, routes, ports, backups and other runtime settings are entered in Admin UI. Integration credentials are encrypted before storage; public paths and URLs remain ordinary configuration as designed.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Apply `db/init/001_baseline.sql` and every file in `deploy/db/migrations/` to a disposable PostgreSQL before release. `web/preview/` is a static visual reference and does not connect to production services.
