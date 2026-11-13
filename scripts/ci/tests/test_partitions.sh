#!/usr/bin/env bash
# Tests for scripts/ci/test_partitions.sh (OPS-24 S2, OPS-30), plus the
# workflow check that `ci.yml` really reads the variable and the three
# numbers the workflow repeats (job name, matrix, sum check) come from the
# script's outputs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../test_partitions.sh"
CI_YML="${SCRIPT_DIR}/../../../.github/workflows/ci.yml"

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

# assert_partitions <label> <expected count> <expected list> [expected warning
# text]; the case's raw variable comes from CASE_PARTITIONS.
assert_partitions() {
  local label="$1" want_count="$2" want_list="$3" want_warning="${4:-}"
  : > "$OUTPUT_FILE"
  local rc=0 stdout_file="$ROOT/stdout"
  (
    PARTITIONS="${CASE_PARTITIONS-}" \
    OUTPUT_NAME=test_partitions \
    GITHUB_OUTPUT="$OUTPUT_FILE" \
    bash "$SCRIPT" >"$stdout_file" 2>&1
  ) || rc=$?

  local got_count got_list got_warning=""
  got_count="$(sed -n 's/^test_partitions=//p' "$OUTPUT_FILE" | head -1)"
  got_list="$(sed -n 's/^test_partitions_list=//p' "$OUTPUT_FILE" | head -1)"
  # The warnings are `::warning::` lines on stdout, where Actions reads them.
  if [ -n "$want_warning" ] && grep -q -- "$want_warning" "$stdout_file"; then
    got_warning="$want_warning"
  fi

  if [ "$rc" = "0" ] && [ "$got_count" = "$want_count" ] && [ "$got_list" = "$want_list" ] &&
     { [ -z "$want_warning" ] || [ -n "$got_warning" ]; }; then
    log_pass "$label"
  else
    log_fail "$label (exit=$rc count=$got_count list=$got_list warning='$got_warning')"
  fi
}

echo "=== Test partition count test suite ==="

# The variable unset or empty: today's REL-20 count, no warning.
CASE_PARTITIONS= assert_partitions "an unset variable gives the default of 3 partitions" 3 '[1,2,3]'
CASE_PARTITIONS='' assert_partitions "an empty variable gives the default of 3 partitions" 3 '[1,2,3]'

# Every accepted value, and the list the matrix expands with it.
CASE_PARTITIONS=1 assert_partitions "1 partition gives the list [1]" 1 '[1]'
CASE_PARTITIONS=2 assert_partitions "2 partitions give the list [1,2]" 2 '[1,2]'
CASE_PARTITIONS=3 assert_partitions "3 partitions give the list [1,2,3]" 3 '[1,2,3]'
CASE_PARTITIONS=4 assert_partitions "4 partitions give the list [1,2,3,4]" 4 '[1,2,3,4]'
CASE_PARTITIONS=5 assert_partitions "5 partitions give the list [1,2,3,4,5]" 5 '[1,2,3,4,5]'
CASE_PARTITIONS=6 assert_partitions "6 partitions give the list [1,2,3,4,5,6]" 6 '[1,2,3,4,5,6]'
CASE_PARTITIONS=04 assert_partitions "a leading zero is read as decimal, not octal" 4 '[1,2,3,4]'

# Anything else is the default with a warning, never a red Rust job (R4).
CASE_PARTITIONS=0 assert_partitions "0 is outside 1 to 6 and gives the default" 3 '[1,2,3]' "outside 1 to 6"
CASE_PARTITIONS=7 assert_partitions "7 is outside 1 to 6 and gives the default" 3 '[1,2,3]' "outside 1 to 6"
CASE_PARTITIONS=99 assert_partitions "99 is outside 1 to 6 and gives the default" 3 '[1,2,3]' "outside 1 to 6"
CASE_PARTITIONS=four assert_partitions "a word is not a number and gives the default" 3 '[1,2,3]' "not a whole number"
CASE_PARTITIONS=3.0 assert_partitions "a decimal is not a whole number and gives the default" 3 '[1,2,3]' "not a whole number"
CASE_PARTITIONS='4 ' assert_partitions "a trailing space is not a number and gives the default" 3 '[1,2,3]' "not a whole number"
CASE_PARTITIONS=-1 assert_partitions "a negative number gives the default" 3 '[1,2,3]' "not a whole number"

