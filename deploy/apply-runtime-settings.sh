#!/usr/bin/env sh
set -eu

env_file=${ENV_FILE:-.env}
compose="docker compose --env-file $env_file -f docker-compose.yml"

settings=$($compose exec -T postgres psql -U vpn_bot -d vpn_bot -At -F '|' -c "
  SELECT concat_ws('|',
    value->>'api_host_port',
    value->>'mini_app_host_port',
    value->>'admin_host_port',
    value->>'telegram_webhook_host_port'
  ) FROM app_settings WHERE key = 'runtime_settings'")
[ -n "$settings" ] || exit 0

IFS='|' read -r api_port mini_app_port admin_port webhook_port <<EOF
$settings
EOF
for port in "$api_port" "$mini_app_port" "$admin_port" "$webhook_port"; do
  case "$port" in
    ''|*[!0-9]*) exit 1 ;;
  esac
done

tmp_file=$(mktemp)
trap 'rm -f "$tmp_file"' EXIT INT TERM
while IFS= read -r line || [ -n "$line" ]; do
  case "$line" in
    API_HOST_PORT=*|MINI_APP_WEB_HOST_PORT=*|ADMIN_WEB_HOST_PORT=*|TELEGRAM_WEBHOOK_HOST_PORT=*) ;;
    *) printf '%s\n' "$line" >> "$tmp_file" ;;
  esac
done < "$env_file"
printf '%s\n' \
  "API_HOST_PORT=$api_port" \
  "MINI_APP_WEB_HOST_PORT=$mini_app_port" \
  "ADMIN_WEB_HOST_PORT=$admin_port" \
  "TELEGRAM_WEBHOOK_HOST_PORT=$webhook_port" >> "$tmp_file"
mv "$tmp_file" "$env_file"
