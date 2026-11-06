# REL-20: Rust tests in CI in about 15 minutes, not 40 to 50

Backlog `REL-20` (P1, Delivery). Planner: Claude Opus, 2026-11-06, at the owner's request
("I need this fast"). Anchors verified at `main` `d2b26f45`. Developer: DeepSeek. Two PRs,
built in parallel: slice A (one build of the API crate) and slice B (parallel test runs).
No test is deleted, skipped or weakened (AGENTS.md rule 2).

## 0. Measured (PR run 42151467857, job "Rust · test (stable)")

- The job: 49 min, of which `cargo test --workspace --locked` (`.github/workflows/ci.yml:462-463`)
  is 2950 s. Compile about 10 min, then 185 test binaries run one after another: sum
  2325 s; 9921 passed, 52 ignored.
- The API's unit tests run twice: `unittests src/lib.rs` (3114 tests, 434 s) and
  `unittests src/main.rs` (3152 tests, 424 s). `rust/crates/lakehouse-api/src/main.rs`
  declares the whole module tree again (`mod auth; mod config; …`, 68 `mod` lines) beside
  `src/lib.rs` ("Library-target mirror of the binary's module tree", 34+ `pub mod`), so the
  crate is compiled twice (test and clippy too) and every unit test runs twice.
- `tests/upload_routes.rs`: 175 tests, 291 s.
- Every other PR job finishes within about 16 min (slowest gate about 14 min).

## 1. Decisions

