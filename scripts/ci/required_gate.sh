#!/usr/bin/env bash
#
# Usage: required_gate.sh [--self-test]
#
# The single gate job `ci-required` required by CI. Reads the outcome of every
# other job in `ci.yml` (passed in as environment variables) and ensures that
# all required checks succeeded or were legitimately skipped based on change scope.
#
# Inputs:
#   EVENT_NAME            github.event_name (push or pull_request)
#   SCOPE_RUST            "true" if rust changed
#   SCOPE_FRONTEND        "true" if frontend changed
#   SCOPE_DAGSTER         "true" if dagster changed
#   SCOPE_STACK           "true" if stack/ops changed
#   SCOPE_DOCS_ONLY       "true" if docs only
#   SCOPE_ALL             "true" if all jobs forced
#   CHANGES_RESULT        result of `changes` job
#   REPO_LINTS_RESULT     result of `repo-lints` job
#   FRONTEND_RESULT       result of `verify` job
#   RUST_FMT_RESULT       result of `fmt` job
#   RUST_CLIPPY_RESULT    result of `clippy` job
#   RUST_BUILD_RESULT     result of `build` job
#   RUST_TEST_RESULT      result of `test` job (stable)
#   RUST_MSRV_RESULT      result of `msrv` job (1.88.0)
#   DAGSTER_TESTS_RESULT  result of `dagster-unit-tests` job
#   G1_RUSTFS_RESULT      result of `g1-rustfs` job
#   G2_SEAWEEDFS_RESULT   result of `g2-seaweedfs` job
#   G3A_DAGSTER_RESULT    result of `g3a-dagster` job
#   G3_MAINTENANCE_RESULT result of `g3-maintenance` job
#   G4_CDC_RESULT         result of `g4-cdc` job
#   G6_INGEST_RESULT      result of `g6-ingest` job
#   GOLD_EXPORT_RESULT    result of `gold-export` job
#   G8_GOVERNANCE_RESULT  result of `g8-governance` job
#   IMAGES_RESULT         result of `images` job (builds the shared images)
#   IMAGE_SMOKE_RESULT    result of `image-smoke` job (/health on the API image)
set -euo pipefail

