#!/usr/bin/env bash
# Detect change scope for CI pipeline.
#
# Classifies changed files into the scope categories the jobs use:
#   rust, frontend, dagster, stack, docs_only, all
# and into the gate groups a pull request has to run (OPS-25):
#   storage, pipelines, cdc, ingest, saas, secrets, alerts, upload, governance
# and writes the results to $GITHUB_OUTPUT.
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
#   ALL_GATES_LABEL — `true` when the pull request carries the label
#                     `ci:all-gates` (OPS-25 D6). The workflow passes only that
#                     boolean, never any label text, and the comparison happens
#                     here. Exactly `true` selects every gate group and sets
#                     `stack=true` so `images` runs too; any other value does
#                     nothing but warn, like CI_PUSH_FULL (R4).
#   GATE_PATHS_FILE — the gate path map (OPS-25 D2). Defaults to
#                     `gate_paths.txt` next to this script. A missing or
#                     unreadable map selects every gate group and warns; a
#                     malformed one exits 1, so `changes` is red (D5).
#
# Outputs written (OPS-25 D7): `gates_<group>=true|false` for each of the nine
# groups and `gate_groups`, which is `none` or the selected groups between
# commas in the order of GATE_GROUPS below (`,cdc,`). `ci-required` reads
# `gate_groups` to decide whether a skipped gate job was allowed to be skipped.
#
# Usage:
#   detect_change_scope.sh                 classify the diff and write outputs
#   detect_change_scope.sh --classify <path>…
#                                          print `<path><TAB><groups>` per path,
#                                          where <groups> is a comma list,
#                                          `all`, `none`, `unmapped` or `docs`;
#                                          no git call is made (used by
#                                          ops/lint/check_gate_groups.py)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE_PATHS="${GATE_PATHS_FILE:-${SCRIPT_DIR}/gate_paths.txt}"

# OPS-25 D1: the nine gate groups, in the order `gate_groups` is written out.
# The membership itself lives in `group` lines of the map; the names and their
# order are repeated here so a missing or unreadable map can still select every
# group (D5) instead of emitting no `gates_*` output at all -- an absent output
# would read as "not selected" and skip the gates that must run.
GATE_GROUPS=(storage pipelines cdc ingest saas secrets alerts upload governance)

MAP_PATTERNS=()
MAP_GROUPS=()
declare -A JOB_GROUP=()
declare -A SELECTED_GROUPS=()

rust=false
frontend=false
dagster=false
stack=false
docs_only=true
all=false
push_full=false

gate_paths_error() {
  echo "::error::$1"
  exit 1
}

known_group() {
  local candidate="$1" group
  for group in "${GATE_GROUPS[@]}"; do
    if [ "$group" = "$candidate" ]; then
      return 0
    fi
  done
  return 1
}

# OPS-25 D2/D5: read the map once. A missing or unreadable file is a warning
# and every group (fail closed); a malformed line, a path line naming an
# unknown group, or a job in two groups is an error and exits 1.
load_gate_paths() {
  if [ ! -r "$GATE_PATHS" ]; then
    echo "::warning::gate path map ${GATE_PATHS} is missing or unreadable: selecting every gate group"
    return 1
  fi
  local line keyword name pattern groups job group
  local -a fields members
  while IFS= read -r line || [ -n "$line" ]; do
    line="${line%$'\r'}"
    case "$line" in
      '' | '#'*) continue ;;
    esac
    read -r -a fields <<< "$line"
    keyword="${fields[0]:-}"
    case "$keyword" in
      group)
        name="${fields[1]:-}"
        if [ -n "$name" ] && ! known_group "$name"; then
          gate_paths_error "gate path map: unknown gate group '${name}' in: ${line}"
        fi
        if [ -z "$name" ] || [ "${#fields[@]}" -lt 3 ]; then
          gate_paths_error "gate path map: a 'group' line needs a name and at least one job: ${line}"
        fi
        for job in "${fields[@]:2}"; do
          if [ -n "${JOB_GROUP[$job]:-}" ] && [ "${JOB_GROUP[$job]}" != "$name" ]; then
            gate_paths_error "gate path map: job '${job}' is in groups '${JOB_GROUP[$job]}' and '${name}': every job belongs to exactly one group"
          fi
          JOB_GROUP[$job]="$name"
        done
        ;;
      path)
        if [ "${#fields[@]}" -ne 3 ]; then
          gate_paths_error "gate path map: a 'path' line needs a pattern and a group list: ${line}"
        fi
        pattern="${fields[1]}"
        groups="${fields[2]}"
        if [ "$groups" != "all" ] && [ "$groups" != "none" ]; then
          IFS=',' read -r -a members <<< "$groups"
          if [ "${#members[@]}" -eq 0 ]; then
            gate_paths_error "gate path map: no gate group in: ${line}"
          fi
          for group in "${members[@]}"; do
            if ! known_group "$group"; then
              gate_paths_error "gate path map: path '${pattern}' names unknown gate group '${group}'"
            fi
          done
        fi
        MAP_PATTERNS+=("$pattern")
        MAP_GROUPS+=("$groups")
        ;;
      *)
        gate_paths_error "gate path map: expected a 'group' or 'path' line, got: ${line}"
        ;;
    esac
  done < "$GATE_PATHS"
  return 0
}

