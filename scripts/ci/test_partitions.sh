#!/usr/bin/env bash
# Choose how many partitions the Rust test run is split into (OPS-24 S2, OPS-30).
#
# Before OPS-30 the partition count was the literal `3` in `ci.yml`, repeated in
# the job name, the matrix and the `ci-required` sum check. It is now one answer
# from this script: the job matrix expands the list and the sum check loops over
# the count, so the two cannot drift apart.
#
# Inputs (environment):
#   PARTITIONS     the raw repository variable CI_TEST_PARTITIONS. A whole
#                  number from 1 to 6. Unset or empty means 3, the REL-20
#                  count; any other value is a warning and 3 as well, the same
#                  rule CI_PUSH_FULL and ALL_GATES_LABEL follow (OPS-24 R4):
#                  one variable cannot turn every Rust run red, and the log
#                  says what was ignored. The count is compared here and
#                  nowhere else.
#   OUTPUT_NAME    the outputs' name, must match ^[a-z_]+$. Two outputs are
#                  written: `<OUTPUT_NAME>` is the count, which the
#                  `ci-required` sum check loops over, and
#                  `<OUTPUT_NAME>_list` is a JSON array `[1,...,M]`, which the
#                  `test` job's matrix expands.
#   GITHUB_OUTPUT  optional; both lines are appended to it
#
# Output: the two lines on stdout and the same lines in $GITHUB_OUTPUT.
#
# Exits 1 only for an unusable OUTPUT_NAME: a caller's mistake, not an answer.
set -euo pipefail

MIN_PARTITIONS=1
MAX_PARTITIONS=6
DEFAULT_PARTITIONS=3

OUTPUT_NAME="${OUTPUT_NAME:-}"
if ! [[ "$OUTPUT_NAME" =~ ^[a-z_]+$ ]]; then
  echo "OUTPUT_NAME must match ^[a-z_]+\$, got '${OUTPUT_NAME}'" >&2
  exit 1
fi

raw="${PARTITIONS:-}"
partitions="$DEFAULT_PARTITIONS"
if [ -n "$raw" ]; then
  case "$raw" in
    # Anything with a character outside 0-9: `4 `, `four`, `3.0`, `-1`, `TRUE`.
    *[!0-9]*)
      echo "::warning::CI_TEST_PARTITIONS is '${raw}', not a whole number between ${MIN_PARTITIONS} and ${MAX_PARTITIONS}; using ${DEFAULT_PARTITIONS}"
      ;;
    *)
      # `10#` so a leading zero is decimal, not octal.
      value=$((10#$raw))
      if [ "$value" -lt "$MIN_PARTITIONS" ] || [ "$value" -gt "$MAX_PARTITIONS" ]; then
        echo "::warning::CI_TEST_PARTITIONS is '${raw}', outside ${MIN_PARTITIONS} to ${MAX_PARTITIONS}; using ${DEFAULT_PARTITIONS}"
      else
        partitions="$value"
      fi
      ;;
  esac
fi

# The matrix list and the count are built from the same number, so a partition
# the matrix runs is always one the sum check expects.
list=""
for ((index = 1; index <= partitions; index++)); do
  list="${list:+${list},}${index}"
done
list="[${list}]"

echo "Test partitions: ${partitions} (CI_TEST_PARTITIONS=${raw:-unset})"
echo "${OUTPUT_NAME}=${partitions}"
echo "${OUTPUT_NAME}_list=${list}"
if [ -n "${GITHUB_OUTPUT:-}" ]; then
  echo "${OUTPUT_NAME}=${partitions}" >> "$GITHUB_OUTPUT"
  echo "${OUTPUT_NAME}_list=${list}" >> "$GITHUB_OUTPUT"
fi
