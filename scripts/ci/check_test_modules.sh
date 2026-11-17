#!/usr/bin/env bash
# Every Rust test file must be compiled into a test binary (OPS-32 D6).
#
# With `autotests = false` in a crate's `[package]` table, Cargo stops building
# each file directly under `tests/` as its own test crate: only the `[[test]]`
# targets named in `Cargo.toml` are built. A new `tests/<name>.rs` that nobody
# declares in a `[[test]]` root is then compiled into nothing, and its tests
# disappear without a single failure -- the run is green and 30 tests are gone.
# The CI count floor (`ci.yml`, 6,826 tests) is far too low to notice one file.
#
# So this script reads the manifests and the roots and fails when a test file
# is not compiled anywhere. Rules, for every `rust/crates/*/Cargo.toml` with
# `autotests = false`:
#
#   1. every top-level `tests/*.rs` is either the `path` of its own `[[test]]`
#      entry, or declared by a `mod <stem>;` line in exactly one root;
#   2. every `tests/<dir>/mod.rs` is declared by `mod <dir>;` in at least one
#      root (a helper may be shared: `common` is declared by `it` and by
#      `identity_password`);
#   3. every `mod <name>;` line in a root names a file that exists, next to
#      that root (`<name>.rs` or `<name>/mod.rs`);
#   4. a top-level file declared by two roots fails: its tests would run twice.
#
# A crate without `autotests = false` passes untouched; Cargo builds every file
# there, so there is nothing to declare.
#
# Limits, stated so nobody trusts it further than it goes: it only understands
# a plain `mod x;` at column 0 of a root. `#[path = "..."] mod x;` and a `mod`
# inside a `cfg` block are not understood (and are not allowed in a root, see
# `tests/it.rs`). A module declared with `pub mod` is accepted as well.
#
# Usage: check_test_modules.sh [ROOT]
#   ROOT defaults to the repository root of this script; the self-test passes a
#   throwaway directory of fixture crates.
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

if [ ! -d "$ROOT" ]; then
  printf '%s: not a directory\n' "$ROOT" >&2
  exit 2
fi

FAIL=0

# report <path>: <reason>
report() {
  printf '%s\n' "$1"
  FAIL=1
}

# parse_manifest <Cargo.toml>
# Prints `autotests<TAB>true|false`, then one `test<TAB>name<TAB>path` line per
# `[[test]]` block. A `[[test]]` block with no `path` prints an empty path (the
# caller skips it: Cargo derives `tests/<name>.rs` on its own).
parse_manifest() {
  awk '
    function flush() {
      if (section == "test" && seen_test) { print "test\t" name "\t" path }
      seen_test = 0
    }
    function value(line,   parts) {
      split(line, parts, "\"")
      return parts[2]
    }
    /^[[:space:]]*\[\[test\]\][[:space:]]*$/ {
      flush()
      section = "test"; seen_test = 1; name = ""; path = ""; next
    }
    /^[[:space:]]*\[package\][[:space:]]*$/ { flush(); section = "package"; next }
    /^[[:space:]]*\[/ { flush(); section = "other"; next }
    section == "package" && /^[[:space:]]*autotests[[:space:]]*=/ {
      print "autotests\t" (($0 ~ /=[[:space:]]*false/) ? "false" : "true")
      next
    }
    section == "test" && /^[[:space:]]*name[[:space:]]*=/ { name = value($0); next }
    section == "test" && /^[[:space:]]*path[[:space:]]*=/ { path = value($0); next }
    END { flush() }
  ' "$1"
}

# declarations <root-file>
# Prints the module names a root declares with a plain `mod x;` / `pub mod x;`.
declarations() {
  sed -n 's/^\(pub \)\{0,1\}mod \([A-Za-z0-9_]\{1,\}\);$/\2/p' "$1"
}

for manifest in "$ROOT"/rust/crates/*/Cargo.toml; do
  [ -f "$manifest" ] || continue
  crate_dir="$(dirname "$manifest")"
  tests_dir="${crate_dir}/tests"
  [ -d "$tests_dir" ] || continue

  parsed="$(parse_manifest "$manifest")"
  autotests="$(printf '%s\n' "$parsed" | sed -n 's/^autotests\t//p')"
  if [ "$autotests" != "false" ]; then
    continue
  fi

  # [[test]] paths, relative to the crate directory.
  roots=()
  while IFS=$'\t' read -r kind name path; do
    [ "$kind" = "test" ] || continue
    if [ -z "$path" ]; then
      path="tests/${name}.rs"
    fi
    roots+=("$path")
  done <<< "$parsed"

  if [ "${#roots[@]}" -eq 0 ]; then
    # Reported and then each file is listed below as undeclared: the useful
    # output names the files the manifest just stopped compiling.
    report "${manifest}: autotests = false and no [[test]] target, so no test file is compiled"
  fi

  # declared_by["<root>\t<module>"]=1, and a module's file per root.
  declare -A declared=()
  for root in "${roots[@]}"; do
    if [ ! -f "${crate_dir}/${root}" ]; then
      report "${root}: declared as a [[test]] path in ${manifest} and the file does not exist"
      continue
    fi
    while IFS= read -r module; do
      [ -n "$module" ] || continue
      declared["${root}"$'\t'"${module}"]=1
      root_dir="$(dirname "$root")"
      if [ ! -f "${crate_dir}/${root_dir}/${module}.rs" ] \
        && [ ! -f "${crate_dir}/${root_dir}/${module}/mod.rs" ]; then
        report "${root_dir}/${module}: declared by \`mod ${module};\` in ${root} and no such file exists"
      fi
    done < <(declarations "${crate_dir}/${root}")
  done

  # The [[test]] roots themselves are compiled on their own; a root that is also
  # a module of another root would run twice (rule 4 below covers it).
  declare -A is_target=()
  for root in "${roots[@]}"; do
    is_target["$root"]=1
  done

  # Rule 1 and 4: every top-level tests/*.rs is either its own target or
  # declared by exactly one root.
  for file in "${tests_dir}"/*.rs; do
    [ -f "$file" ] || continue
    relative="tests/$(basename "$file")"
    stem="$(basename "$file" .rs)"
    if [ -n "${is_target[$relative]:-}" ]; then
      continue
    fi
    declaring=()
    for root in "${roots[@]}"; do
      if [ -n "${declared["${root}"$'\t'"${stem}"]:-}" ]; then
        declaring+=("$root")
      fi
    done
    case "${#declaring[@]}" in
      0)
        report "${relative}: not declared by any [[test]] root of ${manifest} (add \`mod ${stem};\` to tests/it.rs or a [[test]] entry), so its tests would not run"
        ;;
      1) : ;;
      *)
        report "${relative}: declared by ${declaring[*]}, so its tests would run once per root and be counted twice"
        ;;
    esac
  done

  # Rule 2: every tests/<dir>/mod.rs is declared somewhere.
  for dir in "${tests_dir}"/*/; do
    [ -d "$dir" ] || continue
    [ -f "${dir}mod.rs" ] || continue
    name="$(basename "$dir")"
    found=0
    for root in "${roots[@]}"; do
      if [ -n "${declared["${root}"$'\t'"${name}"]:-}" ]; then
        found=1
        break
      fi
    done
    if [ "$found" -eq 0 ]; then
      report "tests/${name}/mod.rs: not declared by any [[test]] root of ${manifest} (add \`mod ${name};\` to a root), so its helpers are dead code"
    fi
  done
done

if [ "$FAIL" -ne 0 ]; then
  exit 1
fi
printf '%s\n' "every test file under rust/crates/*/tests/ is compiled into a test binary"
