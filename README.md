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

Requirements: Docker Compose v2, OpenSSL, `curl`, Python 3 and a host reverse proxy for TLS.

```sh
./setup.sh /opt/vector
# The installer prints the one-time setup token exactly once.
```

The only manually managed `.env` values are:

```env
POSTGRES_PASSWORD=...
APPLICATION_ENCRYPTION_KEY=...
```

PostgreSQL and Redis have no host listener. API, Admin and Mini App bind only to `127.0.0.1` (`18080`, `18082`, `18081`) for an optional host reverse proxy or trusted SSH tunnel. Open Admin through that trusted path and create the first owner with the one-time token. Telegram polling stays disabled until an encrypted `TELEGRAM_BOT_TOKEN` is written in Admin UI. Telegram, Remnawave, public URLs, routes, ports, backups and other runtime settings are entered in Admin UI. Integration credentials are encrypted before storage; public paths and URLs remain ordinary configuration as designed.

## Release delivery

CI validates the source and publishes application images under a full Git SHA. When every image job succeeds on `main`, CI publishes an immutable prerelease tagged `test-<sha>`. CI never opens an SSH connection to a runtime server and has no runtime-server credential.

For initial bootstrap, a trusted operator may run `/opt/vector/deploy/update.sh` with `VPN_BOT_RELEASE` set to the complete published SHA. The server never builds source code: `update.sh` pulls only SHA-tagged GHCR images, runs migrations and writes `data/release` only after health checks pass.

Install ongoing server-initiated updates from a checkout that contains `deploy/`:

```sh
sudo /opt/vector/deploy/install-pull-updater.sh /opt/vector
sudo systemctl start vector-release-pull.service
```

The root-only timer polls GitHub Releases over outbound HTTPS after boot and then about every five minutes with jitter. It accepts only a prerelease whose tag and `target_commitish` both encode the same full SHA, downloads that exact public source archive, and invokes the existing release validator. A failed update restores the previous managed deployment files and leaves the successful release marker intact.

Useful operator checks:

```sh
systemctl status vector-release-pull.timer
journalctl -u vector-release-pull.service -n 100 --no-pager
docker compose --env-file /opt/vector/.env -f /opt/vector/deploy/docker-compose.yml ps
```

To hold automatic promotion during an incident or before a rollback, create `/opt/vector/data/pull-release-hold`. `deploy/rollback.sh` creates this hold automatically. Remove the marker only when automatic promotion should resume.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Apply `db/init/001_baseline.sql` and every file in `deploy/db/migrations/` to a disposable PostgreSQL before release. `web/preview/` is a static visual reference and does not connect to production services.
