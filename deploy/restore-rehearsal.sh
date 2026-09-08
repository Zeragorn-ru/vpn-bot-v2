#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' 'Restore rehearsal is intentionally explicit: choose a disposable PostgreSQL target before importing a production dump.'
printf '%s\n' 'The backup artifact must be verified with its .sha256 file before restore.'
