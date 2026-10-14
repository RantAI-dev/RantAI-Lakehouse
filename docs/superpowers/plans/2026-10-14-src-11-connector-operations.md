# `SRC-11` Connector operations — Implementation Plan

**Status:** ready to build. The product owner signed the feature page's
decisions on 2026-10-14 ("setuju semua"), unchanged from the defaults this
plan was drafted on. Written 2026-10-14 by the planner (Claude Opus) for
developer agents, under the role split in `AGENTS.md`.

**Base and branch:** the Phase 1 branch `feat/phase-1-rest` (pull request
#144, one PR for the rest of the phase), after `main` at `07c7632` (which
holds `SRC-10`, #115) was merged into it. Also closes `SRC-18` (first half:
a paused connector still loads) and `SRC-19` (a PostgreSQL wrong password is
retried).

**Spec:** `docs/core/specs/src-11.md`. **Decisions:** D1 to D14 below are
the feature page's decisions 1 to 14.

---

## 1. What exists today (read at `432a3c9` for the feature page, re-checked at `dfb3048` where marked ✓; the developer re-verifies every anchor before changing it)

| # | Spec row | Code | Gap |
| --- | --- | --- | --- |
| F1 | Retries | `dagster/dispar_orchestrate/op_metadata.py` ✓ `DEFAULT_RETRY_POLICY` (2 retries, about 30 s then 60 s) on `run_ingest` and the per-table op `ingest_source_object` (`ingest_factory.py`). Classification by exception type in `ingest_source_object`: `Failure(allow_retries=False)` for config, refused address, unsupported type, and a refused sign-in for MySQL, SQL Server and Oracle (`_is_authentication_refusal`). PostgreSQL's refused sign-in is not recognised (`SRC-19`). The Kafka path in `run_ingest` has no classification. No attempt number is recorded. | D1, D2 |
| F2 | Auto-disable | `connectors::record_run_once` ✓ (`SRC-17`) counts runs; `REPEATED_FAILURE_STREAK = 3` ✓; the alert kind `connector_disabled` ✓ exists and nothing raises it; the console's Alerts page says so. `paused_reason` is set only by the schema-change policy; no resume route. | D3, D4 |
| F3 | Run log | Per-table rows in ClickHouse `bronze_meta.ingest_run` (`bronze_catalog.py`): no run id, no attempt, no retention. The orchestrator's step log is reachable only from the Pipelines page, with raw text. | D5, D6, D8 |
| F4 | Run history | `GET /api/connectors/{id}/ingest/runs` searches the newest 100 runs of the shared job for this connector and keeps 20; the console joins per-table rows to runs by time (`connector-ingest-panel.tsx` `groupResultsByRun`). | D7 |
| F5 | Column selection | `SourceObject` ✓ (`lakehouse-store/src/ingest_spec.rs`) has no column member. The orchestrator can already leave columns out of a SQL load (`schema_observer.keep_only`, used for `ask_first`), after the before-load observation has seen the whole table. | D9 to D12 |
| F6 | (`SRC-18`) | Only the due list and `POST …/ingest/run` check the pause; the orchestrator never does. | D13 |

## 2. Decisions as built (from the feature page; the planner's notes in italics)

- **D1.** Up to 3 retries per failing table, after 1, 5 and 15 minutes,
  replacing today's policy for ingest only. *The delays come from one
  setting with those defaults (`INGEST_RETRY_DELAYS_SECONDS`, `60,300,900`)
  so the gate can shorten them; the default is the spec's and the gate says
  it changed it.* Dagster's `RetryPolicy` cannot express 1/5/15; the op
  raises `RetryRequested` with the attempt's delay, or the developer shows
  another way that keeps "only the failing table is retried".
- **D2.** Transient (retried): unreachable, timeout, "busy"/server error,
  the product's own API unreachable. Not retried: refused sign-in (now also
  PostgreSQL: *match the server's own refusal, SQLSTATE `28P01`/`28000`
  where the driver exposes it and otherwise its fixed "password
  authentication failed" message, read internally and never shown*),
  refused address, unsupported type or mode, a rejected setting, a table
  waiting for a schema decision. The Kafka path gets the same rule. One
  function decides, with a unit test per class.
