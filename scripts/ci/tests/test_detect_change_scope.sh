#!/usr/bin/env bash
# Tests for scripts/ci/detect_change_scope.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../detect_change_scope.sh"
# OPS-25: the real gate path map, so every case below checks the rule CI acts on.
GATE_MAP="${SCRIPT_DIR}/../gate_paths.txt"

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

REPO_DIR="$(mktemp -d)"
trap 'rm -rf "$REPO_DIR"' EXIT

(
  cd "$REPO_DIR"
  git init -q -b main
  git config user.email "ci-test@example.com"
  git config user.name "CI Test"

  mkdir -p docs GTM src/lib src/components public/geo rust/crates dagster ops/g6 ops/g9 ops/fixtures ops/release/fixtures/sigstore scripts .github/workflows

  : > docs/test.md
  : > GTM/test.md
  : > README.md
  : > LICENSE
  : > NOTICE
  : > src/components/button.tsx
  : > src/lib/dashboard-specs.ts
  : > public/robots.txt
  : > public/geo/id-provinces.geojson
  : > package.json
  : > bun.lock
  : > bunfig.toml
  : > tsconfig.json
  : > next.config.ts
  : > eslint.config.mjs
  : > postcss.config.mjs
  : > components.json
  : > Dockerfile.frontend
  : > rust/crates/lib.rs
  : > dagster/pipeline.py
  : > docker-compose.yml
  : > ops/g6/test.py
  : > ops/g9/upload_test.py
  : > ops/fixtures/upload_load_failure_reasons.json
  : > ops/release/release.pub
  : > ops/release/fixtures/sigstore/release.json
  mkdir -p scripts/ci
  : > scripts/ci/detect_change_scope.sh
  : > scripts/compose.sh
  : > .env.example
  : > .github/workflows/ci.yml

  git add .
  git commit -q -m "initial base commit"
)

BASE_SHA="$(git -C "$REPO_DIR" rev-parse HEAD)"

run_case() {
  local files=("$@")
  (
    cd "$REPO_DIR"
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    for f in "${files[@]}"; do
      mkdir -p "$(dirname "$f")"
      echo "edit-$(date +%s%N)" >> "$f"
      git add "$f"
    done
    git commit -q -m "case commit"
  )

  local output="$REPO_DIR/github_output"
  : > "$output"

  (
    cd "$REPO_DIR"
    EVENT_NAME="${CASE_EVENT:-pull_request}" \
    FORCE_ALL="${CASE_FORCE_ALL:-}" \
    ALL_GATES_LABEL="${CASE_ALL_GATES_LABEL:-}" \
    GATE_PATHS_FILE="${CASE_GATE_PATHS_FILE:-}" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  )

  cat "$output"
}

assert_case() {
  local label="$1"
  local expected_rust="$2"
  local expected_frontend="$3"
  local expected_dagster="$4"
  local expected_stack="$5"
  local expected_docs_only="$6"
  local expected_all="$7"
  shift 7
  local files=("$@")

  local output
  output="$(run_case "${files[@]}")"

  local r f d s doc a
  r="$(grep "^rust=" <<< "$output" | cut -d= -f2)"
  f="$(grep "^frontend=" <<< "$output" | cut -d= -f2)"
  d="$(grep "^dagster=" <<< "$output" | cut -d= -f2)"
  s="$(grep "^stack=" <<< "$output" | cut -d= -f2)"
  doc="$(grep "^docs_only=" <<< "$output" | cut -d= -f2)"
  a="$(grep "^all=" <<< "$output" | cut -d= -f2)"

  if [ "$r" = "$expected_rust" ] && \
     [ "$f" = "$expected_frontend" ] && \
     [ "$d" = "$expected_dagster" ] && \
     [ "$s" = "$expected_stack" ] && \
     [ "$doc" = "$expected_docs_only" ] && \
     [ "$a" = "$expected_all" ]; then
    log_pass "$label"
  else
    log_fail "$label (got rust=$r, frontend=$f, dagster=$d, stack=$s, docs_only=$doc, all=$a)"
  fi
}