# OPS-25 D2: the first matching `path` line wins. The pattern is deliberately
# unquoted so bash matches it as a glob, where `*` also matches `/`.
classify_gate_groups() {
  local file="$1" index
  for ((index = 0; index < ${#MAP_PATTERNS[@]}; index++)); do
    # shellcheck disable=SC2053 # the map's pattern is the glob, on purpose
    if [[ "$file" == ${MAP_PATTERNS[index]} ]]; then
      printf '%s\n' "${MAP_GROUPS[index]}"
      return 0
    fi
  done
  printf 'unmapped\n'
}

select_every_group() {
  local group
  for group in "${GATE_GROUPS[@]}"; do
    SELECTED_GROUPS[$group]=true
  done
}

select_groups_of_file() {
  local file="$1" groups group
  local -a members
  groups="$(classify_gate_groups "$file")"
  case "$groups" in
    all)
      select_every_group
      ;;
    unmapped)
      # D2/D5: an unknown path means everything.
      echo "Unmapped file: ${file} (selecting every gate group)"
      select_every_group
      ;;
    none)
      ;;
    *)
      IFS=',' read -r -a members <<< "$groups"
      for group in "${members[@]}"; do
        SELECTED_GROUPS[$group]=true
      done
      ;;
  esac
}

# OPS-25 D7: `none`, or the selected groups between commas in GATE_GROUPS order.
gate_groups_value() {
  local group value=""
  for group in "${GATE_GROUPS[@]}"; do
    if [ -n "${SELECTED_GROUPS[$group]:-}" ]; then
      value="${value},${group}"
    fi
  done
  if [ -z "$value" ]; then
    printf 'none\n'
  else
    printf '%s,\n' "$value"
  fi
}

# The one place the scope and the gate groups are printed and written: the log
# a reader checks and the outputs `ci-required` reads cannot disagree.
emit_scope() {
  local group line gate_groups
  gate_groups="$(gate_groups_value)"
  echo "Scope detection result:"
  for line in "rust=${rust}" "frontend=${frontend}" "dagster=${dagster}" \
              "stack=${stack}" "docs_only=${docs_only}" "all=${all}" \
              "push_full=${push_full}"; do
    echo "  ${line}"
  done
  echo "  gate_groups=${gate_groups}"

  [ -n "${GITHUB_OUTPUT:-}" ] || return 0
  {
    echo "rust=${rust}"
    echo "frontend=${frontend}"
    echo "dagster=${dagster}"
    echo "stack=${stack}"
    echo "docs_only=${docs_only}"
    echo "all=${all}"
    echo "push_full=${push_full}"
    for group in "${GATE_GROUPS[@]}"; do
      if [ -n "${SELECTED_GROUPS[$group]:-}" ]; then
        echo "gates_${group}=true"
      else
        echo "gates_${group}=false"
      fi
    done
    echo "gate_groups=${gate_groups}"
  } >> "$GITHUB_OUTPUT"
}

# The five fail-closed paths (unknown event, CI_PUSH_FULL, unreadable base,
# failed diff, empty diff) shared one identical output block. One function now:
# every scope true and every gate group selected (OPS-25 D5).
emit_all_outputs() {
  echo "$1"
  rust=true
  frontend=true
  dagster=true
  stack=true
  docs_only=false
  all=true
  select_every_group
  emit_scope
}