- **D3.** A run counts once after its retries. At 30 failed runs in a row
  (`CONNECTOR_AUTO_DISABLE_AFTER`, default 30, for the gate) the connector is
  paused with `AUTO_DISABLE_PAUSE_REASON` ("Paused after 30 failed runs in a
  row", the number from the setting) and `connector_disabled` is raised
  once, in the same place and transaction pattern as `SRC-17`'s count.
  The Alerts page's "nothing raises this yet" note goes.
- **D4.** `POST /api/connectors/{id}/resume` (`connector:manage`, in
  `POLICY_TABLE` and `tests/route_auth.rs`): lifts only the auto-disable
  pause and sets the streak to 0; audited; a schema-change pause answers
  409 with a fixed text pointing to Schema changes. Console button on the
  paused banner.
- **D5, D6.** A run log of the product's own lines, per run and attempt:
  run started (and by what), each table's attempt (number, start, end,
  rows, result), the error class (D2's fixed words), the wait before the
  next attempt, why the run ended. Never driver or source text. Stored by
  the API (Postgres), written from orchestrator reports on the
  `ingest:read` service path that the orchestrator already uses. Kept 30
  days; a daily job removes older lines and attempts (not the per-table
  results). `GET /api/connectors/{id}/runs/{runId}/log`, and the same with
  `?format=text` as a download.
- **D7.** A product-owned run record per connector run: run id, trigger
  (schedule, user, refresh, clear), started, ended, status, rows, attempts.
  `GET /api/connectors/{id}/runs` returns the last 50, exact for any number
  of connectors. The console's Runs card reads it and stops matching by
  time. The old route stays until the console no longer calls it, then
  goes (no new mocks; remove the consumer).
- **D8.** History and log: `connector:manage` on a connector of one's own
  tenant.
- **D9 to D12.** `SourceObject.excludedColumns` (names; absent = none
  excluded) — *stored as what is left out, not what is kept, so a column
  that appears later follows the schema-change policy (D12) instead of being
  left out silently.* Accepted only for PostgreSQL, MySQL, MariaDB and SQL
  Server; any other type answers 422 with a fixed text. Refused when it
  names a key column of a deduplicating or history table or the cursor
  column (D11). The orchestrator applies it with `keep_only` after the
  before-load observation, so a left-out column is never reported as
  removed. In a table that keeps its rows, a left-out column is marked
  inactive (reusing `SRC-8`'s inactive mark, with its own reason); ticking
  it again clears the mark. No alert, nothing waits (D10).
- **D13.** `run_ingest` reads the pause before loading; a paused connector's
  run ends with status "paused" in the run record and loads nothing. This
  covers a queued run and one started from the orchestrator's screen.
- **D14.** The gate steps run in a new job.

## 3. Tasks

Order keeps Rust together and puts each migration with the code that uses it.
One task per commit; cite `SRC-11` and the decision (and `SRC-18`/`SRC-19`
where they apply) at each site.

1. **Store: run record, attempts, log, retention (D5–D7).** Migration(s),
   next free numbers, why-headers. Store functions with `# Errors`. Store
   tests: a run with two attempts; 50 newest; retention removes lines older
   than 30 days only.
2. **Store: auto-disable and resume (D3, D4).** In the run count: the
   threshold, the pause, a "raise `connector_disabled`" signal returned once.
   Resume. Tests: the 30th failure pauses once; a success before it resets;
   resume lifts only its own pause; a schema-change pause is untouched.
3. **Store and spec: excluded columns (D9–D11).** `SourceObject`, the
   validation on save, the inactive mark. Tests for each refusal and the
   mark.
4. **API routes (D4, D5, D7, D8, D9).** Resume, runs, log (JSON and text),
   the orchestrator's report routes for run start, attempts and log lines,
   the `connector_disabled` delivery through `SRC-17`'s path. Every route in
   `POLICY_TABLE`, asserted both ways in `tests/route_auth.rs`. Route
   tests, including a download that holds no source text.
5. **Orchestrator: retries and classification (D1, D2, `SRC-19`).** Unit
   tests per class; a test that only the failing table is retried; the
   delays come from the setting.
6. **Orchestrator: reports, pause check, excluded columns (D5, D7, D13,
   D9).** Reports sent with `timeout=` and `raise_for_status()`; a report
   that cannot be sent does not fail the load (it is logged and the run's
   record shows the gap honestly). Unit tests, no network.
7. **Console (D3, D4, D5, D7, D9–D11).** Runs card from the new route (50,
   trigger, attempts, rows, duration), the log view and download, the
   Resume button, the column picker per table with locked key and cursor
   columns and the "not available for this source type" note. Contracts
   mirror serde names. Component tests; the old time-matching code and its
   tests go.
8. **Gate (D14).** A job of its own. The gate runner forwards the MySQL
   fixture through a TCP forwarder it starts and stops itself (as `SRC-17`'s
   gate runs its own receiver), with `INGEST_RETRY_DELAYS_SECONDS` and
   `CONNECTOR_AUTO_DISABLE_AFTER` lowered for the job and the comment saying
   so. Steps: the source goes away and comes back during the retries → the
   run succeeds, the log shows each attempt; a wrong PostgreSQL password →
   fails at once, one attempt; N failed runs → paused, `connector_disabled`
   arrives at the gate's webhook receiver once, the schedule leaves it out;
   Resume → it runs; a run of the paused connector launched through the
   orchestrator → ends "paused", nothing loaded; a left-out column → absent
   from the next load and marked inactive; the downloaded log holds no
   source text. `ci-required`, `required_gate.sh` and its self-test,
   `docs/CI.md`. **Stop and report** if the gate cannot reach a forwarder it
   controls without a product change.
9. **Docs (planner).** `CHANGELOG.md`, `BACKLOG.md` (`SRC-11`, `SRC-18`
   first half, `SRC-19`), `docs/plans/SRC-11-RESULT.md` with what the gate
   measured (attempt delays as configured, time to auto-disable, a log's
   size), the feature page's status.

## 4. Build limits on this machine

No `cargo test`, `cargo build`, `cargo nextest`, `cargo clean`; no
containers. `df -h /` before any cargo command, stop under 12 GB free. Shared
target `/home/hv/.cache/lakehouse-src6-target`, `CARGO_BUILD_JOBS=2`,
`CARGO_INCREMENTAL=0`; touch every Rust source before the workspace clippy;
run it detached and poll. Rust tests and gates are *not verified* until CI
on #144.

## 5. Out of scope

- A retry setting per connector; a manual Pause; re-running a run from the
  connector page.
- Column choice for Oracle (`SRC-13` first), files, REST, MongoDB, Kafka,
  SFTP.
- The observation routes' permission (`SRC-18`, second half).
- Change-capture connectors.

## Handoff

### Rust side, tasks 1 to 4 (developer, 2026-10-14, branch `feat/phase-1-rest`)

**Commits** (one per task, in order):

| Task | Commit | Title |
| --- | --- | --- |
| 1 | `eca82c2e` | `SRC-11 feat(store): run record, attempts, log and retention` |
| 2 | `f73a0a1e` | `SRC-11 feat(store): auto-disable after N failed runs, and resume` |
| 3 | `0cb8f851` | `SRC-11 feat(store): columns left out of a load (excludedColumns)` |
| 4 | `5f5631d5` | `SRC-11 feat(api): run history, run log, resume, and the orchestrator's run reports` |

**Commands run, and what was not run.** The disk had 7.1 GB free at the
start and 3.8 GB at the end (`df -h /`), under the 12 GB floor, so **no
compiling cargo command was run**: *not verified* are `cargo check`, `cargo
clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test`
(store, API, `route_auth`). Every Rust test written here is therefore *not
run*; CI on #144 is the proof. Run: `df -h /` before work; `rustfmt --edition
2024 <file>` on each Rust file touched (never on a file with foreign hunks);
`cargo fmt --check` from `rust/` at the end (no output, exit 0, with
`CARGO_TARGET_DIR=/home/hv/.cache/lakehouse-src6-target`). Python, TypeScript
and compose were not touched.

**Migrations.**

- `0070_connector_run_record.sql`: `connector_run` (`run_id` PK, `connector_id`
  FK cascade, `trigger_kind`, `status`, `started_at`, `ended_at`,
  `rows_loaded`, `attempts`, `end_reason`), `connector_run_attempt` (PK
  `(run_id, table_name, attempt)`), `connector_run_log`. Closed lists are CHECK
  constraints (trigger, status, end reason, error class, log event).
- `0071_connector_inactive_column_reason.sql`: `connector_inactive_column.reason`
  (`removed_at_source` default, or `excluded`). SRC-8's two deletes of inactive
  marks now say `AND reason = 'removed_at_source'`.

**Settings.** `CONNECTOR_AUTO_DISABLE_AFTER` (default 30; zero, negative or
garbage keeps 30), read into `Config::connector_auto_disable_after`. It is
**not yet passed by `docker-compose.yml` and not in `.env.example`**: task 8
(gate) must add it to the `api` service environment to lower it for its job.

#### Routes (all new; every one is in `POLICY_TABLE`)

Wire names are camelCase. Times are RFC 3339 strings (`2026-10-14T10:00:00Z`).
Errors are the usual `{"error": "<text>"}`.

**The orchestrator's report routes** (tasks 5 and 6 build against these).
Permission `ingest:read`, and the handler adds `require_orchestrator` (a
service identity or an unrestricted administrator; a user holding
`ingest:read` gets 403). Outside the tenant gate. A body with a field not
listed is a 400 (`deny_unknown_fields`); the 400 text is a fixed phrase and may
name a serde variant list, never source text.

1. `POST /api/connectors/{id}/run-reports` : a run started.
   - Request: `{"runId": string (1..128, no control characters), "trigger": "schedule"|"user"|"refresh"|"clear"|"other", "startedAt"?: RFC 3339 (default: now)}`.
   - Response 200: `{"recorded": boolean}`; `false` when this run id was already recorded for this connector (nothing changes).
   - 404 unknown connector; 409 when the run id already belongs to another connector.
   - Send it once, at the very start of `run_ingest`, for every run, `clear` and `refresh` included. `other` is a run launched from the orchestrator's own screen.
2. `POST /api/connectors/{id}/run-reports/{runId}/attempts` : one attempt at one table ended (send it when the attempt ends, success or failure).
   - Request: `{"table": string (1..256, the source object name as the spec spells it), "attempt": integer 1..50, "startedAt": RFC 3339, "endedAt": RFC 3339 (not before startedAt), "rowsLoaded"?: integer >= 0 (default 0), "errorClass"?: <class>, "waitSeconds"?: integer 0..86400}`.
   - `errorClass` absent means success. A failure sends exactly one of: `unreachable`, `timeout`, `source_busy`, `api_unreachable` (the four that are retried) or `sign_in_refused`, `address_refused`, `unsupported`, `setting_rejected`, `schema_decision_pending`, `other` (not retried; `other` is an unclassified failure). Any other value is a 400.
   - `waitSeconds` only on a failure that will be retried (the wait before the next attempt); on a success it is a 400.
   - Response 200: `{"recorded": boolean}`; `false` for a repeat of the same `(run, table, attempt)` (the first report wins, nothing is written twice).
   - 404 when the run is unknown or belongs to another connector. The run must have been started (route 1) first.
   - Effect: stores the attempt, writes the log line(s) ("table T, attempt N: loaded R rows in S s" or "failed, <fixed phrase for the class>", and "next attempt in M min" when `waitSeconds` is set), and recomputes the run's `attempts` (all attempts) and `rowsLoaded` (rows of the attempts that succeeded).
3. `POST /api/connectors/{id}/run-reports/{runId}/log` : one line of a closed list.
   - Request: `{"event": "schema_decision_pending"|"columns_left_out"|"report_gap", "table"?: string, "count"?: integer >= 0, "at"?: RFC 3339 (default: now)}`. `schema_decision_pending` needs `table`; `columns_left_out` needs `table` and `count`; `report_gap` needs neither (use it when an earlier report could not be sent, so the log says it may be incomplete).
   - Response 200: `{"recorded": true}`. 400 for a missing table or count; 404 as above.
4. `POST /api/connectors/{id}/run-reports/{runId}/end` : the run ended.
   - Request: `{"status": "succeeded"|"failed"|"paused", "reason": "completed"|"table_failed"|"connector_paused"|"other", "endedAt"?: RFC 3339 (default: now)}`. `status: "running"` is a 400.
   - `paused` is the D13 case: the connector was paused when the run started, nothing was loaded.
   - Response 200: `{"recorded": boolean}`; `false` when the run already had an end (the first end wins, including one closed by the run sensor).
   - 404 as above.

How the orchestrator learns the pause (D13, no new route): `GET
/api/connectors/ingestible` already returns `paused` for each connector, the
unfiltered list included (only the due list filters it out).
`excludedColumns` reaches the orchestrator in `GET .../ingest-spec`, inside
each `sourceObjects[]` entry, unchanged.

**The person's routes** (task 7 builds against these). Permission
`connector:manage`, behind the tenant gate (another tenant's or unknown
connector: 404).

5. `GET /api/connectors/{id}/runs` -> `{"runs": [Run]}`, newest first, at most 50.
   `Run` = `{"runId": string, "trigger": "schedule"|"user"|"refresh"|"clear"|"other", "status": "running"|"succeeded"|"failed"|"paused", "startedAt": ISO, "endedAt": ISO|null, "durationSeconds": integer|null, "rowsLoaded": integer, "attempts": integer, "endReason": "completed"|"table_failed"|"connector_paused"|"other"|null}`.
   (ISO = `2026-10-14T10:00:00.000Z`, milliseconds.)
6. `GET /api/connectors/{id}/runs/{runId}/log` -> `{"run": Run, "attempts": [Attempt], "lines": [Line]}`.
   `Attempt` = `{"table", "attempt", "startedAt", "endedAt", "rowsLoaded", "result": "succeeded"|"failed", "errorClass": string|null, "waitSeconds": integer|null}`;
   `Line` = `{"at": ISO, "event": "run_started"|"attempt"|"retry_wait"|"schema_decision_pending"|"columns_left_out"|"report_gap"|"run_ended", "table": string|null, "attempt": integer|null, "text": string}`.
   `?format=text` answers `text/plain; charset=utf-8` with `Content-Disposition: attachment; filename="run-<runId>.log"`: a heading line `run <id> (<trigger>), <status>`, then `<ISO>  <text>` per line (the same `text` as the screen). `?format=` anything but `json`/`text` is a 400. 404 for an unknown run or another connector's run. Attempts and lines older than 30 days are gone (empty arrays); the run stays.
7. `POST /api/connectors/{id}/resume` (no body) -> `{"resumed": true}`. 409 `"This connector is not paused."` when nothing is paused; 409 `"This connector is not paused because of repeated failures. If a schema change is waiting, approve it under Schema changes to resume it."` for any other pause (the schema-change policy's included); nothing changes in either case. Audited as `connector.resume`.

**Other API wire changes.**

- `GET .../schema-changes`: each `inactiveColumns[]` entry gains `"reason": "removed_at_source"|"excluded"` (the console contract `InactiveColumn` in `src/services/contracts/connectors.ts` needs the field).
- `PUT .../ingest-spec`: a source object may carry `"excludedColumns": [string]`. 400 for a malformed list (not an array of at most 2000 distinct, non-blank names of at most 128 characters without control characters); 422 with a fixed text when the dial is not `sql` with the `postgres`, `mysql` or `mssql` driver (so Oracle and change capture are refused), or when it names a key column of a `replace_dedup`/`incremental_dedup`/`history` object or the `incrementalKey` of an `incremental`/`incremental_dedup` object. An empty list or `null` means none. A column that does not exist at the source is not checked at save time.
- A column left out of an `append`/`incremental`/`incremental_dedup`/`history` object is marked inactive (reason `excluded`) in the same transaction as the save; ticking it again, a `replace` mode or taking the table out of the selection clears it.
- `connector_disabled` is now delivered by the run sensor path (`routes::load_alerts`) once, for the run whose failure paused the connector.
- `run_failed_event`/`run_finished_event` for an `ingest_job` now: skip a run whose record says `paused` (`{"matched": 0, "reason": "a paused run loads nothing"}`, the connector is not counted), and close a run record still `running` from the confirmed final status.
- A daily task in the API (`spawn_run_log_cleanup`, first pass at start) deletes log lines and attempts older than 30 days.

**Tests added (all *not run*).**

- Store, `tests/connector_run.rs` (7): two attempts and the totals/log order; repeated reports change nothing; another connector's run; newest 50 per connector; retention removes only old lines and attempts and keeps the run rows; contradictory reports; the sensor settle.
- Store, `tests/connectors.rs` (4): the Nth failure pauses once (and its retry/next failure add nothing); a success resets the count; resume lifts only its own pause and resets the count; a schema-change pause is left alone and the reason shape is recognised.
- Store, `tests/schema_change.rs` (1): a left-out column is marked, carries the Bronze name, survives a before-load observation, and is cleared by ticking again, `replace`, and table removal.
- Store, `ingest_spec.rs` unit tests (6): column-choice sources, nothing-left-out, key and cursor refusals, malformed lists, which modes get the mark, `SourceObject` reads `excludedColumns`.
- API, `config.rs` (1): the setting. `load_alerts.rs` (3): `connector_disabled` once; a rule-store failure on the pausing report still delivers on the retry; a paused run is not counted and an unclosed run is settled. `connector_runs.rs` (7): reported run listed with its log (JSON and text), no source text can enter, only the orchestrator files reports and a run belongs to one connector, tenant isolation, resume, 50 runs, file name. `tests/route_auth.rs` (2): the person/orchestrator permission split both ways for all seven routes, and the 422/400 of `excludedColumns` at the ingest-spec route.

**Existing tests changed, and why.**

- `lakehouse-store/tests/connectors.rs`: the three `record_run_once` tests pass the new last argument `DEFAULT_AUTO_DISABLE_AFTER` (the function now takes the threshold); no assertion changed.
- `lakehouse-store/src/ingest_spec.rs`: the two `SourceObject { .. }` literals gain `excluded_columns: None` (new field).
- `lakehouse-store/tests/schema_change.rs`: one assertion added to an existing SRC-8 test (`inactive[0].reason == "removed_at_source"`).
- `lakehouse-api/src/routes/load_alerts.rs`: the test `harness` became `harness_with(.., extra_env)` and `harness` calls it with no extras; no assertion changed. With the default threshold of 30 no existing test reaches the pause.

**Code outside tasks 1 to 4 that moved.** `routes/schema_changes.rs`:
`require_orchestrator` and `check_name` are `pub(super)` now (reused, not
copied, rule 4); the 403 text of `require_orchestrator` is generic ("files this
report"); no test asserted the old text.

**What I think is most likely to fail in CI (nothing was compiled).**

1. A compile error somewhere in `connector_run.rs` / `routes/connector_runs.rs` (about 1,900 lines written without a compiler): deref coercion of `&mut Transaction` to `&mut PgConnection` in `insert_line`/`lock_run` calls, and `Option<&str>` types. I checked each by hand.
2. SQL in `schema_change::sync_excluded_columns_in` (`unnest($2::text[], $3::text[]) AS d(o, c)` in a `DELETE ... NOT EXISTS` and in an `INSERT ... SELECT` with a correlated `jsonb_array_elements` subquery). Most complex statement of the set.
3. Clippy `-D warnings`: `doc_markdown` on a word I did not backtick, `too_many_lines`/`too_many_arguments` (`insert_line` and `add_log_line` have 7 parameters, the limit), `needless_pass_by_value`, `missing_const_for_fn` on small helpers.
4. `tests/route_auth.rs::every_registered_route_has_a_policy_entry` scans `routes/mod.rs` text: I kept every handler name free of the substrings `get(`, `post(` etc., but the scan is textual.
5. `HeaderMap` index comparisons and `Value == integer` assertions in the new API tests, and the exact 400/404 statuses of serde errors in `connector_runs` tests (`parse_body` answers 400 with `invalid JSON: ...`).

**Gaps and decisions to review (none changes a plan decision).**

- A run killed or cancelled in the orchestrator fires neither run sensor; its record stays `running` unless the orchestrator's end report arrived. The console should not call such a run active forever (task 7), and a later pass could close old `running` rows. Not in the plan.
- The pause also holds when the auto-disable meets a connector already paused by the schema-change policy: the run is not reported as having disabled it and `connector_disabled` is not raised for it (its first pause reason is kept). After that pause is approved, the next failure pauses it.
- `connector_disabled` text says "paused after N failed runs", with N from the setting, not the streak.
- `record_run_once` pauses at `streak >= threshold` (so a connector already above it when this ships is paused by its next failure) and raises the alert only when this run did the pausing.

## Review

*(planner)*
