#!/usr/bin/env bash
# Tests for scripts/ci/retry_download.sh (OPS-18). curl and sleep are stubbed
# on PATH, so success-on-retry, the bounded-failure exit code and the absence
# of a sleep after the final attempt are proved without network or real
# waiting. The stubs also record the arguments, which checks that the
# bounded per-attempt timeouts are actually passed to curl.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HELPER="${SCRIPT_DIR}/../retry_download.sh"

RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m'

PASS=0
FAIL=0

log_pass() {
  printf '%b\n' "${GREEN}PASS${NC} $1"
  PASS=$((PASS + 1))
}

log_fail() {
  printf '%b\n' "${RED}FAIL${NC} $1"
  FAIL=$((FAIL + 1))
}

if [ ! -f "$HELPER" ]; then
  printf '%s\n' "Helper not found at $HELPER" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

STUBS="${TMP}/stubs"
mkdir -p "$STUBS"
OUTPUT="${TMP}/tool.tar.gz"
STATE="${TMP}/calls"
CURL_LOG="${TMP}/curl.log"
SLEEP_LOG="${TMP}/sleep.log"

# Stub curl: fail the first FAKE_CURL_FAILS calls (after writing a truncated
# file, as a broken download would), then succeed with a complete file.
cat > "${STUBS}/curl" <<'EOF'
#!/usr/bin/env bash
args="$*"
out=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2 ;;
    --connect-timeout|--max-time) shift 2 ;;
    -*) shift ;;
    *) shift ;;
  esac
done
n=0
if [ -f "$FAKE_CURL_STATE" ]; then n="$(cat "$FAKE_CURL_STATE")"; fi
n=$((n + 1))
printf '%s' "$n" > "$FAKE_CURL_STATE"
printf '%s\n' "$args" >> "$FAKE_CURL_LOG"
if [ "$n" -le "$FAKE_CURL_FAILS" ]; then
  printf 'partial' > "$out"
  exit 22
fi
printf 'complete-archive' > "$out"
EOF

# Stub sleep: record the requested backoff, do not wait.
cat > "${STUBS}/sleep" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$FAKE_SLEEP_LOG"
EOF
chmod +x "${STUBS}/curl" "${STUBS}/sleep"

# run_case <label> <failures-before-success> <want-rc> <want-curl-calls>
#          <want-sleep-calls> [backoff-base]
run_case() {
  local label="$1" fails="$2" want_rc="$3" want_calls="$4" want_sleeps="$5"
  local backoff="${6:-0}"
  rm -f "$STATE" "$CURL_LOG" "$SLEEP_LOG" "$OUTPUT"

  local rc=0
  PATH="${STUBS}:${PATH}" \
    FAKE_CURL_STATE="$STATE" FAKE_CURL_LOG="$CURL_LOG" \
    FAKE_SLEEP_LOG="$SLEEP_LOG" FAKE_CURL_FAILS="$fails" \
    RETRY_DOWNLOAD_BACKOFF_BASE="$backoff" \
    bash "$HELPER" "https://example.invalid/tool.tar.gz" "$OUTPUT" \
    > "${TMP}/stdout" 2> "${TMP}/stderr" || rc=$?

  if [ "$rc" != "$want_rc" ]; then
    log_fail "$label: exit code $rc, wanted $want_rc"
    return
  fi
  local calls=0 sleeps=0
  [ -f "$CURL_LOG" ] && calls="$(wc -l < "$CURL_LOG" | tr -d ' ')"
  [ -f "$SLEEP_LOG" ] && sleeps="$(wc -l < "$SLEEP_LOG" | tr -d ' ')"
  if [ "$calls" != "$want_calls" ]; then
    log_fail "$label: $calls curl calls, wanted $want_calls"
    return
  fi
  if [ "$sleeps" != "$want_sleeps" ]; then
    log_fail "$label: $sleeps sleeps, wanted $want_sleeps"
    return
  fi

  local leftovers
  leftovers="$(find "$TMP" -maxdepth 1 -name 'tool.tar.gz.tmp.*' -print)"
  if [ -n "$leftovers" ]; then
    log_fail "$label: temporary file left behind: $leftovers"
    return
  fi
  log_pass "$label"
}

