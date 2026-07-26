#!/usr/bin/env sh
set -eu

runtime_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository=${VPN_BOT_REPOSITORY:-Zeragorn-ru/vpn-bot-v2}
branch=${VPN_BOT_BRANCH:-main}
api_base="https://api.github.com/repos/$repository/contents/deploy"
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

for path in docker-compose.yml update.sh rollback.sh backup-postgres.sh restore-rehearsal.sh apply-runtime-settings.sh auto-update.sh host-nginx.example.conf .env.example; do
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
  "$work_dir/auto-update.sh" "$work_dir/host-nginx.example.conf" "$work_dir/.env.example" "$runtime_dir/"
mkdir -p "$runtime_dir/db/init" "$runtime_dir/db/migrations"
cp "$work_dir/db/init/001_baseline.sql" "$runtime_dir/db/init/001_baseline.sql"
cp "$work_dir/db/migrations/manifest.txt" "$runtime_dir/db/migrations/manifest.txt"
cp "$work_dir/db/migrations/"*.sql "$runtime_dir/db/migrations/" 2>/dev/null || true
chmod +x "$runtime_dir/update.sh" "$runtime_dir/rollback.sh" "$runtime_dir/backup-postgres.sh" "$runtime_dir/restore-rehearsal.sh" "$runtime_dir/apply-runtime-settings.sh" "$runtime_dir/auto-update.sh"
exec "$runtime_dir/update.sh"
