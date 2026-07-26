#!/usr/bin/env bash
set -eu

repository=${VPN_BOT_REPOSITORY:-Zeragorn-ru/vpn-bot-v2}
branch=${VPN_BOT_BRANCH:-main}
api_base="https://api.github.com/repos/$repository/contents/deploy"
default_dir=/opt/vpn-bot-v2
printf 'Installation directory [%s]: ' "$default_dir"
if ! read -r install_dir </dev/tty; then
  printf '%s\n' 'Interactive input is unavailable. Download setup.sh to a file, then run: sudo sh /path/to/setup.sh' >&2
  exit 64
fi
install_dir=${install_dir:-$default_dir}
case "$install_dir" in
  /*) ;;
  *) printf '%s\n' 'Installation directory must be an absolute path.' >&2; exit 64 ;;
esac
if [ -e "$install_dir" ] && [ "$(ls -A "$install_dir" 2>/dev/null || true)" ]; then
  printf '%s\n' "Installation directory is not empty: $install_dir" >&2
  exit 73
fi

mkdir -p "$install_dir"
work_dir=$(mktemp -d)
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT INT TERM
fetch() {
  path=$1
  destination=$2
  mkdir -p "$(dirname -- "$destination")"
  curl --fail --silent --show-error --location \
    -H 'Accept: application/vnd.github.raw+json' \
    "$api_base/$path?ref=$branch" \
    --output "$destination"
}
for path in docker-compose.yml update.sh rollback.sh backup-postgres.sh restore-rehearsal.sh apply-runtime-settings.sh host-nginx.example.conf .env.example; do
  fetch "$path" "$work_dir/$path"
done
fetch db/init/001_baseline.sql "$work_dir/db/init/001_baseline.sql"
fetch db/migrations/manifest.txt "$work_dir/db/migrations/manifest.txt"
while IFS= read -r migration || [ -n "$migration" ]; do
  [ -n "$migration" ] || continue
  fetch "db/migrations/$migration" "$work_dir/db/migrations/$migration"
done < "$work_dir/db/migrations/manifest.txt"
cp "$work_dir/docker-compose.yml" "$work_dir/update.sh" "$work_dir/rollback.sh" \
  "$work_dir/backup-postgres.sh" "$work_dir/restore-rehearsal.sh" "$work_dir/apply-runtime-settings.sh" \
  "$work_dir/host-nginx.example.conf" "$work_dir/.env.example" "$install_dir/"
mkdir -p "$install_dir/db/init" "$install_dir/db/migrations"
cp "$work_dir/db/init/001_baseline.sql" "$install_dir/db/init/001_baseline.sql"
cp "$work_dir/db/migrations/manifest.txt" "$install_dir/db/migrations/manifest.txt"
cp "$work_dir/db/migrations/"*.sql "$install_dir/db/migrations/" 2>/dev/null || true
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
chmod +x "$install_dir/update.sh" "$install_dir/rollback.sh" "$install_dir/backup-postgres.sh" "$install_dir/restore-rehearsal.sh" "$install_dir/apply-runtime-settings.sh"

(
  cd "$install_dir"
  docker compose --env-file .env -f docker-compose.yml pull
  docker compose --env-file .env -f docker-compose.yml up -d
)

printf '\nInstalled. Create the root administrator at http://127.0.0.1:18082\n'
