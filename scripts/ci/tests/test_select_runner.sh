#!/usr/bin/env bash
# Tests for scripts/ci/select_runner.sh (OPS-24 D10), plus the workflow lint
# that keeps a self-hosted runner out of a job that did not ask for one.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../select_runner.sh"
WORKFLOWS="${SCRIPT_DIR}/../../../.github/workflows"
CI_YML="${WORKFLOWS}/ci.yml"
COVERAGE_YML="${WORKFLOWS}/coverage.yml"

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
OUTPUT_FILE="$ROOT/github_output"

HOSTED='["ubuntu-latest"]'
WORKER='["self-hosted","lakehouse-ci"]'

# assert_runner <label> <expected json>; the case's inputs come from CASE_*.
assert_runner() {
  local label="$1" want="$2"
  : > "$OUTPUT_FILE"
  local rc=0
  (
    EVENT_NAME="${CASE_EVENT:-push}" \
    REPO="${CASE_REPO-example-org/example-repo}" \
    HEAD_REPO="${CASE_HEAD_REPO-}" \
    SWITCH="${CASE_SWITCH-}" \
    OUTPUT_NAME=compile_runner \
    GITHUB_OUTPUT="$OUTPUT_FILE" \
    bash "$SCRIPT" >/dev/null 2>&1
  ) || rc=$?

  local got
  got="$(sed -n 's/^compile_runner=//p' "$OUTPUT_FILE" | head -1)"
  if [ "$rc" = "0" ] && [ "$got" = "$want" ]; then
    log_pass "$label"
  else
    log_fail "$label (exit=$rc got=$got want=$want)"
  fi
}

echo "=== Runner selection test suite ==="

# The switch unset: the workflow is exactly as before.
CASE_EVENT=pull_request assert_runner "the switch unset gives a hosted runner on a pull request" "$HOSTED"
CASE_EVENT=push assert_runner "the switch unset gives a hosted runner on a push" "$HOSTED"

# The switch on.
CASE_EVENT=pull_request CASE_HEAD_REPO=example-org/example-repo CASE_SWITCH=true \
  assert_runner "the switch on with a same-repository pull request gives the worker" "$WORKER"
CASE_EVENT=pull_request CASE_HEAD_REPO=someone-else/fork CASE_SWITCH=true \
  assert_runner "the switch on with a fork's pull request gives a hosted runner" "$HOSTED"
CASE_EVENT=pull_request CASE_HEAD_REPO= CASE_SWITCH=true \
  assert_runner "the switch on with an empty head repository on a pull request gives a hosted runner" "$HOSTED"
CASE_EVENT=push CASE_SWITCH=true assert_runner "the switch on a push gives the worker" "$WORKER"
CASE_EVENT=schedule CASE_SWITCH=true assert_runner "the switch on a scheduled run gives the worker" "$WORKER"
CASE_EVENT=workflow_dispatch CASE_SWITCH=true \
  assert_runner "the switch on a manual run gives the worker" "$WORKER"

# Anything but the exact string `true` keeps the hosted runner.
CASE_EVENT=push CASE_SWITCH=TRUE assert_runner "TRUE gives a hosted runner" "$HOSTED"
CASE_EVENT=push CASE_SWITCH=1 assert_runner "1 gives a hosted runner" "$HOSTED"
CASE_EVENT=push CASE_SWITCH=yes assert_runner "yes gives a hosted runner" "$HOSTED"
CASE_EVENT=push CASE_SWITCH=true CASE_REPO= \
  assert_runner "the switch on with an empty repository gives a hosted runner" "$HOSTED"

# The caller's name is validated: a bad one is an error, not an answer.
BAD_NAME_RC=0
OUTPUT_NAME="Not-Valid" bash "$SCRIPT" >/dev/null 2>&1 || BAD_NAME_RC=$?
if [ "$BAD_NAME_RC" = "1" ]; then
  log_pass "an OUTPUT_NAME outside the lowercase-name pattern exits 1"
else
  log_fail "an OUTPUT_NAME outside the lowercase-name pattern did not exit 1 (exit=$BAD_NAME_RC)"
fi