# A caller's mistake is an error, not a silent answer.
check_bad_output_name() {
  local rc=0
  (
    PARTITIONS=4 OUTPUT_NAME='Test-Partitions' GITHUB_OUTPUT="$OUTPUT_FILE" \
      bash "$SCRIPT" >/dev/null 2>&1
  ) || rc=$?
  if [ "$rc" = "1" ]; then
    log_pass "an OUTPUT_NAME that is not lowercase is refused"
  else
    log_fail "an OUTPUT_NAME that is not lowercase was not refused (exit=$rc)"
  fi
}
check_bad_output_name

# The script writes nothing outside its two outputs, and the count matches the
# length of the list for every accepted value: a partition the matrix runs is
# always one the `ci-required` sum check expects.
check_output_shape() {
  : > "$OUTPUT_FILE"
  PARTITIONS=5 OUTPUT_NAME=test_partitions GITHUB_OUTPUT="$OUTPUT_FILE" \
    bash "$SCRIPT" >/dev/null 2>&1
  local lines
  lines="$(grep -c '^test_partitions' "$OUTPUT_FILE")"
  if [ "$lines" = "2" ] &&
     [ "$(wc -l < "$OUTPUT_FILE")" = "2" ]; then
    log_pass "the script writes exactly the count and the list"
  else
    log_fail "the script wrote ${lines} output line(s) for test_partitions (want 2, nothing else)"
  fi
}
check_output_shape

# The workflow reads the variable through `env:` and exposes the script's
# outputs, so the count is chosen in one place.
test_workflow_consistency() {
  local ci_file="$1"
  python3 - "$ci_file" <<'EOF'
import re
import sys

with open(sys.argv[1]) as f:
    ci = f.read()

problems = []

# The raw repository variable crosses into the script through `env:` only.
if not re.search(r'^\s+PARTITIONS: \$\{\{ vars\.CI_TEST_PARTITIONS \}\}$', ci, re.M):
    problems.append("changes does not pass CI_TEST_PARTITIONS to the script through env:")

# `changes` exposes both outputs of the script.
for output in ("test_partitions", "test_partition_list"):
    pattern = r'^\s+' + output + r': \$\{\{ steps\.partitions\.outputs\.' + output + r' \}\}$'
    if not re.search(pattern, ci, re.M):
        problems.append("changes does not expose " + output + " from steps.partitions")

# The script runs in `changes`, before anything reads its outputs.
if not re.search(r'run: \./scripts/ci/test_partitions\.sh$', ci, re.M):
    problems.append("changes does not run scripts/ci/test_partitions.sh")

# The matrix expands the list. A literal partition array would ignore the
# variable and, worse, disagree with the sum check's loop bound.
if not re.search(r'partition: \$\{\{ fromJSON\(needs\.changes\.outputs\.test_partition_list\) \}\}', ci):
    problems.append("the test matrix does not expand needs.changes.outputs.test_partition_list")

# One leg per partition, numbered against the same count, passed in through
# env: (no expression inside the run body).
for output, source in (
    ("PARTITION", r'\$\{\{ matrix\.partition \}\}'),
    ("TEST_PARTITIONS", r'\$\{\{ needs\.changes\.outputs\.test_partitions \}\}'),
):
    pattern = r'^\s+' + output + r': ' + source + r'$'
    if not re.search(pattern, ci, re.M):
        problems.append("the nextest step does not take " + output + " from " + source)

if not re.search(r'--partition "count:\$\{PARTITION\}/\$\{TEST_PARTITIONS\}"', ci):
    problems.append("the nextest run does not partition count:PARTITION/TEST_PARTITIONS")

# The ci-required sum check loops over the same count, not over three literals
# (a fourth partition would otherwise never be read) and validates it first.
if not re.search(r'for p in \$\(seq 1 "\$TEST_PARTITIONS"\); do', ci):
    problems.append("the ci-required sum check does not loop over TEST_PARTITIONS")

if not re.search(r'^\s*\[1-6\]\) ;;$', ci, re.M):
    problems.append("the sum check does not refuse a partition count outside 1 to 6")

if problems:
    sys.stderr.write("\n".join(problems) + "\n")
    sys.exit(1)
EOF
}

if test_workflow_consistency "$CI_YML" 2>"$ROOT/workflow_err"; then
  log_pass "ci.yml passes CI_TEST_PARTITIONS to the script and exposes both of its outputs"
else
  log_fail "ci.yml does not read the partition count from the script: $(tr '\n' ' ' < "$ROOT/workflow_err")"
