#!/usr/bin/env bash
# Tests for scripts/ci/detect_push_proof.sh (OPS-24 D4).
#
# No network: a fake `gh` is put first on PATH and answers from fixtures the
# case writes. The temporary repository is its own `origin`, with
# `refs/pull/7/head` created by hand, so the script's real
# `git fetch origin refs/pull/7/head` works.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../detect_push_proof.sh"
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
REPO_DIR="$ROOT/repo"
FAKE_BIN="$ROOT/bin"
mkdir -p "$REPO_DIR" "$FAKE_BIN"

GH_MARKER="$ROOT/gh.calls"
GH_RC="$ROOT/gh.rc"
PULLS_JSON="$ROOT/pulls.json"
CHECK_RUNS="$ROOT/check_runs.json"
OUTPUT_FILE="$ROOT/github_output"
export GH_MARKER GH_RC PULLS_JSON CHECK_RUNS

# The fake `gh`: record the call, honour an exit code the case asked for, then
# answer the two endpoints the script uses from the fixture files.
cat > "$FAKE_BIN/gh" <<'EOF'
#!/usr/bin/env bash
echo "$*" >> "$GH_MARKER"
if [ -f "$GH_RC" ]; then
  exit "$(cat "$GH_RC")"
fi
for arg in "$@"; do
  case "$arg" in
    */pulls*) cat "$PULLS_JSON"; exit 0 ;;
    */check-runs*) cat "$CHECK_RUNS"; exit 0 ;;
  esac
done
exit 1
EOF
chmod +x "$FAKE_BIN/gh"

# ── Fixtures ────────────────────────────────────────────────────────────────
# M0 is main's base. P is one commit on a branch off it. H_merge is the merge
# commit of that branch, H_squash the squash of it (same tree as P), M1 moves
# main on, H_stale merges the unchanged branch into M1 (first parent is not an
# ancestor of P), and H_evil has M0 and P as parents but another tree.
(
  cd "$REPO_DIR"
  git init -q -b main
  git config user.email "ci-test@example.com"
  git config user.name "CI Test"

  echo base > base.txt
  git add base.txt
  git commit -q -m "M0"
  git tag m0

  git branch feature
  git switch -q feature
  echo feature > feature.txt
  git add feature.txt
  git commit -q -m "P"
  git tag p
  git switch -q main

  git merge --squash feature >/dev/null
  git commit -q -m "H_squash"
  git tag h_squash

  git reset -q --hard m0
  git merge --no-ff -q -m "H_merge" feature
  git tag h_merge

  git reset -q --hard m0
  echo second > second.txt
  git add second.txt
  git commit -q -m "M1"
  git tag m1
  git merge --no-ff -q -m "H_stale" feature
  git tag h_stale

  git switch -q --detach m0
  echo evil > evil.txt
  git add evil.txt
  git commit -q -m "a tree that is not the pull-request head's"
  git tag evil_tree_commit
  git switch -q main
)

M0_SHA="$(git -C "$REPO_DIR" rev-parse m0)"
P_SHA="$(git -C "$REPO_DIR" rev-parse p)"
H_MERGE="$(git -C "$REPO_DIR" rev-parse h_merge)"
H_SQUASH="$(git -C "$REPO_DIR" rev-parse h_squash)"
H_STALE="$(git -C "$REPO_DIR" rev-parse h_stale)"
M1_SHA="$(git -C "$REPO_DIR" rev-parse m1)"

H_EVIL="$(git -C "$REPO_DIR" commit-tree \
  "$(git -C "$REPO_DIR" rev-parse evil_tree_commit^{tree})" \
  -p "$M0_SHA" -p "$P_SHA" -m "H_evil")"

git -C "$REPO_DIR" remote add origin "$REPO_DIR"
git -C "$REPO_DIR" update-ref refs/pull/7/head "$P_SHA"

