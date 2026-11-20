# `BLD-11`: the pipeline page says what is true (last run, trigger, lag, author, Run now, cast)

Planner and reviewer: Claude Opus. Developer: `Hy4 flash` (OpenCode). The
console changes here are fixes, not a revamp, so they stay with the same
non-Opus developer (`AGENTS.md`, the 2026-11-04 exception covers revamps
and new UI only). Nobody reviews their own work.

Base: `origin/main` at `c7cd01a76`; every anchor below was read there.
Branch: `bld-11-pipeline-page` (this plan is its first commit). One pull
request. The planner opens it as a draft as soon as the Rust tasks are
pushed, so CI runs the full Rust suite.

Backlog: `BLD-11` (Task, P2), under `BLD-2` and `DATA-19`. Feature pages:
`docs/core/features/pipeline-target-lag.md` (`BLD-2`) and
`docs/core/features/pipeline-joins-and-sql.md` (`BLD-1`). QA cases:
`LH-BLD-2-01`, `TC-1733`, `LH-DATA-1-51` (QA round 2026-11-02 to 11-12).

Parallel work checked on 2026-11-20: `gh pr list --state open` and
`git log origin/main -i --grep='BLD-11\|BLD-2\|DATA-19'`. No open PR and no
merged commit fixes any finding below. PR #619 (`BLD-16`, draft, branch
`bld-16-joins-aggregations`) changes nearby files; section 6 names each one
both touch and how this plan stays off its lines.

## 1. Why

