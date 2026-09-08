#!/usr/bin/env bash
set -euo pipefail
install_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
backup_dir="$install_dir/data/backups"
mkdir -p "$backup_dir"
output="$backup_dir/postgres-$(date -u +%Y%m%dT%H%M%SZ).dump"
cd "$install_dir"
docker compose --env-file .env -f deploy/docker-compose.yml exec -T postgres \
  pg_dump --format=custom --no-owner --no-privileges --username=vpn_bot --dbname=vpn_bot > "$output"
sha256sum "$output" > "$output.sha256"
printf '%s\n' "$output"