# ── The workflow lint ───────────────────────────────────────────────────────
# For every job that can land on a runner other than `ubuntu-latest`:
#   (a) its first step refuses a self-hosted runner that is not the CI worker;
#   (b) every step that runs `sudo` or `prune`, and every `Swatinem/rust-cache`
#       step, is guarded to a hosted runner;
#   (c) a workflow that names `lakehouse-ci` has no `pull_request` trigger.
workflow_lint() {
  python3 - "$@" <<'PY'
import sys

import yaml

REFUSE = "Refuse a self-hosted runner that is not the CI worker"
problems = []

for path in sys.argv[1:]:
    with open(path) as f:
        text = f.read()
    doc = yaml.safe_load(text)
    for name, job in (doc.get("jobs") or {}).items():
        runs_on = job.get("runs-on")
        if runs_on == "ubuntu-latest":
            continue
        if isinstance(runs_on, list) and runs_on == ["self-hosted", "staging"]:
            continue
        steps = job.get("steps") or []
        if not steps:
            problems.append("%s:%s: no steps" % (path, name))
            continue
        first = steps[0]
        if first.get("name") != REFUSE:
            problems.append("%s:%s: the first step is not %r" % (path, name, REFUSE))
        if str(first.get("if", "")) != "runner.environment == 'self-hosted'":
            problems.append("%s:%s: the refuse step's if is not runner.environment == 'self-hosted'" % (path, name))
        for index, step in enumerate(steps):
            run = str(step.get("run", ""))
            uses = str(step.get("uses", ""))
            guarded = "sudo" in run or "prune" in run or "Swatinem/rust-cache" in uses
            if guarded and "runner.environment == 'github-hosted'" not in str(step.get("if", "")):
                problems.append("%s:%s: step %d (%r) needs if: runner.environment == 'github-hosted'"
                                % (path, name, index, step.get("name")))

    triggers = doc.get("on")
    if triggers is None:
        triggers = doc.get(True)
    if isinstance(triggers, dict):
        keys = list(triggers.keys())
    elif isinstance(triggers, str):
        keys = [triggers]
    elif isinstance(triggers, list):
        keys = list(triggers)
    else:
        keys = []
    if "lakehouse-ci" in text and "pull_request" in keys:
        problems.append("%s: names lakehouse-ci although it has a pull_request trigger" % path)

for problem in problems:
    sys.stderr.write(problem + "\n")
sys.exit(1 if problems else 0)
PY
}

if workflow_lint "$CI_YML" "$COVERAGE_YML" >/dev/null 2>&1; then
  log_pass "every job that can run self-hosted refuses a foreign runner and guards its cache steps"
else
  log_fail "a job that can run self-hosted lacks the refuse step or a hosted-only guard"
  workflow_lint "$CI_YML" "$COVERAGE_YML" 2>&1 | sed 's/^/  /' || true
fi

# Negative (a): a dynamic job whose first step is not the refuse step.
copy="$ROOT/ci-no-refuse.yml"
python3 - "$CI_YML" "$copy" <<'PY'
import sys

import yaml

with open(sys.argv[1]) as f:
    ci = yaml.safe_load(f)
steps = ci["jobs"]["clippy"]["steps"]
ci["jobs"]["clippy"]["steps"] = [
    s for s in steps
    if s.get("name") != "Refuse a self-hosted runner that is not the CI worker"
]
with open(sys.argv[2], "w") as f:
    yaml.dump(ci, f)
PY
if ! workflow_lint "$copy" >/dev/null 2>&1; then
  log_pass "the lint fails when a job that can run self-hosted lacks the refuse step"
else
  log_fail "the lint passed without the refuse step"
fi

# Negative (b): a `rust-cache` step in a dynamic job without the hosted guard.
copy="$ROOT/ci-unguarded-cache.yml"
python3 - "$CI_YML" "$copy" <<'PY'
import sys

import yaml

with open(sys.argv[1]) as f:
    ci = yaml.safe_load(f)
for step in ci["jobs"]["build"]["steps"]:
    if "Swatinem/rust-cache" in str(step.get("uses", "")):
        step.pop("if", None)
with open(sys.argv[2], "w") as f:
    yaml.dump(ci, f)
PY
if ! workflow_lint "$copy" >/dev/null 2>&1; then
  log_pass "the lint fails when a rust-cache step in a self-hosted-capable job has no hosted guard"
else
  log_fail "the lint passed with an unguarded rust-cache step"
fi

# Negative (c): a workflow with a pull_request trigger that names the worker.
copy="$ROOT/ci-names-worker.yml"
cp "$CI_YML" "$copy"
printf '\n# lakehouse-ci\n' >> "$copy"
if ! workflow_lint "$copy" >/dev/null 2>&1; then
  log_pass "the lint fails when a workflow with a pull_request trigger names lakehouse-ci"
else
  log_fail "the lint passed with lakehouse-ci named in a pull_request workflow"
fi

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
