#!/usr/bin/env bash
# Tests for scripts/ci/check_test_modules.sh (OPS-32 D6).
#
# Each case builds a throwaway crate tree under `mktemp -d` and points the
# guard at it, so the case's name says what it proves. The last case runs the
# guard against this repository, which is the check CI runs.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="${SCRIPT_DIR}/../check_test_modules.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

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

# assert_outcome <label> <root> <pass|fail> [<text the output must contain>]
assert_outcome() {
  local label="$1" dir="$2" want="$3" pattern="${4-}"
  local out rc=0
  out="$(bash "$SCRIPT" "$dir" 2>&1)" || rc=$?
  if [ "$want" = "pass" ]; then
    if [ "$rc" = 0 ]; then
      log_pass "$label"
    else
      log_fail "$label (exit=$rc: $out)"
    fi
    return
  fi
  if [ "$rc" = 0 ]; then
    log_fail "$label (exited 0, expected a failure)"
    return
  fi
  if [ -n "$pattern" ] && ! printf '%s\n' "$out" | grep -qF -- "$pattern"; then
    log_fail "$label (output does not name '$pattern': $out)"
    return
  fi
  log_pass "$label"
}

# new_crate <case> <name>: prints the crate directory, with a plain manifest.
# Each case gets its own tree (`${ROOT}/<case>/rust/crates/<name>`) so the
# fixtures of one case are not seen by the next.
new_crate() {
  local case_dir="${ROOT}/$1" name="$2"
  local dir="${case_dir}/rust/crates/${name}"
  mkdir -p "${dir}/tests"
  printf '[package]\nname = "%s"\nversion = "0.1.0"\n' "$name" > "${dir}/Cargo.toml"
  printf '%s\n' "$dir"
}

# one_binary_manifest <name>: `autotests = false` plus the given [[test]] blocks.
one_binary_manifest() {
  local dir="$1"
  shift
  printf '[package]\nname = "crate"\nversion = "0.1.0"\nautotests = false\n\n' > "${dir}/Cargo.toml"
  while [ "$#" -gt 0 ]; do
    printf '[[test]]\nname = "%s"\npath = "tests/%s.rs"\n\n' "$1" "$1" >> "${dir}/Cargo.toml"
    shift
  done
}

echo "=== Test-module guard test suite ==="

# ── 1. A crate Cargo still autodiscovers needs nothing declared. ────────────
crate1="$(new_crate case1 autodiscovered)"
touch "${crate1}/tests/alpha.rs" "${crate1}/tests/beta.rs" "${crate1}/tests/gamma.rs"
assert_outcome "a crate without autotests = false and three test files passes" "${ROOT}/case1" pass

# ── 2. Both a root file and a helper directory declared. ────────────────────
crate2="$(new_crate case2 declared)"
one_binary_manifest "$crate2" it
mkdir -p "${crate2}/tests/common"
touch "${crate2}/tests/common/mod.rs" "${crate2}/tests/alpha.rs" "${crate2}/tests/beta.rs"
printf 'mod common;\nmod alpha;\nmod beta;\n' > "${crate2}/tests/it.rs"
assert_outcome "a crate with autotests = false whose root declares every file and common passes" "${ROOT}/case2" pass

# ── 3. One declaration missing. ─────────────────────────────────────────────
printf 'mod common;\nmod alpha;\n' > "${crate2}/tests/it.rs"
assert_outcome "the same crate with the \`mod beta;\` line removed fails and names tests/beta.rs" \
  "${ROOT}/case2" fail "tests/beta.rs: not declared"
printf 'mod common;\nmod alpha;\nmod beta;\n' > "${crate2}/tests/it.rs"

# ── 4. The same file declared by two roots. ─────────────────────────────────
crate4="$(new_crate case4 twice)"
one_binary_manifest "$crate4" it one
touch "${crate4}/tests/alpha.rs"
printf 'mod alpha;\n' > "${crate4}/tests/it.rs"
printf 'mod alpha;\n' > "${crate4}/tests/one.rs"
assert_outcome "a file declared in two roots fails and names tests/alpha.rs" \
  "${ROOT}/case4" fail "tests/alpha.rs: declared by tests/it.rs tests/one.rs"

# ── 5. A `mod` line whose file does not exist. ──────────────────────────────
crate5="$(new_crate case5 ghost)"
one_binary_manifest "$crate5" it
touch "${crate5}/tests/alpha.rs"
printf 'mod alpha;\nmod ghost;\n' > "${crate5}/tests/it.rs"
assert_outcome "a \`mod ghost;\` line with no file fails and names ghost" \
  "${ROOT}/case5" fail "tests/ghost: declared by \`mod ghost;\`"

# ── 6. A helper directory no root declares. ─────────────────────────────────
crate6="$(new_crate case6 orphan)"
one_binary_manifest "$crate6" it
touch "${crate6}/tests/alpha.rs"
mkdir -p "${crate6}/tests/helpers"
touch "${crate6}/tests/helpers/mod.rs"
printf 'mod alpha;\n' > "${crate6}/tests/it.rs"
assert_outcome "a tests/helpers/mod.rs that no root declares fails and names it" \
  "${ROOT}/case6" fail "tests/helpers/mod.rs: not declared"

# ── 7. A file that is its own [[test]] target. ──────────────────────────────
crate7="$(new_crate case7 separate)"
one_binary_manifest "$crate7" it parity
touch "${crate7}/tests/alpha.rs"
printf 'mod alpha;\n' > "${crate7}/tests/it.rs"
printf '#[test]\nfn separate() {}\n' > "${crate7}/tests/parity.rs"
assert_outcome "a file that is the path of its own [[test]] entry and is declared nowhere passes" \
  "${ROOT}/case7" pass

# ── 8. This repository. ─────────────────────────────────────────────────────
assert_outcome "the real repository passes" "$REPO_ROOT" pass

echo "=== Results: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
  exit 1
fi
exit 0
