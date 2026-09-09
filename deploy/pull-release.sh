#!/usr/bin/env bash
set -euo pipefail
umask 077

runtime_dir=${VECTOR_RUNTIME_DIR:-/opt/vector}
repository=${VECTOR_RELEASE_REPOSITORY:-Zeragorn-ru/vpn-bot-v2}
release_channel=${VECTOR_RELEASE_CHANNEL:-test}

fail() {
  printf 'pull release failed: %s\n' "$1" >&2
  exit 1
}

case "$runtime_dir" in
  /*) ;;
  *) fail 'VECTOR_RUNTIME_DIR must be an absolute path' ;;
esac
[[ "$repository" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]] \
  || fail 'VECTOR_RELEASE_REPOSITORY must be an owner/name value'
[[ "$release_channel" =~ ^[A-Za-z0-9][A-Za-z0-9_-]*$ ]] \
  || fail 'VECTOR_RELEASE_CHANNEL contains invalid characters'

test -f "$runtime_dir/.env" || fail 'missing runtime .env'
test -d "$runtime_dir/data" || fail 'missing runtime data directory'

exec 9>"$runtime_dir/data/pull-release.lock"
flock -n 9 || {
  printf '%s\n' 'pull release skipped: another update is active'
  exit 0
}

if [ -e "$runtime_dir/data/pull-release-hold" ]; then
  printf '%s\n' 'pull release skipped: hold marker exists'
  exit 0
fi

test -x "$runtime_dir/deploy/update.sh" || fail 'missing managed deployment update script'

current_release=''
if [ -e "$runtime_dir/data/release" ]; then
  current_release=$(<"$runtime_dir/data/release")
  [[ "$current_release" =~ ^[0-9a-f]{40}$ ]] || fail 'current release marker is malformed'
fi

metadata=$(mktemp "$runtime_dir/.pull-release-metadata.XXXXXX")
archive=$(mktemp "$runtime_dir/.pull-release-archive.XXXXXX")
staging=$(mktemp -d "$runtime_dir/.pull-release-stage.XXXXXX")
backup=$(mktemp -d "$runtime_dir/.pull-release-backup.XXXXXX")
cleanup() {
  rm -f -- "$metadata" "$archive"
  rm -rf -- "$staging" "$backup"
}
trap cleanup EXIT

api_url="https://api.github.com/repos/$repository/releases?per_page=100"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --retry 3 --retry-all-errors --connect-timeout 15 --max-time 120 \
  --header 'Accept: application/vnd.github+json' \
  --header 'User-Agent: vector-release-puller' \
  --output "$metadata" "$api_url" \
  || fail 'could not fetch release metadata'

release_sha=$(python3 - "$metadata" "$release_channel" <<'PY'
import json
import re
import sys

metadata_path, channel = sys.argv[1:]
with open(metadata_path, encoding="utf-8") as source:
    releases = json.load(source)

if not isinstance(releases, list):
    raise SystemExit(0)

for release in releases:
    if release.get("draft") or not release.get("prerelease"):
        continue
    target = release.get("target_commitish")
    tag = release.get("tag_name")
    if not isinstance(target, str) or not isinstance(tag, str):
        continue
    if re.fullmatch(r"[0-9a-f]{40}", target) and tag == f"{channel}-{target}":
        print(target)
        break
PY
)
[[ "$release_sha" =~ ^[0-9a-f]{40}$ ]] \
  || fail 'no valid immutable release descriptor was found'

if [ "$release_sha" = "$current_release" ]; then
  printf 'pull release skipped: %s is already verified\n' "$release_sha"
  exit 0
fi

archive_url="https://api.github.com/repos/$repository/tarball/$release_sha"
curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
  --retry 3 --retry-all-errors --connect-timeout 15 --max-time 120 \
  --header 'Accept: application/vnd.github+json' \
  --header 'User-Agent: vector-release-puller' \
  --output "$archive" "$archive_url" \
  || fail 'could not fetch release source archive'

tar --extract --file "$archive" --strip-components=1 --no-same-owner -C "$staging" \
  || fail 'could not extract release source archive'
test -f "$staging/deploy/docker-compose.yml" \
  || fail 'release source archive is missing deployment compose configuration'
test -x "$staging/deploy/update.sh" \
  || fail 'release source archive is missing deployment update script'

mv "$runtime_dir/deploy" "$backup/deploy" \
  || fail 'could not stage the previous managed deployment'
if ! mv "$staging/deploy" "$runtime_dir/deploy"; then
  mv "$backup/deploy" "$runtime_dir/deploy" \
    || fail 'could not restore the previous managed deployment after staging failure'
  fail 'could not stage the new managed deployment'
fi

if VPN_BOT_RELEASE="$release_sha" "$runtime_dir/deploy/update.sh"; then
  install -d -m 700 "$runtime_dir/data/deployment-backups"
  tar --create --gzip --file "$runtime_dir/data/deployment-backups/deploy-before-$release_sha.tar.gz" \
    -C "$backup" deploy
  printf 'pull release %s verified\n' "$release_sha"
  exit 0
fi

failed_deploy=$(mktemp -d "$runtime_dir/data/.failed-deploy.XXXXXX")
mv "$runtime_dir/deploy" "$failed_deploy/deploy"
mv "$backup/deploy" "$runtime_dir/deploy"
if [ -n "$current_release" ]; then
  VPN_BOT_RELEASE="$current_release" "$runtime_dir/deploy/update.sh" \
    || fail 'new release failed and the previous release could not be restored'
fi
fail "release $release_sha did not verify; previous managed deployment was restored"