# OPS-25 T1: the gate groups a diff selects. `assert_groups` reuses the same
# throwaway repository as `assert_case` and reads only the `gate_groups` output.
assert_groups() {
  local label="$1"
  local expected="$2"
  shift 2

  local output got
  output="$(run_case "$@")"
  got="$(grep "^gate_groups=" <<< "$output" | cut -d= -f2-)"

  if [ "$got" = "$expected" ]; then
    log_pass "$label"
  else
    log_fail "$label (got gate_groups=${got}, want ${expected})"
  fi
}

ALL_GROUPS=",storage,pipelines,cdc,ingest,saas,secrets,alerts,upload,governance,"

echo "=== Change-scope test suite ==="

# Docs-only tests
assert_case "docs/test.md is docs-only" "false" "false" "false" "false" "true" "false" "docs/test.md"
assert_case "GTM/test.md is docs-only" "false" "false" "false" "false" "true" "false" "GTM/test.md"
assert_case "README.md is docs-only" "false" "false" "false" "false" "true" "false" "README.md"
assert_case "LICENSE is docs-only" "false" "false" "false" "false" "true" "false" "LICENSE"
assert_case "NOTICE is docs-only" "false" "false" "false" "false" "true" "false" "NOTICE"

# Frontend-only tests
assert_case "src/components/button.tsx is frontend-only" "false" "true" "false" "false" "false" "false" "src/components/button.tsx"
assert_case "public/robots.txt is frontend-only" "false" "true" "false" "false" "false" "false" "public/robots.txt"
assert_case "package.json is frontend-only" "false" "true" "false" "false" "false" "false" "package.json"
assert_case "bun.lock is frontend-only" "false" "true" "false" "false" "false" "false" "bun.lock"
assert_case "bunfig.toml is frontend-only" "false" "true" "false" "false" "false" "false" "bunfig.toml"
assert_case "tsconfig.json is frontend-only" "false" "true" "false" "false" "false" "false" "tsconfig.json"
assert_case "next.config.ts is frontend-only" "false" "true" "false" "false" "false" "false" "next.config.ts"
assert_case "eslint.config.mjs is frontend-only" "false" "true" "false" "false" "false" "false" "eslint.config.mjs"
assert_case "postcss.config.mjs is frontend-only" "false" "true" "false" "false" "false" "false" "postcss.config.mjs"
assert_case "components.json is frontend-only" "false" "true" "false" "false" "false" "false" "components.json"
assert_case "Dockerfile.frontend is frontend-only" "false" "true" "false" "false" "false" "false" "Dockerfile.frontend"

# Cross-boundary: dashboard-specs.ts sets rust AND frontend
assert_case "dashboard-specs.ts sets rust and frontend" "true" "true" "false" "false" "false" "false" "src/lib/dashboard-specs.ts"

# Rust tests
assert_case "rust/crates/lib.rs is rust-only" "true" "false" "false" "false" "false" "false" "rust/crates/lib.rs"

# Dagster tests
assert_case "dagster/pipeline.py is dagster-only" "false" "false" "true" "false" "false" "false" "dagster/pipeline.py"

# Stack tests
# docker-compose.yml sets rust and stack (embedded for demo_connector_compose_properties)
assert_case "docker-compose.yml sets rust and stack" "true" "false" "false" "true" "false" "false" "docker-compose.yml"
assert_case "ops/g6/test.py is stack-only" "false" "false" "false" "true" "false" "false" "ops/g6/test.py"
# SRC-9 task 7: the upload gate's own files select the acceptance jobs (g9-upload) through stack.
assert_case "ops/g9/upload_test.py is stack-only" "false" "false" "false" "true" "false" "false" "ops/g9/upload_test.py"
assert_case "scripts/compose.sh is stack-only" "false" "false" "false" "true" "false" "false" "scripts/compose.sh"
# OPS-24 D1: .env.example is compiled into lakehousectl
# (rust/crates/lakehousectl/src/secrets.rs:66, include_str!), so it sets rust
# as well as stack. Wider than before, not weaker.
assert_case ".env.example sets rust and stack" "true" "false" "false" "true" "false" "false" ".env.example"

# Cross-cutting fixture tests
assert_case "ops/fixtures forces all" "true" "true" "true" "true" "false" "true" "ops/fixtures/upload_load_failure_reasons.json"

