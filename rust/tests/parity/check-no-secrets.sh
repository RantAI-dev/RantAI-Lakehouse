#!/usr/bin/env bash
# Fails if anything credential-shaped is present in the parity corpus.
#
# The corpus is captured from a live system and committed to git, so this runs
# before every commit that touches it and in CI. It matches on SHAPE rather
# than on a list of known values — the leak this was written for was a value
# nobody had thought to list.
#
# Shape matching is defence in depth, NOT the primary defence. Fields already
# known to be credentials (embed secret, sample JWT) are redacted by key name
# in capture.ts, because an operator-supplied secret may be uppercase hex,
# base64, or a passphrase and match none of the patterns below.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
status=0

check() {
  local label="$1" pattern="$2"
  local hits
  # Placeholders and length markers are the expected post-sanitization forms.
  hits=$(grep -rEoh "$pattern" "$DIR" 2>/dev/null \
    | grep -vE '^__[A-Z0-9_]+__$' \
    | grep -vE '^<redacted:[0-9]+>$' \
    | sort -u || true)
  if [ -n "$hits" ]; then
    echo "LEAK — $label:"
    printf '  %s\n' "$hits"
    status=1
  fi
}

# Hex secrets. Case-insensitive and without \b, so a key glued to adjacent
# word characters still trips the check.
check "public dashboard token"   'p_[0-9a-fA-F]{32}'
check "64-hex secret"            '[0-9a-fA-F]{64}'
check "32-hex secret"            '[0-9a-fA-F]{32}'

# Signed tokens and provider keys.
check "signed JWT"               'ey[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}'
check "AWS access key"           'AKIA[0-9A-Z]{16}'
check "OpenAI/Anthropic key"     'sk-[A-Za-z0-9_-]{20,}'
check "GitHub token"             'gh[pousr]_[A-Za-z0-9]{20,}'
check "Google API key"           'AIza[0-9A-Za-z_-]{35}'
check "Slack token"              'xox[baprs]-[0-9A-Za-z-]{10,}'
check "PEM private key"          'BEGIN [A-Z ]*PRIVATE KEY'

# Delivery targets. console.alert_rule.target holds a Slack/Discord webhook or
# an email address and is returned by GET /api/alerts. Empty today only because
# every rule is soft-deleted; the first real rule would leak one.
check "webhook URL"              'https?://hooks\.[A-Za-z0-9.-]+/[A-Za-z0-9/_-]{8,}'
check "discord webhook"          'https?://[A-Za-z0-9.-]*discord[A-Za-z0-9.-]*/api/webhooks/[0-9]+/[A-Za-z0-9_-]+'
check "credentials in URL"       '[a-z]+://[^/[:space:]"]+:[^/@[:space:]"]+@'
check "email address"            '[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}'

if [ "$status" -eq 0 ]; then
  echo "corpus clean — no credential-shaped values found"
fi
exit "$status"
