#!/usr/bin/env bash
# Print the API log lines for every "Reference: <id>" a gate's output quotes.
#
# Why: SEC-11 keeps an upstream error's text out of every response; the
# response carries a fixed sentence and "Reference: <10 hex>", and the raw
# text is in the lakehouse-api log under that id. A gate runs in a container
# with no docker socket, so it cannot read that log; a failed CI job needs the
# lines without scrolling a whole `docker compose logs` dump. Run this after a
# failed gate (PR #105 CI fix). It reads, it never decides pass or fail.
#
# Usage: api_log_for_references.sh <gate-output-file> <docker compose args...>
#   e.g. api_log_for_references.sh "$RUNNER_TEMP/g6-run.log" -p g6ci
set -uo pipefail

run_log="${1:-}"
shift || true
if [ -z "$run_log" ] || [ ! -f "$run_log" ]; then
  echo "no gate output at '${run_log}'; nothing to look up"
  exit 0
fi

refs="$(grep -oE 'Reference: [0-9a-f]{10}' "$run_log" | awk '{print $2}' | sort -u)"
if [ -z "$refs" ]; then
  echo "the gate output quotes no API reference id"
  exit 0
fi

logs="$(docker compose "$@" logs --no-color lakehouse-api 2>&1)" || {
  echo "could not read the lakehouse-api log"
  exit 0
}
for ref in $refs; do
  echo "== API log lines for reference $ref =="
  printf '%s\n' "$logs" | grep -F "$ref" || echo "(none: the API did not log this reference)"
done
