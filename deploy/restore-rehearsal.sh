#!/usr/bin/env sh
set -eu

backup_file=${1:?usage: restore-rehearsal.sh <postgres-custom-format-backup>}
test -s "$backup_file"

rehearsal_db="vpn_bot_restore_$(date -u +%Y%m%d%H%M%S)"
env_file=${ENV_FILE:-.env}
cleanup() {
  docker compose --env-file "$env_file" -f docker-compose.yml exec -T postgres \
    psql -U vpn_bot -d postgres -v ON_ERROR_STOP=1 -c "DROP DATABASE IF EXISTS $rehearsal_db" >/dev/null
}
trap cleanup EXIT INT TERM

docker compose --env-file "$env_file" -f docker-compose.yml exec -T postgres \
  psql -U vpn_bot -d postgres -v ON_ERROR_STOP=1 -c "CREATE DATABASE $rehearsal_db" >/dev/null
docker compose --env-file "$env_file" -f docker-compose.yml exec -T postgres \
  pg_restore -U vpn_bot -d "$rehearsal_db" --no-owner --no-privileges < "$backup_file"
docker compose --env-file "$env_file" -f docker-compose.yml exec -T postgres \
  psql -U vpn_bot -d "$rehearsal_db" -v ON_ERROR_STOP=1 -At -c "SELECT count(*) FROM information_schema.tables WHERE table_schema = 'public'" \
  | grep -Eq '^[1-9][0-9]*$'

printf '%s\n' "restore rehearsal passed: $rehearsal_db"