# OPS-24 R5: the three paths outside rust/ that Rust compiles in. Each sets
# rust on top of the scope its own path already had.
assert_case "ops/release/release.pub sets rust and stack" "true" "false" "false" "true" "false" "false" "ops/release/release.pub"
assert_case "ops/release/fixtures/sigstore/release.json sets rust and stack" "true" "false" "false" "true" "false" "false" "ops/release/fixtures/sigstore/release.json"
assert_case "public/geo/id-provinces.geojson sets rust and frontend" "true" "true" "false" "false" "false" "false" "public/geo/id-provinces.geojson"

# CI script tests
assert_case "scripts/ci forces all" "true" "true" "true" "true" "false" "true" "scripts/ci/detect_change_scope.sh"

# Workflow tests
assert_case ".github/workflows/ci.yml forces all" "true" "true" "true" "true" "false" "true" ".github/workflows/ci.yml"

# Unclassified file tests
assert_case "unclassified file forces all" "true" "true" "true" "true" "false" "true" "some_random_file.xyz"

# Multi-file tests
assert_case "docs + rust sets rust and clears docs_only" "true" "false" "false" "false" "false" "false" "docs/test.md" "rust/crates/lib.rs"
assert_case "frontend + dagster sets both" "false" "true" "true" "false" "false" "false" "src/components/button.tsx" "dagster/pipeline.py"

# Push event tests (OPS-24 D1, D8): a push follows the scope like a pull
# request, and only CI_PUSH_FULL=true (FORCE_ALL) widens it.
CASE_EVENT=push assert_case "a documents-only push is documents-only" "false" "false" "false" "false" "true" "false" "docs/test.md"
CASE_EVENT=push assert_case "a Rust push sets rust only" "true" "false" "false" "false" "false" "false" "rust/crates/lib.rs"
CASE_EVENT=push assert_case "a workflow push forces all" "true" "true" "true" "true" "false" "true" ".github/workflows/ci.yml"
CASE_EVENT=push CASE_FORCE_ALL=true assert_case "a documents-only push with CI_PUSH_FULL=true forces all" "true" "true" "true" "true" "false" "true" "docs/test.md"
CASE_FORCE_ALL=true assert_case "CI_PUSH_FULL does not widen a pull request" "false" "false" "false" "false" "true" "false" "docs/test.md"

# OPS-24 R4: only the exact string counts, and the script says which one it
# read. `TRUE` and `1` are warnings, not switches -- the same answer the
# workflow's `env:` value gets, because the comparison happens here only.
CASE_EVENT=push CASE_FORCE_ALL=TRUE assert_case "CI_PUSH_FULL=TRUE does not widen a push" "false" "false" "false" "false" "true" "false" "docs/test.md"
CASE_EVENT=push CASE_FORCE_ALL=1 assert_case "CI_PUSH_FULL=1 does not widen a push" "false" "false" "false" "false" "true" "false" "docs/test.md"

# OPS-24 R4: `push_full` is the one answer the workflow reads to turn the push
# proof off. It is true only for a push with CI_PUSH_FULL exactly `true`.
assert_push_full() {
  local label="$1" want="$2"
  local output got
  output="$(run_case docs/test.md)"
  got="$(grep "^push_full=" <<< "$output" | cut -d= -f2)"
  if [ "$got" = "$want" ]; then
    log_pass "$label"
  else
    log_fail "$label (got push_full=${got}, want ${want})"
  fi
}

CASE_EVENT=push CASE_FORCE_ALL=true assert_push_full "push_full is true for a push with CI_PUSH_FULL=true" "true"
CASE_EVENT=push CASE_FORCE_ALL=TRUE assert_push_full "push_full is false for a push with CI_PUSH_FULL=TRUE" "false"
CASE_EVENT=push assert_push_full "push_full is false for a push without CI_PUSH_FULL" "false"
CASE_EVENT=pull_request CASE_FORCE_ALL=true assert_push_full "push_full is false on a pull request with CI_PUSH_FULL=true" "false"