run_case "succeeds on the first attempt without sleeping" 0 0 1 0
run_case "succeeds on the third attempt after two failures" 2 0 3 2
run_case "gives up after four failed attempts with no trailing sleep" 4 1 4 3

# A failed run must not publish a partial archive at the destination.
if [ -e "$OUTPUT" ]; then
  log_fail "a bounded failure left a file at the destination"
else
  log_pass "a bounded failure leaves no file at the destination"
fi

# A successful run publishes the complete download, not the partial one.
rm -f "$STATE" "$CURL_LOG" "$SLEEP_LOG" "$OUTPUT"
PATH="${STUBS}:${PATH}" \
  FAKE_CURL_STATE="$STATE" FAKE_CURL_LOG="$CURL_LOG" \
  FAKE_SLEEP_LOG="$SLEEP_LOG" FAKE_CURL_FAILS=2 \
  RETRY_DOWNLOAD_BACKOFF_BASE=0 \
  bash "$HELPER" "https://example.invalid/tool.tar.gz" "$OUTPUT" >/dev/null 2>&1
if [ "$(cat "$OUTPUT")" = "complete-archive" ]; then
  log_pass "a retry publishes only the complete download"
else
  log_fail "a retry published a partial download"
fi

# Backoff grows with the attempt number, and the real per-attempt curl
# timeouts are present. These run with the stubbed sleep, so they are instant.
rm -f "$STATE" "$CURL_LOG" "$SLEEP_LOG" "$OUTPUT"
PATH="${STUBS}:${PATH}" \
  FAKE_CURL_STATE="$STATE" FAKE_CURL_LOG="$CURL_LOG" \
  FAKE_SLEEP_LOG="$SLEEP_LOG" FAKE_CURL_FAILS=4 \
  RETRY_DOWNLOAD_BACKOFF_BASE=10 \
  bash "$HELPER" "https://example.invalid/tool.tar.gz" "$OUTPUT" >/dev/null 2>&1 || true
if [ "$(tr '\n' ' ' < "$SLEEP_LOG" | sed 's/ $//')" = "10 20 30" ]; then
  log_pass "backoff is 10, 20, 30 with no sleep after the last attempt"
else
  log_fail "backoff sequence was $(tr '\n' ' ' < "$SLEEP_LOG")"
fi
first_call="$(head -1 "$CURL_LOG")"
if printf '%s' "$first_call" | grep -q -- '--connect-timeout 15' \
   && printf '%s' "$first_call" | grep -q -- '--max-time 120' \
   && printf '%s' "$first_call" | grep -q -- '--fail' \
   && printf '%s' "$first_call" | grep -q -- '--location'; then
  log_pass "curl is called with --fail, --location and bounded timeouts"
else
  log_fail "curl call is missing a bounded-timeout flag: $first_call"
fi

# The attempt count is overridable too (the workflows use the default of 4).
rm -f "$STATE" "$CURL_LOG" "$SLEEP_LOG" "$OUTPUT"
PATH="${STUBS}:${PATH}" \
  FAKE_CURL_STATE="$STATE" FAKE_CURL_LOG="$CURL_LOG" \
  FAKE_SLEEP_LOG="$SLEEP_LOG" FAKE_CURL_FAILS=9 \
  RETRY_DOWNLOAD_ATTEMPTS=2 RETRY_DOWNLOAD_BACKOFF_BASE=0 \
  bash "$HELPER" "https://example.invalid/tool.tar.gz" "$OUTPUT" >/dev/null 2>&1 || true
if [ "$(wc -l < "$CURL_LOG" | tr -d ' ')" = "2" ]; then
  log_pass "RETRY_DOWNLOAD_ATTEMPTS overrides the attempt count"
else
  log_fail "RETRY_DOWNLOAD_ATTEMPTS was ignored"
fi

printf '\n%d passed, %d failed\n' "$PASS" "$FAIL"
if [ "$FAIL" -ne 0 ]; then
  exit 1
fi
