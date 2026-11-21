# OPS-32 RESULT: one integration-test binary per crate

Measurements for `docs/superpowers/plans/2026-11-13-one-test-binary.md`. The
plan's rule is principle 5: a number here was measured on the machine below, or
it is written as *not measured*. The CI half of the table is the planner's to
fill after the first run of the pull request (`## CI`).

## Machine and environment

- Host: Linux 6.17.0-1008-nvidia, aarch64, 20 cores, 121 GiB RAM (the ARM64
  build machine of the plan's section 3a).
- `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, `cargo 1.99.1 (5f94df478 2026-08-27)`,
  `cargo-nextest 0.9.151 (6b3f99457 2026-11-02)`.
- `CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-3`, `CARGO_BUILD_JOBS=6`,
  plain `cargo` (no wrapper), every nextest run with `--test-threads 4`.
- Before: commit `7e8775fba` (the base) plus the plan commit `a3a40fb0`.
  After: `a2748e51` (the api move and its doc lines); the store and auth moves
  do not touch the api measurement.
- **Deviations from section 3a, and why:**
  - `CARGO_PROFILE_DEV_DEBUG`/`CARGO_PROFILE_TEST_DEBUG` are **not** set to
    `line-tables-only`: the disk had 24 GB free, below the 60 GB the section
    allows for them, so its own rule applies ("leave the two `DEBUG` variables
    out for the whole slice and say so in the RESULT file"). The sizes below
    therefore carry full debug info and are **not CI-comparable**; the ratio
    is.
  - `CARGO_INCREMENTAL` stays at Cargo's default (1) rather than the section's
    `0`: the flag is part of the unit hash, so switching would have re-hashed
    and rebuilt every artifact (about 250 GB) on a disk with 24 GB free.
    Both sides of every measurement ran with the same setting, so the
    comparison holds; the absolute seconds carry incremental's overhead.
  - Measurement A is **workspace-scoped** (`cargo nextest run --workspace
    --no-run` after touching `crates/lakehouse-api/src/lib.rs`) rather than
    `-p lakehouse-api`: the `-p` feature resolution builds 122 test binaries of
    about 1.05 GB each (128 GB), which this disk could not hold beside the
    workspace copies (142 GB) that the before-state needed. The scope is the
    api crate and everything that depends on it, on both sides.
  - The target directory was shared with earlier work, and the disk filled
    three times during the slice. `debug/incremental` was deleted each time
    (36 GB, 7.5 GB, 33 GB); no `cargo clean` was run. Nothing else was
    deleted except the partial artifacts of a `-p lakehouse-api` build that
    had failed for want of space and that the plan does not use.

## Compile time

`/usr/bin/time -v`, wall clock; max RSS is that command's "Maximum resident
set size". Warm-up (`cargo nextest run --workspace --locked --no-run`, not
timed) before each pair.

| Measurement | Before | After | After / Before |
|---|---|---|---|
| A: `lakehouse-api` subtree rebuilt (`touch crates/lakehouse-api/src/lib.rs`) | 14:04.09, 9.1 GiB | 0:48.59, 5.8 GiB | 5.8 % |
| B: every crate rebuilt (`find crates -path '*/src/lib.rs' -o -path '*/src/main.rs' \| xargs touch`) | 13:58.89, 5.8 GiB | 1:17.90, 5.8 GiB | 9.3 % |

The stop rule (D8) asked for A(after) ≤ 70 % of A(before). It is at 5.8 %, so
the slice continued to T5b–T7.

Both sides rebuild the same set: `cargo` recompiled 15 crates in B and only
`lakehouse-api` in A. What fell out is the 118 links and code generations the
api no longer pays: before, the 122 api test crates wrote about 150 GB of new
binaries in one `--no-run`, which is why B(after), with the same 15 crates
rebuilt, takes 78 seconds.

## Binaries and disk

`cargo nextest list --workspace --list-type binaries-only --message-format
json`, then `stat -c %s` of every `binary-path`.

| | Binaries | Total size |
|---|---|---|
| Before, workspace | 191 | 142.4 GB |
| Before, `lakehouse-api` only | 124 (122 integration + lib + bin) | 128.4 GB |
| After T4, workspace (store and auth not yet moved) | 73 | 18.9 GB |
| After T4, `lakehouse-api` only | 6 (4 integration + lib + bin) | 4.9 GB |
| After T6, workspace (all three crates moved) | 26 | 8.3 GB |

The 26 are 10 integration binaries (`it` ×3, `hostile_uploads`,
`identity_password`, `parity`, and the four single-file crates) and 16 lib/bin
unit-test binaries. The plan estimated 28; the difference is two lib/bin
binaries this workspace does not have (`lakehousectl` builds only its bin
target, and `lakehouse-dagster` has no test target of its own), measured with
the command above.

## Test list: no test lost

`list_tests` from the plan (T1), normalizing `crate<TAB>file::name` on both
sides: `LC_ALL=C sort`, then `comm` in both directions.

- Before: 7,826 tests. After: 7,715. Difference: **−111**.
- Only before: **112 lines**, every one
  `lakehouse-api<TAB><file>::common::tls_proxy::tests::the_proxy_terminates_tls_with_the_certificate_it_advertises<TAB>false`,
  one per `it` module that had `mod common;` (the `tls_proxy` test that
  `common/mod.rs` compiles into every binary, so it ran 113 times before).
- Only after: **1 line**, `lakehouse-api<TAB>common::tls_proxy::tests::the_proxy_terminates_tls_with_the_certificate_it_advertises<TAB>false`.
- No other line, and no changed `ignored` flag: all 113 are `false`.
- Per crate: `lakehouse-api` 5,306 → 5,195 (−111), `lakehouse-store` 1,045 →
  1,045, `lakehouse-auth` 179 → 179.

## The crates' whole test suites (once, in the foreground)

`cargo nextest run -p <crate> --locked --profile ci --test-threads 4`, as CI
runs them (one process per test):

| Crate | Result | Time |
|---|---|---|
| `lakehouse-api` | 5,168 tests run: 5,168 passed (2 slow), 27 skipped | 832 s |
| `lakehouse-store` | 1,045 tests run: 1,045 passed, 0 skipped | 143 s |
| `lakehouse-auth` | 179 tests run: 179 passed, 0 skipped | 59 s |

The 27 skipped of `lakehouse-api` are the `#[ignore]`d tests of the modules in
`it` (they need a real Gotenberg, a real browser, and so on); the count is the
same as before the move, which the test-list comparison above shows. No test
failed, so the plan's "run it on the base commit" step was not needed.

## Not measured here

- Everything outside these three crates (`lakehouse-bi`, `-embed`, `-iceberg`,
  `-notify`, every lib and bin unit-test binary) did not change; CI runs them.
- Coverage (`coverage.yml`, `cargo llvm-cov`, one process per binary): not run
  locally. `it.rs`'s doc states what changes there.
- CI's own compile and run times: the planner adds them below.

## CI

*Not measured yet: the planner adds the PR's CI numbers.*