# Move test: file moved from rust/ into docs/ must set rust=true
test_rename_rust_to_docs() {
  local output="$REPO_DIR/github_output_mv"
  : > "$output"
  (
    cd "$REPO_DIR"
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    git mv rust/crates/lib.rs docs/lib_moved.md
    git commit -q -m "move rust file to docs"
  )
  (
    cd "$REPO_DIR"
    EVENT_NAME="pull_request" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  )
  local r doc
  r="$(grep "^rust=" "$output" | cut -d= -f2)"
  doc="$(grep "^docs_only=" "$output" | cut -d= -f2)"
  if [ "$r" = "true" ] && [ "$doc" = "false" ]; then
    log_pass "file moved from rust/ into docs/ sets rust=true"
  else
    log_fail "file moved from rust/ into docs/ did not set rust=true (rust=$r, docs_only=$doc)"
  fi
}
test_rename_rust_to_docs

# Failed git diff test
test_failed_git_diff() {
  local mock_bin="$REPO_DIR/mock_bin"
  mkdir -p "$mock_bin"
  cat <<\EOF > "$mock_bin/git"
#!/usr/bin/env bash
if [ "$1" = "diff" ]; then
  exit 128
fi
exec /usr/bin/git "$@"
EOF
  chmod +x "$mock_bin/git"

  local output="$REPO_DIR/github_output_faileddiff"
  : > "$output"
  (
    cd "$REPO_DIR"
    PATH="$mock_bin:$PATH" \
    EVENT_NAME="pull_request" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  )

  local a
  a="$(grep "^all=" "$output" | cut -d= -f2)"
  if [ "$a" = "true" ]; then
    log_pass "failed git diff forces all=true"
  else
    log_fail "failed git diff did not force all=true (all=$a)"
  fi
}
test_failed_git_diff

# Empty diff file list test
test_empty_diff() {
  local output="$REPO_DIR/github_output_empty"
  : > "$output"
  (
    cd "$REPO_DIR"
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    git commit --allow-empty -q -m "empty commit"
  )
  (
    cd "$REPO_DIR"
    EVENT_NAME="pull_request" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  )

  local a
  a="$(grep "^all=" "$output" | cut -d= -f2)"
  if [ "$a" = "true" ]; then
    log_pass "empty diff file list forces all=true"
  else
    log_fail "empty diff file list did not force all=true (all=$a)"
  fi
}
test_empty_diff

# Unreadable base commit test.
#
# OPS-24 R2: each of these blocks reports through its exit status, and the
# counters are incremented OUTSIDE the subshell. A subshell that sets PASS or
# FAIL cannot change its parent's count, so the same block written that way
# prints FAIL and the suite still reports "0 failed".
test_unreadable_base() {
  local output="$REPO_DIR/github_output_badbase"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    EVENT_NAME="pull_request" \
    BASE_SHA="0000000000000000000000000000000000000000" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  ) || return 1
  [ "$(grep "^all=" "$output" | cut -d= -f2)" = "true" ] || return 1
  # OPS-25 D5: an unreadable base selects every gate group too.
  [ "$(grep "^gate_groups=" "$output" | cut -d= -f2-)" = "$ALL_GROUPS" ]
}

if test_unreadable_base; then
  log_pass "unreadable base SHA forces all=true and every gate group"
else
  log_fail "unreadable base SHA did not force all=true and every gate group"
fi

# A push whose base commit is unreadable fails closed (OPS-24 D1): with the
# scope following the push's own diff, a diff that cannot be read must run
# everything.
test_push_unreadable_base() {
  local output="$REPO_DIR/github_output_push_badbase"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    EVENT_NAME="push" \
    BASE_SHA="0000000000000000000000000000000000000000" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  ) || return 1
  [ "$(grep "^all=" "$output" | cut -d= -f2)" = "true" ]
}

if test_push_unreadable_base; then
  log_pass "a push with an unreadable base forces all=true"
else
  log_fail "a push with an unreadable base did not force all=true"
fi

# A scheduled run has no base commit at all, so every job runs (OPS-24 D3).
test_schedule_forces_all() {
  local output="$REPO_DIR/github_output_schedule"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    EVENT_NAME="schedule" \
    BASE_SHA="" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >/dev/null
  ) || return 1
  [ "$(grep "^all=" "$output" | cut -d= -f2)" = "true" ]
}

if test_schedule_forces_all; then
  log_pass "a scheduled run forces all=true"
