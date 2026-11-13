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

REPO_TMP="$(mktemp -d)"
trap 'rm -rf "$REPO_TMP"' EXIT

expect() {
  local want="$1"
  local label="$2"
  shift 2
  local got=0

  (
    # Clean subshell environment
    unset EVENT_NAME SCOPE_RUST SCOPE_FRONTEND SCOPE_DAGSTER SCOPE_STACK SCOPE_DOCS_ONLY SCOPE_ALL \
          PUSH_PROVEN SELECTED_GATE_GROUPS GATE_PATHS_FILE \
          CHANGES_RESULT REPO_LINTS_RESULT FRONTEND_RESULT RUST_FMT_RESULT RUST_CLIPPY_RESULT \
          RUST_BUILD_RESULT RUST_TEST_BUILD_RESULT RUST_TEST_RESULT RUST_MSRV_RESULT DAGSTER_TESTS_RESULT DAGSTER_LOADER_TESTS_RESULT \
          G1_RUSTFS_RESULT G2_SEAWEEDFS_RESULT G3A_DAGSTER_RESULT G3_MAINTENANCE_RESULT \
          G4_CDC_RESULT G4_MYSQL_RESULT G4_MSSQL_RESULT G4_MONGO_RESULT G4_ORACLE_RESULT G10_SSO_RESULT G6_INGEST_RESULT G6_INGEST_OTHER_RESULT G6_ORACLE_RESULT G9_UPLOAD_RESULT G6_ALERTS_RESULT G6_OPS_RESULT G6_CREDENTIALS_RESULT G6_SECRET_MANAGERS_RESULT G6_OAUTH_RESULT G6_DISCOVERY_SCALE_RESULT G6_SAAS_A_RESULT G6_SAAS_B_RESULT GOLD_EXPORT_RESULT G8_GOVERNANCE_RESULT G8_TIME_TRAVEL_RESULT \
          IMAGES_RESULT IMAGE_SMOKE_RESULT
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
  RUST_BUILD_RESULT=skipped RUST_TEST_BUILD_RESULT=skipped RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped
  DAGSTER_TESTS_RESULT=skipped DAGSTER_LOADER_TESTS_RESULT=skipped G1_RUSTFS_RESULT=skipped G2_SEAWEEDFS_RESULT=skipped
  G3A_DAGSTER_RESULT=skipped G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped G4_MYSQL_RESULT=skipped G4_MSSQL_RESULT=skipped G4_MONGO_RESULT=skipped G4_ORACLE_RESULT=skipped G10_SSO_RESULT=skipped
  G6_INGEST_RESULT=skipped G6_INGEST_OTHER_RESULT=skipped G6_ORACLE_RESULT=skipped G9_UPLOAD_RESULT=skipped G6_ALERTS_RESULT=skipped G6_OPS_RESULT=skipped G6_CREDENTIALS_RESULT=skipped G6_SECRET_MANAGERS_RESULT=skipped G6_OAUTH_RESULT=skipped G6_DISCOVERY_SCALE_RESULT=skipped G6_SAAS_A_RESULT=skipped G6_SAAS_B_RESULT=skipped GOLD_EXPORT_RESULT=skipped G8_GOVERNANCE_RESULT=skipped G8_TIME_TRAVEL_RESULT=skipped
  IMAGES_RESULT=skipped IMAGE_SMOKE_RESULT=skipped
)

base_frontend_green=(
  EVENT_NAME=pull_request SCOPE_FRONTEND=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=success
  RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped RUST_BUILD_RESULT=skipped RUST_TEST_BUILD_RESULT=skipped
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped DAGSTER_TESTS_RESULT=skipped DAGSTER_LOADER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=skipped G2_SEAWEEDFS_RESULT=skipped G3A_DAGSTER_RESULT=skipped
  G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped G4_MYSQL_RESULT=skipped G4_MSSQL_RESULT=skipped G4_MONGO_RESULT=skipped G4_ORACLE_RESULT=skipped G10_SSO_RESULT=skipped G6_INGEST_RESULT=skipped G6_INGEST_OTHER_RESULT=skipped G6_ORACLE_RESULT=skipped G9_UPLOAD_RESULT=skipped G6_ALERTS_RESULT=skipped G6_OPS_RESULT=skipped G6_CREDENTIALS_RESULT=skipped G6_SECRET_MANAGERS_RESULT=skipped G6_OAUTH_RESULT=skipped G6_DISCOVERY_SCALE_RESULT=skipped G6_SAAS_A_RESULT=skipped G6_SAAS_B_RESULT=skipped
  GOLD_EXPORT_RESULT=skipped G8_GOVERNANCE_RESULT=skipped G8_TIME_TRAVEL_RESULT=skipped
  IMAGES_RESULT=skipped IMAGE_SMOKE_RESULT=skipped
)

base_rust_green=(
  EVENT_NAME=pull_request SCOPE_RUST=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success RUST_TEST_BUILD_RESULT=success
  RUST_TEST_RESULT=success RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=skipped DAGSTER_LOADER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G4_MYSQL_RESULT=success G4_MSSQL_RESULT=success G4_MONGO_RESULT=success G4_ORACLE_RESULT=success G10_SSO_RESULT=success G6_INGEST_RESULT=success G6_INGEST_OTHER_RESULT=success G6_ORACLE_RESULT=success G9_UPLOAD_RESULT=success G6_ALERTS_RESULT=success G6_OPS_RESULT=success G6_CREDENTIALS_RESULT=success G6_SECRET_MANAGERS_RESULT=success G6_OAUTH_RESULT=success G6_DISCOVERY_SCALE_RESULT=success G6_SAAS_A_RESULT=success G6_SAAS_B_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success G8_TIME_TRAVEL_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

base_dagster_green=(
  EVENT_NAME=pull_request SCOPE_DAGSTER=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped RUST_BUILD_RESULT=skipped RUST_TEST_BUILD_RESULT=skipped
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped DAGSTER_TESTS_RESULT=success DAGSTER_LOADER_TESTS_RESULT=success
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G4_MYSQL_RESULT=success G4_MSSQL_RESULT=success G4_MONGO_RESULT=success G4_ORACLE_RESULT=success G10_SSO_RESULT=success G6_INGEST_RESULT=success G6_INGEST_OTHER_RESULT=success G6_ORACLE_RESULT=success G9_UPLOAD_RESULT=success G6_ALERTS_RESULT=success G6_OPS_RESULT=success G6_CREDENTIALS_RESULT=success G6_SECRET_MANAGERS_RESULT=success G6_OAUTH_RESULT=success G6_DISCOVERY_SCALE_RESULT=success G6_SAAS_A_RESULT=success G6_SAAS_B_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success G8_TIME_TRAVEL_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

base_stack_green=(
  EVENT_NAME=pull_request SCOPE_STACK=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=skipped RUST_CLIPPY_RESULT=skipped RUST_BUILD_RESULT=skipped RUST_TEST_BUILD_RESULT=skipped
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=skipped DAGSTER_TESTS_RESULT=skipped DAGSTER_LOADER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G4_MYSQL_RESULT=success G4_MSSQL_RESULT=success G4_MONGO_RESULT=success G4_ORACLE_RESULT=success G10_SSO_RESULT=success G6_INGEST_RESULT=success G6_INGEST_OTHER_RESULT=success G6_ORACLE_RESULT=success G9_UPLOAD_RESULT=success G6_ALERTS_RESULT=success G6_OPS_RESULT=success G6_CREDENTIALS_RESULT=success G6_SECRET_MANAGERS_RESULT=success G6_OAUTH_RESULT=success G6_DISCOVERY_SCALE_RESULT=success G6_SAAS_A_RESULT=success G6_SAAS_B_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success G8_TIME_TRAVEL_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

base_all_green=(
  EVENT_NAME=pull_request SCOPE_ALL=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=success
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success RUST_TEST_BUILD_RESULT=success
  RUST_TEST_RESULT=success RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=success DAGSTER_LOADER_TESTS_RESULT=success
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G4_MYSQL_RESULT=success G4_MSSQL_RESULT=success G4_MONGO_RESULT=success G4_ORACLE_RESULT=success G10_SSO_RESULT=success G6_INGEST_RESULT=success G6_INGEST_OTHER_RESULT=success G6_ORACLE_RESULT=success G9_UPLOAD_RESULT=success G6_ALERTS_RESULT=success G6_OPS_RESULT=success G6_CREDENTIALS_RESULT=success G6_SECRET_MANAGERS_RESULT=success G6_OAUTH_RESULT=success G6_DISCOVERY_SCALE_RESULT=success G6_SAAS_A_RESULT=success G6_SAAS_B_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success G8_TIME_TRAVEL_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

base_push_green=(
  EVENT_NAME=push
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=success
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success RUST_TEST_BUILD_RESULT=success
  RUST_TEST_RESULT=success RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=success DAGSTER_LOADER_TESTS_RESULT=success
  G1_RUSTFS_RESULT=success G2_SEAWEEDFS_RESULT=success G3A_DAGSTER_RESULT=success
  G3_MAINTENANCE_RESULT=success G4_CDC_RESULT=success G4_MYSQL_RESULT=success G4_MSSQL_RESULT=success G4_MONGO_RESULT=success G4_ORACLE_RESULT=success G10_SSO_RESULT=success G6_INGEST_RESULT=success G6_INGEST_OTHER_RESULT=success G6_ORACLE_RESULT=success G9_UPLOAD_RESULT=success G6_ALERTS_RESULT=success G6_OPS_RESULT=success G6_CREDENTIALS_RESULT=success G6_SECRET_MANAGERS_RESULT=success G6_OAUTH_RESULT=success G6_DISCOVERY_SCALE_RESULT=success G6_SAAS_A_RESULT=success G6_SAAS_B_RESULT=success
  GOLD_EXPORT_RESULT=success G8_GOVERNANCE_RESULT=success G8_TIME_TRAVEL_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

# OPS-24 D5/D6: a push proven to repeat a passed pull request. The Rust jobs,
# `images` and the lints ran; the acceptance jobs and `image-smoke` were
# skipped because they repeated a run that passed.
base_push_proven_green=(
  EVENT_NAME=push SCOPE_RUST=true PUSH_PROVEN=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success FRONTEND_RESULT=skipped
  RUST_FMT_RESULT=success RUST_CLIPPY_RESULT=success RUST_BUILD_RESULT=success RUST_TEST_BUILD_RESULT=success
  # OPS-30: the partitions are skipped on a proven push; `test-build` above
  # is the job that keeps the cache current instead.
  RUST_TEST_RESULT=skipped RUST_MSRV_RESULT=success DAGSTER_TESTS_RESULT=skipped DAGSTER_LOADER_TESTS_RESULT=skipped
  G1_RUSTFS_RESULT=skipped G2_SEAWEEDFS_RESULT=skipped G3A_DAGSTER_RESULT=skipped
  G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped G4_MYSQL_RESULT=skipped G4_MSSQL_RESULT=skipped G4_MONGO_RESULT=skipped G4_ORACLE_RESULT=skipped G10_SSO_RESULT=skipped G6_INGEST_RESULT=skipped G6_INGEST_OTHER_RESULT=skipped G6_ORACLE_RESULT=skipped G9_UPLOAD_RESULT=skipped G6_ALERTS_RESULT=skipped G6_OPS_RESULT=skipped G6_CREDENTIALS_RESULT=skipped G6_SECRET_MANAGERS_RESULT=skipped G6_OAUTH_RESULT=skipped G6_DISCOVERY_SCALE_RESULT=skipped G6_SAAS_A_RESULT=skipped G6_SAAS_B_RESULT=skipped
  GOLD_EXPORT_RESULT=skipped G8_GOVERNANCE_RESULT=skipped G8_TIME_TRAVEL_RESULT=skipped
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=skipped
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
# OPS-30: the archive build is the only compile, so a skipped or failed one
# must be red on a Rust-scoped pull request.
expect 1 "Failing test build blocks rust PR" "${base_rust_green[@]}" RUST_TEST_BUILD_RESULT=failure
expect 1 "Skipped test build blocks rust PR" "${base_rust_green[@]}" RUST_TEST_BUILD_RESULT=skipped
expect 1 "Failing acceptance blocks rust PR" "${base_rust_green[@]}" G6_INGEST_RESULT=failure
expect 1 "Failing second half of the ingest gate blocks rust PR" "${base_rust_green[@]}" G6_INGEST_OTHER_RESULT=failure
expect 1 "Failing Oracle gate blocks rust PR" "${base_rust_green[@]}" G6_ORACLE_RESULT=failure
expect 1 "Failing upload gate blocks rust PR" "${base_rust_green[@]}" G9_UPLOAD_RESULT=failure
expect 1 "Failing alert delivery gate blocks rust PR" "${base_rust_green[@]}" G6_ALERTS_RESULT=failure
expect 1 "Failing connector operations gate blocks rust PR" "${base_rust_green[@]}" G6_OPS_RESULT=failure
expect 1 "Skipped connector operations gate blocks rust PR" "${base_rust_green[@]}" G6_OPS_RESULT=skipped G6_CREDENTIALS_RESULT=skipped G6_SECRET_MANAGERS_RESULT=skipped G6_OAUTH_RESULT=skipped G6_DISCOVERY_SCALE_RESULT=skipped G6_SAAS_A_RESULT=skipped G6_SAAS_B_RESULT=skipped
expect 1 "Failing OAuth sign-in gate blocks rust PR" "${base_rust_green[@]}" G6_OAUTH_RESULT=failure
expect 1 "Skipped OAuth sign-in gate blocks rust PR" "${base_rust_green[@]}" G6_OAUTH_RESULT=skipped
expect 1 "Failing discovery scale gate blocks rust PR" "${base_rust_green[@]}" G6_DISCOVERY_SCALE_RESULT=failure
expect 1 "Skipped discovery scale gate blocks rust PR" "${base_rust_green[@]}" G6_DISCOVERY_SCALE_RESULT=skipped
expect 1 "Failing business-app gate A blocks rust PR" "${base_rust_green[@]}" G6_SAAS_A_RESULT=failure
expect 1 "Failing business-app gate B blocks rust PR" "${base_rust_green[@]}" G6_SAAS_B_RESULT=failure
expect 1 "Skipped business-app gate A blocks rust PR" "${base_rust_green[@]}" G6_SAAS_A_RESULT=skipped
expect 1 "Skipped business-app gate B blocks rust PR" "${base_rust_green[@]}" G6_SAAS_B_RESULT=skipped
expect 1 "Failing time-travel acceptance blocks rust PR" "${base_rust_green[@]}" G8_TIME_TRAVEL_RESULT=failure
expect 1 "Skipped acceptance blocks rust PR" "${base_rust_green[@]}" G1_RUSTFS_RESULT=skipped
expect 1 "Failing dagster unit test blocks dagster PR" "${base_dagster_green[@]}" DAGSTER_TESTS_RESULT=failure
expect 1 "Failing dagster loader test blocks dagster PR" "${base_dagster_green[@]}" DAGSTER_LOADER_TESTS_RESULT=failure
expect 1 "Skipped dagster unit test blocks dagster PR" "${base_dagster_green[@]}" DAGSTER_TESTS_RESULT=skipped
expect 1 "Skipped dagster loader test blocks dagster PR" "${base_dagster_green[@]}" DAGSTER_LOADER_TESTS_RESULT=skipped
expect 1 "Failing acceptance blocks stack PR" "${base_stack_green[@]}" G4_CDC_RESULT=failure
expect 1 "Failing SSO gate blocks rust PR" "${base_rust_green[@]}" G10_SSO_RESULT=failure
expect 1 "Skipped SSO gate blocks stack PR" "${base_stack_green[@]}" G10_SSO_RESULT=skipped
expect 1 "Cancelled job blocks gate" "${base_rust_green[@]}" RUST_TEST_RESULT=cancelled
# OPS-24 D1: the old two lines here asserted that the event `push` alone made
# every job required. This plan replaces that rule with "the scope decides", so
# the assertions are restated under the new rule: a skipped job the scope
# requires is still an error.
expect 1 "Skipped Rust test on a Rust push blocks gate" "${base_push_green[@]}" SCOPE_RUST=true RUST_TEST_RESULT=skipped
expect 1 "Failing images job blocks rust PR" "${base_rust_green[@]}" IMAGES_RESULT=failure
expect 1 "Cancelled images job blocks rust PR" "${base_rust_green[@]}" IMAGES_RESULT=cancelled
expect 1 "Skipped images job blocks rust PR" "${base_rust_green[@]}" IMAGES_RESULT=skipped
expect 1 "Failing image smoke test blocks rust PR" "${base_rust_green[@]}" IMAGE_SMOKE_RESULT=failure
expect 1 "Skipped image smoke test blocks rust PR" "${base_rust_green[@]}" IMAGE_SMOKE_RESULT=skipped
expect 1 "Failing images job blocks dagster PR" "${base_dagster_green[@]}" IMAGES_RESULT=failure
expect 1 "Failing images job blocks stack PR" "${base_stack_green[@]}" IMAGES_RESULT=failure
expect 1 "Skipped image smoke test on a Rust push blocks gate" "${base_push_green[@]}" SCOPE_RUST=true IMAGE_SMOKE_RESULT=skipped

# OPS-24 D1: a push is gated by its scope exactly like a pull request.
expect 0 "Documents-only push with only the lints passes gate" "${base_docs_green[@]}" EVENT_NAME=push
expect 1 "Failed job on a documents-only push blocks gate" "${base_docs_green[@]}" EVENT_NAME=push RUST_TEST_RESULT=failure
expect 1 "Push that forces all with a skipped gate blocks gate" "${base_push_green[@]}" SCOPE_ALL=true G1_RUSTFS_RESULT=skipped
# OPS-24 D3: the scheduled run has no base commit, so the scope is all=true.
expect 0 "Scheduled run with everything green passes gate" "${base_all_green[@]}" EVENT_NAME=schedule
expect 1 "Scheduled run with a skipped gate blocks gate" "${base_all_green[@]}" EVENT_NAME=schedule G4_CDC_RESULT=skipped
# OPS-24 R3: the two all-jobs events require every job whatever the scope
# says, so an empty scope with skipped jobs is red, not green.
expect 1 "Scheduled run with empty scopes and skipped gates blocks gate" "${base_docs_green[@]}" EVENT_NAME=schedule
expect 1 "Manual run with empty scopes and skipped gates blocks gate" "${base_docs_green[@]}" EVENT_NAME=workflow_dispatch
expect 0 "Manual run with everything green passes gate" "${base_all_green[@]}" EVENT_NAME=workflow_dispatch

# OPS-24 D5/D6: a proven push excuses the acceptance jobs and `image-smoke`,
# and nothing else. A failure or a cancellation is still an error, and the
# proof means nothing on another event or with any other value.
expect 0 "Proven push with the gates skipped passes gate" "${base_push_proven_green[@]}"
expect 0 "Proven push whose gates ran green passes gate" "${base_push_proven_green[@]}" G4_CDC_RESULT=success
expect 1 "A failed gate on a proven push blocks gate" "${base_push_proven_green[@]}" G4_CDC_RESULT=failure
expect 1 "A cancelled gate on a proven push blocks gate" "${base_push_proven_green[@]}" G6_OPS_RESULT=cancelled
expect 1 "A skipped images job on a proven push blocks gate" "${base_push_proven_green[@]}" IMAGES_RESULT=skipped
# OPS-24 D5/D6 used to say the proof never excuses the `test` job, and one leg
# ran there to compile. OPS-30 moved the compile to `test-build`, so the
# partitions are the one Rust job a proven push skips and the archive build is
# the one it requires. A failed or cancelled leg is still red.
expect 0 "Skipped Rust test partitions on a proven push pass the gate" "${base_push_proven_green[@]}"
expect 1 "A failed Rust test job on a proven push blocks gate" "${base_push_proven_green[@]}" RUST_TEST_RESULT=failure
expect 1 "A cancelled Rust test job on a proven push blocks gate" "${base_push_proven_green[@]}" RUST_TEST_RESULT=cancelled
expect 1 "A skipped test build on a proven push blocks gate" "${base_push_proven_green[@]}" RUST_TEST_BUILD_RESULT=skipped
expect 1 "A failed test build on a proven push blocks gate" "${base_push_proven_green[@]}" RUST_TEST_BUILD_RESULT=failure
expect 1 "Skipped Rust test partitions on a pull request block gate" "${base_push_proven_green[@]}" EVENT_NAME=pull_request
expect 1 "A skipped clippy job on a proven push blocks gate" "${base_push_proven_green[@]}" RUST_CLIPPY_RESULT=skipped
expect 1 "Skipped gates on a push that is not proven block gate" "${base_push_proven_green[@]}" PUSH_PROVEN=false
expect 1 "Skipped gates on a push with no proof value block gate" "${base_push_proven_green[@]}" PUSH_PROVEN=
expect 1 "The proof is ignored on a pull request" "${base_push_proven_green[@]}" EVENT_NAME=pull_request
expect 1 "The proof is ignored on a scheduled run" "${base_push_proven_green[@]}" EVENT_NAME=schedule SCOPE_ALL=true
# A failed `images` job makes every job that `needs: images` report
# `skipped`, not `failure`. The gate must still be red: both `images` itself
# and each skipped consumer are errors for a PR that required them.
expect 1 "Failed images job with every consumer skipped blocks rust PR" "${base_rust_green[@]}" \
  IMAGES_RESULT=failure IMAGE_SMOKE_RESULT=skipped G3A_DAGSTER_RESULT=skipped \
  G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped G4_MYSQL_RESULT=skipped G4_MSSQL_RESULT=skipped G4_MONGO_RESULT=skipped G4_ORACLE_RESULT=skipped G10_SSO_RESULT=skipped G6_INGEST_RESULT=skipped \
  G6_INGEST_OTHER_RESULT=skipped G6_ORACLE_RESULT=skipped G9_UPLOAD_RESULT=skipped \
  G6_ALERTS_RESULT=skipped G6_OPS_RESULT=skipped G6_CREDENTIALS_RESULT=skipped G6_SECRET_MANAGERS_RESULT=skipped G6_OAUTH_RESULT=skipped G6_DISCOVERY_SCALE_RESULT=skipped G6_SAAS_A_RESULT=skipped G6_SAAS_B_RESULT=skipped GOLD_EXPORT_RESULT=skipped G8_GOVERNANCE_RESULT=skipped \
  G8_TIME_TRAVEL_RESULT=skipped
# The consumers alone are enough to turn the gate red, even if `images`
# itself were (wrongly) reported as success.
expect 1 "Skipped consumers alone block rust PR even when images reports success" "${base_rust_green[@]}" \
  G3A_DAGSTER_RESULT=skipped G3_MAINTENANCE_RESULT=skipped G4_CDC_RESULT=skipped G4_MYSQL_RESULT=skipped G4_MSSQL_RESULT=skipped G4_MONGO_RESULT=skipped G4_ORACLE_RESULT=skipped G10_SSO_RESULT=skipped \
  G6_INGEST_RESULT=skipped G6_INGEST_OTHER_RESULT=skipped G6_ORACLE_RESULT=skipped \
  G9_UPLOAD_RESULT=skipped G6_ALERTS_RESULT=skipped G6_OPS_RESULT=skipped G6_CREDENTIALS_RESULT=skipped G6_SECRET_MANAGERS_RESULT=skipped G6_OAUTH_RESULT=skipped G6_DISCOVERY_SCALE_RESULT=skipped G6_SAAS_A_RESULT=skipped G6_SAAS_B_RESULT=skipped GOLD_EXPORT_RESULT=skipped \
  G8_GOVERNANCE_RESULT=skipped G8_TIME_TRAVEL_RESULT=skipped
expect 0 "Images and smoke skipped is fine for a docs-only PR" "${base_docs_green[@]}"

# OPS-25 D7: a gate job is required only when its group was selected, and a
# selection that cannot be read means every gate is required. The cases above
# set no SELECTED_GATE_GROUPS at all, which is the fail-closed answer ("the
# scope needs the gates, so every gate is required"); they keep their meaning.
base_cdc_selected=(
  EVENT_NAME=pull_request SCOPE_STACK=true SELECTED_GATE_GROUPS=,cdc,
  CHANGES_RESULT=success REPO_LINTS_RESULT=success
  G4_CDC_RESULT=success G4_MYSQL_RESULT=success G4_MSSQL_RESULT=success
  G4_MONGO_RESULT=success G4_ORACLE_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

base_none_selected=(
  EVENT_NAME=pull_request SCOPE_STACK=true SELECTED_GATE_GROUPS=none
  CHANGES_RESULT=success REPO_LINTS_RESULT=success
  IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

base_cdc_scope_no_selection=(
  EVENT_NAME=pull_request SCOPE_STACK=true
  CHANGES_RESULT=success REPO_LINTS_RESULT=success
  G4_CDC_RESULT=success IMAGES_RESULT=success IMAGE_SMOKE_RESULT=success
)

expect 0 "a pull request selecting only cdc passes with the five cdc gates green" "${base_cdc_selected[@]}"
expect 1 "a pull request selecting only cdc fails when g4-mysql is skipped" "${base_cdc_selected[@]}" G4_MYSQL_RESULT=skipped
expect 1 "a pull request selecting only cdc fails when g6-ingest, not selected, failed" "${base_cdc_selected[@]}" G6_INGEST_RESULT=failure
expect 0 "a pull request selecting no gate group passes with every gate skipped" "${base_none_selected[@]}"
expect 1 "a pull request selecting no gate group fails when a gate failed" "${base_none_selected[@]}" G1_RUSTFS_RESULT=failure

# Fail closed: unset, empty and malformed selections require every gate.
expect 1 "an unset SELECTED_GATE_GROUPS requires every gate" "${base_cdc_scope_no_selection[@]}"
expect 1 "an empty SELECTED_GATE_GROUPS requires every gate" "${base_cdc_selected[@]}" SELECTED_GATE_GROUPS=
expect 1 "a malformed SELECTED_GATE_GROUPS (bare cdc) requires every gate" "${base_cdc_selected[@]}" SELECTED_GATE_GROUPS=cdc

# A selection never overrides "every gate is required": all=true, the nightly
# run and the manual run keep requiring all 25.
expect 1 "all=true requires every gate even when only cdc is selected" "${base_cdc_selected[@]}" SCOPE_ALL=true
expect 1 "a scheduled run requires every gate even when only cdc is selected" "${base_cdc_selected[@]}" EVENT_NAME=schedule
expect 1 "a manual run requires every gate even when only cdc is selected" "${base_cdc_selected[@]}" EVENT_NAME=workflow_dispatch

# A proven push excuses the gates whatever the selection says (OPS-24 D5/D6).
expect 0 "a proven push with the cdc group selected passes with every gate skipped" "${base_push_proven_green[@]}" SELECTED_GATE_GROUPS=,cdc,
expect 1 "a proven push with the cdc group selected still fails on a failed gate" "${base_push_proven_green[@]}" SELECTED_GATE_GROUPS=,cdc, G4_CDC_RESULT=failure

# The map is the only source of "which group is this job in?". A job with no
# `group` line, and an unreadable map, are required.
no_cdc_map="$REPO_TMP/no_cdc_gate_paths.txt"
mkdir -p "$REPO_TMP"
grep -v '^group cdc ' "${SCRIPT_DIR}/../gate_paths.txt" > "$no_cdc_map"
expect 1 "a gate job with no group line is required even when cdc is not selected" "${base_none_selected[@]}" GATE_PATHS_FILE="$no_cdc_map"
expect 1 "an unreadable gate path map makes every gate required" "${base_cdc_selected[@]}" GATE_PATHS_FILE=/nonexistent

# 3. Workflow & Gate structure consistency checks (SHOULD-FIX 6)
CI_YML="${SCRIPT_DIR}/../../../.github/workflows/ci.yml"

test_consistency() {
  local ci_file="$1"
  local gate_file="$2"
  python3 - "$ci_file" "$gate_file" <<'EOF'
import sys, re, yaml

ci_file = sys.argv[1]
gate_file = sys.argv[2]

with open(ci_file) as f:
    ci = yaml.safe_load(f)

jobs = list(ci.get("jobs", {}).keys())
other_jobs = [j for j in jobs if j != "ci-required"]
needs = ci.get("jobs", {}).get("ci-required", {}).get("needs", [])

missing_needs = [j for j in other_jobs if j not in needs]
if missing_needs:
    sys.stderr.write(f"Missing from ci-required needs: {missing_needs}\n")
    sys.exit(1)

steps = ci.get("jobs", {}).get("ci-required", {}).get("steps", [])
env_map = {}
for s in steps:
    if "env" in s:
        for k, v in s["env"].items():
            m = re.search(r"needs\.([a-zA-Z0-9_-]+)\.result", str(v))
            if m:
                env_map[m.group(1)] = k

with open(gate_file) as f:
    gate_sh = f.read()

missing_vars = []
for j in other_jobs:
    var = env_map.get(j)
    if not var or var not in gate_sh:
        missing_vars.append(j)

if missing_vars:
    sys.stderr.write(f"Missing result var in required_gate.sh: {missing_vars}\n")
    sys.exit(2)

sys.exit(0)
EOF
}

# Positive test: real ci.yml and required_gate.sh
if test_consistency "$CI_YML" "$SCRIPT" >/dev/null 2>&1; then
  log_pass "all jobs in ci.yml are present in ci-required needs and have result variables in required_gate.sh"
else
  log_fail "job in ci.yml missing from ci-required needs or lacks result variable in required_gate.sh"
fi

# Negative test 1: fails when any job is missing from ci-required needs
(
  tmp_ci="$(mktemp)"
  python3 - "$CI_YML" "$tmp_ci" <<'EOF'
import sys, yaml
with open(sys.argv[1]) as f:
    ci = yaml.safe_load(f)
ci["jobs"]["ci-required"]["needs"] = [n for n in ci["jobs"]["ci-required"]["needs"] if n != "g1-rustfs"]
with open(sys.argv[2], "w") as f:
    yaml.dump(ci, f)
EOF
  if ! test_consistency "$tmp_ci" "$SCRIPT" >/dev/null 2>&1; then
    log_pass "self-test fails when a job in ci.yml is missing from ci-required needs"
  else
    log_fail "self-test did not fail when job was missing from ci-required needs"
  fi
  rm -f "$tmp_ci"
)

# Negative test 2: fails when any job lacks a result variable in required_gate.sh
(
  tmp_gate="$(mktemp)"
  python3 - "$SCRIPT" "$tmp_gate" <<'EOF'
import sys
with open(sys.argv[1]) as f:
    content = f.read()
content = content.replace("G1_RUSTFS_RESULT", "UNUSED_VAR_TEST")
with open(sys.argv[2], "w") as f:
    f.write(content)
EOF
  if ! test_consistency "$CI_YML" "$tmp_gate" >/dev/null 2>&1; then
    log_pass "self-test fails when a job in ci.yml lacks a result variable in required_gate.sh"
  else
    log_fail "self-test did not fail when result variable was missing"
  fi
  rm -f "$tmp_gate"
)

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
