# CI speed PR 3: faster Rust test job

Parent plan: `docs/plans/CI-SPEED-PLAN.md`, sections 4.3 (steps 1–4), 6
(row 3), 7 ("PR 3" and "Every PR"), 8 and 9. Step 5 (MSRV as a compile
check) shipped in PR 1 (#88). Where this file and the parent disagree,
this file wins and says why.

- Branch: `ci/pr3-fast-rust-tests`, cut from `main` at `a78ad62`. PR 2
  (`ci/pr2-images-once`) is expected to merge first; the planner merges
  `main` into this branch before review.
- Anchors below were verified at `a78ad62`. Re-check them before editing.
  If one is *wrong*, stop and report.
- Roles: `AGENTS.md`.

## Decisions already made

1. **The binary stops re-declaring the library's modules; its own tests
   stay where they are.** (Revised 2026-10-09; see "Plan correction"
   below.) `main.rs:10-42` declares with `mod` the same 32 modules that
   `lib.rs:26-58` declares with `pub mod`, so every module, and every
   unit test in it, is compiled twice. Delete those `mod` lines from
   `main.rs` and import what `main.rs` uses from the library instead
   (`use lakehouse_api::{config, policy, routes, state, …};`), keeping the
   names `main.rs` and its tests already use (`routes::router`,
   `config::Config`, `Config`, `AppState`, `policy::POLICY_TABLE`, …).
   - `main.rs`'s own `mod tests` (46 tests: 31 router tests and the 15 in
     `agent_run_service_bootstrap`, which call functions private to
     `main.rs`) is **not moved and not edited**. The `[[bin]]` keeps
     compiling tests; do **not** set `test = false`.
   - No function body changes. No item moves between files.
   - If an item `main.rs` uses is `pub(crate)` or private in the library,
     change it to `pub`, add the `# Errors` / doc lines `AGENTS.md`
     requires for a public item if it lacks them, and list every such
     item in the handoff. Widen nothing that `main.rs` does not use.
   - If the library then reports an item as unused, or the change needs
     anything beyond imports and visibility, stop and report.
   This is the "thin bin, real lib" split `lib.rs`'s own module doc
   describes as the intent. It also halves the workspace compile in the
   release image build, which currently builds these modules in both
   targets.
2. **No test stops running.** The only names allowed to disappear are the
   copies the binary compiled from the shared modules. Every name under
   the binary's own `tests::` (all 46) must still be listed by the
   binary. Proven by a name diff (T1).
3. **`cargo-nextest`, pinned, no retries.** `taiki-e/install-action` with
   an exact `nextest@<version>`. `rust/.config/nextest.toml` sets
   `retries = 0` with a comment: a flaky test is fixed or pinned to a
   test group, never retried into passing. Doc tests run in a second step,
   `cargo test --workspace --doc --locked`.
4. **Postgres is started by the harness itself, once, before the run.**
   The parent plan says to start a container "using the same image, tag
   and label that `lakehouse-test-support` looks for". Copying those
   constants into YAML would be a second owner for them, and
   `testcontainers`' reuse match is on more than the label. Instead add a
   step that runs the harness's own test binary first, serially:
   `cargo test -p lakehouse-test-support --lib --locked`. Its `#[ctor]`
   (`rust/crates/lakehouse-test-support/src/lib.rs:140`) creates the
   container; every later process finds it. Confirm the binary really
   links the constructor (the container exists afterwards); if it does
   not, stop and report.
5. **Exactly one container is asserted, not assumed.** After the pre-start
   and again after the test run, a step counts
   `docker ps -q --filter label=org.rantai.lakehouse-test-support=postgres-16`
   and fails unless it is 1.
6. **Debug info is trimmed in CI only.**
   `CARGO_PROFILE_DEV_DEBUG: line-tables-only` in the `test` job's `env:`.
   `Cargo.toml` profiles do not change.
7. **Fallback is pre-approved under a stated condition.** If, measured
   locally on this branch, the nextest step is slower than
   `cargo test --workspace` was on the same machine in the same sitting,
   or any test fails under nextest that passes under `cargo test` and
   cannot be fixed by a test group, then drop nextest and split the job
   by crate instead (parent plan 4.3): `lakehouse-api` alone, the rest in
   one or two jobs, each `cargo test -p …`, with the gate and its
   self-tests updated for the new job names. Record the numbers that
   triggered it. Decisions 1, 2, 5 and 6 still apply.
8. **The `Setup Bun` step stays** (`ci.yml:262`; `lakehouse-bi`'s
   `specs_match_typescript` needs it).
9. **Out of scope:** the `Cache cargo` step of the `test` job (PR 2 owns
   it), every other job, `coverage.yml` (it keeps `cargo llvm-cov`),
   merging integration test files (parent plan 4.6), any change to a test
   body.

## Anchors

