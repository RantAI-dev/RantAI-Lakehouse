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

### 2026-10-24 — developer, slice 1 (T1–T3)

**Commits** (branch `fix/src-32-late-run-report`, base `022e3608`):

- `4fae3c09` `fix(store): SRC-32, a late run report no longer undoes a later run`
  — `RECORD_RUN_SQL`, the doc comments of the SQL / `record_run_once` /
  `record_run_result` / `RunHealth`, the `counted` gate on the pause, and the
  six store tests in `rust/crates/lakehouse-store/tests/connectors.rs`.
- `d3d6ed21` `docs(changelog): SRC-32, a late run report does not undo a later run`
  — the `### Fixed` line under `## [Unreleased]`. No product code: T2 found
  nothing in the API to change (below).

**T1 — what the SQL does.** One `UPDATE ... RETURNING`, still one statement,
with `$2` the outcome and `$3` the run end. A report is late when
`last_run_failure_at > $3` (success) or `last_run_success_at > $3` (failure);
`NULL` and equal times are not late. A late report keeps `failure_streak` and
`health`, writes its own timestamp with `GREATEST` (so a time never moves
back), and returns `counted = false`; only `counted` failures reach the pause.
The third column is read inside `record_run_once` and never leaves the store:
`RunRecord` is unchanged (decision 4's "no new public field").
`record_run_result` discards the counted column and its `RunHealth` answer for
a late report is the connector's state, as its doc now says.

**T1 checks** (all from `/home/shiro/rantai/wt-src32/rust`):

- `cargo fmt --check` — no diff.
- `/home/shiro/rantai/bin/cargo-locked clippy -j 2 -p lakehouse-store --all-targets -- -D warnings` — exit 0.
- `… cargo-locked test -j 2 -p lakehouse-store --test connectors` — **61 passed, 0 failed** (26.08 s), including the six new tests (`a_success_reported_after_a_later_failure_keeps_the_streak_and_the_health`, `a_late_success_between_failures_does_not_stop_the_third_from_pausing`, `a_failure_reported_after_a_later_success_counts_nothing_and_pauses_nothing`, `an_older_report_never_moves_its_own_run_time_back`, `a_failure_and_a_success_that_ended_at_the_same_time_reset_the_count`, `record_run_result_reports_a_late_success_as_the_connectors_state`). No existing test was changed.

**T2 — the API needs no change.** `routes/load_alerts.rs` passes
`run_end(info.end_time)` straight through and reads `last_run_failure_at` only
to assert that a time was stamped (`:888`), so neither depends on the old
update order. Its tests give every run of a sequence the same `endTime`
(2000.0, `knows_run_config`), which is not late. Checks:

- `… cargo-locked clippy -j 2 -p lakehouse-api --all-targets -- -D warnings` — exit 0 (3 m 59 s).
- `… cargo-locked test -j 2 -p lakehouse-api load_alerts` — **21 passed, 0 failed** (twice: the unit and the crate test binaries), 0 failed elsewhere as filtered out.

**T3 — full Rust block**, from `rust/`, on `d3d6ed21`:

- `cargo fmt --check` — no diff.
- `… cargo-locked clippy -j 2 --workspace --all-targets --all-features -- -D warnings` — exit 0 (2 m 16 s).
- `… cargo-locked test -j 2 --workspace` — exit 0: **134 test binaries, 7 421 passed, 0 failed, 44 ignored** (logs in `/home/shiro/rantai/review/src32-dev-t3-*.log`).
- `git status` — clean after the two commits; nothing generated staged, no `target/`, no `__pycache__/`.

**Not verified, and why.**

- The new tests were not run against the pre-fix `RECORD_RUN_SQL` (no
  revert-and-test run: a second workspace rebuild for the store crate). By
  construction the `g6-ops` test needs arrival order to pass — with the old
  statement `S1` sets the streak to 0, so `F4` reaches 2 and does not pause —
  but that is reasoning, not a measurement.
- No TypeScript, Python, Dagster, compose or gate checks: the slice touches
  `connectors.rs`, the store test file and `CHANGELOG.md` only.
- No real Dagster sensor run and no `g6-ops` rerun: the ordering is proven by
  the store tests against Postgres, not by the orchestrator.

**Decision needed (planner), found while doing T2.** `deliver_failure_alerts`
still raises `connector_repeated_failure` from
`recorded.failure_streak == Some(REPEATED_FAILURE_STREAK)`
(`routes/load_alerts.rs:535`). For a late failure the store now stores the
connector's unchanged streak, so a late failure that arrives while the
connector sits at 3 raises "3 runs in a row; the latest is run X" for a run
that ended earlier and produced none of the three — and a second
`alert_instance` row. Decision 4 covers the pause only, so this was left as
it is; gating it needs the counted flag on `RunRecord` (or an equivalent),
which decision 4 allows but does not decide. A retry never reaches that code
(the alerted marker returns first), so the retry path needs no answer either
way.

## Review

### Round 1 (planner, 2026-10-24)

Reviewed `4fae3c09`..`c5a44524`. The SQL is right: every `CASE` reads the
pre-update row, `RETURNING`'s `counted` reads the other outcome's
timestamp, which this statement never changes, and `GREATEST` keeps each
time moving forward. Tests cover the five cases of T1. The developer's
handoff numbers stand pending the planner's own run.

**BLOCKER 1 (the developer's "decision needed", answered).** A late
failure stores the connector's current streak on its own dedupe row
(plan decision 4, my wording), so a late failure arriving while the
connector sits at 3 raises `connector_repeated_failure` ("3 runs in a
row; the latest is run X") for a run that produced none of the three, on
the first report and again on a retry after a rule-store failure (the
retry path reads the stored streak). Decision 4 is changed: **a late
report produced no streak, so its dedupe row stores `failure_streak =
NULL` and `RunRecord::failure_streak` is `None` for it** (first report
and retries alike). Then `routes/load_alerts.rs:535`
(`== Some(REPEATED_FAILURE_STREAK)`) cannot fire for it, and
`auto_disabled` stays `false` as now; the plain "run failed" alert still
fires, which is true (the run did fail). Update the doc of
`RunRecord::failure_streak` (today: "`None` only for a run counted before
`0069`") to name the second case, cite this finding at the change, and
add store tests: a late failure's `record_run_once` answer and its retry
both have `failure_streak == None`; and an API test in `load_alerts`
(pattern of its existing tests) that a late failure arriving with the
connector at streak 3 raises no `connector_repeated_failure` alert
instance. No migration (the column is already nullable).

### 2026-10-24 — developer, handoff (round 1 fix)

**Commit.** `4b2a9bc6` `fix(store): SRC-32, a late run report keeps no
streak of its own` — `SRC-32 review round 1 BLOCKER 1`.

**What changed.** `record_run_once` stores `counted.then_some(failure_streak)`
on the dedupe row and returns the same value, so a late report's row keeps
`failure_streak = NULL`, `RunRecord::failure_streak` is `None` for it, and its
retry reads the same `None` back (the retry path already read the column). The
docs of `RunRecord::failure_streak` and `record_run_once` name the second
`None` case and cite the finding; `deliver_failure_alerts`' comment says why
`None` cannot fire the repeated-failure rule. No migration, and no change to
the API's logic: `== Some(REPEATED_FAILURE_STREAK)` is already false for
`None`, and `auto_disabled` stays `false` as before. The plain
`connector_failure` alert still fires for a late failure.

**Tests.**

- Store: new
  `a_late_failure_stores_no_streak_on_its_row_and_its_retry_reads_none`
  (connector at streak 3 from three later failures, a late failure then
  answers `None`, stores no streak, its retry reads `None`, and the
  connector's streak and health are unchanged). Two existing late-report
  tests changed their expectation from `Some(1)`/`Some(0)` to `None`, because
  the review changed what the answer is; their subject (the connector's
  state) is unchanged.
- API: `a_late_failure_at_streak_three_raises_no_repeated_failure_alert` in
  `load_alerts`: success at 2000.0, three failures at 2000.0 (streak 3, the
  repeated rule fires once), then run 1's failure at 500.0 — late: `matched`
  is 1 (the plain rule), `al-r` keeps its one post and its one
  `alert_instance`, and the connector stays `("unhealthy", 3)`. The harness
  gained `knows_run_at` and `knows_run_ending_at` (end time first;
  `knows_run`/`knows_run_config` keep 2000.0), so two runs can end at
  different times.

**Checks**, from `/home/shiro/rantai/wt-src32/rust`, on `4b2a9bc6`:

- `cargo fmt --check` — no diff (`cargo fmt` ran once on the new code first).
- `/home/shiro/rantai/bin/cargo-locked clippy -j 2 -p lakehouse-store -p
  lakehouse-api --all-targets -- -D warnings` — exit 0, 3 m 45 s
  (`/home/shiro/rantai/review/src32-dev-r2-clippy.log`).
- `… cargo-locked test -j 2 -p lakehouse-store --test connectors` — **62
  passed, 0 failed** (24.04 s), the 61 of round 1 plus the new test
  (`/home/shiro/rantai/review/src32-dev-r2-store-test.log`).
- `… cargo-locked test -j 2 -p lakehouse-api load_alerts` — **22 passed, 0
  failed**, twice (the unit and the crate test binaries; 10.09 s and 9.87 s),
  including the new test, 0 failed elsewhere as filtered out
  (`/home/shiro/rantai/review/src32-dev-r2-api-test.log`).
- Waited about 25 minutes for `/home/shiro/rantai/.heavy.lock` before the API
  test run (another worktree's pytest held it). Nothing was killed or
  pre-empted.

**Not verified, and why.**

- No full workspace run: the task scopes this round to the two crates and the
  reviewer runs the full block.
- The new tests were not run against the pre-fix code (that needs a rebuild of
  the pre-fix tree). By construction a late failure's row held the
  connector's streak before, so the store answer would have been `Some(3)` and
  the API's `matched` would have been 2 — reasoning, not a measurement.
- No TypeScript, Python, Dagster, compose or gate checks: the commit touches
  two Rust crates and `CHANGELOG.md` only.

### Round 2 and planner's verification (2026-10-24)

BLOCKER 1 fixed in `4b2a9bc6`: a late report's dedupe row keeps
`failure_streak = NULL` and `record_run_once` answers `None` for it and
its retries, so `routes/load_alerts.rs`'s `== Some(REPEATED_FAILURE_STREAK)`
cannot fire; cited at both sites. The new store test reads the stored
column and the retry; the API test shows the repeated-failure rule posts
once for the real third failure and not again for the late one, while the
plain failure alert still fires. Two existing late-report tests changed
`Some(1)`/`Some(0)` to `None`: the review changed that answer, their
subject (the connector's state) is unchanged, so rule 2 holds.

`review/review-pr.sh` on `d5d9e67a`: fmt and workspace clippy OK; Rust
134 suites, **7424 passed, 0 failed**, 44 ignored; typecheck OK; bun
2711 passed, 0 failed, 2 skipped; Python lints OK; Dagster 2146 passed;
compose config OK; gitleaks no leaks. `main` merged in at `ffa035fe`
(clean). No open `BLOCKER`.
