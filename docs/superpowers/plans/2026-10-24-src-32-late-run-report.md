# SRC-32: a run report that arrives late does not undo a later run's count

| | |
| --- | --- |
| Planner and reviewer | Claude Opus (`AGENTS.md`). Developer: a non-Opus agent |
| Date | 2026-10-24 |
| Backlog | `SRC-32` (P2), Data |
| Base | `origin/main` at `022e3608`; anchors read there |
| Branch | `fix/src-32-late-run-report`, worktree `/home/shiro/rantai/wt-src32` |

## Why

Auto-pause (`SRC-11` D3) pauses a connector after N failed runs in a row.
The failure and success reports come from two independent Dagster
sensors, so they can arrive out of order. `record_run_once` applies each
report in arrival order: a success of run 1 reported after run 2's
failure sets the streak back to 0, although run 2 ended later and is the
connector's latest state. The connector then never pauses (seen in
`g6-ops` on `main` at `c78c9faa`). The fix: a report counts toward the
streak and health only when its run ended at or after the latest run
already counted of the other outcome.

## What is true at `022e3608`

`C` = `rust/crates/lakehouse-store/src/connectors.rs`.

- `C:1674-1681` `RECORD_RUN_SQL`: success sets `last_run_success_at = $3`,
  streak 0, `healthy`; failure sets `last_run_failure_at = $3`, streak +1,
  `degraded`/`unhealthy`. `$3` is the run's end time
  (`routes/load_alerts.rs:459`, `run_end(info.end_time)`).
- Used by `record_run_once` (`C:1731-1826`, the run report, with dedupe
  and auto-pause) and `record_run_result` (`C:1902-1920`).
- Auto-pause: `C:1797`, `!succeeded && failure_streak >= auto_disable_after`.
- Store tests: `rust/crates/lakehouse-store/tests/connectors.rs:1890-2000`
  (`record_run_once_*`).
- `routes/load_alerts.rs:888` reads `last_run_failure_at`; check what it
  expects before changing that column's update.

## Decisions (planner)

1. In `RECORD_RUN_SQL` a report is **late** when the connector already has
   a counted run of the *other* outcome that ended strictly later:
   a success is late when `last_run_failure_at > $3`; a failure is late
   when `last_run_success_at > $3`. `NULL` means not late. Equal times are
   not late (today's behaviour).
2. A late report changes neither `failure_streak` nor `health`. It still
   moves its own timestamp, and only forward: `last_run_success_at =
   GREATEST(last_run_success_at, $3)` for a success (same for failures).
   A non-late report also uses `GREATEST` for its own timestamp, so an
   older report of the same outcome never moves the time back.
3. Two failures arriving out of order both count (the streak is a count of
   failures since the latest success; their order between themselves does
   not matter). Two successes: both set streak 0.
4. The SQL returns whether the report counted (`RETURNING health,
   failure_streak, <counted>`). `record_run_once` pauses only when the
   report counted and is a failure with streak >= threshold. A late
   report's per-run stored streak is the connector's streak at that moment
   (unchanged). `RunRecord` gets no new public field unless the caller
   needs one; if you add one, document it.
5. One SQL statement, still one `UPDATE ... RETURNING` (atomic against a
   concurrent report on the same row). Do it with `CASE` on the old
   values: in Postgres the right-hand side of `SET` reads the row's old
   values, so every `CASE` sees the same pre-update timestamps.
6. Update the doc comments of `RECORD_RUN_SQL`, `record_run_result` and
   `record_run_once` to state the ordering rule and cite `SRC-32`.

## Tasks

- **T1.** Decisions 1-6 in `connectors.rs`, plus store tests in
  `rust/crates/lakehouse-store/tests/connectors.rs` (full-sentence names):
  - late success: failure of run 2 (ended t2) counted, then success of
    run 1 (ended t1 < t2) arrives: streak stays 1, health stays
    `degraded`, `last_run_success_at` = t1;
  - the bug's sequence: with threshold 3, failures of runs 2, 3, 4 (t2 <
    t3 < t4) and a success of run 1 (t1 < t2) reported between them, in
    the order F2, S1, F3, F4: the connector is paused at F4;
  - late failure: success at t2 counted, failure at t1 < t2 arrives:
    streak stays 0, health `healthy`, `last_run_failure_at` = t1, no pause
    even with threshold 1;
  - timestamps never go back: success at t2 then success at t1: 
    `last_run_success_at` stays t2;
  - equal times: failure at t then success at t: streak 0 (today's rule).
  Existing tests keep passing unchanged (rule 2); if one must change, say
  why in the commit. Check: `cargo fmt --check`, `cargo clippy -p
  lakehouse-store --all-targets -- -D warnings`, `cargo test -p
  lakehouse-store --test connectors`.
- **T2.** If `routes/load_alerts.rs` (or the API's tests) depends on the
  old update order, adjust it, with `cargo clippy -p lakehouse-api
  --all-targets -- -D warnings` and `cargo test -p lakehouse-api
  load_alerts`. If nothing needs to change, say so in the handoff and make
  no commit. A `CHANGELOG.md` line under `## [Unreleased]`.
- **T3.** Full Rust verification block from `AGENTS.md` (workspace
  clippy and test), `git status`, and the handoff below.

## Out of scope

The `SRC-25` gate timing, `OPS-10`'s gate wait, a run that ended before a
manual resume, any migration.

## Machine rules (this VM: 11.9 GB RAM, shared disk)

- Every cargo command except `fmt`/`tree`/`update` goes through
  `/home/shiro/rantai/bin/cargo-locked <subcmd> -j 2 ...` (`-j` after the
  subcommand). Never `cargo clean`, never a per-worktree `target/`.
- Before a long run check `free -g` and `df -h /`; stop below 15 GB free
  disk or 2 GB available memory and report.
- Long commands: output to a log file under `/home/shiro/rantai/review/`,
  poll it; do not end your turn while one runs. Never touch other
  worktrees, other processes, or docker objects you did not start.
- On resume after an interruption: `git status` and `git log` first;
  finish the task you were on; never start an unrequested refactor.

## Handoff

(developer appends here)

## Review

(planner appends here)