# OPS-25 D4: documents never reach the map -- the existing rule, kept in one
# place so `--classify` and the loop cannot disagree about it.
is_docs_only() {
  local file="$1"
  [[ "$file" == docs/* ]] || [[ "$file" == GTM/* ]] || \
    [[ "$file" =~ ^[^/]+\.md$ ]] || [[ "$file" == "LICENSE" ]] || [[ "$file" == "NOTICE" ]]
}

# OPS-25 T1: classify mode, before any git call. It prints the map's answer for
# each path and is what `ops/lint/check_gate_groups.py` runs, so the lint checks
# the rule CI acts on. A missing map is an error here: a caller asking for a
# classification with no map has no answer to be given.
if [ "${1:-}" = "--classify" ]; then
  shift
  if [ "$#" -eq 0 ]; then
    echo "usage: $0 --classify <path>..." >&2
    exit 2
  fi
  if ! load_gate_paths; then
    echo "::error::cannot classify: the gate path map ${GATE_PATHS} is missing or unreadable" >&2
    exit 1
  fi
  for file in "$@"; do
    if is_docs_only "$file"; then
      printf '%s\tdocs\n' "$file"
    else
      printf '%s\t%s\n' "$file" "$(classify_gate_groups "$file")"
    fi
  done
  exit 0
fi

EVENT="${EVENT_NAME:-}"
BASE="${BASE_SHA:-}"

echo "Event: ${EVENT:-unknown}"
echo "CI_PUSH_FULL: ${FORCE_ALL:-unset}"
echo "Gate path map: ${GATE_PATHS}"
echo "Base SHA: ${BASE:-none}"

# R4: one comparison, here, and it is case-sensitive. The workflow passes the
# variable through `env:` raw and never compares it again, so `TRUE` or `1`
# cannot mean "on" in one place and "off" in another. Say so in the log.
if [ -n "${FORCE_ALL:-}" ] && [ "$FORCE_ALL" != "true" ]; then
  echo "::warning::CI_PUSH_FULL is '${FORCE_ALL}', not exactly 'true' (lower case); it does not force a full push run"
fi

# OPS-25 D6: the label crosses into this script as one boolean. Exactly `true`
# selects every gate group (and `stack`, so `images` runs on a console-only
# pull request); `TRUE` or any other value is a warning and nothing more, the
# same rule as CI_PUSH_FULL (OPS-24 R4).
all_gates_label=false
if [ "${ALL_GATES_LABEL:-}" = "true" ]; then
  all_gates_label=true
elif [ -n "${ALL_GATES_LABEL:-}" ]; then
  echo "::warning::ALL_GATES_LABEL is '${ALL_GATES_LABEL}', not exactly 'true' (lower case); the label does not select every gate group"
fi

# The push proof is off whenever this switch is on: `push_full` is the single
# answer the workflow reads (R4), so the script's comparison and the proof's
# off switch cannot disagree.
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
    emit_all_outputs "Event '${EVENT:-none}' is neither push nor pull_request: forcing all=true."
    exit 0
    ;;
esac

# A push follows the same change scope as a pull request (OPS-24 D1): the diff
# is the push's own (`github.event.before` to the pushed commit). FORCE_ALL=true
# (repository variable CI_PUSH_FULL) is the switch back to "a push runs
# everything" (OPS-24 D8).
if [ "$EVENT" = "push" ] && [ "${FORCE_ALL:-}" = "true" ]; then
  emit_all_outputs "Push event with CI_PUSH_FULL=true: forcing all=true."
  exit 0
fi

# If BASE_SHA is empty or unreadable, fail-safe to running all jobs.
if [ -z "$BASE" ] || ! git cat-file -e "$BASE^{commit}" 2>/dev/null; then
  emit_all_outputs "Base commit missing or unreadable: forcing all=true."
  exit 0
fi

if ! CHANGED="$(git diff --no-renames --name-only "$BASE" HEAD)"; then
  emit_all_outputs "git diff failed: forcing all=true."
  exit 0
fi

if [ -z "$CHANGED" ]; then
  emit_all_outputs "No changed files detected (empty diff): forcing all=true."
  exit 0
fi

map_readable=true
if ! load_gate_paths; then
  map_readable=false
fi

while IFS= read -r file; do
  [ -z "$file" ] && continue

  classified=false

  # Docs-only set: docs/**, GTM/**, root *.md, LICENSE, NOTICE. They select no
  # gate group, and never reach the map (OPS-25 D4).
  if is_docs_only "$file"; then
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

  # OPS-25 D2/D5: the gate groups this file can affect. An unmapped file
  # selects every group (logged by select_groups_of_file).
  if [ "$map_readable" = true ]; then
    select_groups_of_file "$file"
  fi
done <<< "$CHANGED"

# If all is true, everything runs (OPS-24 D1, unchanged).
if [ "$all" = true ]; then
  rust=true
  frontend=true
  dagster=true
  stack=true
  docs_only=false
fi

# OPS-25 D5: all=true (a workflow or fixture change, an unreadable base, the
# nightly run), a missing map, or the label each select every gate group.
if [ "$all" = true ] || [ "$map_readable" = false ]; then
  select_every_group
fi
if [ "$all_gates_label" = true ]; then
  echo "Label ci:all-gates: selecting every gate group"
  select_every_group
  stack=true
fi

emit_scope
