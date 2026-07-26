#!/usr/bin/env bash
set -eu

for migration in db/migrations/*.sql; do
  [ -f "$migration" ] || continue
  docker compose --env-file .env -f docker-compose.yml exec -T postgres \
    psql -U vpn_bot -d vpn_bot -v ON_ERROR_STOP=1 < "$migration"
done
./apply-runtime-settings.sh
docker compose --env-file .env -f docker-compose.yml pull
docker compose --env-file .env -f docker-compose.yml up -d --remove-orphans
docker compose --env-file .env -f docker-compose.yml exec -T api curl --fail http://127.0.0.1:8080/readyz
docker compose --env-file .env -f docker-compose.yml ps
