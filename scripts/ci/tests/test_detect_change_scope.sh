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

  mkdir -p docs GTM src/lib src/components public rust/crates dagster ops/g6 scripts .github/workflows

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

# Dashboard specs special case: sets both rust and frontend
assert_case "src/lib/dashboard-specs.ts sets rust and frontend" "true" "true" "false" "false" "false" "false" "src/lib/dashboard-specs.ts"

# Rust tests
assert_case "rust/crates/lib.rs is rust-only" "true" "false" "false" "false" "false" "false" "rust/crates/lib.rs"

# Dagster tests
assert_case "dagster/pipeline.py is dagster-only" "false" "false" "true" "false" "false" "false" "dagster/pipeline.py"

# Stack tests
assert_case "docker-compose.yml is stack-only" "false" "false" "false" "true" "false" "false" "docker-compose.yml"
assert_case "ops/g6/test.py is stack-only" "false" "false" "false" "true" "false" "false" "ops/g6/test.py"
assert_case "scripts/compose.sh is stack-only" "false" "false" "false" "true" "false" "false" "scripts/compose.sh"
assert_case ".env.example is stack-only" "false" "false" "false" "true" "false" "false" ".env.example"

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

# Unreadable base SHA test
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
exit 0
