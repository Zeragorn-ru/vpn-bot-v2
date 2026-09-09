#!/usr/bin/env bash
set -euo pipefail

runtime_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$runtime_dir"

release_sha=${VPN_BOT_RELEASE:-}
if [ -z "$release_sha" ] && [ -f data/previous-release ]; then
  release_sha=$(<data/previous-release)
fi
if [[ ! $release_sha =~ ^[0-9a-f]{40}$ ]]; then
  printf '%s\n' 'Rollback requires VPN_BOT_RELEASE or data/previous-release containing a full lowercase Git SHA.' >&2
  exit 64
fi

touch data/pull-release-hold
printf '%s\n' 'Automatic pull updates are paused until data/pull-release-hold is removed.'
VPN_BOT_RELEASE=$release_sha exec ./deploy/update.sh
