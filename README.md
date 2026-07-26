# VPN Bot v2

Telegram VPN sales platform: Rust services, PostgreSQL, Redis, React Mini App and React administrator control plane.

## Install

Requirements: Linux host, Docker Engine with Compose plugin, `curl`, and `openssl`.

```sh
curl -fsSL https://raw.githubusercontent.com/Zeragorn-ru/vpn-bot-v2/main/setup.sh -o /tmp/vpn-bot-v2-setup.sh
sudo bash /tmp/vpn-bot-v2-setup.sh
```

The installer downloads only the runtime files it needs through the GitHub Contents API, generates database credentials and the encryption key, starts the stack, and leaves all services bound to loopback only.

Create the first administrator at `http://127.0.0.1:18082` on the host. Configure host nginx, DNS, and TLS manually. Use `deploy/host-nginx.example.conf` as a reference.

## Runtime Directory

The selected installation directory contains Compose configuration, PostgreSQL and Redis data, backups, generated `.env`, migrations, and update scripts. Stop the stack before moving it to another server.

## Updates

Production deployments are run by GitHub Actions after all images for `main` are published. The workflow uploads the runtime files, connects to the server through SSH, applies migrations, pulls images tagged with the exact commit SHA, restarts the stack, and verifies API readiness.

Configure the GitHub Actions secrets before enabling production deployment: `DEPLOY_HOST`, `DEPLOY_SSH_KEY`, and `DEPLOY_PATH` (for example, `/opt/vpn-bot`). The SSH key must authorize the `root` user on the deployment host.

For a manual update, run `./update.sh` inside the installation directory.
