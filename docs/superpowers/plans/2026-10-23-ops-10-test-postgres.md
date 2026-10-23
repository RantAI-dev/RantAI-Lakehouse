# OPS-10: a full test run is green without re-runs

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-23 |
| Backlog | `OPS-10` (P2, Platform) |
| Base | `origin/main` at `c78c9faa`; anchors read there |
| Branch | `fix/ops-10-test-postgres`, worktree `/home/shiro/rantai/wt-ops10` |

## Why

Every full workspace run on this machine fails a few hundred tests with
`53100 could not resize shared memory segment … No space left on device`,
`UnexpectedEof` or "Connection reset" in sqlx's per-test database setup;
each binary passes alone (PR #216 review, 2026-10-23: 297 failures in the
`lakehouse-api` lib binary, then 2,440/2,440 alone; `connector_repoint` 5
failures, then 12/12 alone). Every reviewer re-runs binaries by hand. The
second half of `OPS-10`: CI's `g6-ops` step "c. auto-disable" times out
after 150 s waiting for the connector to be paused (seen on #160, #212 twice,
#216, and `main` at `c78c9faa`), so the check is red at random.

## What exists

- `rust/crates/lakehouse-test-support/src/lib.rs:62-133`: ONE labelled
  `testcontainers` Postgres (`16-alpine`, label
  `org.rantai.lakehouse-test-support=postgres-16`, `ReuseDirective::Always`)
  shared by every test process. No `shm_size`, no `-c` settings: Docker's
  default `/dev/shm` is 64 MiB, and Postgres' parallel query and dynamic
  shared memory need more under many concurrent databases.
- `rust/crates/lakehouse-api/tests/common/mod.rs:203-228`: each test makes a
  database and a pool of 5; `sqlx::test` in the lib tests does the same.
- `ops/g6/src11_connector_operations_test.py:593-631`: step c fails three
  runs, then polls `GET /api/connectors/{id}` for `pausedAt` for
  `SENSOR_WAIT_S = 150` s (`:123`). Whatever sets `pausedAt` after the third
  failure (the run report, or a sensor/tick) is what is slow or missed.

## Decisions (planner)

1. Give the shared container a larger `/dev/shm` (1 GiB) and turn off
   parallel query workers for it (`max_parallel_workers_per_gather=0`, the
   main dynamic-shared-memory user per query), and raise `max_connections`
   enough for the concurrent pools (measure, then set; say the number and
   why in the comment). Because the container is reused by label, **change
   the label value** (e.g. `postgres-16-shm1g`) so an old 64 MiB container
   is not reused; say so in the comment. No change to how tests use it.
2. The `g6-ops` timeout is diagnosed before it is changed: find what sets
   `pausedAt` after the N-th failed run and why it can take longer than 150
   s. Fix the cause. Raising the wait is acceptable only if the cause is a
   documented interval (say a sensor's minimum interval) and the new wait is
   that interval plus a stated margin; it is never raised blind (rule 2).
   On timeout the step prints the connector's `failureStreak`,
   `pausedAt` and the three run records, so the next failure says why.

## Tasks

- **T1** `lakehouse-test-support`: decision 1. *Check:* on this machine,
  `/home/shiro/rantai/bin/cargo-locked -j 2 test -p lakehouse-api --lib`
  and `… -p lakehouse-api --test upload_routes --test embed_tokens --test
  connector_repoint` pass, then the full workspace run (`… test -j 2
  --workspace --no-fail-fast`) shows **0** failures with `53100`,
  `UnexpectedEof` or "Connection reset". Quote the counts. Remove the old
  container only if it is the test-support one (`docker ps --filter
  label=org.rantai.lakehouse-test-support`), never another container.
- **T2** `g6-ops` step c: decision 2, with the root cause written in the
  commit and at the site. *Check:* the cause is shown (code path, interval),
  the step's diagnostics compile (`python3 -m py_compile`); CI decides the
  rest, three green `g6-ops` runs in a row quoted in the handoff if you can
  trigger them (re-run of the job on the PR), otherwise *not verified*.
- **T3** `docs/CI.md` or the test-support module doc: one paragraph on the
  container settings and why.

## Out of scope

Test changes, test order, `RUST_TEST_THREADS` caps (a cap would hide the
problem, not fix it), other gates.

## Handoff

### Handoff (developer, 2026-10-23)

| Commit | Task | What |
| --- | --- | --- |
| `d39d320e` | T1 (decision 1) | the shared container's settings, from the earlier session: 1 GiB `/dev/shm`, `max_parallel_workers_per_gather=0`, `max_connections=200`, label `postgres-16-shm1g` |
| `22799c3b` | T1 (check) | `wait_until_ready`: a `SELECT 1` before `DATABASE_URL` is published, because the harness's reuse path does not wait for the server |
| `7133f2c0` | T3 | module-doc paragraph on the settings and why |
| `509722ad` | T2 | step c waits for the counted first success; the pause timeout prints the streak, the pause and the three run records |

#### T1: the settings, and the second cause decision 1 did not name

`docker ps -a --filter label=org.rantai.lakehouse-test-support` at the
start showed one container, `275a53a07d3e`, label `postgres-16-shm1g`, 1 GiB
`ShmSize`, cmd `-c fsync=off -c max_parallel_workers_per_gather=0 -c
max_connections=200`: the previous session's, created with the change in the
working tree. No `postgres-16` container existed then, so none was removed.

**A `postgres-16` container exists now** (`562675b58221`, created 10:12, 64 MiB,
cmd `-c fsync=off`, `Up`): another worktree on `main`-based code created it
while this session ran, and is using it. It is left alone — removing a
container another worktree is testing against would fail that run, and this
branch's code never matches it (the label *value* is the match key, which is
why decision 1 changes it). One-time cleanup, when no `main`-based worktree is
testing: `docker rm -f 562675b58221`.

Other checks on the final commit: `cargo fmt --all -- --check` (exit 0),
`cargo-locked clippy -j 2 -p lakehouse-test-support --all-targets -- -D
warnings` (exit 0), and the workspace clippy of the verification block is
*not verified* by the developer (the session ended before it ran; the
reviewer's `review-pr.sh` runs it).

Runs on this machine (`CARGO_TARGET_DIR` shared, `-j 2`, via
`/home/shiro/rantai/bin/cargo-locked`):

| Run | Command | Result |
| --- | --- | --- |
| first, container stopped | `cargo-locked test -j 2 -p lakehouse-api --lib` | **2406 passed, 224 failed, 17 ignored** (157.57 s) |
| second, container running | same | **2406 passed, 0 failed, 17 ignored** (234.53 s) |
| reproduce before the fix, container stopped, binary directly | `lakehouse_api-193c84293f9676b9 --test-threads=4` | 2290 passed, **116 failed**, 17 ignored |
| plan's check, final commit, warm | `cargo-locked test -j 2 -p lakehouse-api --lib` | **2406 passed, 0 failed, 17 ignored** (241.27 s) |
| plan's check, integration binaries | `cargo-locked test -j 2 -p lakehouse-api --test upload_routes --test embed_tokens --test connector_repoint` | upload_routes **94** passed, embed_tokens **21**, connector_repoint **12**; 0 failed |
| full workspace, first attempt | `cargo-locked test -j 2 --workspace --no-fail-fast` | exit 101: **the host disk filled up at 12:21:34** (below), 4 failures in `route_auth` |
| `route_auth` alone, after | `cargo-locked test -j 2 -p lakehouse-api --test route_auth` | **55 passed, 0 failed** (18.78 s) |
| full workspace, final | `cargo-locked test -j 2 --workspace --no-fail-fast -- --quiet` | exit 0: **133 test binaries, 7176 passed, 0 failed, 44 ignored** (libtest's `--quiet` only compacts the per-test lines; the set of tests is the same) |
| cold proof after the fix: container `exited`, binary directly | `lakehouse_api-043d28ab303a0986 --test-threads=4` | **2406 passed, 0 failed, 17 ignored** (272.90 s) |

Every failure in the first three rows was sqlx's per-test setup
(`failed to connect to setup test database`): 219 `ConnectionReset` and 5
`Protocol("unexpected response from SSLRequest")` in the first run, 114 and 2
of the same in the reproduce run. **No SQLSTATE 53100 and no `UnexpectedEof`
in any run of this branch.** The
`/dev/shm` class the plan described from #216 did not appear; it is the
class decision 1 fixes, and it is not the class that was failing here.

Why the resets happened, measured: Postgres logged nothing for the failed
connects and the container's own `TcpExt` counters showed
`ListenOverflows=0`, `ListenDrops=0`, `TCPBacklogDrop=0` — the connects never
reached the server. `docker logs` has "database system is ready to accept
connections" at 09:47:40.869 while the container started at 09:47:31.28: the
run's first 9 s had no listener. `testcontainers` 0.27.3's reuse path starts
a stopped container with `client.start_container(...)` and returns without
running the image's wait strategy (`src/runners/async_runner.rs`), while
`testcontainers-modules` 0.15's `Postgres` is ready on the "database system
is ready" message (`src/postgres/mod.rs:126-131`). `wait_until_ready`
(`rust/crates/lakehouse-test-support/src/lib.rs`) now connects and runs
`SELECT 1` before `DATABASE_URL` is set, 60 s deadline, and panics with the
last error after that. The last row of the table is the proof: the run above
started the stopped container at 13:31:07.855 and Postgres was ready at
13:31:26.591 — 19 s with nothing listening — and all 2406 tests passed, where
the same experiment before the fix failed 116.

**The first full workspace attempt is not a test failure.** At 12:21:34 —
the second that run ended — the container's log has `ERROR: could not write
block … No space left on device` and `FATAL: could not write init file`, and
the run's own log stops at exactly 524288 bytes: the host filesystem was
full (`/` 88% before this session's builds, `cargo-target-prune.sh` keeps it
above 60 GB and freed space before the retry). The 4 `route_auth` failures
fall in that window; `route_auth` alone passes. Nothing in this branch
changed the disk; the note is for the next full run: check `df` first. The
second attempt, after the prune, is green.

#### T2: why the connector never paused, and why a longer wait would not help

The pause is written by `lakehouse_store::connectors::record_run_once`
(`rust/crates/lakehouse-store/src/connectors.rs:1647` `RECORD_RUN_SQL`, and
`:1770` `if !succeeded && failure_streak >= auto_disable_after`), reached from
`routes::load_alerts::connector_run_event`
(`rust/crates/lakehouse-api/src/routes/load_alerts.rs:418`). It applies each
report **in the order the reports arrive**: a success sets
`failure_streak = 0` whatever time that run ended, a failure adds 1.

The reports come from two independent Dagster sensors — the failure sensor
(`dagster/dispar_orchestrate/pipeline_events.py:239`) and the success sensor
(`:254`) — on Dagster's default 30 s interval (`DEFAULT_SENSOR_DAEMON_INTERVAL`,
dagster 1.13.20 `_core/definitions/sensor_definition.py:100`), the interval
the gate's own comment named.

On `main` at `c78c9faa`, job 120318461712 (run 39921765629), the daemon log
for the step-c connector `conn-g11-pause-…` reads:

- 08:33:24 the first, good load ends; 08:33:49 run 1's **failure** is reported;
- 08:33:54 the first load's **success** is reported — 5 s later, for a run
  that ended 30 s before run 1;
- 08:34:19 runs 2 and 3 are reported as failures;
- 08:36:37 the step gives up after 150 s.

Streak: 1 → 0 → 1 → 2. The connector was never going to pause, and no wait
would have covered it. That is why step c now waits for `lastRunSuccessAt` —
the column the same statement that resets the streak writes — before it makes
the password wrong: after that, no older success report can land between the
failures. A lost report was ruled out: the daemon reported all three failures
within 30 s of each ending ("Completed a reaction request … FAILURE") and
logged no failed POST (`gave up for run`: 0).

On a pause timeout the step now also prints `failureStreak`, `pausedAt`,
`pausedReason`, `lastRunSuccessAt`, `lastRunFailureAt` and each failed run's
record (`pause_diagnostics`), so the next red run can tell "never counted"
from "counted and then reset" without the daemon log.

**Finding for the reviewer, not fixed here.** The store still applies a late
success report to a streak that began after that success had ended, i.e.
`SRC-11` D3 can under-count "N failed runs in a row" whenever the success
sensor's tick lands after the next failure's. Fixing it changes
`record_run_once`'s semantics and its tests, which is a `SRC-11` decision and
a backlog item, not a gate step in this plan. The gate fix above also means
`g6-ops` no longer exercises that ordering.

#### T3

`rust/crates/lakehouse-test-support/src/lib.rs`'s module doc has a "The
container's server settings" paragraph; the per-setting reasoning stays at
`start_postgres_container`, which it points at.

#### Not verified

- `g6-ops` itself: CI only, no `docker compose up` on this machine. T2's local
  check is `python3 -m py_compile ops/g6/src11_connector_operations_test.py`
  (passes) plus the code path above; three green `g6-ops` runs in a row are
  for the PR.
- TypeScript: nothing in `src/` changed, so `bun run typecheck|lint|test` were
  not run. Python lints that do apply were run:
  `check_intra_package_imports.py`, `check_bare_iceberg_count.py`,
  `check_connector_gate_coverage.py` — all OK.
- The `postgres-16` container from another worktree is still running (see
  above); its owner removes it.

## Review

(planner appends here)