| What | Where (`a78ad62`) |
| --- | --- |
| Binary's own tests (46; do not move) | `rust/crates/lakehouse-api/src/main.rs:774-end`, nested `agent_run_service_bootstrap` from `:1105` |
| Functions those 15 call, private to `main.rs` | `main.rs:238-272` (constants), `:344-606` |
| Modules declared twice | `main.rs:10-42`, `lib.rs:26-58` |
| `lib.rs` says `main.rs` is untouched | `lib.rs:18-23` (update: it no longer carries tests) |
| Shared Postgres, label, reuse | `rust/crates/lakehouse-test-support/src/lib.rs:88-92`, `140` |
| The test job | `.github/workflows/ci.yml:239-271` |

## Tasks

One task per commit, in order. Rust work is T1 only; do it in one
sitting.

### T1. One copy of the `lakehouse-api` unit tests

Before changing anything, on `a78ad62`, save the test names:

```bash
cd rust
cargo test --workspace --locked -- --list 2>/dev/null | grep ': test$' | sort > /tmp/before-all.txt
cargo test -p lakehouse-api --lib --locked -- --list | grep ': test$' | sort > /tmp/before-lib.txt
cargo test -p lakehouse-api --bin lakehouse-api --locked -- --list | grep ': test$' | sort > /tmp/before-bin.txt
```

Then apply decision 1 and update `lib.rs`'s module doc (`lib.rs:1-23`
says the binary keeps its own private `mod` declarations; that stops
being true).

Acceptance:
- After the change the binary lists exactly the names under its own
  `tests::` module, 46 of them, and each was in `before-bin`.
- Every name in `before-bin` that the binary no longer lists is in
  `before-lib` (it was a copy, and the library still runs it). Quote the
  counts: before-bin, after-bin, removed, removed-and-in-lib.
- `after-lib` equals `before-lib`.
- `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets --all-features -- -D warnings`
  and `cargo test -p lakehouse-api` pass; quote the counts.
- `tests/parity.rs` still finds the binary (`CARGO_BIN_EXE_lakehouse-api`).
- `sg docker -c 'docker build …'` of `rust/Dockerfile` still succeeds
  (the release build of the binary is the product). One build, quoted.

### T2. nextest in the `test` job

Decisions 3, 4, 5, 6. Install nextest locally to prove it before writing
the YAML.

Acceptance, all local, quoted in the handoff:
- `cargo nextest list --workspace` plus the doc-test list equals the
  post-T1 `cargo test` list. Quote counts and the (empty) diff.
- Three consecutive full local runs pass with `retries = 0`. Any test
  pinned to a test group carries a comment naming the shared state it
  protects.
- One Postgres container before and after each run.
- Wall time of `cargo nextest run --workspace` against
  `cargo test --workspace` (tests only, both already compiled), same
  sitting. This is the number decision 7 turns on.

### T3. `docs/CI.md`

Describe the test job as it now stands, including that the binary now
imports the library instead of compiling its modules a second time, and
the no-retry rule. Update
the status line of `docs/plans/CI-SPEED-PLAN.md`, and correct its 3.3
cause 3 ("the tests compile into both targets") to say the binary also
had 46 tests of its own, 15 of them on functions private to `main.rs`,
and its 4.3 step 1 to say what was done instead of `test = false`.

## Verification before handoff

```bash
cd rust && cargo fmt --check \
  && cargo clippy --workspace --all-targets --all-features -- -D warnings \
  && cargo test --workspace
bash scripts/ci/tests/test_required_gate.sh
git status
```

Local notes: wrap commands that need Docker in `sg docker -c '…'`. Use
`CARGO_TARGET_DIR=/home/shiro/rantai/cargo-target`; never `cargo clean`.
The machine has 6 cores and 7 GB of memory: one heavy build at a time,
`-j 4` if another agent is building.

## What the developer cannot verify, and must say so

CI: the job's time on a runner (target 8 minutes or less), five
consecutive green runs, and the debug-info setting's effect on runner
disk. The planner measures those on the PR and records them under Review.

## Plan correction (planner, 2026-10-09)

The first version of decision 1 said to move `main.rs`'s `mod tests` to
`tests/router_registration.rs` and set `test = false` on the binary. The
planner had read only the first tests in that module. The developer
stopped, correctly, before writing anything: 15 of the 46 tests sit in a
nested `agent_run_service_bootstrap` module and call six `bootstrap_*`
functions and six constants that are private to `main.rs`, and nothing
else covers them. `test = false` would have deleted them.

Decision 1 above replaces it. The duplicate compile is removed at its
cause (the binary re-declaring the library's modules) instead of by
switching the binary's tests off, so no test moves and none is lost.
Decision 9's "any change to test bodies" is now simply "no test body
changes".

## Handoff

(Developer: append here. Commits, exact commands with their counts, what
was skipped or not verified and why, then the output of
`git status --short`, `git log --oneline origin/main..HEAD` and
`git ls-remote --heads origin ci/pr3-fast-rust-tests`.)

## Review

(Planner.)
