#!/usr/bin/env bash
set -euo pipefail

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
pull_script="$repo_root/deploy/pull-release.sh"
test_root=$(mktemp -d)
cleanup() {
  rm -rf -- "$test_root"
}
trap cleanup EXIT

fail() {
  printf 'pull-release test failed: %s\n' "$1" >&2
  exit 1
}

make_runtime() {
  local runtime_dir=$1
  install -d -m 700 "$runtime_dir/data"
  install -d -m 755 "$runtime_dir/deploy"
  printf '%s\n' 'POSTGRES_PASSWORD=test' 'APPLICATION_ENCRYPTION_KEY=test' > "$runtime_dir/.env"
  chmod 600 "$runtime_dir/.env"
  printf '%s\n' '#!/usr/bin/env bash' 'exit 0' > "$runtime_dir/deploy/update.sh"
  chmod 750 "$runtime_dir/deploy/update.sh"
  printf '%s\n' 'name: test' > "$runtime_dir/deploy/docker-compose.yml"
}

fake_bin="$test_root/bin"
install -d -m 700 "$fake_bin"
cat > "$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

output=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output)
      output=$2
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

test -n "$output"
case "${TEST_CURL_URL:-}" in
  metadata)
    cp "$TEST_RELEASE_METADATA" "$output"
    ;;
  archive)
    cp "$TEST_RELEASE_ARCHIVE" "$output"
    ;;
  *)
    printf '%s\n' 'unexpected curl invocation' >&2
    exit 1
    ;;
esac
EOF
chmod 700 "$fake_bin/curl"

if VECTOR_RUNTIME_DIR=relative "$pull_script" >/dev/null 2>&1; then
  fail 'relative runtime path was accepted'
fi

hold_runtime="$test_root/hold-runtime"
make_runtime "$hold_runtime"
touch "$hold_runtime/data/pull-release-hold"
if ! PATH="$fake_bin:$PATH" VECTOR_RUNTIME_DIR="$hold_runtime" "$pull_script" | grep -Fx 'pull release skipped: hold marker exists' >/dev/null; then
  fail 'hold marker did not skip the release poll'
fi

release_sha=0123456789abcdef0123456789abcdef01234567
source_root="$test_root/source"
install -d -m 755 "$source_root/deploy"
printf '%s\n' 'name: test' > "$source_root/deploy/docker-compose.yml"
cat > "$source_root/deploy/update.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
runtime_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
printf '%s\n' "$VPN_BOT_RELEASE" > "$runtime_dir/data/release"
printf '%s\n' "$VPN_BOT_RELEASE" > "$runtime_dir/data/update-ran"
EOF
chmod 750 "$source_root/deploy/update.sh"
release_archive="$test_root/release.tar.gz"
tar --create --gzip --file "$release_archive" -C "$test_root" source
release_metadata="$test_root/release.json"
printf '[{"draft":false,"prerelease":true,"target_commitish":"%s","tag_name":"test-%s"}]\n' \
  "$release_sha" "$release_sha" > "$release_metadata"

valid_runtime="$test_root/valid-runtime"
make_runtime "$valid_runtime"

# The updater performs metadata and archive requests, so dispatch the fake responses by URL.
cat > "$fake_bin/curl" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

output=''
previous=''
for argument in "$@"; do
  if [ "$previous" = '--output' ]; then
    output=$argument
    break
  fi
  previous=$argument
done
case "${!#}" in
  *'/releases?per_page=100') cp "$TEST_RELEASE_METADATA" "$output" ;;
  *'/tarball/'*) cp "$TEST_RELEASE_ARCHIVE" "$output" ;;
  *) exit 1 ;;
esac
EOF
chmod 700 "$fake_bin/curl"
PATH="$fake_bin:$PATH" TEST_RELEASE_METADATA="$release_metadata" TEST_RELEASE_ARCHIVE="$release_archive" \
  VECTOR_RUNTIME_DIR="$valid_runtime" "$pull_script" >/dev/null

test "$(<"$valid_runtime/data/release")" = "$release_sha" || fail 'valid release did not update the marker'
test "$(<"$valid_runtime/data/update-ran")" = "$release_sha" || fail 'valid release did not run the updater'

mismatched_metadata="$test_root/mismatched.json"
printf '[{"draft":false,"prerelease":true,"target_commitish":"%s","tag_name":"test-deadbeef"}]\n' \
  "$release_sha" > "$mismatched_metadata"
mismatch_runtime="$test_root/mismatch-runtime"
make_runtime "$mismatch_runtime"
if PATH="$fake_bin:$PATH" TEST_RELEASE_METADATA="$mismatched_metadata" TEST_RELEASE_ARCHIVE="$release_archive" \
  VECTOR_RUNTIME_DIR="$mismatch_runtime" "$pull_script" >/dev/null 2>&1; then
  fail 'mismatched release descriptor was accepted'
fi

echo 'pull-release tests passed'
