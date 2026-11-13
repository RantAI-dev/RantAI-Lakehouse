#!/usr/bin/env bash
# Detect change scope for CI pipeline.
# Classifies changed files into scope categories:
#   rust, frontend, dagster, stack, docs_only, all
# and writes results to $GITHUB_OUTPUT.
#
# Required environment variables:
#   GITHUB_OUTPUT   — GitHub Actions output file (optional for local testing)
#   EVENT_NAME      — github.event_name: push, pull_request, schedule or
#                     workflow_dispatch. Anything other than `push` and
#                     `pull_request` has no base commit this script can read,
#                     so the event rule below forces all=true (OPS-24 D3, R3).
#   BASE_SHA        — base commit SHA to diff against
#   FORCE_ALL       — the raw repository variable CI_PUSH_FULL. Exactly
#                     `true` (lower case) forces all=true on a push and turns
#                     the push proof off; any other value is a warning and
#                     nothing else (OPS-24 D8, R4). Ignored on other events.
set -euo pipefail

EVENT="${EVENT_NAME:-}"
BASE="${BASE_SHA:-}"

echo "Event: ${EVENT:-unknown}"
echo "CI_PUSH_FULL: ${FORCE_ALL:-unset}"
echo "Base SHA: ${BASE:-none}"

# R4: one comparison, here, and it is case-sensitive. The workflow passes the
# variable through `env:` raw and never compares it again, so `TRUE` or `1`
# cannot mean "on" in one place and "off" in another. Say so in the log.
if [ -n "${FORCE_ALL:-}" ] && [ "$FORCE_ALL" != "true" ]; then
  echo "::warning::CI_PUSH_FULL is '${FORCE_ALL}', not exactly 'true' (lower case); it does not force a full push run"
fi

# The push proof is off whenever this switch is on: `push_full` is the single
# answer the workflow reads (R4), so the script's comparison and the proof's
# off switch cannot disagree.
push_full=false
if [ "$EVENT" = "push" ] && [ "${FORCE_ALL:-}" = "true" ]; then
  push_full=true
fi

# R3: only `push` and `pull_request` carry a diff this script can read. A
# scheduled run, a manual run and an event this script does not know have no
# base commit, so they force a full run explicitly instead of relying on the
# empty base below (which a future change to that rule could narrow).
case "$EVENT" in
  push | pull_request) ;;
  *)
    echo "Event '${EVENT:-none}' is neither push nor pull_request: forcing all=true."
    if [ -n "${GITHUB_OUTPUT:-}" ]; then
      {
        echo "rust=true"
        echo "frontend=true"
        echo "dagster=true"
        echo "stack=true"
        echo "docs_only=false"
        echo "all=true"
        echo "push_full=${push_full}"
      } >> "$GITHUB_OUTPUT"
    fi
    exit 0
    ;;
esac

# A push follows the same change scope as a pull request (OPS-24 D1): the diff
# is the push's own (`github.event.before` to the pushed commit). FORCE_ALL=true
# (repository variable CI_PUSH_FULL) is the switch back to "a push runs
# everything" (OPS-24 D8).
if [ "$EVENT" = "push" ] && [ "${FORCE_ALL:-}" = "true" ]; then
  echo "Push event with CI_PUSH_FULL=true: forcing all=true."
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "rust=true"
      echo "frontend=true"
      echo "dagster=true"
      echo "stack=true"
      echo "docs_only=false"
      echo "all=true"
      echo "push_full=${push_full}"
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
      echo "push_full=${push_full}"
    } >> "$GITHUB_OUTPUT"
  fi
  exit 0
fi

if ! CHANGED="$(git diff --no-renames --name-only "$BASE" HEAD)"; then
  echo "git diff failed: forcing all=true."
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "rust=true"
      echo "frontend=true"
      echo "dagster=true"
      echo "stack=true"
      echo "docs_only=false"
      echo "all=true"
      echo "push_full=${push_full}"
    } >> "$GITHUB_OUTPUT"
  fi
  exit 0
fi

if [ -z "$CHANGED" ]; then
  echo "No changed files detected (empty diff): forcing all=true."
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
      echo "rust=true"
      echo "frontend=true"
      echo "dagster=true"
      echo "stack=true"
      echo "docs_only=false"
      echo "all=true"
      echo "push_full=${push_full}"
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

  # Rust: rust/**, src/lib/dashboard-specs.ts, docker-compose.yml, .env.example,
  # ops/release/release.pub, ops/release/fixtures/**, public/geo/**
  # Note: docker-compose.yml is embedded via include_str! in
  # rust/crates/lakehouse-store/src/cdc.rs:1131 for demo_connector_compose_properties.
  # .env.example is embedded via include_str! in
  # rust/crates/lakehousectl/src/secrets.rs:66, so a change to it changes the
  # installer's build; the `stack` rule below keeps covering it as well
  # (OPS-24 D1).
  # OPS-24 R5: three more paths outside rust/ are compiled into Rust by
  # include_bytes!/include_str! -- ops/release/release.pub and
  # ops/release/fixtures/** (rust/crates/lakehousectl/src/verify.rs:51,291-295)
  # and public/geo/** (rust/crates/lakehouse-bi/src/formula/compile.rs:2535).
  # ops/lint/check_rust_embedded_paths.py keeps this list honest: it fails when
  # a file compiled into Rust outside rust/ does not set rust here.
  if [[ "$file" == rust/* ]] || [[ "$file" == "src/lib/dashboard-specs.ts" ]] || \
     [[ "$file" == "docker-compose.yml" ]] || [[ "$file" == ".env.example" ]] || \
     [[ "$file" == "ops/release/release.pub" ]] || [[ "$file" == ops/release/fixtures/* ]] || \
     [[ "$file" == public/geo/* ]]; then
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

  # CI automation scripts: scripts/ci/** modifications affect CI automation itself
  # and must force all=true.
  if [[ "$file" == scripts/ci/* ]]; then
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
echo "  push_full=$push_full"

if [ -n "${GITHUB_OUTPUT:-}" ]; then
  {
    echo "rust=$rust"
    echo "frontend=$frontend"
    echo "dagster=$dagster"
    echo "stack=$stack"
    echo "docs_only=$docs_only"
    echo "all=$all"
    echo "push_full=${push_full}"
  } >> "$GITHUB_OUTPUT"
fi