# ── Fixture writers ─────────────────────────────────────────────────────────
# write_check_runs <spec>… where a spec is name|conclusion|status|completed_at|suite|app.
write_check_runs() {
  python3 - "$CHECK_RUNS" "$@" <<'PY'
import json, sys

out = sys.argv[1]
runs = []
for i, spec in enumerate(sys.argv[2:], start=1):
    name, conclusion, status, completed_at, suite, app = spec.split("|")
    runs.append({
        "id": 1000 + i,
        "name": name,
        "status": status,
        "conclusion": conclusion or None,
        "completed_at": completed_at or None,
        "app": {"slug": app},
        "check_suite": {"id": int(suite)},
    })
with open(out, "w") as f:
    json.dump({"total_count": len(runs), "check_runs": runs}, f)
PY
}

# write_pull <merge_commit_sha> <head_sha> <base_ref> <number> <merged_at>
write_pull() {
  python3 - "$PULLS_JSON" "$@" <<'PY'
import json, sys

out, merge_sha, head_sha, base_ref, number, merged_at = sys.argv[1:7]
entry = {
    "number": int(number),
    "merge_commit_sha": merge_sha,
    "head": {"sha": head_sha},
    "base": {"ref": base_ref},
    "merged_at": None if merged_at == "null" else merged_at,
}
with open(out, "w") as f:
    json.dump([entry], f)
PY
}

write_no_pulls() {
  printf '[]\n' > "$PULLS_JSON"
}

rust_run() { echo "Rust · fmt|$1|${2:-completed}|${3:-2026-11-13T10:00:00Z}|${4:-77}|github-actions"; }
acc_run() { echo "Images · build lakehouse-api + dagster once|$1|${2:-completed}|${3:-2026-11-13T10:00:00Z}|${4:-77}|github-actions"; }
gate_run() { echo "ci-required|$1|${2:-completed}|${3:-2026-11-13T10:10:00Z}|${4:-77}|${5:-github-actions}"; }

# ── Runner ──────────────────────────────────────────────────────────────────
run_proof() {
  : > "$GH_MARKER"
  : > "$OUTPUT_FILE"
  PROOF_RC=0
  PROOF_OUT="$(
    cd "$REPO_DIR"
    EVENT_NAME="${CASE_EVENT:-push}" \
    PUSH_SHA="${CASE_SHA:-$H_MERGE}" \
    REPO="${CASE_REPO:-example-org/example-repo}" \
    BASE_BRANCH="${CASE_BASE:-main}" \
    SCOPE_RUST="${CASE_RUST:-true}" \
    SCOPE_DAGSTER="${CASE_DAGSTER:-false}" \
    SCOPE_STACK="${CASE_STACK:-false}" \
    SCOPE_ALL="${CASE_ALL:-false}" \
    GITHUB_OUTPUT="$OUTPUT_FILE" \
    PATH="$FAKE_BIN:$PATH" \
    bash "$SCRIPT" 2>&1
  )" || PROOF_RC=$?
}

# assert_proof <label> <proven> <proven_by> <proven_pr> <gh: called|not called> [reason substring]
assert_proof() {
  local label="$1" want_proven="$2" want_by="$3" want_pr="$4" want_gh="$5" want_reason="${6:-}"
  run_proof

  local got_proven got_by got_pr got_reason calls gh_state ok=true
  got_proven="$(sed -n 's/^proven=//p' "$OUTPUT_FILE" | head -1)"
  got_by="$(sed -n 's/^proven_by=//p' "$OUTPUT_FILE" | head -1)"
  got_pr="$(sed -n 's/^proven_pr=//p' "$OUTPUT_FILE" | head -1)"
  got_reason="$(sed -n 's/^reason=//p' "$OUTPUT_FILE" | head -1)"
  calls="$(wc -l < "$GH_MARKER" | tr -d ' ')"
  gh_state="not called"
  [ "$calls" = "0" ] || gh_state="called"

  [ "$PROOF_RC" = "0" ] || ok=false
  [ "$got_proven" = "$want_proven" ] || ok=false
  [ "$got_by" = "$want_by" ] || ok=false
  [ "$got_pr" = "$want_pr" ] || ok=false
  [ "$gh_state" = "$want_gh" ] || ok=false
  if [ -n "$want_reason" ]; then
    case "$got_reason" in
      *"$want_reason"*) ;;
      *) ok=false ;;
    esac
  fi

  if [ "$ok" = true ]; then
    log_pass "$label"
  else
    log_fail "$label (exit=$PROOF_RC proven=$got_proven proven_by=$got_by proven_pr=$got_pr gh=$gh_state reason=$got_reason)"
  fi
}

