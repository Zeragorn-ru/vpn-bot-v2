#!/usr/bin/env bash
set -euo pipefail
umask 077

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_dir=${1:-/opt/vpn-bot-v2}
case "$install_dir" in
  /*) ;;
  *) printf 'installation directory must be absolute\n' >&2; exit 64 ;;
esac

if [ -e "$install_dir/.env" ]; then
  printf 'refusing to overwrite existing %s/.env\n' "$install_dir" >&2
  exit 73
fi

if [ "$install_dir" != "$source_dir" ]; then
  if [ ! -d "$source_dir/deploy" ]; then
    printf 'source checkout is incomplete: %s/deploy\n' "$source_dir" >&2
    exit 66
  fi
  mkdir -p "$install_dir"
  cp -R "$source_dir/deploy" "$install_dir/"
fi

install -d -m 700 "$install_dir/data/postgres" "$install_dir/data/redis" \
  "$install_dir/data/backups" "$install_dir/data/bootstrap"
postgres_password=$(openssl rand -hex 32)
encryption_key=$(openssl rand -base64 32 | tr -d '\n')
setup_token=$(openssl rand -base64 48 | tr -d '\n' | tr '/+' '_-')
printf 'POSTGRES_PASSWORD=%s\nAPPLICATION_ENCRYPTION_KEY=%s\n' \
  "$postgres_password" "$encryption_key" > "$install_dir/.env"
printf '%s\n' "$setup_token" > "$install_dir/data/bootstrap/setup-token"
chmod 600 "$install_dir/.env"

if ! chown 65532:65532 "$install_dir/data/bootstrap" \
  "$install_dir/data/bootstrap/setup-token"; then
  rm -f "$install_dir/data/bootstrap/setup-token"
  printf 'cannot protect bootstrap token for API runtime user; installation aborted\n' >&2
  exit 77
fi
chmod 700 "$install_dir/data/bootstrap"
chmod 600 "$install_dir/data/bootstrap/setup-token"

printf 'Bootstrap token (displayed once): %s\n' "$setup_token"
printf 'Runtime initialized at %s. Deploy a full SHA release through CI/CD.\n' "$install_dir"
