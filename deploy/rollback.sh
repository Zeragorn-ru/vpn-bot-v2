#!/usr/bin/env sh
set -eu

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <previous-image-sha>" >&2
  exit 64
fi

previous_sha=$1
case "$previous_sha" in
  *[!A-Za-z0-9._-]* | "")
    echo "release SHA contains unsupported characters" >&2
    exit 64
    ;;
esac

image_for_sha() {
  image=$1
  printf '%s:%s' "${image%:*}" "$previous_sha"
}

VPN_API_IMAGE=$(image_for_sha "$(grep '^VPN_API_IMAGE=' .env | cut -d= -f2-)")
VPN_TELEGRAM_BOT_IMAGE=$(image_for_sha "$(grep '^VPN_TELEGRAM_BOT_IMAGE=' .env | cut -d= -f2-)")
VPN_BILLING_WORKER_IMAGE=$(image_for_sha "$(grep '^VPN_BILLING_WORKER_IMAGE=' .env | cut -d= -f2-)")
VPN_PROVISIONING_WORKER_IMAGE=$(image_for_sha "$(grep '^VPN_PROVISIONING_WORKER_IMAGE=' .env | cut -d= -f2-)")
VPN_NOTIFICATION_WORKER_IMAGE=$(image_for_sha "$(grep '^VPN_NOTIFICATION_WORKER_IMAGE=' .env | cut -d= -f2-)")
VPN_ADMIN_WEB_IMAGE=$(image_for_sha "$(grep '^VPN_ADMIN_WEB_IMAGE=' .env | cut -d= -f2-)")
VPN_MINI_APP_WEB_IMAGE=$(image_for_sha "$(grep '^VPN_MINI_APP_WEB_IMAGE=' .env | cut -d= -f2-)")
export VPN_API_IMAGE VPN_TELEGRAM_BOT_IMAGE VPN_BILLING_WORKER_IMAGE VPN_PROVISIONING_WORKER_IMAGE VPN_NOTIFICATION_WORKER_IMAGE VPN_ADMIN_WEB_IMAGE VPN_MINI_APP_WEB_IMAGE

docker compose --env-file .env -f docker-compose.yml pull
docker compose --env-file .env -f docker-compose.yml up -d
docker compose --env-file .env -f docker-compose.yml exec -T api curl --fail http://127.0.0.1:8080/readyz
docker compose --env-file .env -f docker-compose.yml ps
