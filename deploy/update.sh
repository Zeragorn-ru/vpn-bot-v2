#!/usr/bin/env bash
set -euo pipefail

runtime_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
release_sha=${VPN_BOT_RELEASE:-}

fail() {
  printf 'release failed: %s\n' "$1" >&2
  exit 1
}

if [[ ! $release_sha =~ ^[0-9a-f]{40}$ ]]; then
  fail 'VPN_BOT_RELEASE must be a full lowercase Git SHA'
fi

cd "$runtime_dir"
test -f .env || fail 'missing runtime .env; run setup.sh once before deploying'
test -f deploy/docker-compose.yml || fail 'missing managed deployment files'

compose=(docker compose --env-file .env -f deploy/docker-compose.yml)
export VPN_BOT_RELEASE=$release_sha
"${compose[@]}" config --quiet
pull_compose=("${compose[@]}" --profile telegram)
"${pull_compose[@]}" pull --policy missing
"${compose[@]}" up -d --no-build postgres redis

for _ in $(seq 1 30); do
  if "${compose[@]}" exec -T postgres pg_isready -U vpn_bot -d vpn_bot >/dev/null 2>&1; then
    break
  fi
  sleep 2
done
"${compose[@]}" exec -T postgres pg_isready -U vpn_bot -d vpn_bot >/dev/null 2>&1 \
  || fail 'PostgreSQL did not become ready'

shopt -s nullglob
migrations=(deploy/db/migrations/*.sql)
for migration in "${migrations[@]}"; do
  "${compose[@]}" exec -T postgres psql -U vpn_bot -d vpn_bot -v ON_ERROR_STOP=1 < "$migration"
done

for migration in "${migrations[@]}"; do
  version=$(basename "${migration%.sql}")
  [[ $version =~ ^[A-Za-z0-9_-]+$ ]] || fail "invalid migration filename: $migration"
  applied=$("${compose[@]}" exec -T postgres psql -U vpn_bot -d vpn_bot -tAc \
    "SELECT EXISTS (SELECT 1 FROM schema_migrations WHERE version = '$version');")
  [ "$applied" = 't' ] || fail "migration ledger does not include $version"
done

telegram_enabled=$("${compose[@]}" exec -T postgres psql -U vpn_bot -d vpn_bot -tAc \
  "SELECT EXISTS (SELECT 1 FROM app_secrets WHERE key = 'TELEGRAM_BOT_TOKEN');")
if [ "$telegram_enabled" = 't' ]; then
  compose+=(--profile telegram)
else
  "${compose[@]}" stop telegram-bot >/dev/null 2>&1 || true
fi

"${compose[@]}" up -d --no-build --remove-orphans

wait_for_http() {
  local url=$1
  for _ in $(seq 1 30); do
    if curl --fail --silent --show-error "$url" >/dev/null; then
      return 0
    fi
    sleep 2
  done
  fail "health check did not pass: $url"
}

wait_for_http http://127.0.0.1:18080/healthz
wait_for_http http://127.0.0.1:18080/readyz
wait_for_http http://127.0.0.1:18082/healthz
wait_for_http http://127.0.0.1:18081/healthz

running=$("${compose[@]}" ps --status running --services)
for service in postgres redis api billing-worker provisioning-worker notification-worker admin-web mini-app-web; do
  printf '%s\n' "$running" | grep -Fx "$service" >/dev/null \
    || fail "expected service is not running: $service"
done
if [ "$telegram_enabled" = 't' ]; then
  printf '%s\n' "$running" | grep -Fx telegram-bot >/dev/null \
    || fail 'Telegram token is configured but telegram-bot is not running'
fi

mkdir -p data
if [ -f data/release ]; then
  cp data/release data/previous-release
fi
release_tmp=$(mktemp data/release.XXXXXX)
printf '%s\n' "$release_sha" > "$release_tmp"
mv "$release_tmp" data/release
printf 'release %s verified%s\n' "$release_sha" \
  "$([ "$telegram_enabled" = 't' ] && printf ' with Telegram polling enabled' || printf ' (Telegram polling disabled: no token configured)')"
