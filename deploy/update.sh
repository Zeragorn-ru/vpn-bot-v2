#!/usr/bin/env bash
set -eu

if [ -n "${VPN_BOT_IMAGE_TAG:-}" ]; then
  case "$VPN_BOT_IMAGE_TAG" in
    *[!A-Za-z0-9._-]* | "")
      echo "release image tag contains unsupported characters" >&2
      exit 64
      ;;
  esac
  image_for_tag() {
    image=$1
    printf '%s:%s' "${image%:*}" "$VPN_BOT_IMAGE_TAG"
  }
  VPN_API_IMAGE=$(image_for_tag "$(grep '^VPN_API_IMAGE=' .env | cut -d= -f2-)")
  VPN_TELEGRAM_BOT_IMAGE=$(image_for_tag "$(grep '^VPN_TELEGRAM_BOT_IMAGE=' .env | cut -d= -f2-)")
  VPN_BILLING_WORKER_IMAGE=$(image_for_tag "$(grep '^VPN_BILLING_WORKER_IMAGE=' .env | cut -d= -f2-)")
  VPN_PROVISIONING_WORKER_IMAGE=$(image_for_tag "$(grep '^VPN_PROVISIONING_WORKER_IMAGE=' .env | cut -d= -f2-)")
  VPN_NOTIFICATION_WORKER_IMAGE=$(image_for_tag "$(grep '^VPN_NOTIFICATION_WORKER_IMAGE=' .env | cut -d= -f2-)")
  VPN_ADMIN_WEB_IMAGE=$(image_for_tag "$(grep '^VPN_ADMIN_WEB_IMAGE=' .env | cut -d= -f2-)")
  VPN_MINI_APP_WEB_IMAGE=$(image_for_tag "$(grep '^VPN_MINI_APP_WEB_IMAGE=' .env | cut -d= -f2-)")
  export VPN_API_IMAGE VPN_TELEGRAM_BOT_IMAGE VPN_BILLING_WORKER_IMAGE VPN_PROVISIONING_WORKER_IMAGE VPN_NOTIFICATION_WORKER_IMAGE VPN_ADMIN_WEB_IMAGE VPN_MINI_APP_WEB_IMAGE
fi

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
