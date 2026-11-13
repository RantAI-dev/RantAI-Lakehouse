#!/usr/bin/env bash
# Tests for scripts/ci/worker_prepare.sh (OPS-24 D10).
#
# Everything points into a temporary directory: HOME, WORKER_CACHE_ROOT,
# RUNNER_NAME, GITHUB_ENV and GITHUB_WORKSPACE. The two "over the limit" cases
# need more than the 5 GiB floor, so they allocate a sparse-free file with
# `fallocate` (real blocks, no I/O; `truncate` would make a sparse file that
# `du -sk` does not count) and fall back to `dd` where `fallocate` is missing.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../worker_prepare.sh"

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

if [ ! -f "$SCRIPT" ]; then
  printf '%s\n' "Script not found at $SCRIPT" >&2
  exit 1
fi

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

LOG="$ROOT/prepare.log"

# run_prepare uses CASE_HOME, CASE_ROOT, CASE_NAME, CASE_MAX, CASE_JOBS,
# CASE_ENV and CASE_WS, and returns the script's exit code.
run_prepare() {
  local rc=0
  (
    HOME="$CASE_HOME" \
    WORKER_CACHE_ROOT="$CASE_ROOT" \
    RUNNER_NAME="$CASE_NAME" \
    TARGET_MAX_GB="${CASE_MAX:-}" \
    WORKER_CARGO_JOBS="${CASE_JOBS:-}" \
    GITHUB_ENV="$CASE_ENV" \
    GITHUB_WORKSPACE="$CASE_WS" \
    bash "$SCRIPT" > "$LOG" 2>&1
  ) || rc=$?
  return "$rc"
}

# set_case <name>: a fresh home, root, env file and workspace for one case.
set_case() {
  CASE_HOME="$ROOT/$1/home"
  CASE_ROOT="$ROOT/$1/cache"
  CASE_NAME="runner-a"
  CASE_ENV="$ROOT/$1/github_env"
  CASE_WS="$ROOT/$1/work"
  CASE_MAX=""
  CASE_JOBS=""
  mkdir -p "$CASE_HOME" "$CASE_ROOT" "$CASE_WS"
  : > "$CASE_ENV"
}

make_big_file() {
  local path="$1"
  if fallocate -l 5300M "$path" 2>/dev/null; then
    return 0
  fi
  dd if=/dev/zero of="$path" bs=1M count=5300 status=none 2>/dev/null
}

echo "=== Worker preparation test suite ==="

# 1. A first run creates the directory and exports it.
set_case first
if run_prepare && [ -d "$CASE_ROOT/target/runner-a" ] \
   && grep -qx "CARGO_TARGET_DIR=$CASE_ROOT/target/runner-a" "$CASE_ENV"; then
  log_pass "a first run creates the target directory and writes CARGO_TARGET_DIR"
else
  log_fail "a first run did not create the target directory or did not write CARGO_TARGET_DIR"
fi

# 2. A runner name outside the safe alphabet becomes one name inside the root.
set_case name
CASE_NAME="a/b c"
if run_prepare && [ -d "$CASE_ROOT/target/a_b_c" ] \
   && grep -qx "CARGO_TARGET_DIR=$CASE_ROOT/target/a_b_c" "$CASE_ENV"; then
  log_pass "a runner name with a slash and a space becomes a name inside the root"
else
  log_fail "a runner name with a slash and a space was not mapped inside the root"
fi

# 3. No name, no directory: the cache is per registration.
set_case empty
CASE_NAME=""
rc=0
run_prepare || rc=$?
if [ "$rc" = "1" ]; then
  log_pass "an empty runner name fails"
else
  log_fail "an empty runner name did not fail (exit=$rc)"
fi

# 4. Under the limit nothing is deleted.
set_case under
CASE_MAX=5
mkdir -p "$CASE_ROOT/target/runner-a"
echo old > "$CASE_ROOT/target/runner-a/old.txt"
touch -a -d "10 days ago" "$CASE_ROOT/target/runner-a/old.txt"
echo new > "$CASE_ROOT/target/runner-a/new.txt"
if run_prepare && [ -e "$CASE_ROOT/target/runner-a/old.txt" ] && [ -e "$CASE_ROOT/target/runner-a/new.txt" ]; then
  log_pass "under the limit nothing is deleted"
else
  log_fail "under the limit a file was deleted"
fi

# 5. Over the limit, stage 1 deletes the old files and keeps the new ones.
set_case stage1
CASE_MAX=5
mkdir -p "$CASE_ROOT/target/runner-a"
if make_big_file "$CASE_ROOT/target/runner-a/old-big.bin"; then
  touch -a -d "10 days ago" "$CASE_ROOT/target/runner-a/old-big.bin"
  echo keep > "$CASE_ROOT/target/runner-a/new-small.txt"
  if run_prepare && [ ! -e "$CASE_ROOT/target/runner-a/old-big.bin" ] \
     && [ -e "$CASE_ROOT/target/runner-a/new-small.txt" ]; then
    log_pass "over the limit the old files go and the new stay"
  else
    log_fail "over the limit the old files did not go, or the new file went with them"
  fi
else
  log_fail "could not allocate the 5.3 GiB test file (neither fallocate nor dd worked)"
fi

# 6. Still over the limit: stage 2 empties this runner's directory and leaves
#    a sibling alone.
set_case stage2
CASE_MAX=5
mkdir -p "$CASE_ROOT/target/runner-a" "$CASE_ROOT/target/sibling"
echo keep > "$CASE_ROOT/target/sibling/keep.txt"
if make_big_file "$CASE_ROOT/target/runner-a/new-big.bin"; then
  if run_prepare && [ -d "$CASE_ROOT/target/runner-a" ] \
     && [ -z "$(ls -A "$CASE_ROOT/target/runner-a")" ] \
     && [ -e "$CASE_ROOT/target/sibling/keep.txt" ] \
     && grep -q '::warning::' "$LOG"; then
    log_pass "still over the limit the directory is emptied and a sibling is untouched"
  else
    log_fail "still over the limit the directory was not emptied, or the sibling was touched, or no warning was printed"
  fi
else
  log_fail "could not allocate the 5.3 GiB test file (neither fallocate nor dd worked)"
fi

# 7. A limit that is not a number in range falls back to 40 GiB.
set_case defaultlimit
CASE_MAX=abc
if run_prepare && grep -q 'limit 41943040 KiB (40 GiB)' "$LOG"; then
  log_pass "TARGET_MAX_GB=abc uses 40 GiB"
else
  log_fail "TARGET_MAX_GB=abc did not use 40 GiB"
fi

# 8. CARGO_BUILD_JOBS is exported only for a number in range.
set_case jobs
CASE_JOBS=4
if run_prepare && grep -qx "CARGO_BUILD_JOBS=4" "$CASE_ENV"; then
  log_pass "WORKER_CARGO_JOBS=4 writes CARGO_BUILD_JOBS"
else
  log_fail "WORKER_CARGO_JOBS=4 did not write CARGO_BUILD_JOBS"
fi

set_case jobs_bad
CASE_JOBS=99
if run_prepare && ! grep -q '^CARGO_BUILD_JOBS=' "$CASE_ENV"; then
  log_pass "WORKER_CARGO_JOBS=99 writes no CARGO_BUILD_JOBS"
else
  log_fail "WORKER_CARGO_JOBS=99 wrote CARGO_BUILD_JOBS"
fi

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
