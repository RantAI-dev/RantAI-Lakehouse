#!/usr/bin/env bash
# Detect change scope for CI pipeline.
# Classifies changed files into scope categories:
#   rust, frontend, dagster, stack, docs_only, all
# and writes results to $GITHUB_OUTPUT.
#
# Required environment variables:
#   GITHUB_OUTPUT   — GitHub Actions output file (optional for local testing)
#   EVENT_NAME      — github.event_name (push or pull_request)
#   BASE_SHA        — base commit SHA to diff against
set -euo pipefail

EVENT="${EVENT_NAME:-}"
BASE="${BASE_SHA:-}"

echo "Event: ${EVENT:-unknown}"
echo "Base SHA: ${BASE:-none}"

# On push to main, run everything regardless of scope.
if [ "$EVENT" = "push" ]; then
  echo "Push event: forcing all=true."
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "rust=true"
      echo "frontend=true"
      echo "dagster=true"
      echo "stack=true"
      echo "docs_only=false"
      echo "all=true"
    } >> "$GITHUB_OUTPUT"
  fi
  exit 0
fi

# If BASE_SHA is empty or unreadable, fail-safe to running all jobs.
if [ -z "$BASE" ] || ! git cat-file -e "$BASE^{commit}" 2>/dev/null; then
  echo "Base commit missing or unreadable: forcing all=true."
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "rust=true"
      echo "frontend=true"
      echo "dagster=true"
      echo "stack=true"
      echo "docs_only=false"
      echo "all=true"
    } >> "$GITHUB_OUTPUT"
  fi
  exit 0
fi

CHANGED="$(git diff --no-renames --name-only "$BASE" HEAD || true)"
if [ -z "$CHANGED" ]; then
  echo "No changed files detected."
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "rust=false"
      echo "frontend=false"
      echo "dagster=false"
      echo "stack=false"
      echo "docs_only=false"
      echo "all=false"
    } >> "$GITHUB_OUTPUT"
  fi
  exit 0
fi

rust=false
frontend=false
dagster=false
stack=false
docs_only=true
all=false

while IFS= read -r file; do
  [ -z "$file" ] && continue

  classified=false

  # Docs-only set: docs/**, GTM/**, root *.md, LICENSE, NOTICE
  if [[ "$file" == docs/* ]] || [[ "$file" == GTM/* ]] || \
     [[ "$file" =~ ^[^/]+\.md$ ]] || [[ "$file" == "LICENSE" ]] || [[ "$file" == "NOTICE" ]]; then
    classified=true
    continue
  fi

  # Any file outside the docs-only set means docs_only is false
  docs_only=false

  # Rust: rust/**, src/lib/dashboard-specs.ts
  if [[ "$file" == rust/* ]] || [[ "$file" == "src/lib/dashboard-specs.ts" ]]; then
    rust=true
    classified=true
  fi

  # Frontend: src/**, public/**, package.json, bun.lock, bunfig.toml,
  # tsconfig.json, next.config.ts, eslint.config.mjs, postcss.config.mjs,
  # components.json, Dockerfile.frontend
  if [[ "$file" == src/* ]] || [[ "$file" == public/* ]] || \
     [[ "$file" == "package.json" ]] || [[ "$file" == "bun.lock" ]] || \
     [[ "$file" == "bunfig.toml" ]] || [[ "$file" == "tsconfig.json" ]] || \
     [[ "$file" == "next.config.ts" ]] || [[ "$file" == "eslint.config.mjs" ]] || \
     [[ "$file" == "postcss.config.mjs" ]] || [[ "$file" == "components.json" ]] || \
     [[ "$file" == "Dockerfile.frontend" ]]; then
    frontend=true
    classified=true
  fi

  # Dagster: dagster/**
  if [[ "$file" == dagster/* ]]; then
    dagster=true
    classified=true
  fi

  # Test fixtures: ops/fixtures/** is read across scopes by Rust tests (lakehouse-api
  # upload_parse.rs, routes/uploads.rs, tests/upload_routes.rs), frontend test
  # (src/lib/uploads.test.ts) and Dagster test (test_file_ingest.py). Must force all=true.
  if [[ "$file" == ops/fixtures/* ]]; then
    all=true
    classified=true
  fi

  # Stack: docker-compose.yml, ops/**, scripts/**, .env.example
  if [[ "$file" == "docker-compose.yml" ]] || [[ "$file" == ops/* ]] || \
     [[ "$file" == scripts/* ]] || [[ "$file" == ".env.example" ]]; then
    stack=true
    classified=true
  fi

  # .github/** forces all
  if [[ "$file" == .github/* ]]; then
    all=true
    classified=true
  fi

  # Any file the rules above do not classify sets all (fail-safe)
  if [ "$classified" = false ]; then
    echo "Unclassified file changed: $file (forcing all=true)"
    all=true
  fi
done <<< "$CHANGED"

# If all is true, everything runs
if [ "$all" = true ]; then
  rust=true
  frontend=true
  dagster=true
  stack=true
  docs_only=false
fi

echo "Scope detection result:"
echo "  rust=$rust"
echo "  frontend=$frontend"
echo "  dagster=$dagster"
echo "  stack=$stack"
echo "  docs_only=$docs_only"
echo "  all=$all"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "rust=$rust"
    echo "frontend=$frontend"
    echo "dagster=$dagster"
    echo "stack=$stack"
    echo "docs_only=$docs_only"
    echo "all=$all"
  } >> "$GITHUB_OUTPUT"
fi