echo "=== Push-proof test suite ==="

# 1. Only a push repeats a pull request's run.
CASE_EVENT=pull_request assert_proof \
  "a pull-request event is not proven and gh is never called" \
  "false" "" "" "not called" "not a push event"

# 2. Nothing heavy in the push's scope: nothing to skip.
CASE_RUST=false CASE_DAGSTER=false CASE_STACK=false CASE_ALL=false assert_proof \
  "a push whose scope has no Rust or acceptance job is not proven and gh is never called" \
  "false" "" "" "not called" "no Rust test or acceptance job in this push's scope"

# 3. The merge-commit path.
write_check_runs "$(rust_run success)" "$(acc_run success)" "$(gate_run success)"
assert_proof \
  "a merge commit whose second parent passed ci-required with both groups run is proven by that parent" \
  "true" "$P_SHA" "" "called"

# 4-9. What the ci-required check run on the head must say.
write_check_runs "$(rust_run success)" "$(acc_run success)" "$(gate_run failure)"
assert_proof "a failed ci-required on the head is not proven" \
  "false" "" "" "called" "the latest ci-required on the pull-request head ended failure"

write_check_runs "$(rust_run success)" "$(acc_run success)" "$(gate_run "" in_progress "")"
assert_proof "a ci-required still in progress on the head is not proven" \
  "false" "" "" "called" "has not completed"

write_check_runs "$(rust_run success)" "$(acc_run success)"
assert_proof "a head with no ci-required check run is not proven" \
  "false" "" "" "called" "no ci-required check run on the pull-request head"

write_check_runs "$(rust_run success)" "$(acc_run success)" \
  "$(gate_run success completed 2026-11-13T10:00:00Z)" \
  "$(gate_run failure completed 2026-11-13T10:10:00Z)"
assert_proof "an older success followed by a newer failure is not proven" \
  "false" "" "" "called" "the latest ci-required on the pull-request head ended failure"

write_check_runs "$(rust_run success)" "$(acc_run success)" \
  "$(gate_run failure completed 2026-11-13T10:00:00Z)" \
  "$(gate_run success completed 2026-11-13T10:10:00Z)"
assert_proof "an older failure followed by a newer success is proven" \
  "true" "$P_SHA" "" "called"

write_check_runs "$(rust_run success)" "$(acc_run success)" \
  "$(gate_run success completed 2026-11-13T10:10:00Z 77 other-app)"
assert_proof "a ci-required made by another app than GitHub Actions is not proven" \
  "false" "" "" "called" "no ci-required check run on the pull-request head"

# 10-13. The group each push needs has to have run in that suite.
write_check_runs "$(acc_run success)" "$(gate_run success)"
assert_proof "a Rust scope with the Rust group skipped on the head is not proven" \
  "false" "" "" "called" "the pull-request run did not run the Rust jobs"

write_check_runs "$(acc_run skipped)" "$(gate_run success)"
CASE_RUST=false CASE_STACK=true assert_proof "an acceptance scope with the images job skipped on the head is not proven" \
  "false" "" "" "called" "the pull-request run did not run the acceptance jobs"

write_check_runs "$(rust_run success completed 2026-11-13T10:00:00Z 78)" "$(acc_run success)" "$(gate_run success)"
assert_proof "a sentinel that succeeded in another check suite is not proven" \
  "false" "" "" "called" "the pull-request run did not run the Rust jobs"

write_check_runs "$(acc_run success)" "$(gate_run success)"
CASE_RUST=false CASE_DAGSTER=true assert_proof "a Dagster-only scope needs only the acceptance group" \
  "true" "$P_SHA" "" "called"

# 14-15. The two tree conditions.
CASE_SHA="$H_STALE" assert_proof \
  "a merge whose first parent is not an ancestor of the head is not proven and gh is never called" \
  "false" "" "" "not called" "the pull-request head did not contain the tip of main it was merged into"

CASE_SHA="$H_EVIL" assert_proof "a merge commit whose tree differs from the head's tree is not proven" \
  "false" "" "" "not called" "the pushed tree differs from the pull-request head's tree"

