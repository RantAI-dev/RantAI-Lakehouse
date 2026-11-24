#!/usr/bin/env bash
# OPS-18: bounded retry for the pinned security-tool binaries downloaded by
# the security workflows. A GitHub release-asset outage on 2026-11-08 (16:02
# to 17:35 WIB) failed `cargo-deny`, `gitleaks` and `history-scan` before any
# of them scanned anything: the `curl | tar` one-liners had no retry, and a
# truncated stream made `tar` fail with a misleading error.
#
# Only the DOWNLOAD is retried. curl writes to a sibling temporary file, so a
# failed attempt can never publish a partial archive at the destination, and
# extraction runs exactly once against a complete file. The temporary file is
# removed on every exit path. Each attempt has its own connect and total
# timeout, so a hung asset cannot hold a job past its own timeout-minutes;
# the last failed attempt is not followed by a sleep nothing waits for.
#
# Usage: retry_download.sh <url> <output-path>
# Env (overridable by the self-test, not by the workflows):
#   RETRY_DOWNLOAD_ATTEMPTS      number of attempts (default 4)
#   RETRY_DOWNLOAD_BACKOFF_BASE  backoff seconds per attempt number (default 10)
set -euo pipefail

url="${1:?usage: retry_download.sh <url> <output-path>}"
output="${2:?usage: retry_download.sh <url> <output-path>}"
max_attempts="${RETRY_DOWNLOAD_ATTEMPTS:-4}"
backoff_base="${RETRY_DOWNLOAD_BACKOFF_BASE:-10}"

tmp="$(mktemp "${output}.tmp.XXXXXX")"
trap 'rm -f "$tmp"' EXIT

attempt=1
while :; do
  # `--output` rather than stdout: a retry overwrites the same temp file and
  # the destination is replaced only by a complete download (`mv` is atomic
  # within the filesystem). --fail turns an HTTP error into a retriable exit.
  if curl --fail --silent --show-error --location \
       --connect-timeout 15 --max-time 120 \
       --output "$tmp" "$url"; then
    mv -f "$tmp" "$output"
    exit 0
  fi

  printf 'download of %s failed (attempt %s of %s)\n' "$url" "$attempt" "$max_attempts" >&2
  if [ "$attempt" -ge "$max_attempts" ]; then
    exit 1
  fi
  sleep $((attempt * backoff_base))
  attempt=$((attempt + 1))
done
