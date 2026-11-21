#!/usr/bin/env bash
# Choose the runner of a job that may run on the CI worker (OPS-24 D10).
#
# The switch is a repository variable, off by default, so the workflow is
# exactly as before until the owner turns it on. A fork's pull request always
# gets a hosted runner.
#
# Inputs (environment):
#   EVENT_NAME   github.event_name
#   REPO         github.repository (owner/name)
#   HEAD_REPO    github.event.pull_request.head.repo.full_name; empty on other
#                events and when the fork was deleted
#   SWITCH       repository variable CI_WORKER_COMPILE or CI_COVERAGE_ON_WORKER
#   REF          optional; the run's ref (github.ref)
#   REQUIRE_REF  optional; when set, the worker is chosen only when REF equals
#                it (coverage.yml passes `refs/heads/main`, OPS-24 R10)
#   OUTPUT_NAME  the output's name, must match ^[a-z_]+$
#   GITHUB_OUTPUT  optional; `<OUTPUT_NAME>=<json>` is appended to it
#
# Output: one line `<OUTPUT_NAME>=<json>` on stdout, the same line in
# $GITHUB_OUTPUT. The value is ["self-hosted","lakehouse-ci"] only when all of
# these hold: the switch is exactly `true` (lower case -- `TRUE`, `1` and
# `true ` do not count, OPS-24 R4), the repository is known, the event is
# `push`, `schedule`, `workflow_dispatch`, or a `pull_request` whose head
# repository IS this repository (OPS-24 R8), and, when REQUIRE_REF is set, the
# ref is the one it names (R10). Every other case -- another event, an empty
# event, an empty repository, a fork -- is ["ubuntu-latest"]. Anything
# unexpected therefore keeps the hosted runner.
#
# Exits 1 only for an unusable OUTPUT_NAME: a caller's mistake, not an answer.
set -euo pipefail

# The labels are constants, not read from a variable, so no value of a
# variable can name another runner. The staging runner carries
# `self-hosted, Linux, X64, staging` and not `lakehouse-ci`, so no switch can
# reach it (OPS-24 D10).
HOSTED_LABELS='["ubuntu-latest"]'
WORKER_LABELS='["self-hosted","lakehouse-ci"]'

OUTPUT_NAME="${OUTPUT_NAME:-}"
if ! [[ "$OUTPUT_NAME" =~ ^[a-z_]+$ ]]; then
  echo "OUTPUT_NAME must match ^[a-z_]+\$, got '${OUTPUT_NAME}'" >&2
  exit 1
fi

EVENT="${EVENT_NAME:-}"
REPO_NAME="${REPO:-}"
HEAD="${HEAD_REPO:-}"
REF_NAME="${REF:-}"
REQUIRE="${REQUIRE_REF:-}"

runner="$HOSTED_LABELS"
if [ "${SWITCH:-}" = "true" ] && [ -n "$REPO_NAME" ]; then
  if [ -n "$REQUIRE" ] && [ "$REF_NAME" != "$REQUIRE" ]; then
    # R10: a run of another ref stays hosted, even with the switch on.
    runner="$HOSTED_LABELS"
  else
    case "$EVENT" in
      push | schedule | workflow_dispatch)
        runner="$WORKER_LABELS"
        ;;
      pull_request)
        # A fork's pull request: its code is not this repository's, and the
        # repository setting that approves such a run is the owner's (D10).
        # An empty head repository (a deleted fork) is not this repository
        # either.
        if [ -n "$HEAD" ] && [ "$HEAD" = "$REPO_NAME" ]; then
          runner="$WORKER_LABELS"
        fi
        ;;
      *)
        # R8: `pull_request_target`, an empty event, anything else: hosted.
        # This script cannot tell what such a run would execute, so it keeps
        # the machine it knows.
        runner="$HOSTED_LABELS"
        ;;
    esac
  fi
fi

echo "${OUTPUT_NAME}=${runner}"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "${OUTPUT_NAME}=${runner}" >> "$GITHUB_OUTPUT"
fi
