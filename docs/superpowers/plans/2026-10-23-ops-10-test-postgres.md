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

(developer appends here)

## Review

(planner appends here)
