# Production Runbook

## First Deployment

1. Run `sudo ./setup.sh` from the repository root.
2. Enter the absolute runtime directory.
3. The installer generates database credentials and `APPLICATION_ENCRYPTION_KEY`, then starts the stack using `latest` images.
4. Open `http://127.0.0.1:18082` on the server and create the root administrator login and password.
5. Configure host nginx and TLS to proxy the required public domains to loopback services. Do not expose PostgreSQL or Redis.
6. Set public URLs, loopback port plan and Telegram transport from the admin control plane. Apply port changes through `update.sh`, then update host nginx manually.

## Host Reverse Proxy

Docker publishes application ports only on `127.0.0.1`:

- API: `API_HOST_PORT`, default `18080`.
- Mini App: `MINI_APP_WEB_HOST_PORT`, default `18081`.
- Admin: `ADMIN_WEB_HOST_PORT`, default `18082`.
- Telegram webhook: `TELEGRAM_WEBHOOK_HOST_PORT`, default `18083`.

The host administrator owns nginx configuration, certificates, DNS and firewall.
Proxy Mini App `/api/` and `/sub/` to the API port, its static traffic to the
Mini App port, the admin domain to the Admin port, and `/telegram/webhook` to
the Telegram webhook port. TLS private keys remain host-managed and never enter
the deployment directory or application containers. Start from
`deploy/host-nginx.example.conf` and replace domains and port values if needed.

## Portable Directory

`deploy/` is the complete runtime directory. It contains Compose configuration,
`.env`, `data/postgres`, `data/redis`, `backups`, and the clean database
baseline under `db/init`. Stop the stack before moving it. Copying this one
directory moves the application state, except host-managed nginx/TLS files and
any off-host backup destination.

The root encryption key remains outside PostgreSQL by design. It must be kept
with the generated `.env`; losing it makes encrypted integration values and
subscription URLs unreadable.

## Update

1. GitHub Actions deploys every successful push to `main` after image publishing. It uploads runtime files and runs `VPN_BOT_IMAGE_TAG=<commit-sha> ./update.sh` over SSH.
2. Configure repository secrets: `DEPLOY_HOST`, `DEPLOY_SSH_KEY`, and `DEPLOY_PATH`. The deployment key must be allowed to log in as `root` on the host.
3. The deployment script applies SQL migrations, applies the saved loopback port plan, pulls the exact SHA-tagged images, restarts services and verifies API readiness.
4. For an emergency manual update, run `./update.sh` from the runtime directory. Without `VPN_BOT_IMAGE_TAG`, it pulls the configured image tags, normally `latest`.

## Rollback

1. Run `deploy/rollback.sh <previous-image-sha>` from `deploy/`.
2. The script verifies API readiness. Verify Telegram webhook delivery.
3. Do not remove `data/`, PostgreSQL volumes, or Redis volumes during rollback.

## Backups

Run `deploy/backup-postgres.sh` from `deploy/` to create a PostgreSQL custom-format
backup in `./backups` by default. Set `POSTGRES_BACKUP_DIR` to a mounted,
encrypted backup destination outside application data.

Run `deploy/restore-rehearsal.sh <backup-file>` from `deploy/` to restore that
backup into a temporary database and verify its schema. The script always drops
the temporary rehearsal database and never replaces `vpn_bot`.

Store encrypted backups off-host and record a successful restore rehearsal for
each release candidate.
