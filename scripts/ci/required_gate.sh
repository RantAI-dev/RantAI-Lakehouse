#!/usr/bin/env bash
#
# Usage: required_gate.sh [--self-test]
#
# The single gate job `ci-required` required by CI. Reads the outcome of every
# other job in `ci.yml` (passed in as environment variables) and ensures that
# all required checks succeeded or were legitimately skipped based on change scope.
#
# Inputs:
#   EVENT_NAME            github.event_name (push, pull_request, schedule or
#                         workflow_dispatch). A push and a pull request are
#                         gated by the scope alone (OPS-24 D1); the nightly
#                         run and a manual run require every job whatever the
#                         scope says (OPS-24 D3, R3).
#   SCOPE_RUST            "true" if rust changed
#   SCOPE_FRONTEND        "true" if frontend changed
#   SCOPE_DAGSTER         "true" if dagster changed
#   SCOPE_STACK           "true" if stack/ops changed
#   SCOPE_DOCS_ONLY       "true" if docs only
#   SCOPE_ALL             "true" if all jobs forced
#   PUSH_PROVEN           "true" when a push was proven to repeat a passed
#                         pull request (OPS-24 D4). It excuses the acceptance
#                         jobs and `image-smoke` on a `push` only; it never
#                         excuses the Rust jobs, the Dagster jobs, `images` or
#                         anything on another event (D6).
#   CHANGES_RESULT        result of `changes` job
#   REPO_LINTS_RESULT     result of `repo-lints` job
#   FRONTEND_RESULT       result of `verify` job
#   RUST_FMT_RESULT       result of `fmt` job
#   RUST_CLIPPY_RESULT    result of `clippy` job
#   RUST_BUILD_RESULT     result of `build` job
#   RUST_TEST_RESULT      result of `test` job (stable)
#   RUST_MSRV_RESULT      result of `msrv` job (1.99.0)
#   DAGSTER_TESTS_RESULT  result of `dagster-unit-tests` job
#   DAGSTER_LOADER_TESTS_RESULT result of `dagster-loader-tests` job
#   G1_RUSTFS_RESULT      result of `g1-rustfs` job
#   G2_SEAWEEDFS_RESULT   result of `g2-seaweedfs` job
#   G3A_DAGSTER_RESULT    result of `g3a-dagster` job
#   G3_MAINTENANCE_RESULT result of `g3-maintenance` job
#   G4_CDC_RESULT         result of `g4-cdc` job
#   G4_MYSQL_RESULT       result of `g4-mysql` job (SRC-4 slice 3)
#   G4_MSSQL_RESULT       result of `g4-mssql` job (SRC-4 slice 3)
#   G4_MONGO_RESULT       result of `g4-mongo` job (SRC-4 slice 5)
#   G4_ORACLE_RESULT      result of `g4-oracle` job (SRC-4 slice 6)
#   G10_SSO_RESULT        result of `g10-sso` job (SEC-3 Keycloak sign-in)
#   G6_INGEST_RESULT      result of `g6-ingest` job
#   G6_INGEST_OTHER_RESULT result of `g6-ingest-other` job
#   G6_ORACLE_RESULT      result of `g6-oracle` job
#   G9_UPLOAD_RESULT      result of `g9-upload` job
#   G6_ALERTS_RESULT      result of `g6-alerts` job (SRC-17 alert delivery)
#   G6_OPS_RESULT         result of `g6-ops` job (SRC-11 connector operations)
#   G6_CREDENTIALS_RESULT result of `g6-credentials` job (SRC-12 credentials at rest)
#   G6_SECRET_MANAGERS_RESULT result of `g6-secret-managers` job (SRC-12 part B, Vault and AWS)
#   G6_OAUTH_RESULT       result of `g6-oauth` job (SRC-12 part C, OAuth sign-in against the gate's own provider)
#   G6_DISCOVERY_SCALE_RESULT result of `g6-discovery-scale` job (SRC-13, 10,000 tables listed and searched)
#   G6_SAAS_A_RESULT      result of `g6-saas-a` job (SRC-3, business-app connectors against the mock vendor server: HubSpot, Zendesk, Salesforce, GA4, ServiceNow)
#   G6_SAAS_B_RESULT      result of `g6-saas-b` job (SRC-3, the other five: Confluence, SharePoint, Dynamics 365, Workday, NetSuite; AD9 amended)
#   GOLD_EXPORT_RESULT    result of `gold-export` job
#   G8_GOVERNANCE_RESULT  result of `g8-governance` job
#   G8_TIME_TRAVEL_RESULT result of `g8-time-travel` job
#   IMAGES_RESULT         result of `images` job (builds the shared images)
#   IMAGE_SMOKE_RESULT    result of `image-smoke` job (/health on the API image)
#   SELECTED_GATE_GROUPS  the gate groups this run selected: `none`, or the
#                         selected groups between commas (`,cdc,`), from
#                         `changes.outputs.gate_groups` (OPS-25 D7). A gate
#                         job's group is not in it -> a skipped job is
#                         accepted. An unset, empty or malformed value, or a
#                         map that does not place the job in a group, requires
#                         every gate job (fail closed).
#   GATE_PATHS_FILE       the gate path map: its `group` lines say which group
#                         each gate job belongs to (OPS-25 D2). Defaults to
#                         `gate_paths.txt` next to this script.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# OPS-25 D7: the map the scope script also reads, so "which group is this job
# in?" has one answer in the whole pipeline.
GATE_PATHS="${GATE_PATHS_FILE:-${SCRIPT_DIR}/gate_paths.txt}"

