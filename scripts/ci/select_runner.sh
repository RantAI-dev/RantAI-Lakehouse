#!/usr/bin/env bash
# Choose the runner of the small Rust jobs (OPS-24 D10).
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
#   SWITCH       repository variable CI_WORKER_COMPILE
#   OUTPUT_NAME  the output's name, must match ^[a-z_]+$
#   GITHUB_OUTPUT  optional; `<OUTPUT_NAME>=<json>` is appended to it
#
# Output: one line `<OUTPUT_NAME>=<json>` on stdout, the same line in
# $GITHUB_OUTPUT. The value is ["self-hosted","lakehouse-ci"] only when the
# switch is exactly `true`, the repository is known, and the run is not a
# pull request from another repository; every other case is
# ["ubuntu-latest"]. Anything unexpected therefore keeps the hosted runner.
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

runner="$HOSTED_LABELS"
if [ "${SWITCH:-}" = "true" ]; then
  if [ -z "$REPO_NAME" ]; then
    runner="$HOSTED_LABELS"
  elif [ "$EVENT" = "pull_request" ] && [ "$HEAD" != "$REPO_NAME" ]; then
    # A fork's pull request: its code is not this repository's, and the
    # repository setting that approves such a run is the owner's (D10).
    runner="$HOSTED_LABELS"
  else
    runner="$WORKER_LABELS"
  fi
fi

echo "${OUTPUT_NAME}=${runner}"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "${OUTPUT_NAME}=${runner}" >> "$GITHUB_OUTPUT"
fi
