# OPS-32: one integration-test binary per crate

The Rust test jobs of CI spend most of their compile time building and linking test binaries, not
compiling the product. Measured on CI today (runs 43400226518, 43413267364, product owner
2026-11-13): each of the three `Rust · test N/3` partitions compiles the test workspace for 7 to 9
minutes before its first test starts, then runs about 2,600 tests in about 13.5 minutes. There are
193 test binaries of about 400 MB each (about 51 GB unpacked, 9.6 GB as a zstd archive). The
cause: Cargo builds every file directly under `rust/crates/<crate>/tests/` as its own crate and
links it on its own, and every one of those binaries links the whole crate under test and all of
its dependencies again. `lakehouse-api/tests/` alone holds 122 such files. Compiling once and
shipping an archive to the partitions was measured and rejected (`OPS-30`, #513: slower).

This plan uses the standard Cargo technique: one integration-test target per crate, a root file
that declares every former test file as a module, and `autotests = false` so Cargo stops building
each file as its own binary. No test body changes. nextest still runs every test in a process of
its own, so isolation does not change.

Base: `main` at `7e8775fba` (2026-11-13, "fix(query): Query Studio shows a fixed sentence…
(#491)"); every anchor below was checked there. Branch: `refactor/one-test-binary`. Backlog ID:
`OPS-32` (free on 2026-11-13). Cite it in every commit and at every changed site. There are no QA
case IDs. One PR.

Planner and reviewer: Claude Opus. Developer: another agent (not Opus). The developer works on
the ARM64 build machine (section 3a), not on this VM.

## 0. Inventory (verified at `7e8775fba`)

| Crate | `tests/*.rs` | Helper module | Test attributes (`#[test]`/`#[tokio::test]`/`#[sqlx::test]`, approx.) | Action |
|---|---|---|---|---|
| `lakehouse-api` | 122 | `tests/common/mod.rs` (636 lines), used by 113 files through `mod common;` | about 1,527 (12 `#[ignore]`), 88,389 lines | one `it` binary with 119 modules; 3 files stay separate (D3) |
| `lakehouse-store` | 41 | none | about 723 | one `it` binary with 41 modules |
| `lakehouse-auth` | 8 | none | about 101 | one `it` binary with 8 modules |
| `lakehouse-bi` | 1 (`specs_match_typescript.rs`, plus `extract_specs.ts`) | none | | unchanged (already one binary) |
| `lakehouse-embed` | 1 (`cross_language.rs`) | none | | unchanged |
| `lakehouse-iceberg` | 1 (`g1_lakekeeper.rs`, the G1 gate's test, own `gate_paths.txt` line) | none | | unchanged |
| `lakehouse-notify` | 1 (`smtp_private_ca.rs`) | none | | unchanged |
| `lakehousectl`, `lakehouse-dagster` | 0 (`tests/fixtures/` only) | | | unchanged |
| all others | no `tests/` | | | unchanged |

Integration binaries today: 122 + 41 + 8 + 4 = 175; the other 18 of the 193 are the crates' own
lib and bin unit-test binaries, which this plan does not touch. After this plan: 4 + 1 + 1 + 4 = 10
integration binaries, 28 binaries in all.

Note: the owner's brief says 119 files in `lakehouse-api/tests/`; at `7e8775fba` there are 122
top-level `.rs` files. 119 is the number that becomes modules of `it`.

### Process-wide state and files that need care

- **`lakehouse-test-support`'s `#[ctor]`** (`rust/crates/lakehouse-test-support/src/lib.rs:602-605`
  calls `start_postgres_container`, `:239`): runs before `main` in every process that links the
  crate. It finds or starts the ONE labelled Postgres container
  (`org.rantai.lakehouse-test-support=postgres-16-shm1g`, `:311-312`), sets `DATABASE_URL` with
  `set_var` (`:351-356`, overwriting any value already there), and starts the leftover-database
  sweeper (`:343`). `tests/common/mod.rs:40` force-links it (`use lakehouse_test_support as _;`).
  Today 113 api files link it through `common`, and `crypto_provider.rs`, `sec41_verify_ca.rs`
  and a few others name it directly. The other files do not reference it, so their binaries may
  not run the ctor (`common/mod.rs:35-39` says why). Once they become modules of `it`, every test
  process of `it` runs the ctor. That is harmless for a module whose tests need no Postgres
  (`sec11_guard`, `permission_catalog`, `sql_lex_corpus`, `breakdown_cap`, `chart_numbers`,
  `pdf_real_gotenberg`), but those tests now need Docker, as every other api test already does,
  and their processes pay the ctor's start-up cost (a container lookup by label). Two files must
  NOT get the ctor: `hostile_uploads.rs` and `parity.rs` (D3).
- **`hostile_uploads.rs`** (`SEC-22`, both tests `#[ignore]`d): `:111-121` re-runs
  `std::env::current_exe()` under `ulimit -v 512 MiB` with `--exact
  reads_the_hostile_fixture_named_by_the_environment` (`CHILD_TEST`, `:52`). As a module of `it`,
  the test's name becomes `hostile_uploads::reads_…`, so `--exact` matches nothing and both
  fixture cases fail. The child would also be the much larger `it` binary, and its ctor would
  start a tokio runtime and a Docker client inside a 512 MiB address-space limit. **It stays a
  binary of its own.**
- **`parity.rs`** (both tests `#[ignore]`d, opt-in tooling documented in
  `rust/tests/parity/README.md:210` and `parity.rs:24`, `:599`): `resolve_service_token` (`:528-563`)
  reads `DATABASE_URL` to mint a token against a real, migrated database the operator names. Inside
  `it`, the ctor would overwrite `DATABASE_URL` with the test container's URL, so the tool would
  mint against the wrong database. **It stays a binary of its own.**
- **Global `tracing` subscriber, two files:** `identity_password.rs:216-226` installs one with
  `try_init()` (it ignores a failure, and then its buffer never fills).
  `report_runner.rs:979-990` installs one with `set_global_default(..).expect("this test binary
  installs no other global subscriber")`. Under nextest (one process per test) both work. Under
  plain `cargo test` or `cargo llvm-cov` (the Coverage workflow, `coverage.yml:180`), where every
  test of a binary shares one process, the second installer in the same binary fails. **One of the
  two stays separate: `identity_password.rs`** (11 tests, the smaller one). It keeps its own
  `mod common;`.
- **The rustls process default provider:** `crypto_provider.rs:78`, `:110`
  (`install_default_provider`, `src/crypto_provider.rs:65-74`, idempotent: `Ok` when a provider is
  already set) and `sec41_verify_ca.rs:207-214` (a `OnceLock`, ignores a loser). Both tolerate
  sharing a process, so both become modules. Under plain `cargo test` the `crypto_provider`
  regression is weaker, because another test may install the provider first; under nextest (CI) it
  is unchanged. This is written in `it.rs`'s doc (T3).
- **Per-file `static CLICKHOUSE: tokio::sync::OnceCell<…>`** in `ai_sessions.rs:31`,
  `chart_numbers.rs:52`, `breakdown_cap.rs:41`, `gold_publication.rs:51`, `query_result_cap.rs:80`.
  These are module-scoped, so they stay five separate statics. Under nextest each test process
  starts its own container, as today. Under plain `cargo test` the five can now run at the same
  time in one process (before, they ran one binary after another). Memory, not correctness; it is
  noted in `it.rs`'s doc.
- **`crate::` paths:** `query_history.rs:738`, `:771`, `:775` call `crate::filters::get_history`,
  where `filters` is a module of the file (`:46`). As a module of `it`, `crate::` is `it.rs`, so
  these three must become `crate::query_history::filters::get_history`. Its `use crate::common::…`
  lines (`:54`, `:408`) keep working as they are. No other test file uses `crate::` in code (only in
  doc comments, e.g. `security_regressions.rs:151`, which stay as they are).
- **`mod common;` in 113 files**, at line 1 in most and further down in five
  (`connector_delete_deprovision.rs:44`, `connector_secret_rotate.rs:41`, `route_auth.rs:41`,
  `crypto_provider.rs:52`, `query_history.rs:43`). 111 files also have `use common::…` lines,
  e.g. `route_auth.rs:48`.
- **The `tls_proxy` test that every binary runs today:** `common/mod.rs:42-43` includes
  `#[path = "../../src/tls_proxy.rs"] pub mod tls_proxy;`. Its one test
  (`the_proxy_terminates_tls_with_the_certificate_it_advertises`, `src/tls_proxy.rs:304-320`) is
  compiled under `cfg(test)` into every binary that declares `mod common`, so it runs 113 times
  today (plus once in the lib's own unit tests, `src/lib.rs:71`). Afterwards it runs in `it` and in
  `identity_password`, twice. **The test total therefore drops by exactly 111**, all of them copies
  of that one test. T8's list comparison expects exactly that and nothing else.
- **Files that read the source tree, tested as they are:** `sec11_guard.rs:420`
  (`CARGO_MANIFEST_DIR/src/routes`), `permission_catalog.rs:96`, `:127`
  (`include_str!("fixtures/permissions.json")`), `ai_build_tool_list.rs:143`
  (`include_str!("fixtures/tool_schemas.json")`), and the store/auth files that `include_str!` a
  migration (`lakehouse-store/tests/governance.rs:270`, `agents.rs:497`, `pipelines.rs:1717`,
  `lakehouse-auth/tests/pages_per_role_migration.rs:20`). `include_str!` resolves relative to the
  file it is in, so these paths stay valid only if the files do not move (D2).
- **`route_auth.rs`** (95 tests, the authz matrix, in-process only) and
  **`security_regressions.rs`**: nothing process-wide, so they become plain modules.
- **`env!("CARGO_BIN_EXE_lakehouse-api")`** (`common/mod.rs:341-343`): Cargo sets it for every
  `[[test]]` target of the package, so it still works.
- **No `#[serial]`, no `serial_test`, no `std::env::set_var` in any test file** (the only `set_var`
  is the ctor's). No test reads server-wide Postgres state in a way that assumes it is alone
  (`connector_secret_rotate.rs:268`, `connector_repoint.rs:488`, `crypto_provider.rs`,
  `sec41_verify_ca.rs` connect to the server but use their own databases or `SELECT 1`). No test
  uses `module_path!`, `file!()` or its own test name except `hostile_uploads.rs`.
- **Store and auth:** no shared helper, no `crate::`, no global state except module-scoped
  `static MIGRATOR` (`lakehouse-store/tests/uploads.rs:56`, `semantic.rs:24`, `query_limits.rs:17`,
  `upload_folders.rs:33`, `queries.rs:46`, `migrations_upgrade.rs:44`). All 41 store files and 7 of
  the 8 auth files name `lakehouse_test_support` directly; `lakehouse-auth/tests/openfga.rs` uses
  `wiremock` only.

### Everything that names a test by binary or path (checked)

- `rust/.config/nextest.toml`: only `[profile.ci]` (slow-timeout, fail-fast, junit). No
  overrides, no test groups, no filter by binary. Nothing to change. nextest runs every test in
  its own process whatever the binary layout, so isolation is kept.
- `.github/workflows/*.yml`, `ops/`, `scripts/`: no `--test <file>` anywhere (grep `--test ` is
  empty). CI runs `cargo nextest run --workspace` (`ci.yml:713`, `:726`), `cargo test --workspace
  --doc` (`:761`) and `cargo llvm-cov --workspace` (`coverage.yml:180`). None of them names a binary.
- `scripts/ci/required_gate.sh`: reads only job results; no test names.
- The count check: `ci.yml:4286-4307` sums the three partitions' `tests run` and fails below a
  fixed floor of 6,826. It does not compare against the previous run, and a floor that low cannot
  see one file's tests going missing. That is why D6 adds a guard.
- `scripts/ci/gate_paths.txt:54`: `rust/crates/*/tests/*` → `none`. Gate membership does not look
  at test binaries, so grouping the api files "by gate group" would buy nothing (D1). Note: the
  `Cargo.toml` edits match `rust/*` → `all`, and the new script matches `scripts/*` → `all`, so this
  PR runs every gate once.
- Doc lines that tell a reader to run `--test <file>` and must change because the file becomes a
  module: `permission_catalog.rs:130-132` (an assertion message) and `:137-138` (doc),
  `pdf_real_gotenberg.rs:9-10` (doc). The ones for files that stay separate do not change:
  `hostile_uploads.rs:24`, `parity.rs:24`, `:599`, `rust/tests/parity/README.md:210`,
  `lakehouse-iceberg/tests/g1_lakekeeper.rs:30`. Historical narrative comments
  (`agents_approval.rs:344-347`, `lakehouse-auth/tests/repository.rs:168`) stay as they are:
  they say what was run at the time.
- 97 plans in `docs/superpowers/plans/` and rows in `docs/core/BACKLOG.md` say `cargo test -p
  lakehouse-api --test <file>`. They are history and are not edited. AGENTS.md gets the new form
  (T7), and that is what new plans follow.

## 1. Decisions

- **D1. One `it` binary per crate for `lakehouse-api`, `lakehouse-store` and `lakehouse-auth`, not
  a few binaries per gate group.** Gate selection maps every `tests/*` file to `none`
  (`gate_paths.txt:54`), and CI's `test` job always runs the whole workspace, so a split by gate
  group would keep extra links and buy no selectivity. The only argument for a few api binaries is
  rustc's front end, which type-checks one crate on one thread: 88,389 lines in one crate is a long
  serial step. T5 measures it. If the stop rule fails (D8), the first fallback to report is "api
  split in two or three `it_*` targets", not something to improvise. The four single-file crates
  are already one binary each and stay as they are.
- **D2. The files do not move.** The root is `tests/it.rs` (not `tests/it/main.rs`), a crate root
  in `tests/`, so a plain `mod upload_routes;` there resolves `tests/upload_routes.rs` and `mod
  common;` resolves `tests/common/mod.rs`, exactly as today's per-file `mod common;` does. No
  `#[path]`, no `git mv`. Why: every `include_str!("fixtures/…")` and `include_str!("../../../migrations/…")`
  stays valid unchanged; the many docs and comments that cite `tests/route_auth.rs` and
  `tests/upload_routes.rs` (AGENTS.md included) stay true; and open PRs that edit an existing test
  file rebase without a rename. Target name `it` in all three crates.
- **D3. Three api files stay binaries of their own:** `hostile_uploads` (re-runs itself with
  `--exact <bare test name>` under `ulimit -v`), `parity` (its tooling reads a real
  `DATABASE_URL`, which the ctor would overwrite), and `identity_password` (global `tracing`
  subscriber, clashes with `report_runner` in one process under `cargo test`/`llvm-cov`). Each gets
  an explicit `[[test]]` entry with its current name, so `cargo test -p lakehouse-api --test
  hostile_uploads` and `--test parity` keep working as documented. Nothing in their bodies changes.
- **D4. `mod common;` becomes `use crate::common;`, in place, on the same line, in the 112 files
  that join `it`.** `it.rs` declares `mod common;` once. The existing `use common::{…}` lines stay
  as they are: they resolve through the `use crate::common;` import above them. If the compiler
  refuses that in a file, or `unused_imports` fires on the new line, delete the `use crate::common;`
  line in that file and rewrite its `use common::` lines to `use crate::common::`; name each such
  file in the handoff. `identity_password.rs` keeps `mod common;`.
- **D5. The only edits inside test files** are: D4's one line; the three `crate::filters::` →
  `crate::query_history::filters::` in `query_history.rs`; the two `--test permission_catalog`
  instructions and the one `--test pdf_real_gotenberg` instruction (T4). No test body, assertion
  logic, name or attribute changes (rule 2). Each changed doc or message carries `(OPS-32)`.
- **D6. Guard against a file that silently stops running:** `scripts/ci/check_test_modules.sh` with
  a self-test, `scripts/ci/tests/test_check_test_modules.sh`, run in the `Repo lints` job, which
  runs on every push and PR. Rules, for every `rust/crates/*/Cargo.toml` that has `autotests =
  false`:
  1. every `tests/*.rs` file is either the `path` of a `[[test]]` entry of that `Cargo.toml` or
     declared by a line matching `^mod <stem>;$` in exactly one `[[test]]` root;
  2. every directory `tests/<dir>/` that has a `mod.rs` is declared `mod <dir>;` in at least one
     root (`common` is declared in `it.rs` and in `identity_password.rs`; a helper may be shared);
  3. a `mod <name>;` line in a root names a file that exists (`tests/<name>.rs` or
     `tests/<name>/mod.rs`);
  4. a top-level file declared in two roots fails (its tests would be counted twice).
  A crate without `autotests = false` passes untouched (Cargo builds every file there). The script
  prints each violation as `path: reason` and exits 1. It only understands plain `mod x;` lines in
  roots; the `it.rs` header says so, and says that a `#[path]` or a `mod` inside a `cfg` block is
  not allowed there. Bash, like the other `scripts/ci/*` scripts, with the `PASS`/`FAIL` self-test
  style of `scripts/ci/tests/test_select_runner.sh:1-30`. CI's count floor (`ci.yml:4304`) stays at
  6,826: the measured total after this change is recorded (T8) and stays well above it. Raising the
  floor is the planner's call after the first green CI run, not part of this plan.
- **D7. Commits, in this order:** the guard (T2), the whole `lakehouse-api` move in ONE commit
  (T3), the api doc lines (T4), store (T5b), auth (T6), docs (T7). Why the api move is one commit:
  with `autotests = false` set, a file not yet declared stops building, and without it a declared
  file would also build alone and fail on `use crate::common;`. So every intermediate state would
  either drop tests or not compile. The move is one line per file and stays reviewable as one
  commit.
- **D8. Measure first and stop if the gain is not real.** T1 measures before and T5 after, on the
  build machine, as the owner asked. **Stop rule:** if the after time of the `lakehouse-api` build
  (T1 measurement A) is not at least 30% shorter than before, stop after T5: push the branch,
  write both measurements in the handoff, do not do T5b–T7, and report. The planner then decides
  between D1's fallback and dropping the plan.
- **D9. Measurements go in `docs/plans/OPS-32-RESULT.md`** (principle 5): before and after
  times, binary counts and sizes, test-list diff counts, machine, jobs, environment. After the PR's
  first CI run the planner adds the CI numbers (compile time per partition, run time, `Report disk
  and test binary sizes` step output) to it. Anything not measured is written as *not measured*.
- **D10. Rebase rule for test files added on `main` after this PR is opened or merged** (written
  into `it.rs`'s header and AGENTS.md by T3/T7): a new `tests/<name>.rs` in `lakehouse-api`,
  `lakehouse-store` or `lakehouse-auth` gets a `mod <name>;` line in that crate's `tests/it.rs`, in
  alphabetical order, and a `mod common;` in it becomes `use crate::common;`. An edit to an existing
  test file needs nothing. Without the `mod` line the guard (D6) fails the `Repo lints` job.

### 1b. Expected effect (estimates, stated as such; T1/T5 and the PR's CI run measure)

- **Binaries and disk.** 193 → 28 binaries. The `lakehouse-api` integration tests go from 122
  links of the whole crate to 4. `it` should be about the size of one of today's api test binaries
  plus the test code of 119 files: estimated 0.5 to 0.9 GB (with CI's `line-tables-only`) instead
  of 119 × about 0.4 GB. Store's 41 and auth's 8 links of their dependency trees become one each.
  Estimated total about 8 to 15 GB instead of about 51 GB. Basis: the 400 MB per binary and 51 GB
  measured on CI (`ci.yml:732-751`, the `Report disk and test binary sizes` step).
- **Compile time per partition.** Today's 7 to 9 minutes include building the workspace crates
  themselves (the dependency cache holds dependencies only), and that part does not change. What
  shrinks is the cost each of the 165 removed integration crates pays: loading the metadata of
  `lakehouse-api` and its dependencies, generating code for its own generic instances, and
  linking a 400 MB binary. Estimate: 2 to 4 minutes saved per partition, so about 4 to 6 minutes
  of compile, applied to each of the three partitions and to the Coverage job. The risk on the other
  side is one 88,000-line crate type-checked on one thread; T5 measures it, and D1 names the
  fallback.
- **Test time.** Unchanged in principle: nextest still starts one process per test (no `nextest.toml`
  setting changes that). Small effects either way: nextest's list phase runs each binary with
  `--list` (each run also fires the ctor), so 28 instead of 193 binaries saves list time; each test
  process now execs a larger binary, which may add a little start-up time per test. The ctor now
  also runs for the roughly 30 api tests whose binaries did not link it before. Expected net:
  within ±1 minute of 13.5; measured on the PR's CI run, not assumed.
- **Test count.** −111 exactly, all copies of the `tls_proxy` test (section 0). No other test is
  added or lost.

### Open pull requests that touch `rust/crates/*/tests/` or a crate's `Cargo.toml` (read 2026-11-13, 12 open)

- #519 `fix/dashboards-p4-s2` (BI-57 S2) edits `lakehouse-api/tests/chart_numbers.rs`. That file has
  no `mod common;` and does not move, so no conflict is expected.
- #431, #427 (dependabot: iceberg 0.11, parquet 59) edit dependency lines in
  `lakehouse-api/Cargo.toml`; T3 adds `autotests = false` in `[package]` and `[[test]]` blocks
  after `[[bin]]`. Separate hunks; a textual conflict is unlikely, and a rebase fixes one if it
  happens.
- #430 edits `lakehouse-iceberg/Cargo.toml` only: no overlap.

So few PRs are open that the slice can start now. Before pushing the final commit, rebase on
`origin/main` and apply D10 to every test file added on `main` since `7e8775fba`; the guard is the
check.

## 2. Tasks

Every Rust command runs from `rust/` in the checkout of section 3a, with the environment of
section 3a.

### T1. Measure before (no commit)

On the branch, before any change (it starts at `origin/main`; write the commit in the handoff):

- Warm-up (dependencies built with the measurement environment, not timed):
  `cargo nextest run --workspace --locked --no-run`.
- **Measurement A, the stop rule's number** (the `lakehouse-api` crate rebuilt, its dependencies
  cached; touching the file instead of `cargo clean`):
  `touch crates/lakehouse-api/src/lib.rs && /usr/bin/time -v cargo nextest run -p lakehouse-api --locked --no-run 2>&1 | tail -30`
  Record the wall time ("Elapsed (wall clock) time") and the maximum resident set size.
- **Measurement B, CI-like** (every workspace crate rebuilt, dependencies cached, as on a CI
  runner after the cache restore):
  `find crates -path '*/src/lib.rs' -o -path '*/src/main.rs' | xargs touch && /usr/bin/time -v cargo nextest run --workspace --locked --no-run 2>&1 | tail -30`
- **Sizes:** `cargo nextest list --workspace --locked --list-type binaries-only --message-format json | jq -r '."rust-binaries"[]."binary-path"' | xargs stat -c %s | awk '{s+=$1; n++} END {printf "binaries=%d bytes=%d\n", n, s}'`,
  and the same for `-p lakehouse-api` only.
- **Test list:** save the normalized list with the function below as `/tmp/ops32-before.tsv`
  (keep this file and the function outside the repository):
  ```bash
  list_tests() {  # $1 = output file
    cargo nextest list --workspace --locked --message-format json \
    | jq -r '."rust-suites"[] | ."binary-id" as $b | .testcases | to_entries[] | "\($b)\t\(.key)\t\(.value.ignored)"' \
    | awk -F'\t' 'BEGIN{OFS="\t"} {
        b=$1; t=$2
        if (b ~ /::it$/) { sub(/::it$/, "", b) }
        else if (b ~ /^[^:]+::[a-z0-9_]+$/) { split(b, p, "::"); b=p[1]; t=p[2] "::" t }
        print b, t, $3 }' \
    | LC_ALL=C sort > "$1"
  }
  ```
  It turns `lakehouse-api::upload_routes` + `name` (before) and `lakehouse-api::it` +
  `upload_routes::name` (after) into the same `lakehouse-api<TAB>upload_routes::name` line. Lib and
  bin binaries (`lakehouse-api`, `lakehouse-api::bin/lakehouse-api`) pass through unchanged. If the
  installed nextest names the JSON fields differently, adapt the `jq` paths, say how in the
  handoff, and use the same function after.
- Check: the handoff has the commit, A and B (wall, max RSS), both sizes, `wc -l
  /tmp/ops32-before.tsv`, the nextest version (`cargo nextest --version`) and `rustc -V`.

### T2. The guard: every test file is declared (`OPS-32` D6)

- New `scripts/ci/check_test_modules.sh` (executable, `set -euo pipefail`, a header comment saying
  why: with `autotests = false` an undeclared file compiles into nothing and its tests disappear
  without any failure; `OPS-32`). It takes an optional root directory argument (default: the
  repository root found from the script's location) so the self-test can point it at fixtures. It
  implements D6 rules 1–4 by reading `Cargo.toml` with `grep`/`awk` (`autotests = false` in
  `[package]`; each `[[test]]` block's `path = "…"`).
- New `scripts/ci/tests/test_check_test_modules.sh`, building throwaway crates under `mktemp -d`
  (trap removes it). Cases, each named as a full sentence in its `log_pass`/`log_fail` line:
  1. a crate without `autotests = false` and three test files passes;
  2. a crate with `autotests = false`, an `it.rs` declaring every file and `common`, passes;
  3. the same with one `mod` line removed fails and names that file;
  4. a file declared in two roots fails and names it;
  5. a `mod ghost;` line with no file fails and names `ghost`;
  6. a `tests/helpers/mod.rs` that no root declares fails and names it;
  7. a file that is the `path` of its own `[[test]]` entry and is declared nowhere passes;
  8. the real repository passes (run against the repository root).
- `.github/workflows/ci.yml`: in the `repo-lints` job, after the step at `:229-233`
  (`check_rust_embedded_paths.py`), add a step named
  `Check every Rust test file is compiled into a test binary (OPS-32)` running
  `bash ./scripts/ci/tests/test_check_test_modules.sh && ./scripts/ci/check_test_modules.sh`, with a
  two-line comment saying why (D6).
- Check: `bash scripts/ci/tests/test_check_test_modules.sh` reports 8 PASS, 0 FAIL;
  `./scripts/ci/check_test_modules.sh` exits 0 on this commit (no crate has `autotests = false`
  yet). Then, temporarily, add `autotests = false` to `crates/lakehouse-api/Cargo.toml` without
  committing it: the script must fail and list all 122 files; revert. Quote both outputs' last
  lines in the handoff.

### T3. `lakehouse-api`: one `it` binary (`OPS-32` D1–D5, D7)

- `crates/lakehouse-api/Cargo.toml`: in `[package]` (`:1-6`) add `autotests = false` with a comment:
  integration tests are the explicit `[[test]]` targets below (`OPS-32`), and a file in `tests/`
  that is not declared does not run (the guard, `scripts/ci/check_test_modules.sh`). After the
  `[[bin]]` block (`:12-14`) add four `[[test]]` blocks, each with a one-line comment saying why it
  exists:
  - `name = "it"`, `path = "tests/it.rs"`: every integration test module;
  - `name = "hostile_uploads"`, `path = "tests/hostile_uploads.rs"`: re-runs itself with
    `--exact` and a bare test name under `ulimit -v` and must not link the Postgres ctor (`SEC-22`);
  - `name = "identity_password"`, `path = "tests/identity_password.rs"`: installs a global
    `tracing` subscriber, as `report_runner` in `it` does, and one process can hold only one;
  - `name = "parity"`, `path = "tests/parity.rs"`: its opt-in tooling reads the operator's
    `DATABASE_URL`, which the test-support ctor would overwrite.
- New `crates/lakehouse-api/tests/it.rs`: a `//!` module doc (the crate root needs one,
  `missing_docs` is on) saying: what this binary is and why (`OPS-32`: one link of the crate instead
  of 119; CI runs 3 + 1 Coverage builds of it); that nextest still runs every test in its own
  process, so a test does not share state with another under CI; what plain `cargo test` and
  `cargo llvm-cov` change (one process for every module: five `ClickHouse` containers can be up at
  once, and `crypto_provider`'s check can find a provider another test installed); the three
  separate targets and why (one line each, as in `Cargo.toml`); the rule for a new file (D10); the
  guard's limits (plain `mod x;` lines only, no `#[path]`, no `cfg`). Then `mod common;` and 119
  `mod <stem>;` lines in `LC_ALL=C` sorted order, one per `tests/*.rs` file except `it`,
  `hostile_uploads`, `identity_password` and `parity`. Generate the list, do not type it:
  `ls tests/*.rs | xargs -n1 basename | sed 's/\.rs$//' | grep -vxE 'it|hostile_uploads|identity_password|parity' | LC_ALL=C sort | sed 's/.*/mod &;/'`.
- In each of the 112 files that join `it` and have `^mod common;$` (all 113 except
  `identity_password.rs`): replace that line with `use crate::common;` (D4). Use `sed -i
  's/^mod common;$/use crate::common;/'` on exactly that file list, then
  `grep -l '^mod common;' tests/*.rs` must print only `tests/identity_password.rs`.
- `tests/query_history.rs:738`, `:771`, `:775`: `crate::filters::get_history` →
  `crate::query_history::filters::get_history` (a `// OPS-32:` comment above the first one:
  `crate` is `tests/it.rs` now).
- Check (scoped, section 3): `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets
  --locked -- -D warnings`; `./scripts/ci/check_test_modules.sh` exits 0; `cargo nextest list -p
  lakehouse-api --locked --list-type binaries-only` lists exactly `lakehouse-api`,
  `lakehouse-api::bin/lakehouse-api`, `lakehouse-api::it`, `lakehouse-api::hostile_uploads`,
  `lakehouse-api::identity_password`, `lakehouse-api::parity`. A quick smoke run:
  `cargo nextest run -p lakehouse-api --locked --test it --test-threads 4 -E 'test(/^(route_auth|sec11_guard|query_history)::/)'`
  passes. The full run is T8.

### T4. `lakehouse-api`: the instructions that named a former binary (`OPS-32` D5)

- `tests/permission_catalog.rs:130-132` (the assertion message) and `:137-138` (the doc comment):
  `cargo test -p lakehouse-api --test permission_catalog write_permission_catalog_fixture --
  --ignored` → `cargo test -p lakehouse-api --test it
  permission_catalog::write_permission_catalog_fixture -- --ignored` (keep the existing line
  breaks and `\` continuations of the string; add `(OPS-32)` in the doc comment only, not in the
  message).
- `tests/pdf_real_gotenberg.rs:9-10`: `--test pdf_real_gotenberg -- --ignored` → `--test it
  pdf_real_gotenberg:: -- --ignored`.
- Check: `cargo fmt --check`; `cargo clippy -p lakehouse-api --all-targets --locked -- -D
  warnings`; `cargo test -p lakehouse-api --locked --test it permission_catalog:: -- --ignored
  --list` lists `permission_catalog::write_permission_catalog_fixture` (list only, do not run it:
  it rewrites the fixture).

### T5. Measure after and apply the stop rule (no commit)

- Repeat T1's measurements A and B and both sizes on the T4 commit, with the same environment and
  the same steps (warm-up first).
- Stop rule (D8): if A(after) > 0.7 × A(before), stop here. Push the branch, write the numbers and
  the stop in the handoff, and do nothing more. Otherwise continue.
- Check: the handoff has A, B, sizes, before and after, and the ratio A(after)/A(before).

### T5b. `lakehouse-store`: one `it` binary

- `crates/lakehouse-store/Cargo.toml`: `autotests = false` in `[package]` and one `[[test]] name =
  "it"`, `path = "tests/it.rs"`, with T3's comments (shorter).
- New `crates/lakehouse-store/tests/it.rs`: a `//!` doc (what, why, the rule for a new file,
  nextest isolation) and 41 `mod <stem>;` lines, generated as in T3 (no exclusions). No test file
  changes (no `common`, no `crate::`).
- Check: `cargo fmt --check`; `cargo clippy -p lakehouse-store --all-targets --locked -- -D
  warnings`; the guard exits 0; `cargo nextest list -p lakehouse-store --locked --list-type
  binaries-only` lists `lakehouse-store` and `lakehouse-store::it` only.

### T6. `lakehouse-auth`: one `it` binary

- Same as T5b for `crates/lakehouse-auth` (8 modules: `oidc openfga pages_per_role_migration
  password repository service_token session throttle`).
- Check: as T5b with `-p lakehouse-auth`.

### T7. Docs and the CI comment (`OPS-32`)

- `AGENTS.md`, after the bullet at `:215-218` ("CI's `test` job is …"), add:
  > - **Integration tests are one binary per crate** (`OPS-32`): `lakehouse-api`, `lakehouse-store`
  >   and `lakehouse-auth` build `tests/it.rs`, which declares every test file as a module. Run one
  >   file with `cargo test -p lakehouse-api --test it route_auth::` (or `cargo nextest run -p
  >   lakehouse-api --test it -E 'test(/^route_auth::/)'`); `--test <file>` works only for
  >   `hostile_uploads`, `identity_password` and `parity`, which stay separate. A new test file gets a
  >   `mod <name>;` line in `tests/it.rs` (and `use crate::common;` instead of `mod common;`), or the
  >   `Repo lints` job fails.
- `docs/CODE-STANDARD.md:231-232`: replace with "Unit tests in-module under `#[cfg(test)]`;
  integration tests in the crate's `tests/`, declared as modules of one `tests/it.rs` binary
  (`OPS-32`), with a shared `tests/common/mod.rs` that `it.rs` declares once."
- `docs/CI.md:226` (Rust test bullet): append "The integration tests are one binary per crate
  (`tests/it.rs`, `OPS-32`), so the job links 28 test binaries instead of 193; measurements in
  `docs/plans/OPS-32-RESULT.md`." `docs/CI.md:217` (`repo-lints` bullet): add "every Rust test
  file being declared in a test binary (`OPS-32`, `scripts/ci/check_test_modules.sh`)" to the list
  of folded lints.
- `.github/workflows/ci.yml:602-609` (the `OPS-12` comment, "links one test binary per file in
  `rust/crates/lakehouse-api/tests/` (68 of them)"): add a sentence after it: "Since `OPS-32` the
  integration tests of `lakehouse-api`, `lakehouse-store` and `lakehouse-auth` are one binary per
  crate (`tests/it.rs`); the `line-tables-only` setting stays." Comment only; no command changes.
- New `docs/plans/OPS-32-RESULT.md` (D9): machine, `rustc -V`, nextest version, environment
  (section 3a), measurements A and B before and after, binary counts and sizes before and after,
  T8's list diff counts and the full run counts. A `## CI` heading with the line "*Not measured
  yet: the planner adds the PR's CI numbers.*"
- Check: `git diff --stat` shows only these five files; no test or Rust change in this commit.

### T8. Prove every test is still there and passes (no commit; numbers go in the handoff and the RESULT file)

- `list_tests /tmp/ops32-after.tsv` (T1's function), then:
  - `LC_ALL=C comm -23 /tmp/ops32-before.tsv /tmp/ops32-after.tsv` (only before) must be exactly
    112 lines, each `lakehouse-api<TAB><file>::common::tls_proxy::tests::the_proxy_terminates_tls_with_the_certificate_it_advertises<TAB>false`,
    one per file of `it` that had `mod common;`;
  - `LC_ALL=C comm -13 …` (only after) must be exactly 1 line:
    `lakehouse-api<TAB>common::tls_proxy::tests::the_proxy_terminates_tls_with_the_certificate_it_advertises<TAB>false`;
  - `wc -l` of both files: after = before − 111.
  Any other line, a changed `ignored` flag included, is a stop: report it, do not fix it by
  editing a test.
- The full runs, in the foreground, once (section 3a):
  `cargo nextest run -p lakehouse-api --locked --profile ci --test-threads 4 --color never 2>&1 | tee /tmp/ops32-api.log`,
  then the same for `-p lakehouse-store` and `-p lakehouse-auth`. Quote each summary line (`N tests
  run: N passed, N skipped`). If a test fails, run that one test on the base commit (`git switch
  --detach 7e8775fba`, the same command with `-E 'test(=<name>)'` and the old binary, `--test
  <file>`), then `git switch refactor/one-test-binary`. A test that fails on both is environmental:
  list it with the reason. A test that fails only after the change is a stop.
- Then the repo verification of section 3 (the scripts and lints this PR touched).

## 3. Verification

Per commit (scoped; AGENTS.md "Keep build time down"):

- T2: `bash scripts/ci/tests/test_check_test_modules.sh && ./scripts/ci/check_test_modules.sh`;
  `docker compose` is not touched; no cargo.
- T3, T4, T5b, T6: `cargo fmt --check`, `cargo clippy -p <crate> --all-targets --locked -- -D
  warnings`, `./scripts/ci/check_test_modules.sh`, and the task's own check.
- T7: `./scripts/ci/check_test_modules.sh`; `python3 ops/lint/check_gate_groups.py` (it reads
  `ci.yml`); no cargo.
- Every commit: `git status` (no `target/`, no `/tmp/ops32-*`, nothing generated staged).

Before handoff, in the foreground, on the final commit (T8 plus):

```bash
cd rust && cargo fmt --check \
  && cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo nextest run -p lakehouse-api   --locked --profile ci --test-threads 4
cargo nextest run -p lakehouse-store --locked --profile ci --test-threads 4
cargo nextest run -p lakehouse-auth  --locked --profile ci --test-threads 4
cd .. && bash scripts/ci/tests/test_check_test_modules.sh && ./scripts/ci/check_test_modules.sh
python3 ops/lint/check_intra_package_imports.py && python3 ops/lint/check_bare_iceberg_count.py
python3 ops/lint/check_gate_groups.py && python3 ops/lint/check_rust_embedded_paths.py
git status
```

The rest of the workspace's tests (`lakehouse-bi`, `-embed`, `-iceberg`, `-notify`, every lib and
bin unit-test binary) did not change. They are left to CI and listed in the handoff as *not
verified locally, CI*. Coverage (`coverage.yml`, `cargo llvm-cov`, one process per binary) is not
run locally either; it runs on the first push to `main` that changes the Rust tests, and the
reviewer names its result.

## 3a. Change of machine

This slice is built on the ARM64 build machine, not on the planner's VM. This section replaces the
VM's limits from the common brief for this plan only.

- Checkout: `/home/rismana/rantai/lakehouse-agents-3`. First: `git fetch -q origin && git switch
  -c refactor/one-test-binary origin/main`. Quote `git rev-parse --short HEAD`; if it is not
  `7e8775fba`, re-check the anchors of section 0 that the tasks edit, and report any that moved.
- Environment, in every shell:
  ```bash
  export CARGO_TARGET_DIR=/home/rismana/rantai/cargo-target-3 CARGO_BUILD_JOBS=6
  export CARGO_INCREMENTAL=0                      # as on CI (rust-cache); keeps T1/T5 honest
  export CARGO_PROFILE_DEV_DEBUG=line-tables-only CARGO_PROFILE_TEST_DEBUG=line-tables-only  # as ci.yml:622-623
  ```
  The two `DEBUG` variables change the dependencies' build hash, so the first build rebuilds the
  dependencies once next to the existing ones. Check `df -h /home/rismana/rantai` first. If less
  than 60 GB is free, leave the two `DEBUG` variables out for the whole slice and say so in the
  RESULT file (the sizes are then not CI-comparable; the ratio is).
- Plain `cargo` (no `cargo-locked` wrapper on that machine). `cargo nextest`: if `cargo nextest
  --version` fails, `cargo install cargo-nextest --locked`.
- `--test-threads 4` on every `nextest run`: a CI runner runs 4 tests at once. On this machine the
  default (one per core) would open more Postgres connections than the shared server's
  `max_connections=200` allows (`lakehouse-test-support/src/lib.rs:280-291`), and start a
  `ClickHouse` container per process for the five modules that use one.
- For this slice the developer DOES run the touched crates' whole test suites once, at the end
  (T8): moving every test file must be proven to keep every test. That is an exception to the
  common brief's "integration tests are left to CI" rule, for this plan only.
- Never `cargo clean` (not even `-p`). If the disk fills, delete only
  `$CARGO_TARGET_DIR/debug/incremental`, and say so in the handoff.
- Docker: before the first run, `docker ps -aq | sort > /tmp/ops32-containers-before.txt`. After
  T8, remove only the containers your runs started (`comm -13` against a fresh `docker ps -aq |
  sort`), except the labelled shared Postgres
  (`docker ps -aq --filter label=org.rantai.lakehouse-test-support`), which stays: other checkouts
  reuse it.
- Push after every commit (`git push -u origin refactor/one-test-binary`). Do not open a PR: the
  planner does.
- Commit messages: `refactor(tests): … (OPS-32)` / `ci: … (OPS-32)` / `docs: … (OPS-32)`, the body
  saying what moved and the check run with its counts, ending with the attribution line of your
  environment. No hostnames, IPs or people's names.

## 4. Out of scope

- Splitting or merging the crates' lib or bin unit-test binaries (the 18 others), and the four
  single-file crates.
- Moving or renaming any test file (D2), changing any test body, name, `#[ignore]` or attribute
  (D5), and making `hostile_uploads`, `parity` or `identity_password` fit into `it` (D3); each
  would be a test change with its own review.
- Changing `nextest.toml`, the partition count or method, the CI count floor (D6), the
  `line-tables-only` setting, the linker, or `sccache`/cache settings.
- `cargo nextest archive` / compile-once (`OPS-30`, cancelled).
- Editing historical plans or backlog rows that say `--test <file>`.
- The backlog row: the planner adds `OPS-32` to `docs/core/BACKLOG.md` (and the base) when the PR is
  opened, not the developer.

## 5. Handoff

Developer: a DeepSeek V4.1 Flash (OpenCode) session, on the ARM64 build machine of section 3a,
checkout `/home/rismana/rantai/lakehouse-agents-3`, branch `refactor/one-test-binary` off
`origin/refactor/one-test-binary` at `a3a40fb0` (the plan commit; the base `main` it sits on is
`7e8775fba`). Every commit was pushed to `refactor/one-test-binary`; no pull request was opened.

### Commits

| Commit | What |
|---|---|
| `59c47764` | `ci: fail when a Rust test file is not compiled into a test binary (OPS-32 D6)`: the guard, its 8-case self-test, the `Repo lints` step |
| `a2d7aad1` | `refactor(tests): lakehouse-api builds one integration-test binary (OPS-32)`: T3, one commit as D7 requires (manifest, `tests/it.rs`, 112 `use crate::common;`, the three `crate::query_history::` paths) |
| `a2748e51` | `docs(tests): the two instructions that named a former test binary (OPS-32 D5)`: T4 |
| `64b20368` | `refactor(tests): lakehouse-store builds one integration-test binary (OPS-32)`: T5b |
| `28a3895f` | `refactor(tests): lakehouse-auth builds one integration-test binary (OPS-32)`: T6 |
| `0c14ae6e` | `docs: one test binary per crate in AGENTS.md, CI.md, the code standard and the CI comment (OPS-32)`: T7, five files, plus `docs/plans/OPS-32-RESULT.md` |
| this one | `docs(plan): handoff for OPS-32` |

### Measurements (detail and the deviation log in `docs/plans/OPS-32-RESULT.md`)

- **A, the stop rule's number** (`lakehouse-api` rebuilt, dependencies cached): **14:04.09 before,
  0:48.59 after**, 5.8 % of the before time. Stop rule (D8, "at least 30 % shorter") **passed**, so
  T5b–T7 were done.
- **B, CI-like** (every crate rebuilt): **13:58.89 before, 1:17.90 after**, 9.3 %.
- **Binaries and size** (`nextest list --list-type binaries-only` + `stat`): 191 binaries /
  142.4 GB before, **26 binaries / 8.3 GB after**; `lakehouse-api` alone 124 / 128.4 GB before,
  6 / 4.9 GB after T4.
- **Test list**: 7,826 before, 7,715 after. `comm -23` = exactly **112** lines, one per `it`
  module that had `mod common;`, every one the `tls_proxy` test; `comm -13` = exactly **1** line,
  `lakehouse-api<TAB>common::tls_proxy::…<TAB>false`; no other line, no changed `ignored` flag.
  **No test was lost**: −111, the copies of that one test.

### Commands run, with counts

Per commit, scoped (all from `rust/` unless said otherwise):

- T2 (`59c47764`): `bash scripts/ci/tests/test_check_test_modules.sh` → 8 passed, 0 failed;
  `./scripts/ci/check_test_modules.sh` → exit 0; with a temporary `autotests = false` in
  `lakehouse-api/Cargo.toml` (reverted, not committed) → exit 1 and 122 `tests/*.rs` lines plus
  `tests/common/mod.rs`; `python3 ops/lint/check_gate_groups.py` → consistent; no cargo.
- T3 (`a2d7aad1`): `cargo fmt --check` → 0; `cargo check --workspace --all-targets --locked` → no
  error, no warning; `cargo clippy -p lakehouse-api --all-targets --locked -- -D warnings` → 0
  (1m45s); `./scripts/ci/check_test_modules.sh` → 0; `cargo nextest list -p lakehouse-api
  --list-type binaries-only` → exactly `lakehouse-api`, `lakehouse-api::bin/lakehouse-api`,
  `lakehouse-api::it`, `lakehouse-api::hostile_uploads`, `lakehouse-api::identity_password`,
  `lakehouse-api::parity`; smoke run
  `cargo nextest run -p lakehouse-api --test it --test-threads 4 -E 'test(/^(route_auth|sec11_guard|query_history)::/)'`
  → 126 tests run, 126 passed, 1387 skipped.
- T4 (`a2748e51`): `cargo fmt --check` → 0; `cargo clippy -p lakehouse-api …` → 0;
  `cargo test -p lakehouse-api --test it permission_catalog:: -- --ignored --list` → 1 test, listed
  and not run.
- T5b (`64b20368`), T6 (`28a3895f`): `cargo fmt --check` → 0; `cargo clippy -p lakehouse-store` /
  `-p lakehouse-auth --all-targets --locked -- -D warnings` → 0; guard → 0;
  `nextest list --list-type binaries-only` → exactly `<crate>` and `<crate>::it`.
- T7 (`0c14ae6e`): `./scripts/ci/check_test_modules.sh` → 0; `python3 ops/lint/check_gate_groups.py`
  → consistent; no cargo.
- Once, before this handoff, on `0c14ae6e`'s tree: `cargo fmt --check` → 0;
  `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` → 0 (1m13s);
  `cargo nextest run -p lakehouse-api --locked --profile ci --test-threads 4` → **5,168 tests run:
  5,168 passed (2 slow), 27 skipped** (832 s); `-p lakehouse-store` → **1,045 run: 1,045 passed,
  0 skipped** (143 s); `-p lakehouse-auth` → **179 run: 179 passed, 0 skipped** (59 s);
  guard self-test → 8/0; `python3 ops/lint/check_intra_package_imports.py`,
  `check_bare_iceberg_count.py`, `check_gate_groups.py`, `check_rust_embedded_paths.py` → all 0;
  `git status` → clean.
- Containers: `docker ps -aq` snapshotted before the first run; the 44 my runs started were removed
  afterwards, nothing else. The shared labelled Postgres was left up.

### Not verified

- `lakehouse-bi`, `-embed`, `-iceberg`, `-notify` and every lib/bin unit-test binary: unchanged by
  this slice, left to CI (the plan's verification section says so).
- `cargo llvm-cov` / the Coverage workflow: not run locally. Written in `it.rs`'s doc instead.
- `docker compose` is not touched; no compose command was run.
- CI's compile and run times: the planner's, after the first PR run (`OPS-32-RESULT.md`, `## CI`).

### Deviations from the plan's section 3a (all in `docs/plans/OPS-32-RESULT.md`)

1. `CARGO_PROFILE_DEV_DEBUG` / `CARGO_PROFILE_TEST_DEBUG` **left unset**: the disk had 24 GB free,
   below the 60 GB the section names, so its own fallback applies. **Sizes are not
   CI-comparable; the ratios are.**
2. `CARGO_INCREMENTAL` left at Cargo's default (`1`), not `0`: the flag is part of the unit hash, so
   switching would have re-hashed and rebuilt ~250 GB on a 24 GB disk. Both sides ran with the same
   setting.
3. Measurement A is workspace-scoped (`--workspace --no-run` after touching
   `lakehouse-api/src/lib.rs`), not `-p lakehouse-api`: the `-p` resolution needs 122 api test
   binaries (~128 GB) beside the workspace copies (~142 GB). Same scope on both sides.
4. The shared target directory filled three times; `debug/incremental` was deleted each time (36 GB,
   7.5 GB, 33 GB), never `cargo clean`. The only other deletion was the partial output of a
   `-p lakehouse-api` build that had failed for want of space and that this plan does not use.

### MISMATCH (code as found vs. what the plan says)

- **MISMATCH: the binary count.** The plan's inventory says 193 integration-and-unit binaries at
  `7e8775fba`, from CI's own file count; `cargo nextest list --workspace --list-type binaries-only`
  on this checkout lists **191** (122 + 41 + 8 + 4 integration + 16 lib/bin unit tests). Nothing was
  missing from the tests themselves; the difference is how the two counts are taken. The plan's
  "after" estimate of 28 is likewise **26** measured (2 fewer lib/bin unit binaries than assumed).
  Recorded in `OPS-32-RESULT.md`; no test or target was changed to reach a number.
- **No other mismatch.** The inventory held exactly: 122 `tests/*.rs` in `lakehouse-api` (119
  modules, 113 with `mod common;`, one of them `identity_password`), 41 store files, 8 auth files,
  `tests/common/mod.rs` with the `tls_proxy` include, the five `#[path]`-free `mod common;` files
  further down, `query_history.rs`'s three `crate::filters::get_history` calls at `:738`, `:771`,
  `:775`, and `permission_catalog.rs` / `pdf_real_gotenberg.rs`'s `--test` lines.
- **One file the compiler rejected as D4 anticipated:** `query_history.rs`. Its `use crate::common;`
  line was unused (every module in it already reaches the harness as `crate::common::…` from inside
  the module), so that line is dropped, per D4, with a comment saying why. It is the only such file.

## 6. Review