# 16-17. A failing API and a body that is not JSON: still exit 0, still not proven.
write_check_runs "$(rust_run success)" "$(acc_run success)" "$(gate_run success)"
echo 1 > "$GH_RC"
assert_proof "a failing gh is not proven and the script still exits 0" \
  "false" "" "" "called" "the GitHub API could not be asked for the check runs"
rm -f "$GH_RC"

printf 'this is not json\n' > "$CHECK_RUNS"
assert_proof "a body that is not JSON is not proven and the script still exits 0" \
  "false" "" "" "called" "not JSON this script can read"

# 18-21. The squash path: the API names the merge commit, then the head is
# fetched as refs/pull/<n>/head and has to be that commit.
write_check_runs "$(rust_run success)" "$(acc_run success)" "$(gate_run success)"
write_pull "$H_SQUASH" "$P_SHA" main 7 "2026-11-13T09:00:00Z"
CASE_SHA="$H_SQUASH" assert_proof \
  "a squash commit whose pull request passed is proven by that pull request's head, with its number" \
  "true" "$P_SHA" "7" "called"

write_pull "$M1_SHA" "$P_SHA" main 7 "2026-11-13T09:00:00Z"
CASE_SHA="$H_SQUASH" assert_proof "a single-parent commit that no merged pull request names is not proven" \
  "false" "" "" "called" "no merged pull request has this commit as its merge commit"

write_pull "$H_SQUASH" "$P_SHA" "lakehouse/x" 7 "2026-11-13T09:00:00Z"
CASE_SHA="$H_SQUASH" assert_proof "a squash whose pull request targeted another base is not proven" \
  "false" "" "" "called" "no merged pull request has this commit as its merge commit"

write_pull "$H_SQUASH" "$M1_SHA" main 7 "2026-11-13T09:00:00Z"
CASE_SHA="$H_SQUASH" assert_proof "a squash whose fetched head is not the head the API named is not proven" \
  "false" "" "" "called" "the fetched pull-request head is not the head the API named"

write_no_pulls
CASE_SHA="$H_SQUASH" assert_proof "a squash with no pull request at all is not proven" \
  "false" "" "" "called" "no merged pull request has this commit as its merge commit"

# 22. Input validation, before any API call.
CASE_SHA="not-a-sha" assert_proof "a malformed PUSH_SHA is not proven" \
  "false" "" "" "not called" "not a 40-hex-digit SHA"

# ── Consistency: the two sentinels equal the job names in ci.yml ─────────────
test_consistency() {
  local proof_file="$1" ci_file="$2"
  python3 - "$proof_file" "$ci_file" <<'PY'
import re
import sys

import yaml

proof_file, ci_file = sys.argv[1], sys.argv[2]
text = open(proof_file).read()

def const(name):
    m = re.search(r'^%s="(.*)"$' % name, text, re.M)
    if not m:
        sys.stderr.write("constant %s not found\n" % name)
        sys.exit(3)
    return m.group(1)

with open(ci_file) as f:
    ci = yaml.safe_load(f)

for const_name, job in (("RUST_SENTINEL", "fmt"), ("ACCEPTANCE_SENTINEL", "images")):
    want = ci.get("jobs", {}).get(job, {}).get("name")
    got = const(const_name)
    if got != want:
        sys.stderr.write("%s=%r != jobs.%s.name=%r\n" % (const_name, got, job, want))
        sys.exit(1)
sys.exit(0)
PY
}

if test_consistency "$SCRIPT" "$CI_YML" >/dev/null 2>&1; then
  log_pass "the two sentinels equal the fmt and images job names in ci.yml"
else
  log_fail "a sentinel does not equal the fmt or images job name in ci.yml"
fi

(
  tmp_proof="$ROOT/proof-renamed.sh"
  sed 's/^RUST_SENTINEL=".*"/RUST_SENTINEL="Rust · renamed"/' "$SCRIPT" > "$tmp_proof"
  if ! test_consistency "$tmp_proof" "$CI_YML" >/dev/null 2>&1; then
    log_pass "the consistency check fails when a sentinel no longer matches its job name"
  else
    log_fail "the consistency check passed with a renamed sentinel"
  fi
)

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
