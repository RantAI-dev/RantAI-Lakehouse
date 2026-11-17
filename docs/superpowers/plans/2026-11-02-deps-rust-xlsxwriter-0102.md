# Dependency fix: `rust_xlsxwriter` 0.102, so `zip` 8.6.0 leaves the tree (RUSTSEC-2026-0429)

- **Status:** a planner fix plan, 2026-11-02. **Base:** `main` at `2bd5b3a4` or later.
- **Branch:** `fix/deps-rust-xlsxwriter-0102`.

## 0. What is wrong

- **The advisory.** RUSTSEC-2026-0429 was published on 2026-11-02 for `zip` 8.6.0: "`ZipArchive::extract` aborts the process on a symlink entry with a large declared size". The fix is to upgrade to 9.0 or later.
- **It blocks every PR.** `cargo audit (advisories)` and `cargo deny check (all)` are required checks, and both fail on `main` and on every open PR. Two runs fail this way: #335's run `41424695540` and #343's run `41424749910`.
- **Where 8.6.0 comes from.** `cargo tree -i zip@8.6.0` on `main` shows one path, `rust_xlsxwriter` 0.99.1 → `lakehouse-api`. Every other user, including `calamine` 0.37, is already on `zip` 9.1.2.
- **The fix version.** `rust_xlsxwriter` 0.102.0 depends on `zip ^9.0`.
- **Why the dependabot PR is not the fix.** Dependabot's #329 bumps to 0.102.0 but does not compile. In 0.102 `Workbook::add_worksheet` (and the calls chained on it) return `Result<&mut Worksheet, XlsxError>`, so the code at `lakehouse-api/src/export.rs` fails with E0599 and E0614.

## 1. Decisions

- **D1. Upgrade; do not ignore.** We never call `extract`: `rust_xlsxwriter` only writes, and we only read entries. Even so, the dependency is upgraded instead of adding a 10th `--ignore`. An upgrade that exists is cheaper than an exception to keep track of (rule 2).
- **D2. Errors use the existing path.** A new `Result` from `add_worksheet` goes through the same error the export already returns for an `XlsxError`: `?` into the export's own error type. No `unwrap` or `expect` outside tests. No new error text reaches a response (principle 4).
- **D3. Replaces #329.** The reviewer closes #329, naming this PR.

## 2. Tasks

1. **T1. The bump and the code it needs.**
   - Set `rust_xlsxwriter = "0.102"` in `rust/crates/lakehouse-api/Cargo.toml`.
   - Run `cargo update -p rust_xlsxwriter`, and nothing else in the lock.
   - Fix every call site the compiler names.
   - *Accept:*
     - `cargo tree -i zip@8.6.0` prints "nothing to print" (or an error that the package is not in the tree).
     - `cargo fmt --check`.
     - `cargo clippy -p lakehouse-api --all-targets -- -D warnings`.
     - `cargo test -p lakehouse-api export` and `cargo test -p lakehouse-api --test dashboard_export`.
     - `cargo audit` with the workflow's exact ignore list, if `cargo audit` is installed.
     - `git diff --stat -- rust/Cargo.lock` shows only `rust_xlsxwriter`, `zip` and their own dependencies changed.

## 3. Out of scope

Any other dependency, and the other dependabot PRs.

## 4. Handoff (developer appends below)

### Handoff (2026-11-02, round 1)

#### 1. Commits
- `ac681a5e` fix(deps): rust_xlsxwriter 0.102, so zip 8.6.0 leaves the tree (RUSTSEC-2026-0429) — T1.
- `b5bc0488` docs(plan): addendum, parquet 58.4.1 for RUSTSEC-2026-0435 (thrift 0.17.0), same PR — the planner's addendum (D4/T2).
- `f88616f5` fix(deps): parquet 58.4.1, so thrift 0.17.0 leaves the tree (RUSTSEC-2026-0435) — T2, this round.

Pushed to `origin/fix/deps-rust-xlsxwriter-0102` as `b5bc0488..f88616f5`, fast-forward, no force-push.

#### 2. T2 lock update, actual output

`cargo update -p parquet --precise 58.4.1` from `rust/`:

```
    Updating crates.io index
    Updating arrow-array v58.4.0 -> v58.4.1 (available: v60.1.0)
    Updating arrow-buffer v58.4.0 -> v58.4.1
    Updating arrow-data v58.4.0 -> v58.4.1
    Updating arrow-ipc v58.4.0 -> v58.4.1
    Updating arrow-schema v58.4.0 -> v58.4.1 (available: v60.1.0)
    Updating arrow-select v58.4.0 -> v58.4.1
    Updating ordered-float v2.10.1 -> v3.9.2
    Updating parquet v58.4.0 -> v58.4.1
    Updating thrift v0.17.0 -> v0.23.0
```

`cargo tree -i thrift@0.17.0` (exit 101, which is the accept condition — the package is gone):

```
error: package ID specification `thrift@0.17.0` did not match any packages
help: there are similar package ID specifications:

  thrift@0.23.0
```

