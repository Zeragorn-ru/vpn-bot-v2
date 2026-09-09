#!/usr/bin/env bash
set -euo pipefail
umask 077

source_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
runtime_dir=${1:-/opt/vector}
repository=${VECTOR_RELEASE_REPOSITORY:-Zeragorn-ru/vpn-bot-v2}
release_channel=${VECTOR_RELEASE_CHANNEL:-test}
environment_file=/etc/vector-release-pull.env

fail() {
  printf 'pull updater installation failed: %s\n' "$1" >&2
  exit 1
}

case "$runtime_dir" in
  /opt/vector) ;;
  /*) fail 'the pull updater unit supports only /opt/vector' ;;
  *) fail 'runtime directory must be absolute' ;;
esac
[[ "$repository" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] \
  || fail 'repository must be an owner/name value'
[[ "$release_channel" =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] \
  || fail 'release channel contains invalid characters'
test -f "$runtime_dir/.env" || fail "missing $runtime_dir/.env"
test -d "$runtime_dir/data" || fail "missing $runtime_dir/data"
for required_tool in curl python3 flock systemctl tar; do
  command -v "$required_tool" >/dev/null || fail "missing required tool: $required_tool"
done
test -x "$source_dir/deploy/pull-release.sh" || fail 'missing pull-release.sh'
test -f "$source_dir/deploy/systemd/vector-release-pull.service" \
  || fail 'missing pull service unit'
test -f "$source_dir/deploy/systemd/vector-release-pull.timer" \
  || fail 'missing pull timer unit'

if [ "$source_dir/deploy/pull-release.sh" != "$runtime_dir/deploy/pull-release.sh" ]; then
  install -m 750 "$source_dir/deploy/pull-release.sh" "$runtime_dir/deploy/pull-release.sh"
fi
install -D -m 644 "$source_dir/deploy/systemd/vector-release-pull.service" \
  /etc/systemd/system/vector-release-pull.service
install -D -m 644 "$source_dir/deploy/systemd/vector-release-pull.timer" \
  /etc/systemd/system/vector-release-pull.timer

if [ ! -e "$environment_file" ]; then
  temporary_file=$(mktemp /etc/.vector-release-pull.env.XXXXXX)
  printf 'VECTOR_RUNTIME_DIR=%s\nVECTOR_RELEASE_REPOSITORY=%s\nVECTOR_RELEASE_CHANNEL=%s\n' \
    "$runtime_dir" "$repository" "$release_channel" > "$temporary_file"
  install -m 600 "$temporary_file" "$environment_file"
  rm -f "$temporary_file"
fi

systemctl daemon-reload
systemctl enable --now vector-release-pull.timer
printf 'Pull updater is installed. Trigger once with: systemctl start vector-release-pull.service\n'
