# VPN Bot v2

Telegram VPN sales platform: Rust services, PostgreSQL, Redis, React Mini App and React administrator control plane.

## Install

Requirements: Linux host, Docker Engine with Compose plugin, `curl`, `openssl`, and systemd if automatic updates are enabled.

```sh
git clone https://github.com/Zeragorn-ru/vpn-bot-v2.git
cd vpn-bot-v2
sudo ./setup.sh
```

The installer asks for one absolute installation directory and whether to enable automatic updates. It then generates database credentials and the encryption key, starts the stack, and leaves all services bound to loopback only.

Create the first administrator at `http://127.0.0.1:18082` on the host. Configure host nginx, DNS, and TLS manually. Use `deploy/host-nginx.example.conf` as a reference.

## Runtime Directory

The selected installation directory contains Compose configuration, PostgreSQL and Redis data, backups, generated `.env`, migrations, and update scripts. Stop the stack before moving it to another server.

## Updates

When automatic updates are enabled, `vpn-bot-v2-auto-update.timer` checks the `main` release stream every 15 minutes. It downloads the listed runtime files through the GitHub Contents API, applies SQL migrations, pulls `latest` images, restarts the stack, and verifies API readiness. It does not clone the repository.

For a manual update, run `./update.sh` inside the installation directory.