run_gate() {
  local event_name="${EVENT_NAME:-}"
  local scope_rust="${SCOPE_RUST:-false}"
  local scope_frontend="${SCOPE_FRONTEND:-false}"
  local scope_dagster="${SCOPE_DAGSTER:-false}"
  local scope_stack="${SCOPE_STACK:-false}"
  local scope_docs_only="${SCOPE_DOCS_ONLY:-false}"
  local scope_all="${SCOPE_ALL:-false}"

  echo "=== Scope Evaluation ==="
  echo "EVENT_NAME:           $event_name"
  echo "SCOPE_RUST:           $scope_rust"
  echo "SCOPE_FRONTEND:       $scope_frontend"
  echo "SCOPE_DAGSTER:        $scope_dagster"
  echo "SCOPE_STACK:          $scope_stack"
  echo "SCOPE_DOCS_ONLY:      $scope_docs_only"
  echo "SCOPE_ALL:            $scope_all"

  local failures=0

  check_job() {
    local job_name="$1"
    local result="${2:-skipped}"
    local should_run="$3"

    echo "Job [$job_name]: result=$result, required=$should_run"

    if [ "$result" = "failure" ] || [ "$result" = "cancelled" ]; then
      echo "ERROR: $job_name finished with status '$result'."
      failures=$((failures + 1))
      return
    fi

    if [ "$should_run" = "true" ]; then
      if [ "$result" != "success" ]; then
        echo "ERROR: $job_name should have run according to change scope, but got '$result'."
        failures=$((failures + 1))
      fi
    else
      if [ "$result" != "skipped" ] && [ "$result" != "success" ]; then
        echo "ERROR: $job_name was expected to be skipped or success, but got '$result'."
        failures=$((failures + 1))
      fi
    fi
  }

  # Scope setup job must succeed
  check_job "changes" "${CHANGES_RESULT:-skipped}" "true"

  # Repo lints run unconditionally
  check_job "repo-lints" "${REPO_LINTS_RESULT:-skipped}" "true"

  # Determine required flags based on scope
  local req_all="false"
  if [ "$event_name" = "push" ] || [ "$scope_all" = "true" ]; then
    req_all="true"
  fi

  local req_frontend="false"
  if [ "$req_all" = "true" ] || [ "$scope_frontend" = "true" ]; then
    req_frontend="true"
  fi

  local req_rust="false"
  if [ "$req_all" = "true" ] || [ "$scope_rust" = "true" ]; then
    req_rust="true"
  fi

  local req_dagster="false"
  if [ "$req_all" = "true" ] || [ "$scope_dagster" = "true" ]; then
    req_dagster="true"
  fi

  local req_acceptance="false"
  if [ "$req_all" = "true" ] || [ "$scope_rust" = "true" ] || [ "$scope_dagster" = "true" ] || [ "$scope_stack" = "true" ]; then
    req_acceptance="true"
  fi

  # Frontend
  check_job "verify" "${FRONTEND_RESULT:-skipped}" "$req_frontend"

  # Rust fast feedback & tests
  check_job "fmt" "${RUST_FMT_RESULT:-skipped}" "$req_rust"
  check_job "clippy" "${RUST_CLIPPY_RESULT:-skipped}" "$req_rust"
  check_job "build" "${RUST_BUILD_RESULT:-skipped}" "$req_rust"
  check_job "test" "${RUST_TEST_RESULT:-skipped}" "$req_rust"
  check_job "msrv" "${RUST_MSRV_RESULT:-skipped}" "$req_rust"

  # Dagster
  check_job "dagster-unit-tests" "${DAGSTER_TESTS_RESULT:-skipped}" "$req_dagster"

  # Acceptance
  check_job "g1-rustfs" "${G1_RUSTFS_RESULT:-skipped}" "$req_acceptance"
  check_job "g2-seaweedfs" "${G2_SEAWEEDFS_RESULT:-skipped}" "$req_acceptance"
  check_job "g3a-dagster" "${G3A_DAGSTER_RESULT:-skipped}" "$req_acceptance"
  check_job "g3-maintenance" "${G3_MAINTENANCE_RESULT:-skipped}" "$req_acceptance"
  check_job "g4-cdc" "${G4_CDC_RESULT:-skipped}" "$req_acceptance"
  check_job "g6-ingest" "${G6_INGEST_RESULT:-skipped}" "$req_acceptance"
  check_job "gold-export" "${GOLD_EXPORT_RESULT:-skipped}" "$req_acceptance"
  check_job "g8-governance" "${G8_GOVERNANCE_RESULT:-skipped}" "$req_acceptance"

  # Shared images and their smoke test. Required exactly when the acceptance
  # jobs are: `images` has the same `if:` as they do and is what they consume.
  # This is also what keeps the gate red when `images` fails. Every job that
  # `needs: images` is then SKIPPED, not failed, and a skip alone would read
  # as "not required"; but each of them is required here, so a skip is an
  # error on its own, and `images` failing is reported by name as well.
  check_job "images" "${IMAGES_RESULT:-skipped}" "$req_acceptance"
  check_job "image-smoke" "${IMAGE_SMOKE_RESULT:-skipped}" "$req_acceptance"

  if [ "$failures" -gt 0 ]; then
    echo "ci-required gate FAILED: $failures check(s) did not meet requirements."
    return 1
  fi

  echo "ci-required gate PASSED: all checks satisfied."
  return 0
}

if [ "${1:-}" = "--self-test" ]; then
  SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
  exec bash "${SCRIPT_DIR}/tests/test_required_gate.sh"
fi

run_gate