fi

# Negative test: a `changes` job that dropped an output is caught.
tmp_ci="$ROOT/ci_missing_output.yml"
grep -vF 'test_partition_list: ${{ steps.partitions.outputs.test_partition_list }}' "$CI_YML" > "$tmp_ci"
if ! test_workflow_consistency "$tmp_ci" >/dev/null 2>&1; then
  log_pass "the workflow check fails when changes does not expose the matrix list"
else
  log_fail "the workflow check did not fail when changes lost the matrix list output"
fi

# Negative test: a sum check that loops over a fixed count is caught.
tmp_ci="$ROOT/ci_fixed_loop.yml"
sed 's/for p in \$(seq 1 "\$TEST_PARTITIONS"); do/for p in 1 2 3; do/' "$CI_YML" > "$tmp_ci"
if ! grep -qF 'for p in 1 2 3; do' "$tmp_ci"; then
  log_fail "the negative fixture did not replace the sum loop"
elif ! test_workflow_consistency "$tmp_ci" >/dev/null 2>&1; then
  log_pass "the workflow check fails when the sum check loops over a fixed count"
else
  log_fail "the workflow check did not fail on a fixed count in the sum check"
fi

# The `ci-required` sum check itself, extracted from ci.yml and run against
# fake partition counts: the loop bound must be the count `changes` chose, and
# a missing partition, a partition that ran nothing and a total below the floor
# must each fail. This is the check that notices a partition the matrix
# dropped, or one that ran an empty archive.
SUM_SCRIPT="$ROOT/sum_check.sh"
if python3 - "$CI_YML" "$SUM_SCRIPT" <<'EOF'
import sys
import yaml

with open(sys.argv[1]) as f:
    ci = yaml.safe_load(f)
for step in ci["jobs"]["ci-required"]["steps"]:
    if str(step.get("name", "")).startswith("Check nextest partition sum"):
        with open(sys.argv[2], "w") as out:
            out.write(step["run"])
        sys.exit(0)
sys.exit(1)
EOF
then
  log_pass "the ci-required sum check step is present and extractable"
else
  log_fail "the ci-required sum check step could not be extracted from ci.yml"
fi

# assert_sum <label> <expected exit> <counts dir entries...>; the count comes
# from CASE_PARTITIONS and each entry is `N=<count>` (or `N=missing`).
assert_sum() {
  local label="$1" want="$2"
  shift 2
  local work counts entry
  work="$(mktemp -d "$ROOT/sum.XXXXXX")"
  counts="$work/counts"
  mkdir -p "$counts"
  for entry in "$@"; do
    if [ "${entry#*=}" != "missing" ]; then
      printf '%s\n' "${entry#*=}" > "$counts/partition-${entry%%=*}.txt"
    fi
  done
  : > "$work/summary"
  local rc=0
  (
    RUNNER_TEMP="$work" GITHUB_STEP_SUMMARY="$work/summary" \
      TEST_PARTITIONS="${CASE_PARTITIONS-}" \
      bash "$SUM_SCRIPT" >"$work/log" 2>&1
  ) || rc=$?
  if [ "$rc" = "$want" ]; then
    log_pass "$label"
  else
    log_fail "$label (exit=$rc, want $want): $(tr '\n' ' ' < "$work/log")"
  fi
}

CASE_PARTITIONS=3 assert_sum "the sum check passes for three full partitions" 0 1=3000 2=3000 3=3000
CASE_PARTITIONS=4 assert_sum "the sum check expects the fourth partition it is told about" 1 1=2000 2=2000 3=2000
CASE_PARTITIONS=4 assert_sum "the sum check passes for four partitions" 0 1=2000 2=2000 3=2000 4=2000
CASE_PARTITIONS=2 assert_sum "the sum check fails when a partition's count is missing" 1 1=4000 2=missing
CASE_PARTITIONS=3 assert_sum "the sum check fails when a partition ran zero tests" 1 1=3000 2=0 3=3000
CASE_PARTITIONS=3 assert_sum "the sum check fails when the total is below the floor" 1 1=2000 2=2000 3=2000
CASE_PARTITIONS= assert_sum "the sum check fails when the count is empty" 1 1=3000 2=3000 3=3000
CASE_PARTITIONS=7 assert_sum "the sum check fails when the count is outside 1 to 6" 1 1=1000 2=1000 3=1000 4=1000 5=1000 6=1000 7=1000

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