run_gate() {
  local event_name="${EVENT_NAME:-}"
  local scope_rust="${SCOPE_RUST:-false}"
  local scope_frontend="${SCOPE_FRONTEND:-false}"
  local scope_dagster="${SCOPE_DAGSTER:-false}"
  local scope_stack="${SCOPE_STACK:-false}"
  local scope_docs_only="${SCOPE_DOCS_ONLY:-false}"
  local scope_all="${SCOPE_ALL:-false}"
  # OPS-24 D6: the proof excuses a gate only on the event `push`. Any other
  # value of PUSH_PROVEN -- unset, empty, a typo -- is "not proven".
  local push_proven="false"
  if [ "$event_name" = "push" ] && [ "${PUSH_PROVEN:-false}" = "true" ]; then
    push_proven="true"
  fi

  echo "=== Scope Evaluation ==="
  echo "EVENT_NAME:           $event_name"
  echo "SCOPE_RUST:           $scope_rust"
  echo "SCOPE_FRONTEND:       $scope_frontend"
  echo "SCOPE_DAGSTER:        $scope_dagster"
  echo "SCOPE_STACK:          $scope_stack"
  echo "SCOPE_DOCS_ONLY:      $scope_docs_only"
  echo "SCOPE_ALL:            $scope_all"
  echo "PUSH_PROVEN:          $push_proven"

  # OPS-24 D3, R3: the nightly run and a manual run have no base commit, so
  # their scope is `all=true`. This makes "every job is required" explicit for
  # those two events instead of depending on that scope output; a partial or
  # empty scope on either event is still required to have run everything.
  local event_requires_all="false"
  if [ "$event_name" = "schedule" ] || [ "$event_name" = "workflow_dispatch" ]; then
    event_requires_all="true"
    echo "EVENT_REQUIRES_ALL:   true (the ${event_name} run requires every job)"
  fi

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
  # OPS-24 D1: the event no longer requires anything by itself; the scope does.
  # A push and a pull request are gated identically, and `all=true` (a workflow
  # or fixture change, an unreadable base, the nightly run) is what forces
  # every job. R3: the nightly run and a manual run require every job even if
  # their scope output says otherwise.
  if [ "$scope_all" = "true" ] || [ "$event_requires_all" = "true" ]; then
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

  # OPS-24 D5, D6: a push proven to repeat a passed pull request does not need
  # the acceptance jobs or `image-smoke`. Only the event `push` together with
  # the proof excuses a gate; a failed or cancelled gate is still an error
  # whatever the proof says (see `check_job` above).
  local req_gates="$req_acceptance"
  if [ "$push_proven" = "true" ]; then
    req_gates="false"
  fi

  # OPS-25 D7: a gate job is required when its group was selected. The value
  # `changes` writes is `none` or `,<groups>,`; anything else -- unset, empty,
  # a bare `cdc`, a value from another job -- requires every gate, so a broken
  # selection can only make the gate stricter.
  local selected_groups="${SELECTED_GATE_GROUPS:-}"
  local selected_groups_ok="false"
  if [ "$selected_groups" = "none" ] || [[ "$selected_groups" =~ ^,([a-z]+,)+$ ]]; then
    selected_groups_ok="true"
  fi
  if [ "$selected_groups_ok" = "true" ]; then
    echo "SELECTED_GATE_GROUPS: ${selected_groups}"
  else
    echo "SELECTED_GATE_GROUPS: '${selected_groups}' is neither 'none' nor a comma list; every gate job is required"
  fi

  # OPS-25 D2: the job-to-group lines of the map, read once. A job that is not
  # in any `group` line, or an unreadable map, is required (D7): the gate must
  # not go green because the map lost a line.
  declare -A job_group=()
  local gate_paths_readable="false"
  if [ -r "$GATE_PATHS" ]; then
    gate_paths_readable="true"
    local map_line map_job
    local -a map_fields
    while IFS= read -r map_line || [ -n "$map_line" ]; do
      map_line="${map_line%$'\r'}"
      case "$map_line" in
        '' | '#'*) continue ;;
      esac
      read -r -a map_fields <<< "$map_line"
      if [ "${map_fields[0]:-}" != "group" ] || [ "${#map_fields[@]}" -lt 3 ]; then
        continue
      fi
      for map_job in "${map_fields[@]:2}"; do
        job_group[$map_job]="${map_fields[1]}"
      done
    done < "$GATE_PATHS"
  else
    echo "WARNING: gate path map ${GATE_PATHS} is unreadable; every gate job is required"
  fi

  # The D7 table in one place. `req_gate_for` prints its decision on stdout
  # (check_job compares it) and the trace line on stderr, so a command
  # substitution cannot capture the log line as if it were the answer.
  req_gate_for() {
    local job="$1"
    local group="${job_group[$1]:-}"
    local selected_for_job="unknown"
    local required="false"

    if [ -n "$group" ] && [ "$gate_paths_readable" = "true" ] && [ "$selected_groups_ok" = "true" ]; then
      case "$selected_groups" in
        none) selected_for_job="false" ;;
        *",${group},"*) selected_for_job="true" ;;
        *) selected_for_job="false" ;;
      esac
    fi

    if [ "$req_gates" != "true" ]; then
      # Nothing gates this run's acceptance jobs (a proven push, or no scope).
      required="false"
    elif [ "$req_all" = "true" ]; then
      # The nightly run, a manual run, `all=true`: every gate, whatever the
      # selection says.
      required="true"
    elif [ "$selected_for_job" = "false" ]; then
      required="false"
    else
      # Selected, or unknown (no group line, unreadable map, malformed value):
      # required (fail closed).
      required="true"
    fi

    echo "Job [${job}]: group=${group:-unknown}, selected=${selected_for_job}" >&2
    printf '%s\n' "$required"
  }

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
  check_job "dagster-loader-tests" "${DAGSTER_LOADER_TESTS_RESULT:-skipped}" "$req_dagster"

  # Acceptance
  check_job "g1-rustfs" "${G1_RUSTFS_RESULT:-skipped}" "$(req_gate_for g1-rustfs)"
  check_job "g2-seaweedfs" "${G2_SEAWEEDFS_RESULT:-skipped}" "$(req_gate_for g2-seaweedfs)"
  check_job "g3a-dagster" "${G3A_DAGSTER_RESULT:-skipped}" "$(req_gate_for g3a-dagster)"
  check_job "g3-maintenance" "${G3_MAINTENANCE_RESULT:-skipped}" "$(req_gate_for g3-maintenance)"
  check_job "g4-cdc" "${G4_CDC_RESULT:-skipped}" "$(req_gate_for g4-cdc)"
  check_job "g4-mysql" "${G4_MYSQL_RESULT:-skipped}" "$(req_gate_for g4-mysql)"
  check_job "g4-mssql" "${G4_MSSQL_RESULT:-skipped}" "$(req_gate_for g4-mssql)"
  check_job "g4-mongo" "${G4_MONGO_RESULT:-skipped}" "$(req_gate_for g4-mongo)"
  check_job "g4-oracle" "${G4_ORACLE_RESULT:-skipped}" "$(req_gate_for g4-oracle)"
  check_job "g10-sso" "${G10_SSO_RESULT:-skipped}" "$(req_gate_for g10-sso)"
  check_job "g6-ingest" "${G6_INGEST_RESULT:-skipped}" "$(req_gate_for g6-ingest)"
  check_job "g6-ingest-other" "${G6_INGEST_OTHER_RESULT:-skipped}" "$(req_gate_for g6-ingest-other)"
  check_job "g6-oracle" "${G6_ORACLE_RESULT:-skipped}" "$(req_gate_for g6-oracle)"
  check_job "g9-upload" "${G9_UPLOAD_RESULT:-skipped}" "$(req_gate_for g9-upload)"
  check_job "g6-alerts" "${G6_ALERTS_RESULT:-skipped}" "$(req_gate_for g6-alerts)"
  check_job "g6-ops" "${G6_OPS_RESULT:-skipped}" "$(req_gate_for g6-ops)"
  check_job "g6-credentials" "${G6_CREDENTIALS_RESULT:-skipped}" "$(req_gate_for g6-credentials)"
  check_job "g6-secret-managers" "${G6_SECRET_MANAGERS_RESULT:-skipped}" "$(req_gate_for g6-secret-managers)"
  check_job "g6-oauth" "${G6_OAUTH_RESULT:-skipped}" "$(req_gate_for g6-oauth)"
  check_job "g6-discovery-scale" "${G6_DISCOVERY_SCALE_RESULT:-skipped}" "$(req_gate_for g6-discovery-scale)"
  check_job "g6-saas-a" "${G6_SAAS_A_RESULT:-skipped}" "$(req_gate_for g6-saas-a)"
  check_job "g6-saas-b" "${G6_SAAS_B_RESULT:-skipped}" "$(req_gate_for g6-saas-b)"
  check_job "gold-export" "${GOLD_EXPORT_RESULT:-skipped}" "$(req_gate_for gold-export)"
  check_job "g8-governance" "${G8_GOVERNANCE_RESULT:-skipped}" "$(req_gate_for g8-governance)"
  check_job "g8-time-travel" "${G8_TIME_TRAVEL_RESULT:-skipped}" "$(req_gate_for g8-time-travel)"

  # Shared images and their smoke test. Required exactly when the acceptance
  # jobs are: `images` has the same `if:` as they do and is what they consume.
  # This is also what keeps the gate red when `images` fails. Every job that
  # `needs: images` is then SKIPPED, not failed, and a skip alone would read
  # as "not required"; but each of them is required here, so a skip is an
  # error on its own, and `images` failing is reported by name as well.
  # OPS-24 D5: `images` stays required on a proven push too, because it is the
  # only writer of the image layer cache the pull requests read; only
  # `image-smoke` is excused there.
  check_job "images" "${IMAGES_RESULT:-skipped}" "$req_acceptance"
  check_job "image-smoke" "${IMAGE_SMOKE_RESULT:-skipped}" "$req_gates"

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