`Cargo.lock` now pins `thrift` 0.23.0 and `parquet` 58.4.1, and `cargo tree -i thrift@0.23.0` shows the same path as before the fix minus the vulnerable version: `thrift` → `parquet` → `iceberg` 0.10.1, `lakehouse-iceberg`, `lakehouse-api`. So the advisory leaves the tree without changing who pulls it in.

#### 3. Lock scope

`git diff --stat origin/main -- rust/Cargo.lock`: `1 file changed, 33 insertions(+), 52 deletions(-)`.

Comparing `name`/`version` of every package in `origin/main`'s lock with this branch's, the complete set of differences is:

- `parquet` 58.4.0 → 58.4.1, with its own `arrow-array`, `arrow-buffer`, `arrow-data`, `arrow-ipc`, `arrow-schema`, `arrow-select` 58.4.0 → 58.4.1 (the patch release moves the arrow crates in lockstep).
- `thrift` 0.17.0 → 0.23.0 and `ordered-float` 2.10.1 → 3.9.2 (`thrift` 0.23's own dependency; the workspace's separate `ordered-float` 4.6.0 is untouched).
- `rust_xlsxwriter` 0.99.1 → 0.102.0, with `zip` 8.6.0 and `typed-path` 0.12.3 removed — these three are T1's, already on `main`'s lock diff for this branch.

Nothing else moved. No direct dependency in any `Cargo.toml` changed in T2.

#### 4. Verification commands and counts

- `bin/cargo-locked check -p lakehouse-iceberg -p lakehouse-api --all-targets -j 2` → exit 0, `Finished dev profile … in 2m 30s`, 15 crates checked, no errors or warnings. (`cargo-locked` touches this worktree's `rust/` files first when the shared `CARGO_TARGET_DIR` was last built by another worktree; that changes mtimes only, not content — `git status` was clean apart from `rust/Cargo.lock`.)
- `bin/cargo-locked test -p lakehouse-api upload_parquet -j 2` → exit 0; **82 passed, 0 failed** (41 in `unittests src/lib.rs`, 41 in `unittests src/main.rs`; no integration binary had a matching test).
- `bin/cargo-locked test -p lakehouse-api export -j 2` → exit 0; **194 passed, 0 failed** over 10 binaries: `src/lib.rs` 84, `src/main.rs` 88, `ai_export_tools` 3, `dashboard_access_routes` 1, `dashboard_export` 4, `dashboard_import` 1, `gold_export_auth` 3, `gold_publication` 6, `render_data` 1, `route_auth` 3.

#### 5. Not verified (this round, with the reason)

- `cargo fmt --check` and `cargo clippy -p lakehouse-api --all-targets -- -D warnings`: not run, because T2 touched only `rust/Cargo.lock` and no source file changed.
- The full PR-slice block (`cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace`, the TypeScript, Python and compose lines): not run in this round; left to the reviewer's run and CI, as the build-time rule intends.
- `cargo audit` / `cargo deny` with the workflow's ignore list: not run here, so RUSTSEC-2026-0435 being *reported green* by the scanners is not claimed — only that `thrift` 0.17.0 is absent from the lock and the tree, which is the condition those scanners key on.
- T1's own acceptance lines (`cargo tree -i zip@8.6.0`, `cargo test -p lakehouse-api --test dashboard_export`, the audit run with the ignore list): run by the T1 round on `ac681a5e`, not re-run by this round. `dashboard_export` did execute here as part of the `export` filter (4 passed).

## 5. Review (planner appends below)

## Addendum (planner, 2026-11-02): a second advisory published the same day, RUSTSEC-2026-0435 (`thrift` 0.17.0)

The PR's first CI run (`41440534170`) shows that `zip` 8.6.0 is gone. `cargo audit` and `cargo deny` still fail, now on **RUSTSEC-2026-0435**, published 2026-11-02 (CVE-2026-43868). The advisory is about an unbounded allocation in `thrift`'s binary protocol, `TBinaryInputProtocol::read_bytes` and `read_string`, below 0.23.0.

- **Path:** `cargo tree -i thrift@0.17.0` on `main` shows `thrift` 0.17.0 → `parquet` 58.4.0 → `iceberg` 0.10.1, `lakehouse-iceberg` and `lakehouse-api`.
- **Fix available:** `parquet` 58.4.1, a patch release, depends on `thrift ^0.23`.
- **Exposure:** `parquet` 58.4.0's library code reads metadata with its own compact-protocol reader. The binary protocol shows up only in its `parquet-layout` binary, which we do not build. So the exposure is low, but D1 still applies: upgrade, do not ignore.

**D4.** Run `cargo update -p parquet --precise 58.4.1` (and let `thrift` follow), in the same PR, because both advisories block the same required checks.

**T2.** That lock update and nothing else.
- *Accept:* `cargo tree -i thrift@0.17.0` reports that the package is not in the tree, and `git diff origin/main -- rust/Cargo.lock` changes only `rust_xlsxwriter`, `zip`, `parquet`, `thrift` and their own dependencies.
- Then `cargo check -p lakehouse-iceberg -p lakehouse-api --all-targets` and `cargo test -p lakehouse-api upload_parquet`.