- **D1 (slice A). One module tree.** `src/main.rs` keeps only the binary's own code (the
  `main` function, start-up wiring) and reaches everything else through
  `use lakehouse_api::…`. Every `mod` line in `main.rs` goes. Items `main.rs` needs that are
  private in the lib become `pub` (or `pub(crate)` stays and the code moves into the lib,
  e.g. a `lakehouse_api::run()`); prefer moving start-up code into the lib over widening
  visibility of internals. Unit tests that exist only under `main.rs` (3152 − 3114 = 38,
  `main.rs`'s own `#[cfg(test)]` modules) move with the code they test, and still run.
  `lib.rs`'s module doc is rewritten: it is the crate, not a mirror. Behaviour is unchanged.
- **D2 (slice A).** The count is the proof: after the change, `cargo test -p lakehouse-api`
  lists one `unittests src/lib.rs` binary whose test count is at least the number of distinct
  tests before (3152 if all `main.rs` tests are distinct from lib's; quote both). No
  `unittests src/main.rs` binary with thousands of tests remains (a handful for `main.rs`
  itself is fine).
- **D3 (slice B). cargo-nextest.** The Rust test job runs `cargo nextest run --workspace
  --locked` (install with `taiki-e/install-action@v2` pinned by SHA, tool `cargo-nextest`),
  then `cargo test --workspace --locked --doc` for doctests (nextest does not run them).
  A `.config/nextest.toml` at `rust/` sets `slow-timeout` (60 s, terminate after 10 periods)
  and `fail-fast = false` in a `ci` profile, and JUnit output for the job summary.
- **D4 (slice B). Three runners.** The job becomes a matrix of 3 partitions:
  build once per partition from the shared cache, `--partition count:N/3`
  (or `hash:` if counts are uneven). Total tests across the partitions must equal the
  single-run count: the job summary prints each partition's count and a final step checks
  the sum is not lower than `main`'s (pass the expected minimum as the number measured in
  this PR's first run; write it in the plan's handoff).
- **D5 (slice B). Required checks.** Whatever aggregates required checks (`ci-required`
  job, branch protection names) is updated so the three partitions are required and the
  old single job name is not waited for forever. List what you changed; if branch
  protection itself must change in GitHub settings, say so in the handoff (the planner does
  it).
- **D6 (slice B). Tests that break under nextest** (process per test: global state, env
  vars, fixed ports, a shared database name) are fixed in the test, never skipped or
  `#[ignore]`d. Each fix says why at the site and cites `REL-20`. If more than five break,
  stop and report the list before fixing.
- **D7.** `docs/CI.md` and `AGENTS.md` "Keep build time down" mention nextest and the
  partitions in one or two lines (slice B). Local developers may keep `cargo test`.

## 2. Tasks

| # | Slice | Task | Acceptance |
| --- | --- | --- | --- |
| A1 | A | D1: `main.rs` uses the lib; start-up code moved into the lib where needed | `cargo clippy -p lakehouse-api --all-targets -- -D warnings` clean; the binary starts (`cargo run -p lakehouse-api` reaches "listening" or its config error, same as before) |
| A2 | A | D2: counts | `cargo test -p lakehouse-api 2>&1 \| grep -E 'Running\|test result'` before and after, quoted; no test lost |
| B1 | B | D3 + D4 + D5 in `.github/workflows/ci.yml`, `rust/.config/nextest.toml` | The PR's CI: the three partitions pass; quote their wall times and test counts, and the sum vs 9921 |
| B2 | B | D6 fixes, if any | Each named in the handoff |
| B3 | B | D7 docs | — |

## 3. Out of scope

Running fewer tests per PR (changed-crate selection). Speeding up `upload_routes.rs`
(next step if needed). The gates (`ops/g*`).

## 4. Handoff

### Handoff (slice B, D6)

**Developer report, 2026-11-06.** Branch `ci/rel-20-nextest`, off
`origin/ci/rel-20-nextest` `5b9025ba` ("ci(rel-20): one count artifact per
partition (review R1)"). Two commits: the fix, then this entry. No PR opened
or merged.

#### The two failures (D6)

Both in `rust/crates/lakehouse-api/src/routes/metrics.rs`, `mod tests`:

| Test | The `unwrap` that failed (before the fix) |
|---|---|
| `a_chart_the_caller_cannot_see_blocks_a_delete_without_being_named` | `metrics.rs:1075` |
| `a_chart_on_a_restricted_dashboard_blocks_a_delete_without_being_named` | `metrics.rs:1142` |

Both failed with `Unprocessable("The metric could not be read or saved.")` —
the fixed message of `classify_ch` (`routes/metrics.rs:52-54`), so no
upstream error text reached the assertion.

#### Cause

`lakehouse_bi::store::ensure_bi_table` runs the BI DDL bootstrap at most once
per PROCESS, guarded by the `BI_TABLE_ENSURED` atomic
(`rust/crates/lakehouse-bi/src/store.rs:676`). Both tests read through
`Seen::read` (`routes/dashboard_organise.rs:314`), which calls
`store::list_stored_charts` and `store::list_boards`; each of those calls
`ensure_bi_table` first (`store.rs:3041`, `store.rs:1328`).

Under `cargo test` one process runs the whole binary, so an earlier test had
already run the bootstrap and the guard was set: the DDL never reached the
test's `wiremock` server. `cargo-nextest` gives every test its own process,
so the guard starts "not started", the DDL goes to THIS test's mock server,
the unanswered statement is a 404, and the read that follows it fails.

#### The fix (in the test, not in production code)

One helper, `answer_bi_ddl`, in the same `mod tests`, answers the bootstrap:

- the 36 DDL statements of `ensure_bi_table_uncached` and its three helpers
  (`store.rs:989`, `:728`, `:923`, `:1121`). Every one of them carries `IF NOT
  EXISTS` (checked statement by statement), and `ChClient::exec` discards the
  body (`lakehouse-clickhouse/src/lib.rs:453-470`), so an empty 200 is enough;
- the bootstrap's last step, `copy_legacy_embed_state`'s read of
  `system.columns` (`store.rs:818-825`), which asks whether the legacy embed
  columns are still on `console.bi_board`. It is a read, so it needs a body:
  no rows means "no legacy columns", the copy is skipped, exactly as on a
  fresh database.

Both tests call it before they read. No assertion was changed, no test was
skipped or `#[ignore]`d, and no production file was touched: D6 fixes the
test, not the code.

#### Commands, as run

All from `rust/`, with `CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target` (the
shared target dir) and `CARGO_BUILD_JOBS=2` (the memory cap on this machine).

- **Before the fix** — the nextest condition reproduced locally by running
  the one test in its own process (`cargo test -p lakehouse-api --lib
  routes::metrics::tests::a_chart_the_caller_cannot_see_blocks_a_delete_without_being_named
  -- --exact`): **FAILED** with
  `called 'Result::unwrap()' on an 'Err' value: Unprocessable("The metric could not be read or saved. Reference: 1e7792a948")`
  at `metrics.rs:1075:63`.
- `cargo fmt --check` — no output, exit 0.
- `cargo test -p lakehouse-api --lib routes::metrics` — **3 passed; 0 failed;
  0 ignored; 3136 filtered out**.
- `cargo test -p lakehouse-api --lib routes::metrics::tests::a_chart_the_caller_cannot_see_blocks_a_delete_without_being_named -- --exact` —
  **1 passed; 0 failed; 0 ignored; 3138 filtered out**. Its own process, so
  the `BI_TABLE_ENSURED` guard starts unset, exactly as under nextest.
- `cargo test -p lakehouse-api --lib routes::metrics::tests::a_chart_on_a_restricted_dashboard_blocks_a_delete_without_being_named -- --exact` —
  **1 passed; 0 failed; 0 ignored; 3138 filtered out**. Same.
- `cargo clippy -p lakehouse-api --all-targets -- -D warnings` — clean, no
  warnings.
- `cargo test -p lakehouse-api --lib` (the whole crate's unit tests, with the
  fix) — **3121 passed; 0 failed; 18 ignored** (401.89 s). The base commit
  `5b9025ba` gives the same count on the same machine: **3121 passed; 0
  failed; 18 ignored** (412.51 s).

#### Not verified

- **`cargo nextest run` itself.** cargo-nextest is not installed on this
  machine and the task says not to install it, so this is **not verified
  locally, CI proves it**. The local stand-in for "each test in its own
  process" is `cargo test --lib <one test> -- --exact`: the test binary
  starts fresh, so `BI_TABLE_ENSURED` starts "not started" and the bootstrap
  reaches the test's own mock server, which is the condition nextest
  creates. That is how the failure was reproduced before the fix and shown to
  be gone after it. The CI `test` job runs `cargo nextest run --workspace
  --locked --profile ci --color never --partition "count:N/3"`
  (`.github/workflows/ci.yml:486-490`) and is the real proof.
- **The three partitions' counts** are not moved by this slice: the commit
  touches no test, it fixes two of them.

#### One note for the reviewer

The FIRST full-crate run of `cargo test -p lakehouse-api --lib` with the fix
in place had one failure,
`routes::alerts::alert_run_tenant_scope::every_mart_rule_is_refused_with_d6_and_the_next_rule_still_runs`
(a `#[sqlx::test]`): `failed to connect test pool: database "_sqlx_test_..."
does not exist. It seems to have just been dropped or renamed.` That is the
test-database sweeper of `lakehouse-test-support` racing with another cargo
process on the shared Postgres container — the repo documents the hazard
(`lakehouse-test-support/src/lib.rs:77-87`, and `:100-105`: "the sweep of one
process can drop a database that a test of another process has just created
and has not connected to yet"), and another developer was running cargo in the
main checkout at the same time. The same test passes in isolation and on the
base commit, and the second full-crate run with the fix in place is **3121
passed; 0 failed; 18 ignored**. Nothing in this slice touches that path.

## 5. Review

- **R1 (BLOCKER, fixed by the planner in CI config).** The three partitions uploaded their
  count under one artifact name, `test-counts`; `upload-artifact` v4 and later refuse a
  second upload under an existing name, so partitions 2 and 3 would fail. Now one artifact
  per partition (`test-counts-N`), downloaded with `pattern: test-counts-*`.
- **R2 (SHOULD-FIX, for whichever slice merges second).** The sum check's floor is 9921, the
  count with the API's unit tests run twice. Slice A removes the duplicate copy (about
  3,100 tests), so the floor must be lowered in the same PR to the new measured total from
  that PR's CI, with the before/after numbers in the comment. Never lower it for any other
  reason (AGENTS.md rule 2).
