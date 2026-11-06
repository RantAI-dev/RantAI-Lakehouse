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
