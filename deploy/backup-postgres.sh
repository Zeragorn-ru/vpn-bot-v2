#!/usr/bin/env sh
set -eu

backup_dir=${POSTGRES_BACKUP_DIR:-./backups}
timestamp=$(date -u +%Y%m%dT%H%M%SZ)
backup_file="$backup_dir/vpn_bot_$timestamp.dump"
env_file=${ENV_FILE:-.env}

mkdir -p "$backup_dir"
docker compose --env-file "$env_file" -f docker-compose.yml exec -T postgres \
  pg_dump -U vpn_bot -Fc vpn_bot > "$backup_file"

test -s "$backup_file"
printf '%s\n' "$backup_file"
