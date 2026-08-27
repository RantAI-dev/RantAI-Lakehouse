#!/usr/bin/env bash
# Fails if anything credential-shaped is present in the parity corpus.
#
# The corpus is captured from a live system and committed to git, so this runs
# before every commit that touches it and in CI. It deliberately matches on
# SHAPE rather than on a list of known secrets — the leak this was written for
# was a value nobody had thought to add to a list.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
status=0

check() {
  local label="$1" pattern="$2"
  local hits
  # Placeholders are the expected post-sanitization form; never flag them.
  hits=$(grep -rEoh "$pattern" "$DIR" 2>/dev/null | grep -vE '^__[A-Z0-9_]+__$' | sort -u || true)
  if [ -n "$hits" ]; then
    echo "LEAK — $label:"
    printf '  %s\n' "$hits"
    status=1
  fi
}

check "public dashboard token"  'p_[0-9a-f]{32}'
check "64-hex secret"           '\b[0-9a-f]{64}\b'
check "signed JWT"              '\bey[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b'
check "AWS-style key"           '\bAKIA[0-9A-Z]{16}\b'
check "bearer/api key"          '\bsk-[A-Za-z0-9]{20,}\b'

if [ "$status" -eq 0 ]; then
  echo "corpus clean — no credential-shaped values found"
fi
exit "$status"