else
  log_fail "a scheduled run did not force all=true"
fi

# OPS-24 R3: any event that is neither `push` nor `pull_request` forces all,
# explicitly and with a log line, whatever base SHA it carries -- a base that
# happens to be readable must not narrow the run.
test_unknown_event_forces_all() {
  local output="$REPO_DIR/github_output_unknown_event"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    EVENT_NAME="release" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >"$REPO_DIR/unknown_event.log" 2>&1
  ) || return 1
  [ "$(grep "^all=" "$output" | cut -d= -f2)" = "true" ] || return 1
  grep -q "is neither push nor pull_request" "$REPO_DIR/unknown_event.log"
}

if test_unknown_event_forces_all; then
  log_pass "an event other than push and pull_request forces all=true"
else
  log_fail "an event other than push and pull_request did not force all=true"
fi

# ── OPS-25: a pull request runs only the gates its files can affect ──────
# One path per group selects only that group, so each group is proved reachable
# on its own (D1).
assert_groups "a G2 tenant test selects only storage" ",storage," "ops/g2/g2_tenant_test.py"
assert_groups "a G3a test selects only pipelines" ",pipelines," "ops/g3a/g3a_test.py"
assert_groups "a G4 test selects only cdc" ",cdc," "ops/g4/g4_test.py"
assert_groups "a discovery-scale test selects only ingest" ",ingest," "ops/g6/src13_discovery_scale_test.py"
assert_groups "a vendor stub selects only saas" ",saas," "ops/g6/saas_stub_hubspot.py"
assert_groups "the Keycloak realm template selects only secrets" ",secrets," "ops/sso/realm.template.json"
assert_groups "the notify crate selects only alerts" ",alerts," "rust/crates/lakehouse-notify/src/lib.rs"
assert_groups "the upload gate selects only upload" ",upload," "ops/g9/upload_test.py"
assert_groups "a governance test selects only governance" ",governance," "ops/g8/g8_governance_test.py"

# Two narrow paths select their union, written in the canonical group order.
assert_groups "a storage and an upload change select both groups" ",storage,upload," "ops/g2/x.py" "ops/g9/y.py"

# A shared path selects every group: one binary and one schema serve every gate
# (D3). These are the paths a feature pull request most often touches.
for shared in docker-compose.yml .env.example rust/migrations/0999_x.sql rust/Cargo.lock \
              rust/crates/lakehouse-store/src/lib.rs \
              rust/crates/lakehouse-api/src/policy.rs \
              rust/crates/lakehouse-api/src/routes/mod.rs dagster/Dockerfile \
              ops/gates/gate_summary.py scripts/compose.sh; do
  assert_groups "the shared path ${shared} selects every gate group" "$ALL_GROUPS" "$shared"
done

# D2/D5: a path the map does not know selects every group, not none.
assert_groups "an unmapped path selects every gate group" "$ALL_GROUPS" "newdir/file.txt"

# Paths no gate can load select nothing (D4).
assert_groups "a documents-only change selects no gate group" "none" "docs/test.md"
assert_groups "a console-only change selects no gate group" "none" "src/components/button.tsx"
assert_groups "an API integration test selects no gate group" "none" "rust/crates/lakehouse-api/tests/route_auth.rs"

# D6: the label selects every group, and only the exact lower-case value does.
CASE_ALL_GATES_LABEL=true assert_groups "the label ci:all-gates selects every gate group on a console change" "$ALL_GROUPS" "src/components/button.tsx"
CASE_ALL_GATES_LABEL=TRUE assert_groups "ALL_GATES_LABEL=TRUE does not select every gate group" "none" "src/components/button.tsx"

# D5: the events and switches that fail closed.
CASE_EVENT=schedule assert_groups "a scheduled run selects every gate group" "$ALL_GROUPS" "src/components/button.tsx"
CASE_EVENT=push CASE_FORCE_ALL=true assert_groups "CI_PUSH_FULL=true on a push selects every gate group" "$ALL_GROUPS" "docs/test.md"
CASE_GATE_PATHS_FILE=/nonexistent assert_groups "a missing gate path map selects every gate group" "$ALL_GROUPS" "src/components/button.tsx"

