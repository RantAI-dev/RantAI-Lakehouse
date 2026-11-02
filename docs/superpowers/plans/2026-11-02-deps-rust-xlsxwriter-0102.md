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
