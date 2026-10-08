#!/usr/bin/env bash
# Tests for scripts/ci/required_gate.sh
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../required_gate.sh"

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

expect() {
  local want="$1"
  local label="$2"
  shift 2
  local got=0

  (
    # Clean subshell environment
    unset EVENT_NAME SCOPE_RUST SCOPE_FRONTEND SCOPE_DAGSTER SCOPE_STACK SCOPE_DOCS_ONLY SCOPE_ALL \
          CHANGES_RESULT REPO_LINTS_RESULT FRONTEND_RESULT RUST_FMT_RESULT RUST_CLIPPY_RESULT \
          RUST_BUILD_RESULT RUST_TEST_RESULT RUST_MSRV_RESULT DAGSTER_TESTS_RESULT \
          G1_RUSTFS_RESULT G2_SEAWEEDFS_RESULT G3A_DAGSTER_RESULT G3_MAINTENANCE_RESULT \
          G4_CDC_RESULT G6_INGEST_RESULT GOLD_EXPORT_RESULT G8_GOVERNANCE_RESULT
    export "$@"
    bash "$SCRIPT" >/dev/null 2>&1
  ) || got=$?

  if [ "$got" != "$want" ]; then
    log_fail "$label — expected exit $want, got $got"
  else
    log_pass "$label"
  fi
}

echo "=== Required Gate test suite ==="

# Base success environments
base_docs_green=(
  EVENT_NAME=pull_request SCOPE_DOCS_ONLY=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success
  FRONTEND_RESULT=skipped RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped
  RUST_BUILD_RESULT=skipped RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped
  DAGSTER_TESTS_RESULT=skipped G1_RUSTFS_RESULT=skipped G2_SEAWEEDFS_RESULT=skipped
  G3A_DAGSTER_RESULT=skipped G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped
  G6_INGEST_RESULT=skipped GOLD_EXPORT_RESULT=skipped G8_GOVERNANCE_RESULT=skipped
)

base_frontend_green=(
  EVENT_NAME=pull_request SCOPE_FRONTEND=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=success
  RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped RUST_BUILD_RESULT=skipped
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped DAGSTER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=skipped G2_SEAWEEDFS_RESULT=skipped G3A_DAGSTER_RESULT=skipped
  G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped G6_INGEST_RESULT=skipped
  GOLD_EXPORT_RESULT=skipped G8_GOVERNANCE_RESULT=skipped
)

base_rust_green=(
  EVENT_NAME=pull_request SCOPE_RUST=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success
  RUST_TEST_RESULT=success RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G6_INGEST_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success
)

base_dagster_green=(
  EVENT_NAME=pull_request SCOPE_DAGSTER=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped RUST_BUILD_RESULT=skipped
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped DAGSTER_TESTS_RESULT=success
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G6_INGEST_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success
)

base_stack_green=(
  EVENT_NAME=pull_request SCOPE_STACK=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped RUST_BUILD_RESULT=skipped
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped DAGSTER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G6_INGEST_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success
)

base_all_green=(
  EVENT_NAME=pull_request SCOPE_ALL=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=success
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success
  RUST_TEST_RESULT=success RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=success
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G6_INGEST_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success
)

base_push_green=(
  EVENT_NAME=push
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=success
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success
  RUST_TEST_RESULT=success RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=success
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G6_INGEST_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success
)

# 1. Happy path cases
expect 0 "Docs-only green PR passes gate" "${base_docs_green[@]}"
expect 0 "Frontend green PR passes gate" "${base_frontend_green[@]}"
expect 0 "Rust green PR passes gate" "${base_rust_green[@]}"
expect 0 "Dagster green PR passes gate" "${base_dagster_green[@]}"
expect 0 "Stack green PR passes gate" "${base_stack_green[@]}"
expect 0 "Scope-all green PR passes gate" "${base_all_green[@]}"
expect 0 "Push to main green passes gate" "${base_push_green[@]}"

# 2. Failure scenarios
expect 1 "Failing changes job blocks gate" "${base_docs_green[@]}" CHANGES_RESULT=failure
expect 1 "Failing repo-lints blocks docs-only PR" "${base_docs_green[@]}" REPO_LINTS_RESULT=failure
expect 1 "Failing frontend blocks frontend PR" "${base_frontend_green[@]}" FRONTEND_RESULT=failure
expect 1 "Skipped frontend blocks frontend PR" "${base_frontend_green[@]}" FRONTEND_RESULT=skipped
expect 1 "Failing rust test blocks rust PR" "${base_rust_green[@]}" RUST_TEST_RESULT=failure
expect 1 "Skipped rust test blocks rust PR" "${base_rust_green[@]}" RUST_TEST_RESULT=skipped
expect 1 "Failing MSRV check blocks rust PR" "${base_rust_green[@]}" RUST_MSRV_RESULT=failure
expect 1 "Skipped MSRV check blocks rust PR" "${base_rust_green[@]}" RUST_MSRV_RESULT=skipped
expect 1 "Failing clippy blocks rust PR" "${base_rust_green[@]}" RUST_CLIPPY_RESULT=failure
expect 1 "Failing fmt blocks rust PR" "${base_rust_green[@]}" RUST_FMT_RESULT=failure
expect 1 "Failing build blocks rust PR" "${base_rust_green[@]}" RUST_BUILD_RESULT=failure
expect 1 "Failing acceptance blocks rust PR" "${base_rust_green[@]}" G6_INGEST_RESULT=failure
expect 1 "Skipped acceptance blocks rust PR" "${base_rust_green[@]}" G1_RUSTFS_RESULT=skipped
expect 1 "Failing dagster unit test blocks dagster PR" "${base_dagster_green[@]}" DAGSTER_TESTS_RESULT=failure
expect 1 "Skipped dagster unit test blocks dagster PR" "${base_dagster_green[@]}" DAGSTER_TESTS_RESULT=skipped
expect 1 "Failing acceptance blocks stack PR" "${base_stack_green[@]}" G4_CDC_RESULT=failure
expect 1 "Cancelled job blocks gate" "${base_rust_green[@]}" RUST_TEST_RESULT=cancelled
expect 1 "Skipped job on push blocks gate" "${base_push_green[@]}" RUST_TEST_RESULT=skipped

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