# The label also turns `stack` on, so `images` runs on a console-only pull
# request it was added to (D6).
test_label_sets_stack() {
  local output stack_value
  output="$(run_case src/components/button.tsx)"
  stack_value="$(grep "^stack=" <<< "$output" | cut -d= -f2)"
  [ "$stack_value" = "true" ]
}

if CASE_ALL_GATES_LABEL=true test_label_sets_stack; then
  log_pass "the label ci:all-gates sets stack=true"
else
  log_fail "the label ci:all-gates did not set stack=true"
fi

# D2: the unmapped line names the file, so a reader of the log sees what
# selected everything.
test_unmapped_is_logged() {
  local log="$REPO_DIR/unmapped.log" output="$REPO_DIR/github_output_unmapped"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    mkdir -p newdir
    echo "edit-$(date +%s%N)" >> newdir/file.txt
    git add newdir/file.txt
    git commit -q -m "case commit"
  ) || return 1
  (
    cd "$REPO_DIR" || return 1
    EVENT_NAME="pull_request" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    bash "$SCRIPT" >"$log" 2>&1
  ) || return 1
  grep -q "Unmapped file: newdir/file.txt (selecting every gate group)" "$log"
}

if test_unmapped_is_logged; then
  log_pass "an unmapped path is logged with its name"
else
  log_fail "an unmapped path was not logged with its name"
fi

# D5: a missing map warns (and still selects every group; see the case above).
test_missing_map_warns() {
  local log="$REPO_DIR/missing_map.log" output="$REPO_DIR/github_output_missing_map"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    echo "edit-$(date +%s%N)" >> src/components/button.tsx
    git add src/components/button.tsx
    git commit -q -m "case commit"
  ) || return 1
  (
    cd "$REPO_DIR" || return 1
    EVENT_NAME="pull_request" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    GATE_PATHS_FILE=/nonexistent \
    bash "$SCRIPT" >"$log" 2>&1
  ) || return 1
  grep -q "::warning::gate path map /nonexistent is missing or unreadable" "$log"
}

if test_missing_map_warns; then
  log_pass "a missing gate path map warns"
else
  log_fail "a missing gate path map did not warn"
fi

# D5: a path line naming a group that does not exist is an error, so `changes`
# is red instead of quietly selecting nothing.
test_unknown_group_in_map() {
  local map="$REPO_DIR/unknown_group_paths.txt"
  local log="$REPO_DIR/unknown_group.log" output="$REPO_DIR/github_output_unknown_group"
  cp "$GATE_MAP" "$map"
  printf 'path ops/g2/* nosuchgroup\n' >> "$map"
  : > "$output"
  (
    cd "$REPO_DIR" || return 1
    git reset --hard "$BASE_SHA" >/dev/null 2>&1
    echo "edit-$(date +%s%N)" >> src/components/button.tsx
    git add src/components/button.tsx
    git commit -q -m "case commit"
  ) || return 1
  local status=0
  (
    cd "$REPO_DIR" || exit 99
    EVENT_NAME="pull_request" \
    BASE_SHA="$BASE_SHA" \
    GITHUB_OUTPUT="$output" \
    GATE_PATHS_FILE="$map" \
    bash "$SCRIPT" >"$log" 2>&1
  ) || status=$?
  [ "$status" -eq 1 ] && grep -q "unknown gate group 'nosuchgroup'" "$log"
}

if test_unknown_group_in_map; then
  log_pass "a path line naming an unknown gate group exits 1"
else
  log_fail "a path line naming an unknown gate group did not exit 1"
fi

# T1 mode 5: `--classify` prints the map's answer per path and makes no git
# call; ops/lint/check_gate_groups.py runs it.
test_classify_mode() {
  local out
  out="$(cd "$REPO_DIR" && bash "$SCRIPT" --classify ops/g4/g4_test.py docs/a.md)" || return 1
  [ "$(sed -n '1p' <<< "$out")" = $'ops/g4/g4_test.py\tcdc' ] || return 1
  [ "$(sed -n '2p' <<< "$out")" = $'docs/a.md\tdocs' ]
}

if test_classify_mode; then
  log_pass "--classify prints the gate groups of each path and docs for a document"
else
  log_fail "--classify did not print the expected groups"
fi

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
