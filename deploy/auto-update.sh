#!/usr/bin/env sh
set -eu

runtime_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repository=${VPN_BOT_REPOSITORY:-https://github.com/Zeragorn-ru/vpn-bot-v2.git}
work_dir=$(mktemp -d)
cleanup() { rm -rf "$work_dir"; }
trap cleanup EXIT INT TERM

git clone --depth 1 "$repository" "$work_dir/source"
for path in docker-compose.yml update.sh rollback.sh backup-postgres.sh restore-rehearsal.sh apply-runtime-settings.sh host-nginx.example.conf .env.example; do
  cp "$work_dir/source/deploy/$path" "$runtime_dir/$path"
done
mkdir -p "$runtime_dir/db/init" "$runtime_dir/db/migrations"
cp "$work_dir/source/deploy/db/init/001_baseline.sql" "$runtime_dir/db/init/001_baseline.sql"
cp "$work_dir/source/deploy/db/migrations/"*.sql "$runtime_dir/db/migrations/" 2>/dev/null || true
chmod +x "$runtime_dir/update.sh" "$runtime_dir/rollback.sh" "$runtime_dir/backup-postgres.sh" "$runtime_dir/restore-rehearsal.sh" "$runtime_dir/apply-runtime-settings.sh"
exec "$runtime_dir/update.sh"
