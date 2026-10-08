#!/usr/bin/env bash
# Tests for scripts/ci/detect_change_scope.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../detect_change_scope.sh"

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

  mkdir -p docs GTM src/lib src/components public rust/crates dagster ops/g6 ops/fixtures scripts .github/workflows

  : > docs/test.md
  : > GTM/test.md
  : > README.md
  : > LICENSE
  : > NOTICE
  : > src/components/button.tsx
  : > src/lib/dashboard-specs.ts
  : > public/robots.txt
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
  : > ops/fixtures/upload_load_failure_reasons.json
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
    EVENT_NAME="pull_request" \
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
assert_case "scripts/compose.sh is stack-only" "false" "false" "false" "true" "false" "false" "scripts/compose.sh"
assert_case ".env.example is stack-only" "false" "false" "false" "true" "false" "false" ".env.example"

# Cross-cutting fixture tests
assert_case "ops/fixtures forces all" "true" "true" "true" "true" "false" "true" "ops/fixtures/upload_load_failure_reasons.json"

# CI script tests
assert_case "scripts/ci forces all" "true" "true" "true" "true" "false" "true" "scripts/ci/detect_change_scope.sh"

# Workflow tests
assert_case ".github/workflows/ci.yml forces all" "true" "true" "true" "true" "false" "true" ".github/workflows/ci.yml"

# Unclassified file tests
assert_case "unclassified file forces all" "true" "true" "true" "true" "false" "true" "some_random_file.xyz"

# Multi-file tests
assert_case "docs + rust sets rust and clears docs_only" "true" "false" "false" "false" "false" "false" "docs/test.md" "rust/crates/lib.rs"
assert_case "frontend + dagster sets both" "false" "true" "true" "false" "false" "false" "src/components/button.tsx" "dagster/pipeline.py"

# Push event test
(
  output="$REPO_DIR/github_output_push"
  : > "$output"
  cd "$REPO_DIR"
  EVENT_NAME="push" \
  BASE_SHA="$BASE_SHA" \
  GITHUB_OUTPUT="$output" \
  bash "$SCRIPT" >/dev/null
  a="$(grep "^all=" "$output" | cut -d= -f2)"
  r="$(grep "^rust=" "$output" | cut -d= -f2)"
  if [ "$a" = "true" ] && [ "$r" = "true" ]; then
    log_pass "push event forces all=true"
  else
    log_fail "push event did not force all=true"
  fi
)

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

# Unreadable base commit test
(
  output="$REPO_DIR/github_output_badbase"
  : > "$output"
  cd "$REPO_DIR"
  EVENT_NAME="pull_request" \
  BASE_SHA="0000000000000000000000000000000000000000" \
  GITHUB_OUTPUT="$output" \
  bash "$SCRIPT" >/dev/null
  a="$(grep "^all=" "$output" | cut -d= -f2)"
  if [ "$a" = "true" ]; then
    log_pass "unreadable base SHA forces all=true"
  else
    log_fail "unreadable base SHA did not force all=true"
  fi
)

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
