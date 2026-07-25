#!/usr/bin/env sh
set -eu

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
default_dir=/opt/vpn-bot-v2
printf 'Installation directory [%s]: ' "$default_dir"
read -r install_dir
install_dir=${install_dir:-$default_dir}
case "$install_dir" in
  /*) ;;
  *) printf '%s\n' 'Installation directory must be an absolute path.' >&2; exit 64 ;;
esac
printf 'Enable automatic updates from latest images? [y/N]: '
read -r auto_update

if [ -e "$install_dir" ] && [ "$(ls -A "$install_dir" 2>/dev/null || true)" ]; then
  printf '%s\n' "Installation directory is not empty: $install_dir" >&2
  exit 73
fi

mkdir -p "$install_dir"
cp -R "$source_dir/deploy/." "$install_dir/"
mkdir -p "$install_dir/data/postgres" "$install_dir/data/redis" "$install_dir/backups"

postgres_password=$(openssl rand -base64 36 | tr -d '\n' | tr '/+' '_-')
encryption_key=$(openssl rand -base64 32 | tr -d '\n')
printf '%s\n' \
  'VPN_API_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-vpn-api:latest' \
  'VPN_TELEGRAM_BOT_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-vpn-telegram-bot:latest' \
  'VPN_BILLING_WORKER_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-vpn-billing-worker:latest' \
  'VPN_PROVISIONING_WORKER_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-vpn-provisioning-worker:latest' \
  'VPN_NOTIFICATION_WORKER_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-vpn-notification-worker:latest' \
  'VPN_ADMIN_WEB_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-admin-web:latest' \
  'VPN_MINI_APP_WEB_IMAGE=ghcr.io/zeragorn-ru/vpn-bot-v2-mini-app-web:latest' \
  "POSTGRES_PASSWORD=$postgres_password" \
  "DATABASE_URL=postgres://vpn_bot:$postgres_password@postgres:5432/vpn_bot" \
  'REDIS_URL=redis://redis:6379' \
  "APPLICATION_ENCRYPTION_KEY=$encryption_key" \
  'API_HOST_PORT=18080' \
  'MINI_APP_WEB_HOST_PORT=18081' \
  'ADMIN_WEB_HOST_PORT=18082' \
  'TELEGRAM_WEBHOOK_HOST_PORT=18083' > "$install_dir/.env"
chmod 600 "$install_dir/.env"
chmod +x "$install_dir/update.sh" "$install_dir/rollback.sh" "$install_dir/backup-postgres.sh" "$install_dir/restore-rehearsal.sh" "$install_dir/auto-update.sh" "$install_dir/apply-runtime-settings.sh"

(
  cd "$install_dir"
  docker compose --env-file .env -f docker-compose.yml pull
  docker compose --env-file .env -f docker-compose.yml up -d
)

case "$auto_update" in
  y|Y|yes|YES)
    sed "s|__INSTALL_DIR__|$install_dir|g" "$install_dir/vpn-bot-v2-auto-update.service" > /etc/systemd/system/vpn-bot-v2-auto-update.service
    cp "$install_dir/vpn-bot-v2-auto-update.timer" /etc/systemd/system/vpn-bot-v2-auto-update.timer
    systemctl daemon-reload
    systemctl enable --now vpn-bot-v2-auto-update.timer
    ;;
esac

printf '\nInstalled. Create the root administrator at http://127.0.0.1:18082\n'