`BLD-1` part 1 (PRs #481, #499, #500) rewrote the pipeline editor and took
the Kind and incremental column off the page. The `BLD-11` row asked for its
findings to be checked again after that. Each was re-read at `c7cd01a76`.

### Findings that remain

- **F1. The list never shows a last run for an authored pipeline.**
  `pipeline_definition.last_run_at` is read
  (`lakehouse-store/src/pipelines.rs:91`, `:559`) and nothing writes it: the
  create path leaves it `NULL` on purpose (`:1308-1314`), and neither run
  event route writes it (`routes/pipelines.rs:2591` `run_failed_event`,
  `:2785` `run_finished_event`). The list's "Last Run" column
  (`pipeline-columns.tsx:296-314`) and the masthead's "Never run" pill
  (`pipeline-detail-header.tsx:101`) both read it, so every authored
  pipeline says "Never" however often it ran.
- **F2. A pipeline driven by its target lag is labelled "Manual only".**
  With a target lag the orchestrator builds no schedule
  (`authored_factory.py:716-719`, BLD-2 D3), but the page decides the
  trigger from `schedule` alone: the masthead chip
  (`pipeline-detail-header.tsx:30-37`), the Settings tab
  (`pipeline-detail-page.tsx:322-327`), the "Next run" figure
  (`pipeline-detail-header.tsx:237-254`) and the list's Schedule column
  (`pipeline-columns.tsx:269-286`, "On demand"). With a stored cron and a
  target lag, the chip shows a cron that does not fire and the button says
  "Pause schedule" (`pipeline-detail-page.tsx:375-394`).
- **F3. "Current lag" shows "—" both when the pipeline has caught up and
  when nothing was measured.** `pipeline-freshness-panel.tsx:107-108`. The
  API sends `lagSeconds: null` in both cases (`pipeline_lag.rs:1385-1397`
  caught up; `:1734-1742` not measured) and `status` tells them apart.
- **F4. The lag clock starts when the check first sees the change, not when
  the data was loaded.** Every path starts it with
  `state.pending_since.unwrap_or(now)` (`pipeline_lag.rs:1103`, `:1217`,
  `:1261`, `:1400`). A check runs every 30 s
  (`CHECK_INTERVAL_SECONDS`, `:53`), so the clock starts up to one interval
  after the load (QA measured about 40 s). The target then reads Met while
  the run that delivers the change ends after the target has passed. A
  Bronze input's commit time is already read by the same call that reads
  its token: `load_table_summary` returns `last_updated_ms`, the current
  snapshot's commit time (`lakehouse-iceberg/src/rest.rs:192`, `:220`), and
  `iceberg_snapshot_token` (`pipeline_lag.rs:228-265`) drops it.
- **F5. The header says "Reads as" a person while the assign panel says
  nobody can.** `authorChoices` (`pipeline-reads-as.ts:85-99`) answers "no
  active user in this pipeline's tenant can read as it" whenever the user
  list holds nobody else, including when the current author is not in the
  list the admin can see, and when the list failed to load
  (`pipeline-detail-page.tsx:187-194` turns an error into `[]`). The
  masthead meanwhile names the author (`pipeline-reads-as.ts:106-118`).
- **F6. The first Run now on a new pipeline answers 409, and "Mark ready"
  keeps saying "Marking ready…".** Mark ready
  (`routes/pipelines.rs:3598` `set_status_route`) awaits a full
  orchestrator reload before it answers (`:3655-3658`), and the page shows
  "Marking ready…" for that whole time (`pipeline-detail-page.tsx:358-370`).
  Run now (`:2938` `authored_launch_target`) reloads once and checks once
  (`:2952-2961`); a job the reload has not finished loading is a 409. The
  page reports a refused launch only in a toast
  (`pipeline-detail-page.tsx:247-251`), and the "not loaded" banner
  (`:467-473`) does not refresh, because the detail is not polled.
- **F7. A cast does not change the column's type unless the step also has a
  select.** `render_transform_step` (`pipeline_compile.rs:546-598`) applies
  `cast` and `rename` only inside an explicit `select`; with no `select`
  the step is `SELECT *` and both are silently dropped (`:569-586`). The
  output table is rebuilt from the compiled statement every run
  (`authored_factory.py:474-546`), so the Gold table keeps the source type.

### Found while re-checking, on the same list page

- **F8. The list shows "Breached" for every pipeline with no SLA.** `slaOk`
  is `null` for every authored row (`lakehouse-store/src/pipelines.rs:95-101`)
  and the cell renders anything falsy as "Breached"
  (`pipeline-columns.tsx:391-402`). The detail page already says "Not
  defined per pipeline" (`pipeline-detail-page.tsx:135`).
- **F9. The list says an authored pipeline cannot run.** The Origin tooltip
  says "No engine is attached yet, so it cannot run", and the row menu
  offers a disabled "Trigger run — no engine attached"
  (`pipeline-columns.tsx:154-158`, `:446-453`). Authored pipelines run;
  the trigger route launches them (`routes/pipelines.rs:2051-2057`).
- **F10. Pausing a pipeline with a target lag does not stop it (not fixed
  here).** Pause switches the cron schedule and the `_after` sensor
  (`routes/pipelines.rs:3754-3790`) but not the `_lag` sensor, which is
  skipped only at the next orchestrator reload
  (`authored_factory.py:1313`), and the lag check does not look at the
  status. This plan stops offering the button for such a pipeline (D3);
  making pause work for it is a new backlog item, filed by the planner at
  review.

### Dropped, with the reason

- **An incremental column that is not in the source table is accepted.**
  `BLD-1` part 1 took the field off the page (decision 18): the editor only
  passes the stored value through (`pipeline-editor.tsx:252-256`) and no one
  can type one. What is left is the API field, which `BLD-12` owns (build
  incremental runs or remove the stored fields). Moved to `BLD-12`'s row.

No finding was fixed by part 1 other than that one.

## 2. Decisions

Items marked *(planner default, owner may change)* are the planner's choice
on the owner's behalf; the developer builds them as written, and the owner
may change them before merge. The rest follow from the feature pages or
from `AGENTS.md`.

- **D1. What "last run" is.** The start time of the newest *finished* run
  (succeeded or failed), written when the orchestrator reports the run
  (`run_finished_event`, `run_failed_event`), after the route has confirmed
  the run's state with Dagster. A run still going is shown by the detail
  page's run list, not by this field; a cancelled run is not reported by
  either event and is not counted. Start time, because the list's Dagster
  rows already use it (`routes/pipelines.rs:343-356` `last_run_for`).
  *(planner default, owner may change)*
- **D2. The write is monotonic.** One store function,
  `record_last_run(pool, id, started_at)`:
  `UPDATE pipeline_definition SET last_run_at = $2 WHERE id = $1 AND
  (last_run_at IS NULL OR last_run_at < $2)`. Events can arrive out of
  order (a sensor retry), so an older run never moves the field back. No
  migration: the column is already nullable (`0048_pipeline_never_run.sql`).
  Existing rows stay `NULL` until their next run; no backfill.
- **D3. How a target-lag pipeline's trigger is shown.** When `targetLag`
  is set it wins over `schedule` everywhere the page names the trigger:
  `{ kind: "lag", minutes: N }` reads "Within N min of its inputs"
  (minutes over 59 as hours and minutes, e.g. "Within 2 h of its inputs");
  `{ kind: "downstream" }` reads "When a downstream pipeline needs it"; an
  unknown kind reads "Target lag (unknown kind)". "Next run" reads "—"
  with the foot "Runs when an input changes". The Pause schedule and
  Resume schedule buttons are not shown while a target lag is set: there is
  no schedule to pause, and pausing would not stop the lag sensor (F10).
  *(planner default, owner may change)*
- **D4. Caught up is not "—".** When `lag.status` is `met` and
  `lagSeconds` is `null`, "Current lag" reads "Up to date"; when the status
  is `not_measured`, it reads "Not measured". A number stays a number
  (format it with the console's duration formatter, not raw seconds). The
  API is not changed: `null` stays the honest value (`AGENTS.md`
  principle 2). *(planner default, owner may change)*
- **D5. When the lag clock starts.** For a Bronze input, at the commit time
  of the snapshot the check saw (`last_updated_ms`). For a Silver or Gold
  input, at the first check that sees the change, as today: `ClickHouse`
  gives no commit time for the fingerprint the check reads. With several
  changed inputs, the earliest of their times. The start is never later
  than `now` (clock skew) and never earlier than the last consumed run's
  start (`consumed_at`; a change committed before that run started may
  already be in its output). Once set, `pending_since` is kept as today.
  The freshness panel says which rule applies (D6). *(planner default,
  owner may change)*
- **D6. The freshness panel says how lag is counted.** One line under the
  three figures: "Lag counts from when a Bronze input was loaded, and from
  the first check that saw the change for a Silver or Gold input."
- **D7. The author panel never contradicts the header.** `authorChoices`
  takes the current author and whether the user list loaded:
  - list failed to load: `blocked` = "the user list could not be loaded, so
    no one can be offered here"; the masthead line is unchanged;
  - an author is set and nobody else can be offered: `blocked` = "it reads
    as <name>; no other active user in this pipeline's tenant can be
    chosen" (`<name>` as the masthead names it, "a user you cannot see"
    when null);
  - no author and nobody to offer: today's sentence.
  The empty option's label follows the same state ("Nobody — this pipeline
  cannot run" only when no author is set, as today).
- **D8. Mark ready answers at once.** `set_status_route` writes the status
  and starts the orchestrator reload in the background (`tokio::spawn` on
  a clone of `AppState`); it does not wait for it. The reload stays best
  effort, as its comment says today (`:3655-3657`).
- **D9. Run now waits briefly for a job that is loading.** In
  `authored_launch_target`, after the one reload, the route checks
  `job_is_loaded` every 2 s for up to 20 s before it answers 409. The 409
  text becomes: "the orchestrator has not loaded this pipeline's job yet;
  try again in a minute. If it never loads, check that PIPELINE_RUN_TOKEN
  is set for both the API and the Dagster code location". The 20 s is a
  bound on a request the user is waiting on, not a measurement; the
  handoff reports how long the first Run now took on the compose stack.
  *(planner default, owner may change)*
- **D10. The page shows a refused Run now where the button is.** The toast
  stays; the page also shows the refusal under the masthead (`role="alert"`,
  the text through `readableErrorMessage` from `@/lib/notify-message`),
  cleared on the next launch. While an authored pipeline is `ready` and
  `orchestratorJob` is false, the page reloads the detail every 4 s for at
  most 2 minutes after it opened or after Mark ready, and the banner
  (`:467-473`) reads "The orchestrator is loading this pipeline's job." during
  that window and today's sentence after it.
- **D11. A cast or rename with no select applies to every column.**
  `render_transform_step`, when the step has no `select` and has a `cast`
  or `rename`:
  `SELECT * [EXCEPT (<renamed>)] [REPLACE (CAST(c AS T) AS c, ...)][, <expr> AS <new>, ...] FROM ...`
  where a renamed column is excluded from `*` and added at the end as
  `<old> AS <new>` (or `CAST(<old> AS T) AS <new>` when it is also cast),
  and a cast column that is not renamed is replaced in place. `EXCEPT`
  comes before `REPLACE` (the order `sqlparser`'s ClickHouse dialect
  parses). With a `select`, nothing changes. A renamed column therefore
  moves to the end of the table. *(planner default, owner may change)*
  The access rewrite substitutes tables, so `* EXCEPT`/`* REPLACE` is
  covered (`sql_rewrite.rs:1-27`); column lineage reads such a step as
  partial (`sql_lineage.rs:543-547`), which is honest and accepted.
- **D12. The list (F8, F9).** SLA `null` reads "Not defined" in a neutral
  tone, never "Breached". The Origin tooltip for an authored row reads
  "Authored in the console; the orchestrator runs it." The row menu offers
  "Trigger run" for an authored `ready` row (the same `onTrigger` as a
  Dagster row) and a disabled "Trigger run — mark it ready first" for a
  draft.
- **D13. No assistant change.** `BLD-11` changes no tool, schema or tool
  answer; `tests/fixtures/tool_schemas.json` is not regenerated. (Assistant
  parity applies to BI features; this is a Build fix.)

## 3. Anchors (at `c7cd01a76`)

Rust:

- `rust/crates/lakehouse-store/src/pipelines.rs`: `Pipeline.last_run_at`
  `:86-91`; create comment `:1308-1314`; `set_status` `:2058` (put
  `record_last_run` directly after it, far from PR #619's hunks at
  `:254-430`).
- `rust/crates/lakehouse-store/tests/it.rs`: the one test binary; a new
  file needs a `mod` line in alphabetical order.
- `rust/crates/lakehouse-api/src/routes/pipelines.rs`: `run_failed_event`
  `:2591`, its FAILURE check `:2643-2650`; `run_finished_event` `:2785`, its
  SUCCESS check `:2833-2840`; `authored_launch_target` `:2938-2963`;
  `set_status_route` `:3598`, the reload `:3655-3658`; test helpers: the
  `set_status` tests `:8096-8150` (`status_body`, `create_draft`), a
  Postgres plus `wiremock` state `:9638` `state_for(pool, dagster_uri)`.
- `rust/crates/lakehouse-api/src/routes/authored_pipelines.rs`:
  `reload_orchestrator` `:1635`, `job_is_loaded` `:1652`.
- `rust/crates/lakehouse-dagster/src/lib.rs`: `RunStatusInfo.start_time`
  `:637` (unix seconds, `f64`).
- `rust/crates/lakehouse-api/src/pipeline_compile.rs`:
  `render_transform_step` `:537-598`; the compiled-text test `:723-745`.
- `rust/crates/lakehouse-api/src/pipeline_lag.rs`: `LagTokenReader`
  `:163-195`; `StateTokenReader` `:213-220`; `iceberg_snapshot_token`
  `:228-265`; `CheckState` `:576-614`; `settle_request`'s
  unix-seconds conversion `:684-694` (the pattern to reuse); clock starts
  `:1103`, `:1217`, `:1261`, `:1400`; token reads `:1168`, `:1365`;
  `FakeTokens` `:1966-1978`; `lag_json` `:1720-1830`.

Console:

- `src/features/pipelines/pipeline-columns.tsx`: Origin `:137-172`, Schedule
  `:263-295`, Last Run `:296-314`, SLA `:385-414`, row menu `:436-453`.
- `src/features/pipelines/pipeline-detail-header.tsx`: `ScheduleChip`
  `:21-55`, masthead pill `:98-102`, "Next run" `:236-254`.
- `src/features/pipelines/pipeline-detail-page.tsx`: users `:187-194`,
  `runNow` `:247-251`, `activate` `:262-266`, `SettingsTab` `:118-150`,
  `scheduleText` `:322-327`, actions `:358-406`, author panel `:424-465`,
  banner `:467-473`.
- `src/features/pipelines/pipeline-schedule.ts` (`parseSchedule` `:21`).
- `src/features/pipelines/pipeline-freshness-panel.tsx`: `currentLagDesc`
  `:107-108`, figures `:279-286`.
- `src/features/pipelines/pipeline-reads-as.ts`: `authorChoices` `:85-99`,
  `readsAsLine` `:106-118`.
- Tests: `pipeline-columns.test.tsx`, `pipeline-freshness-panel.test.tsx`,
  `pipeline-reads-as.test.ts`, `pipeline-detail-page.test.tsx` (harness
  `:1-60`; BLD-8 block ends `:225`).

## 4. Migrations

None. `last_run_at` is already nullable with no default (`0048`). For the
record: the highest migration on `main` is `0151`; `0152` is reserved by
branch `data-4-silver-publish`; a migration added here would be `0153`.

## 5. Tasks

One task per commit, in order, Rust first. Push after every commit (a worker
session that dies loses nothing pushed). Each commit body cites `BLD-11`, the
finding (`F1`…`F9`) and the QA case where there is one; each fix site carries
a short comment naming the finding, for example
`// BLD-11 F7: a cast with no select used to be dropped`.

Per commit, run only the scoped checks (`AGENTS.md`, "Keep build time
down"): for Rust `cargo fmt --check`, then
`cargo clippy -p <crate> --all-targets -- -D warnings` and
`cargo test -p <crate> <filter>`, through the shared cargo wrapper and the
shared `CARGO_TARGET_DIR` (never a `target/` in the worktree, never
`cargo clean`). For TypeScript `bun run typecheck`, `bun run lint` and
`bun test <the files touched>`. Integration tests are one binary per crate:
`cargo test -p lakehouse-store --test it pipeline_last_run::`.

Every test named below that pins new behaviour must fail on `c7cd01a76`
and pass after the task; say in the handoff how you saw it fail (revert the
fix line, run, restore). The ones that pin today's behaviour on purpose
(`a_filter_or_dedupe_alone_still_selects_star`,
`an_input_with_no_commit_time_starts_the_clock_at_the_first_check`,
`with no author and nobody to offer, today's sentence stays`,
`no target lag keeps today's manual and cron labels`) pass on both.

### Rust (T1 to T4)

- **T1. Last run is written when a run finishes (F1; D1, D2).**
  `lakehouse-store`: `pub async fn record_last_run(pool: &PgPool, id: &str,
  started_at: OffsetDateTime) -> Result<bool, StoreError>` (true when a row
  changed), with `# Errors`, the value bound, the `WHERE` of D2.
  `lakehouse-api`: in `run_finished_event` after the SUCCESS check, and in
  `run_failed_event` after the FAILURE check (before its dedupe return, so
  a retried event still writes), convert `info.start_time` to
  `OffsetDateTime` (reuse an existing helper if `grep` finds one; otherwise
  the conversion of `pipeline_lag.rs:684-694`) and call it. A store error
  here is logged through `crate::upstream_error` and does not fail the
  event: the alerts are the event's job, the timestamp is a by-product.
  A missing `start_time` writes nothing.
  *Check:* new `rust/crates/lakehouse-store/tests/pipeline_last_run.rs`
  (plus its `mod` line in `tests/it.rs`):
  `a_finished_run_sets_last_run_at_on_a_pipeline_that_never_ran`,
  `an_older_run_reported_late_does_not_move_last_run_at_back`,
  `an_unknown_pipeline_id_changes_no_row`. In `routes::pipelines` tests,
  with the `:9638` Postgres plus `wiremock` state (mock `runOrError` for
  `pipeline_run_status` with `SUCCESS`/`FAILURE` and a `startTime`):
  `a_successful_run_event_writes_the_runs_start_as_last_run_at` and
  `a_failed_run_event_writes_the_runs_start_as_last_run_at`, each reading
  the pipeline back through `pipelines::get_pipeline`.
- **T2. A cast or rename with no select applies (F7; D11; `LH-DATA-1-51`).**
  `render_transform_step` only; its doc comment states the new rule.
  *Check:* unit tests in `pipeline_compile.rs` pinning the exact text:
  `a_cast_with_no_select_replaces_the_column_in_place`
  (`["cast(amount,Float64)"]` →
  `SELECT * REPLACE (CAST(amount AS Float64) AS amount) FROM ...`),
  `a_rename_with_no_select_moves_the_column_to_the_end`,
  `a_cast_and_rename_of_one_column_with_no_select_cast_it_under_its_new_name`,
  `a_filter_or_dedupe_alone_still_selects_star` (unchanged text), and the
  existing `a_transform_step_renders_select_rename_cast_filter_and_dedupe`
  unchanged. One more test feeds each new statement to
  `crate::sql_rewrite::referenced_tables` with the ClickHouse dialect and
  asserts it parses and names the input.
  *Live check (rule 7):* on the compose stack's ClickHouse, run
  `CREATE TABLE default.bld11_src (id Int32, amount Decimal(12,2)) ENGINE = Memory`,
  insert one row, then
  `CREATE TABLE default.bld11_out ENGINE = MergeTree ORDER BY tuple() AS <the compiled cast-only SELECT over default.bld11_src>`,
  and quote `SELECT toTypeName(amount) FROM default.bld11_out` (expect
  `Float64`) in the handoff; drop both tables after. If the stack cannot
  run, write *not verified* with the reason.
- **T3. The lag clock starts at a Bronze input's commit time (F4; D5).**
  `pipeline_lag.rs`:
  1. `LagTokenReader` gains `async fn token_at(&self, input) ->
     Result<Option<(String, Option<OffsetDateTime>)>, TokenError>` with a
     default that calls `token` and returns `None` for the time, and
     `async fn tokens_at(&self, inputs) -> Result<(TokenMap,
     BTreeMap<String, OffsetDateTime>), TokenError>` with a default built
     on `token_at` and the same first-failure rule as `tokens`. `tokens`
     stays and calls `tokens_at`, dropping the times. `FakeTokens` gains an
     optional time per key.
  2. `StateTokenReader` implements `token_at`: the Bronze branch returns
     `last_updated_ms` converted to `OffsetDateTime` (an out-of-range value
     is `None`, never a guess); the Silver/Gold branch returns `None`.
     `iceberg_snapshot_token` returns both; `token` keeps its answer.
  3. One helper, `fn clock_start(times, tokens, consumed, consumed_at, now)
     -> OffsetDateTime`: the earliest time among inputs whose token differs
     from `consumed` (all inputs when nothing was consumed), clamped to
     `[consumed_at, now]`; `now` when no changed input has a time.
  4. `decide` (`:1365`) and the chain (`:1168`) read with `tokens_at`; the
     four `pending_since.unwrap_or(now)` sites use
     `unwrap_or(clock_start(...))`: P's own tokens at `:1400`, U's tokens
     at `:1217` and `:1261`, and `:1103` keeps `now` (P's tokens are not
     read on that path; say so in a comment).
  *Check:* unit tests with `FakeTokens`:
  `the_clock_starts_at_a_bronze_inputs_commit_time`
  (commit 100 s before `now`: `lag_seconds == 100` on the first check),
  `a_commit_time_in_the_future_starts_the_clock_now`,
  `a_commit_time_before_the_last_consumed_run_starts_at_that_run`,
  `an_input_with_no_commit_time_starts_the_clock_at_the_first_check`
  (today's behaviour), `with_two_changed_inputs_the_earlier_commit_starts_the_clock`,
  and `a_downstream_chain_starts_ps_clock_at_its_upstream_inputs_commit`.
  Every existing `pipeline_lag` test passes unchanged.
- **T4. Mark ready answers at once; Run now waits for a loading job (F6;
  D8, D9).** `set_status_route`: spawn the reload. `authored_launch_target`:
  after the reload, call a new private
  `async fn wait_for_job(state, id, attempts: u32, interval: Duration) ->
  bool` (the route passes 10 and 2 s, as named constants with a comment
  saying they bound a waiting request and are not a measurement); the new
  409 text of D9.
  *Check:* `routes::pipelines` tests with the `:9638` state:
  `marking_ready_answers_before_the_orchestrator_reload_finishes` (the
  `reloadRepositoryLocation` mock answers after a 5 s `set_delay`; the
  route returns inside `tokio::time::timeout(2 s)` and the row is `ready`);
  `wait_for_job_finds_a_job_that_appears_on_the_second_look` (the jobs
  mock answers without the job once, `up_to_n_times(1)`, then with it;
  `attempts = 3`, `interval = 50 ms` → `true`);
  `wait_for_job_gives_up_after_its_attempts` (→ `false`);
  `run_now_on_a_job_that_never_loads_says_to_try_again` (409, body contains
  "try again in a minute"). The existing `set_status` tests pass unchanged.

### Console (T5 to T9)

New tests for the detail page go in a new `describe` block inside
`pipeline-detail-page.test.tsx`, inserted right after the BLD-8 block
(after `:225`), not at the end of the file: PR #619 appends at the end and
edits `:312`. Reuse the file's harness; do not create a second file that
mocks `next/navigation` again (bun runs every test file in one process).

- **T5. The trigger names the target lag (F2; D3).** In
  `pipeline-schedule.ts`, `export function triggerSummary(schedule:
  ParsedSchedule, targetLag: TargetLag | null | undefined)` returning
  `{ kind: "lag" | "downstream" | "unknown-lag" | "manual" | "cron" |
  "other", label: string }` per D3. Use it in `ScheduleChip` (pass
  `targetLag`), `SettingsTab`'s `scheduleText`, the "Next run" figure and
  the list's Schedule column; hide Pause schedule / Resume schedule while a
  target lag is set (D3).
  *Check:* new `src/features/pipelines/pipeline-schedule.test.ts`:
  `a 10-minute target lag reads "Within 10 min of its inputs" over a manual schedule`,
  `a target lag wins over a stored cron`, `a downstream target lag reads "When a downstream pipeline needs it"`,
  `120 minutes reads as 2 h`, `no target lag keeps today's manual and cron labels`.
  In `pipeline-detail-page.test.tsx`:
  `a pipeline with a target lag and no schedule never says "Manual only"`,
  `a pipeline with a target lag and a stored cron offers no Pause schedule button`.
  In `pipeline-columns.test.tsx`: the Schedule cell of a target-lag row.
- **T6. Current lag says "Up to date" and how lag is counted (F3; D4, D6).**
  `pipeline-freshness-panel.tsx` only.
  *Check:* in `pipeline-freshness-panel.test.tsx`:
  `a met pipeline with no pending change shows "Up to date", not a dash`,
  `a not measured pipeline shows "Not measured" as its current lag`,
  `a pending lag of 125 seconds shows as 2 min 5 s` (whatever the shared
  formatter prints; pin its output), and `the panel says how lag is counted`.
- **T7. The author panel agrees with the header (F5; D7).**
  `authorChoices(users, tenantId, current: ReadsAs, listFailed: boolean)`;
  the page passes `readsAsState` and `usersState.status === "error"`.
  *Check:* in `pipeline-reads-as.test.ts`:
  `with an author and nobody else to offer, the reason names the author`,
  `an author the list cannot name is "a user you cannot see" in the reason`,
  `a user list that failed to load says so, not that nobody can read as it`,
  `with no author and nobody to offer, today's sentence stays`. Existing
  cases updated only for the new parameters.
- **T8. Mark ready and Run now show what happened (F6; D10).** The inline
  refusal, the bounded detail poll while the job loads, the banner text.
  *Check:* in `pipeline-detail-page.test.tsx` (fetch stubbed):
  `a refused Run now shows the API's reason under the masthead`,
  `a ready pipeline whose job is loading reloads its detail until the job is there`
  (first detail `orchestratorJob: false`, second `true`; the banner goes
  away without a manual reload), and
  `after Mark ready the page shows the pipeline as ready without a reload`.
- **T9. The list stops saying false things (F8, F9; D12).**
  `pipeline-columns.tsx` only (Last Run needs no change: T1 fills it).
  *Check:* in `pipeline-columns.test.tsx`:
  `a pipeline with no SLA reads "Not defined", not "Breached"`,
  `an authored ready row offers Trigger run`,
  `an authored draft row's Trigger run is disabled and says to mark it ready`,
  `the Origin tooltip does not say an authored pipeline cannot run`.

### Docs (T10)

- **T10. The feature page says what is now true.**
  `docs/core/features/pipeline-target-lag.md`: under Limits, add "Lag is
  counted from when a Bronze input was loaded (its snapshot's commit time),
  and from the first check that saw the change for a Silver or Gold input
  (up to 30 s later)." Under Decisions, add rows 6 (D3 wording), 7 (D4) and
  8 (D5), each with Signed "Planner default (`BLD-11`), owner may change".
  No other page changes.

## 6. PR slicing and overlap with PR #619

One PR, ten commits (T1 to T10), Rust before TypeScript. Draft as soon as
T4 is pushed. Languages touched: Rust (`lakehouse-store`, `lakehouse-api`),
TypeScript (console), Markdown. No Python, no compose, no migration.

Files this plan and PR #619 (`BLD-16`, not merged) both touch:

| File | #619 | Here | How we stay apart |
| --- | --- | --- | --- |
| `rust/crates/lakehouse-api/src/pipeline_compile.rs` | validation and `render_step`/`step_body` (`:57-160`, `:337-540`), tests appended | `render_transform_step` body `:546-598` and new tests | #619 calls `render_transform_step` unchanged; our tests go right after `:745`, not at the end |
| `rust/crates/lakehouse-store/src/pipelines.rs` | `PipelineInput`/`PipelineStep` `:254-430` | a new function after `set_status` `:2058` | different region |
| `src/features/pipelines/pipeline-detail-page.test.tsx` | edits `:312`, appends after `:336` | a new block after `:225` | different region |

Whichever merges second rebases and resolves hunk by hunk (`AGENTS.md`
rule 9). Not touched here, though #619 touches them:
`pipeline-definition.tsx`, `pipeline-flow-layout.ts`, `pipeline-editor*.tsx`,
`src/lib/pipeline-editor.ts`, `src/services/contracts/pipelines.ts`,
`src/services/clients/pipelines.ts`, `routes/mod.rs`, `policy.rs`,
`tool_schemas.json`. If a task seems to need one of them, stop and report.

## 7. Verification before handoff

On the final commit, in the foreground, quoting each command and its counts
(rule 7). The local full Rust run may be replaced by CI on the draft PR
(owner, 2026-10-30); say which you did.

```bash
cd rust && cargo fmt --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace
bun run typecheck && bun run lint && bun run test
git status   # nothing generated staged
```

Also in the handoff: the T2 live ClickHouse result; the time the first Run
now took after Mark ready on the compose stack, or *not verified*; which
pages were opened in a browser and in which theme, or *not verified*.

## 8. Out of scope

- Refreshing only changed rows, and the stored `kind`/`incrementalColumn`
  fields, including checking that the column exists (`BLD-12`).
- A commit time for Silver or Gold inputs (D5 keeps the first check).
- Judging a change by when its run landed (a "missed by N s" record after
  the run); the status during the run already turns Behind once the lag
  passes the target.
- The checks on `generate` and on ready (`BLD-13`).
- Backfilling `last_run_at` for pipelines that ran before this change.
- The never-written `freshness_lag_seconds` column and the list's
  "Freshness" column.
- Any change to the assistant's tools (D13).
- Making Pause stop a pipeline that has a target lag (F10; a new backlog
  item).

## Handoff

*(the developer appends one entry per round)*

### Round 1 — all ten tasks (Hy4 flash, 2026-11-20)

Branch `bld-11-pipeline-page`, base `eafff51e3`; one commit per task, pushed
after each. No PR opened (developer never opens/merges). No migration, no
Python, no compose, no assistant change.

| Task | Commit | Scoped checks actually run (counts) |
| --- | --- | --- |
| T1 F1/D1/D2 | `a4f11c81e` | `cargo test -p lakehouse-store --test it pipeline_last_run::` → 3 passed; `cargo test -p lakehouse-api --lib run_event_writes_the_runs_start_as_last_run_at` → 2 passed; clippy both crates clean |
| T2 F7/D11 | `3ae339282` | `cargo test -p lakehouse-api --lib pipeline_compile::tests::` → 43 passed; clippy clean |
| T3 F4/D5 | `20121edde` | `cargo test -p lakehouse-api --lib pipeline_lag::tests::` → 49 passed; clippy clean |
| T4 F6/D8/D9 | `8e3f5c2d6` | `cargo test -p lakehouse-api --lib status_route::` → 9 passed; clippy clean |
| T5 F2/D3 | `985dc040f` | `bun test` on `pipeline-schedule`, `pipeline-columns`, `pipeline-detail-page` → 27 passed; typecheck clean |
| T6 F3/D4/D6 | `87f877942` | `bun test pipeline-freshness-panel.test.tsx` → 18 passed; typecheck clean |
| T7 F5/D7 | `8658589ec` | `bun test pipeline-reads-as.test.ts pipeline-detail-page.test.tsx` → 26 passed (re-run reads-as alone → 14); typecheck clean |
| T8 F6/D10 | `b774c7d7e` | `bun test pipeline-detail-page.test.tsx` → 15 passed; typecheck clean |
| T9 F8/F9/D12 | `e570ded66` | `bun test pipeline-columns.test.tsx` → 14 passed; typecheck clean |
| T10 docs | `b221ae837` | docs only |

Each commit ran `cargo fmt --check` (Rust) / `bun run typecheck` (TS) and the
scoped clippy/test above, in the foreground, from the shared `rust/target`
(no `cargo clean`). Every new behaviour test was mutation-checked by reverting
its fix line, running, and restoring: T1 5 fail, T2 3 fail, T3 4 fail, T4 3
fail (blocking reload, single-look poll, old 409 text), T5 9 fail, T6 6 fail,
T7 3 fail (the no-author case correctly passes on both), T8 inline-alert and
poll-disabled fail, T9 3 fail. The tests that pin today's behaviour on purpose
(`a_filter_or_dedupe_alone_still_selects_star`, no-time/future-commit lag,
no-author reads-as, no-target-lag labels, draft Trigger-run) pass on both.

**Final verification (final commit `b221ae837`, foreground):**

- `bun run typecheck` — pass.
- `bun run lint` — pass.
- `bun run test` — first run 4658 pass / 1 skip / 1 fail
  (`schema-browser.test.tsx:360` "keeps the table's name unknown on a Trino
  tab until its detail loads", expected `SELECT * FROM ` got
  `SELECT * FROM undefined.undefined`); that test passes in isolation and the
  immediate re-run of the whole suite was **4659 pass / 1 skip / 0 fail**, so
  it was a shared-process flake in an unrelated area, not caused by this
  branch. No test here was weakened to get green.
- `cd rust && cargo fmt --check && cargo clippy -p lakehouse-store -p
  lakehouse-api --all-targets -- -D warnings` — pass.

**Extras:**

- T2 live SQL check: on a real `ClickHouse` 26.8 (testcontainer), created
  `default.bld11_src (id Int32, amount Decimal(12,2))`, inserted one row, then
  `CREATE TABLE default.bld11_out ENGINE = MergeTree ORDER BY tuple() AS
  SELECT * REPLACE (CAST(amount AS Float64) AS amount) FROM default.bld11_src`;
  `SELECT toTypeName(amount) FROM default.bld11_out` returned **Float64** (the
  source type was Decimal). The `EXCEPT`-with-rename statements also executed.
  Both tables were dropped after. This was a testcontainer ClickHouse, not a
  `docker compose up` of the project stack.
- The time the first Run now took after Mark ready on the compose stack:
  **not verified** (no compose stack was started; the Rust route tests use
  wiremock mocks).
- Browser pages/themes opened: **not verified** (no browser session).
- No full Rust workspace run locally (CI runs it on the PR, per the owner's
  2026-10-30 decision). Scoped per-crate clippy/test above are the local proof.

### Fix round 1 — planner review R1–R5 (Hy4 flash, 2026-11-20)

One commit per finding, in order, pushed after each. Every fix site cites
`BLD-11 review R<n>`. No force push, no PR.

| Finding | Commit | Fix + scoped check (counts) |
| --- | --- | --- |
| R1 | `b7d78b767` | Keep Resume for a paused target-lag pipeline (hide Pause only); reword feature-page decision 6. `bun test pipeline-detail-page.test.tsx` → 16 passed. Mutation: old hide-both condition → new test fails (no Resume) |
| R2 | `ee74930ce` | Move the D9 constants and `wait_for_job` above `authored_launch_target`'s doc block. Structural (no test asked): `cargo fmt --check`, `cargo clippy -p lakehouse-api --all-targets -- -D warnings`, existing Run-now 409 test → 1 passed |
| R3 | `d5e530f24` | Wrap the bounded looks in `tokio::time::timeout(20 s)`; same 409 on expiry; fix the "first attempt is that look" comment. `cargo test -p lakehouse-api --lib status_route::` → 10 passed. Mutation: remove the timeout → the stalled-probe test ran 269 s and failed |
| R4 | `4fd9dbde5` | Enable Trigger run for any non-draft authored row (paused included); correct the list page's comment. `bun test pipeline-columns.test.tsx` → 15 passed. Mutation: ready-only condition → new paused-row test fails |
| R5 | `15d8ec773` | Limits line in `pipeline-joins-and-sql.md` + changelog entry about D11's changed output; refuse duplicate output names at save with a fixed sentence. `cargo test -p lakehouse-api --lib pipeline_compile::tests::` → 46 passed. Mutation: remove the check → old code compiled the duplicate-column SQL and both refusal tests failed |

**Final verification (final commit `15d8ec773`, foreground):**

- `bun run typecheck` — pass.
- `bun run lint` — 0 errors, 13 pre-existing warnings (none from this branch).
- `bun run test` — first run 4660 pass / 1 skip / 1 fail in an unrelated
  shared-process test (`schema-browser.test.tsx` Trino-tab case, which passes
  in isolation); the immediate re-run was **4661 pass / 1 skip / 0 fail**.
  No test was weakened to get green.
- Per-commit scoped Rust checks: `cargo fmt --check`, `cargo clippy -p
  lakehouse-api --all-targets -- -D warnings`, and the filters above; all pass.

## Review

*(the reviewer appends one entry per round)*

### Round 1 — planner review, 2026-11-20, at `26dc94c78`

No BLOCKER. Holds: `last_run_at` is written by both run-event routes after
Dagster confirms the end, with one monotonic bound `UPDATE`; the lag clock
uses the snapshot's own `timestamp_ms`, clamped; "Up to date" shows only for
a measured `met`; the wait uses `tokio::time::sleep`; no route, policy or
upstream-text change; no new mocks.

**R1 SHOULD-FIX: a paused target-lag pipeline cannot be resumed.**
`src/features/pipelines/pipeline-detail-page.tsx:437` hides Resume with
Pause. D3 is amended: hide **Pause** for a target-lag pipeline, but keep
**Resume** while `status === "paused"`. Reword `pipeline-target-lag.md`
row 6 (pausing does stop the lag sensor at the next reload, as F10 says).
Test: a paused target-lag pipeline shows Resume.

**R2 SHOULD-FIX: doc comments attach to the wrong item.**
`rust/crates/lakehouse-api/src/routes/pipelines.rs:2960-2976`: the new
constants sit between `authored_launch_target`'s `///` doc and its
`#[allow]` reason. Move the constants and `wait_for_job` above that doc
block.

**R3 SHOULD-FIX: the 20 s bound is not a bound.** `routes/pipelines.rs:
2970-2995`: sleeps plus untimed `list_jobs` calls can run to the 60 s
route deadline and answer 408 instead of the 409 sentence. Wrap the wait
in `tokio::time::timeout(Duration::from_secs(20), …)` and answer the 409 on
expiry; fix the comment ("the first attempt is that look").

**R4 SHOULD-FIX: two list sentences are still untrue.**
`src/features/pipelines/pipeline-columns.tsx:463-475`: a `paused` authored
row is told "mark it ready first", but the route launches it (only drafts
are refused). Say the draft sentence only for drafts. And
`src/features/pipelines/pipelines-page.tsx:57-59` still says "an authored
pipeline has no engine behind it". Tests.

**R5 SHOULD-FIX: D11 changes existing pipelines' output, and nobody is
told.** A saved pipeline with a cast or rename and no select gets a
different table on its next run (types change; a renamed column loses its
old name and moves to the end), so a chart or a downstream pipeline reading
the old name breaks. Add a Limits line to `pipeline-joins-and-sql.md` and a
`CHANGELOG.md` entry saying so. Also check a rename to a name that already
exists: if the save does not refuse it, refuse it at save with a fixed
sentence (test), rather than let the run fail on a duplicate column.
