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

### Orchestrator side, tasks 5 and 6 (developer, 2026-10-14, branch `feat/phase-1-rest`)

Only `dagster/` was touched. **Commits** (one per task):

| Task | Commit | Title |
| --- | --- | --- |
| 5 | `1260e2ac` | `SRC-11 feat(dagster): ingest retries on a 1, 5 and 15 minute schedule, one classification of failures` |
| 6 | `e6c2a5ff` | `SRC-11 feat(dagster): run reports, the pause check, and left-out columns` |

(The handoff commit that adds this entry follows them.)

**Commands run (from `/home/hv/lakehouse-sec27`, venv `~/.cache/rantai-dagster-venv`).**

- After task 5: `(cd dagster && python -m pytest dispar_orchestrate -q -m "not loader")`: 853 passed, 258 deselected.
- After task 6: the same: 907 passed, 258 deselected.
- `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: OK (both after each task).
- `loader` group, once, detached, after the last code commit: `(cd dagster && python -m pytest dispar_orchestrate -q -m loader)` (detached, log in the scratchpad): 258 passed, 907 deselected, in 1030 s (17 min).
- Not run: cargo, docker, TypeScript (not in scope). No gate (task 8).

**What was built.**

- `ingest_errors.py` (new): `classify(exc) -> Classified(error_class, retried)` is the one function (D2). Closed list as in the route handoff; retried: `unreachable`, `timeout`, `source_busy`, `api_unreachable`. By type and driver code (`pymysql` number, `pyodbc` SQLSTATE, `oracledb` code, `psycopg2` SQLSTATE, HTTP status, `kafka`/`paramiko`/`pymongo` class). `other` (nobody taught the module about it) is NOT retried, as the route handoff lists it. `RetrySettings.from_env()` reads `INGEST_RETRY_DELAYS_SECONDS` (default `60,300,900`; one to three whole numbers, 0 to 86400; anything else falls back to the default with one log line that does not echo the value). `UnknownAdapter` and `UnsupportedKafkaAuth` moved here from `ingest_factory.py` (re-imported there, same names). `is_authentication_refusal` replaces `_is_authentication_refusal` (alias kept in `ingest_factory`).
- SRC-19: PostgreSQL refusal = SQLSTATE `28P01`/`28000` when `psycopg2` gives one, else the fixed wording `password authentication failed`, read for `psycopg2` errors only, only inside the process; never logged, stored or sent.
- `ingest_factory.py`: `_decide` + `_fail_or_retry` (raises `RetryRequested(max_retries=len(delays), seconds_to_wait=...)`, or `Failure(allow_retries=False)`, or the exception itself for `other`); both ingest ops lost `retry_policy=DEFAULT_RETRY_POLICY` (other jobs keep it). Only the failing table retries because the mapped step is the retry unit (tested through a real in-process job run: `{"orders": 1, "customers": 3, "invoices": 1}` calls, only `customers` restarted). `run_ingest` retries the connector fetch (`api_unreachable`) and the Kafka path by the same rule.
- `run_reports.py` (new): `RunReporter` (`start`, `attempt`, `log`, `end`), built on `schema_observer.ObserverConfig` and `schema_observer._post_json` (no second helper): service token, `timeout=10`, `raise_for_status()`. A report that fails returns `False`, logs a fixed warning (kind of failure, no URL, token or error text) and never raises; the next report that can be sent is preceded by a `report_gap` line (per reporter, i.e. per step; if the gap line itself cannot be sent, the report is not tried either).
- Reports sent: `start` first thing in `run_ingest`; one `attempt` per attempt in `ingest_source_object` and for the Kafka micro-batch (rows on success; `errorClass` and `waitSeconds` when the failure will be retried); `schema_decision_pending` class on the attempt of a table that waits, plus its log line; `columns_left_out` (table, count) on the final attempt only (not repeated on a retried one); `end` for a paused run, a CDC run, and a Kafka micro-batch (`succeeded`/`completed`, or `failed`/`table_failed` when no retry is left).
- D13: `run_ingest` ends `paused`/`connector_paused` and yields nothing when `connector["paused"] is True` (only an explicit `true`). Applies to scheduled, queued, single-table and orchestrator-screen runs alike.
- D9: `excludedColumns` applied in the before-load gate (`_leave_out_columns`), after `post_observation` has seen the whole table, by subtracting from the API's column list (or all reflected columns). Covers the PostgreSQL path (`dlt_pipeline` gate) and the MySQL/MariaDB/SQL Server path through the same gate; Oracle can't carry the field (the API refuses it). A name the table lacks leaves nothing out and reports nothing; every column excluded raises `ObservationRefused` (class `setting_rejected`, not retried, nothing loaded). `keep_only` is the existing function; the new code only chooses the list.
- `run_kafka_stream_batch` / `_run_stream_connector` now return the row count (were `None`).
- `conftest.py` (new): an autouse fixture replaces `RunReporter.for_run` with a recorder for every unit test (no test can reach a network); its `attempt` has the real keyword signature.

**Things you need to know (decisions I could not take from the plan).**

1. **Trigger `user` is not detectable today.** The API's `POST .../ingest/run` and the orchestrator's own screen both launch a run with no `lakehouse/trigger` tag (checked in `lakehouse-dagster`: `launch_run_with_config` sends none), and an ordinary run's config is identical in both. `_trigger_of` therefore reports: `clear`/`refresh` from the config's action, `schedule` from the sensor's tag, any known value of the tag if one is ever present, else `other`. A run started by a person from the connector page is reported `other` until the Rust side tags that launch `lakehouse/trigger: user` (a one-line change in `launch_run_with_config`'s caller in `routes/connectors.rs`, which I was not to touch); the Python already honours the tag. Not a missing route, so I did not stop.
2. **A batch run's end is not sent by the orchestrator.** The job graph has no step after the mapped steps (`.collect()` ends it), and adding one changes the graph the gate and several tests assert. The API's run sensors close a record still `running` from Dagster's confirmed final status (tasks 1-4), which covers success and failure; a run killed without either sensor still stays `running` (already noted in the Rust handoff). The orchestrator sends `end` where only it knows (paused, CDC, Kafka).
3. **`schema_decision_pending` is reported as an attempt with that class** (rows 0) plus the log line, while the step and the run still succeed (SRC-8 D7). The sheets `unsupported` return (a step that succeeds without loading) is reported the same way, class `unsupported`.
4. **`other` is not retried.** Unclassified failures that used to retry (any driver error nobody listed) now fail at once; the list of known transient errors is in `ingest_errors._classify_driver`/`_classify_one`. This is the main behavioural risk to look at in review and at the gate. The ClickHouse/Iceberg sink side is only covered where it surfaces as `requests`/`OSError`/`pyiceberg` `ServiceUnavailableError`.
5. A connector that is no longer ingestible (`StopIteration` from the fetch) now fails at once (`setting_rejected`) instead of being retried.
6. `INGEST_RETRY_DELAYS_SECONDS` is **not yet passed by `docker-compose.yml`** (not mine to edit, `ops/`/compose are task 8); the gate must add it to the `dagster-daemon`/code-location services to lower it.

**Existing tests changed, and why.**

- `test_ingest_factory.py::test_the_op_retries_an_unreachable_api_but_not_a_refusal` and `::test_the_op_does_not_retry_a_refused_credential_but_still_retries_other_driver_errors`: they asserted that a transient failure leaves the op as a retryable `Failure`/the raw exception for `DEFAULT_RETRY_POLICY`; it now leaves as `RetryRequested` (first wait 60 s). The assertions for refusals are unchanged.
- `test_op_source_metadata.py::test_every_op_carries_the_default_retry_policy`: skips the two ingest ops (D1 replaces their policy); a new test in the same file pins that those two carry NO policy, so the default cannot also apply to them. Every other op is checked as before.

**New tests** (`test_ingest_errors.py` 66, `test_ingest_retries.py` 12, `test_run_reports.py` 20, `test_ingest_run_reports.py` 34): one case per class and per driver; PostgreSQL refusal by SQLSTATE and by wording on a real `psycopg2.OperationalError`, and that the wording is not read for other exceptions; the delay setting (default, set, every bad shape, no echo); only the failing table retried; reports' payloads, `timeout=`, token, gap, never raising; trigger table; paused run; left-out columns (whole table observed first, subtraction from the API's list, unknown name, all excluded, count reported once).

**Not verified.** Nothing against a real API, Dagster daemon, or databases (no gate, no docker): in particular that Dagster's executor in the real deployment honours `RetryRequested.seconds_to_wait` for the mapped step (it does in the in-process executor, tested), and the report routes' behaviour against the Rust code (written against the route handoff; the Rust tests are not run either). Real `psycopg2` refusal text was not produced by a server: the test uses a hand-built `OperationalError` with the server's wording.

### Console, task 7 (developer, 2026-10-14, branch `feat/phase-1-rest`)

Only `src/` was touched. **Commit** (task 7): `0db25b17` `SRC-11 feat(frontend): run history and log, resume, and column choice on the connector page`.

**What was built** (files under `src/features/connectors/` unless noted):

- Runs card (`connector-ingest-panel.tsx`): reads `GET /api/connectors/{id}/runs` (`listConnectorRuns`), one card per run with trigger, status, started, duration, rows, attempts and the end reason in words. A run recorded `running` for more than `STALE_RUN_MS` (6 hours, comment at the constant: a table holds a run open about 21 minutes of retry waits plus the load, and a killed run fires neither report) shows "No end reported", is not active, and does not disable Run now. A run launched by Run now is waited for until the record lists its id (the orchestrator's run id is the record's `runId`, as the handoff of tasks 5 and 6 should confirm).
- Run log: "Show log" on a run reads `GET …/runs/{runId}/log` only when asked; attempts (table, attempt, rows, result, the error class in product words, the wait before the next attempt) then the lines (event in product words plus the API's `text`). "Download log" fetches `…/log?format=text` through the service and saves it with the existing download pattern (Blob and anchor, as `pipelines/run-log-console.tsx`) as `run-<runId>.log`.
- Resume: in the Runs card where the paused note is, shown when the reason matches `Paused after <N> failed runs in a row` (`isAutoDisablePause`, mirrors `AUTO_DISABLE_REASON_HEAD`/`_TAIL` in `lakehouse-store/src/connectors.rs`); calls `POST …/resume`, shows the 409's text as it came, and calls the new `onResumed` prop (the detail page passes `state.reload`, so the pause pill goes). A schema-change pause shows no button.
- Column choice: a "Columns" section per table for `sql` connectors on the postgres, mysql or mssql driver (`supportsColumnChoice`); columns come from this visit's discovery (a saved table not found yet says "Find this table above to list its columns."); unticked columns go to `excludedColumns`. Key columns of a key-using mode and the cursor are ticked, disabled, with the reason (`lockedColumns`), and `effectiveExcluded` never sends one as excluded (the API would answer 422). Other source types: "Column choice is not available for this source type." (once above the table list). Not offered for CDC.
- Schema changes tab: new "Inactive columns" card (shown only when there are some) with the reason: "removed at the source", "left out by you" for `excluded`, plain "inactive" for any other string. `InactiveColumn.reason` is a required contract field; `SourceObject.excludedColumns?` is the contract field.
- Overview: Last run tile and "Needs attention" read the run record (`recentProblems(probes, runs)`); a failed last run says "did not load every table" for `table_failed`, else "failed"; a running or no-end run is not judged, a paused run is not a failure.
- Alerts page: the `connector_disabled` note is now "Raised once when a connector fails runs in a row (30 by default) and is paused. Resume it from its page." The test constant that pinned "Nothing raises this yet" is corrected to the new sentence (it was false once D3 landed).
- Removed: `groupResultsByRun`, `RunGroupCard`, `RUN_TONE`, `RUN_STATUS_LABEL/TONE`, the `IngestRun` and `IngestJobRun` contract types, `listIngestRuns` (`/api/governance/ingest-runs?connectorId=`) and `listIngestJobRuns` (`/api/connectors/{id}/ingest/runs`) from the service and client. No console code calls either route now, so **the Rust route `GET /api/connectors/{id}/ingest/runs` can go** (D7); `/api/governance/ingest-runs` was used by the console for this card only (grep of `src/`), the governance route itself is not mine to judge.

**Commands run** (all from `/home/hv/lakehouse-sec27`):

- `bun run typecheck`: no errors.
- `bun run lint`: 0 errors, 6 warnings, all in code I did not change (`data-table.tsx`, `sidebar.tsx`, `alerts-page.tsx:692` the `rules` conditional, `open-format-card.test.tsx`, `use-data-table.ts`, `dashboard-specs.ts`).
- `bun run test`: **could not complete as one process**. Three attempts were killed with SIGKILL (exit 137) by memory pressure on this shared machine: `src/features/catalog` alone peaks at 4.5 GB (measured with `/usr/bin/time -v`, 95 s, unchanged code of mine: I did not touch it) and the single `bun test src` process grows past what is free next to other developers' builds. So the same suite was run **per directory** (`bun test` on each of `src/features/*`, `src/components`, `src/hooks`, `src/lib`, `src/services`, `src/app`, `src/config`, `src/types`), twice in a row, summing the counts: run 1: 1194 pass, 0 fail; run 2: 1194 pass, 0 fail. `src/features/connectors`: 267 pass in both. `bun run test` as one process is *not verified*.

**Not verified.** No browser check of the new Runs card, the log, the Columns section or the Resume button (component tests only, rendered in the test DOM; the details/summary layout, the "Download log" file save and the long-column-list scrolling were not seen in a browser). Resume against the real 409 was not run (stubbed with the handoff's text). The run-record `runId` equalling the Dagster run id (so a launched run is recognised) is read from the handoff, not run.

**Existing tests changed, and why.**

- `connector-ingest-panel.test.tsx`: `stubFetch` answers `GET …/runs`, the run log and `POST …/resume` instead of the removed routes; the "a run's row" fixture became a run record (summary "7 rows · 1 attempt", pill "Succeeded" instead of "1 of 1 tables · 7 rows" and "Completed"); the "is disabled while a run is going" fixture uses a run recorded now (an old `running` is "no end reported" by design); the `groupResultsByRun` tests were removed with the function; the test "names the table and the action in the run list" was removed: the run record has the trigger (`refresh`, `clear`) but **not the table**, so the Runs card no longer names it. **This drops a SRC-10 decision 5 line ("Clear · public.orders" in the run list); if the product owner wants the table back it needs a field on the run record.** The plan did not ask for it, so I did not add one.
- `connector-overview.test.tsx`: fixtures are run records; `recentProblems` takes two arguments (the two probe-history tests changed only by dropping the third argument); the "Completed" assertions became "Succeeded"; "Could not ask the orchestrator" became "Run history could not be loaded" (the text no longer names the orchestrator).
- `connector-detail-page.test.tsx`, `connector-create-page.test.tsx`: the stubs of the two removed routes became one stub of `GET …/runs`.
- `connector-schema-changes-panel.test.tsx`: the one `inactiveColumns` fixture gained the now-required `reason`.
- `alerts-page.test.tsx`: the `connector_disabled` note constant and test name (see above).

**Added tests**: runs from the record (trigger, rows, attempts, duration; end reasons; empty; stale running; fresh running; load error), the log (open on request and not before, download via `?format=text`, empty log, error), Resume (recognition, success and `onResumed`, 409 text, schema pause has none, Run now disabled while paused), column choice (untick and save, tick again saves none, locked key and cursor, never sends a locked one, not-found-yet note, mysql and mssql, oracle/files/REST note, CDC none, `lockedColumns`), inactive columns (two reasons, none, unknown reason), overview (no-end tile, failed/paused/running problems).

**Things the reviewer may want to look at.** (1) The Resume button sits in the Runs card, not in a banner above the tabs; the page's header pill still shows the pause. (2) Column choice needs discovery in the same visit to list columns; a saved table opened without "Find tables" shows only what is already left out. (3) The six-hour bound is mine; the plan named none.

### Fix round 1 (developer, 2026-10-14, branch `feat/phase-1-rest`)

Review 1 of the planner, three items. One commit each, in this order.

**Commits.**

1. `046e68aa` `SRC-11 fix(api,store): make the SRC-11 Rust code compile and pass clippy` (BLOCKER 1).
2. `35ba4395` `SRC-11 fix(api,store,dagster,frontend): the run record names the table of a refresh or clear` (BLOCKER 2).
3. `9ed760dc` `SRC-11 fix(api,dagster): tag a person's ingest launch lakehouse/trigger: user` (SHOULD-FIX 1).

**BLOCKER 1.** Fixed at: `ingest_spec.rs` (the `target_identity_tests` module now imports `validate_excluded_columns`, `InvalidExcludedColumns`, `SourceObject`, `supports_column_choice`, `inactive_exclusions`); `routes/load_alerts.rs` (the failed-run alerts moved to `deliver_failure_alerts`, `connector_run_event` is under 100 lines); `routes/mod.rs` (`connectors_router` split, the tenant-gated half is `per_connector_router`, as `pipelines_router` is split from its parent). One more clippy finding found by reading, not in the CI log: `tests/connector_run.rs` `fn failed<'a>(table: &'a str, ..) -> AttemptReport<'a>` (`needless_lifetimes`), now `AttemptReport<'_>`. I read every Rust file of `git diff --name-only dfb3048f..HEAD -- rust/` for the classes named (`&mut Transaction` to `&mut PgConnection` coercions, `Option<&str>` binds, INTEGER/BIGINT/TIMESTAMPTZ against `i32`/`i64`/`OffsetDateTime`, `doc_markdown`, `missing_const_for_fn`, `too_many_arguments`, `needless_pass_by_value`, unused imports) and found nothing else wrong: `record_run_once` has exactly 7 arguments (the limit), `insert_line` and `add_log_line` 7; visibility of `check_name`, `require_orchestrator`, `pool`, `parse_body` (all `pub(super)`) matches the new module; column types in `RunRow`/`AttemptRow`/`LineRow` match migration 0070. This was reading only; nothing was compiled.

**BLOCKER 2.** Migration `0072_connector_run_table.sql` (0070 not edited): `connector_run.table_name TEXT NULL` with a CHECK that a table is only on a `refresh` or `clear` run. Store: `start_run(pool, connector, run, trigger, table: Option<&str>, at)` cleans the name like the other table names and returns `Validation` for a table on another trigger; `RunSummary.table` (`table`, null when none) is returned by `GET .../runs` and the run log. API: `StartBody.table` is checked with `check_name("table", .., MAX_TABLE_LEN)` (the attempt report's check) and answers 400 for a table on a trigger that is not `refresh`/`clear`. Orchestrator: `RunReporter.start(trigger, started_at=None, table=None)`; `run_ingest` sends the run config's `object` when the trigger is `refresh` or `clear`. Console: `ConnectorRun.table: string | null` (required, the API always sends it), the run row shows "Clear · public.orders" / "Refresh · public.customers" and just "Schedule" for a run without a table.

**SHOULD-FIX 1.** `DgClient::launch_run_with_config_and_tags(job, config, &[(key, value)])` (tags in `executionMetadata.tags`, mutation declares `$tags: [ExecutionTag!]!`); `launch_run_with_config` delegates with no tags and sends the unchanged request, so `uploads.rs` and `pipelines.rs` callers are untouched. `routes/connectors.rs` `ingest_run_request` launches with `("lakehouse/trigger", "user")`. A refresh or clear launched this way is still reported as `refresh`/`clear` (the action wins over the tag in `_trigger_of`). Comments in `ingest_factory.py` that said the API sends no tag are corrected. *Not verified against a real Dagster:* the GraphQL input type names `ExecutionTag` and `executionMetadata.tags` are from the Dagster schema as I know it; the wiremock tests cannot check them (same caveat as the existing `RunConfigData!` test). The gate (task 8) is the place that would show a wrong name as a failed launch.

**Commands and outcomes.**

- `cargo fmt` (via `rustfmt --edition 2024 --check` on each touched `.rs` file): clean.
- `cd dagster && ~/.cache/rantai-dagster-venv/bin/python -m pytest dispar_orchestrate -q -m "not loader"`: 908 passed, 258 deselected.
- `bun run typecheck`: clean. `bun run lint`: 0 errors, 6 warnings, none in files I changed.
- `bun test src/features/connectors`: 268 pass, 0 fail (14 files). The whole suite was not run (killed by the machine's memory).

**Not run.** `cargo check`, `cargo clippy`, `cargo test` and `sqlx::test`s of any crate: `/` had 8.2 GB free, under the 12 GB floor for compiling. So every Rust edit above, and the Rust tests added below, are *not verified*; the migration 0072 was not applied to any database.

**Tests added or changed.**

- Added (Rust, not run): `connector_run.rs` store test `a_refresh_or_clear_run_keeps_its_table_and_no_other_run_has_one`; API test `a_clear_run_lists_its_table_and_other_runs_have_none` (`routes/connector_runs.rs`); `lakehouse-dagster` test `launch_run_with_config_and_tags_sends_the_tags_as_execution_metadata`; route test `ingest_run_tags_the_launch_as_started_by_a_user` (`tests/ingest_run.rs`, the mock answers only a launch carrying the tag).
- Added (run): Python `test_a_refresh_start_report_names_its_table_and_an_ordinary_one_sends_none`; console "names the table and the action in the run list" restored in `connector-ingest-panel.test.tsx`, adapted to run records.
- Changed: every existing `start_run(..)` call gained the `None` table argument (store tests, `connector_runs.rs`, `load_alerts.rs` tests); `RecordingReporter.start` in `conftest.py` takes `table` and records `("start", trigger, table)` for a refresh or clear; the parametrized trigger test expects that tuple for `refresh`/`clear`; the console `record()` and overview `run()` fixtures gained `table: null`. No assertion was weakened.

**Most likely to fail in CI next.** (1) Anything in the 3,900 added Rust lines that clippy `pedantic` flags and I could not see without compiling (`doc_markdown` on a CamelCase word in a doc comment, `unnecessary_wraps` on the `-> sqlx::Result<()>` tests, `uninlined_format_args`); (2) sqlx runtime type or SQL errors in the new queries, in particular `sync_excluded_columns_in` (`unnest($2::text[], $3::text[])` and the `ON CONFLICT (connector_id, object_name, column_name)` target, which depends on the primary key of 0064) and `RECORD_RUN_SQL` untouched; (3) the Dagster `ExecutionTag` input name (above); (4) `route_auth.rs` both-ways assertions for the seven new routes were not re-read against the policy table.

### Task 8, the gate (developer, 2026-10-14, branch `feat/phase-1-rest`)

Only `docker-compose.yml`, `.env.example`, `ops/`, `.github/workflows/ci.yml`, `scripts/ci/` and `docs/CI.md` were touched. No product change was needed in `rust/`, `dagster/` or `src/`, and the gate does reach a forwarder it controls (see "How it reaches the forwarder").

**Commits.**

| Task | Commit | Title |
| --- | --- | --- |
| 8a | `27e5a69b` | `SRC-11 feat(compose): pass the retry delays and the auto-disable threshold` |
| 8b | `4ae1bcdd` | `SRC-11 test(gate): connector operations in a job of its own (g6-ops)` |

(The handoff commit that adds this entry follows them.)

**What was built.**

- 8a. `INGEST_RETRY_DELAYS_SECONDS: ${INGEST_RETRY_DELAYS_SECONDS:-}` on `dagster-code-location` (the service whose run process executes an ingest run; the daemon only runs sensors, so it gets none) and `CONNECTOR_AUTO_DISABLE_AFTER: ${CONNECTOR_AUTO_DISABLE_AFTER:-}` on `lakehouse-api`, each with a why-comment, and both (empty, commented) in `.env.example`. Empty or unset falls back to the code (`RetrySettings.from_env` treats blank as default; `parse_positive_u32_or_default` treats empty as unparseable, so 30). Keys are unique (counted); no credential has a default.
- 8b. `ops/g6/src11_connector_operations_test.py` (steps a to g as in the task), runner service `g6-ops-test-runner` (profile `g6-ops`) in `ops/g6/docker-compose.g6.override.yml`, job `g6-ops` ("Rust · SRC-11 connector operations") in `ci.yml` with `INGEST_RETRY_DELAYS_SECONDS=5,10,20` and `CONNECTOR_AUTO_DISABLE_AFTER=3` in the job's `.env`, with a comment at the job, in the step and in the script's docstring that the numbers are lowered and why (checklist row 6). `ci-required` `needs` + `G6_OPS_RESULT`, `required_gate.sh`, two self-test cases (failure and skipped), `docs/CI.md`. The job copies `g6-alerts`: `--no-deps`, an explicit service list, `step_wait_for_services` (which waits for `ingest_job` to be loaded; the gate launches only `ingest_job`, so `g6-alerts`' extra `file_ingest_job` wait is not needed), `always()` summary, logs on failure, `down -v`.

**How it reaches the forwarder (the stop condition).** The gate process starts `Forwarder` (a thread, `0.0.0.0:13306`, `SRC11_FORWARDER_PORT`) and finds its own address with `src17.own_address("lakehouse-api")` (the UDP route probe of `SRC-17`, imported, not copied). The connector's host is that address and port. The API's address check and Dagster's dial guard both accept it through the gate-only `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS` and `INGEST_ALLOW_INTERNAL_HOSTS` that `ops/g6/docker-compose.g6.override.yml` already sets for every g6 fixture, so no new opt-in and no product change. "OFF" closes the listening socket (the port refuses at once, which a driver reports as MySQL error 2003, class `unreachable`); "ON" binds it again.

**Steps, as built (each prints `[g6-ops] SRC-11 ...`; a failure is `OpsGateFailure` with the step in the message).**

- a. MySQL connector via the forwarder (OFF); run; polls the run log until an attempt `failed` / `unreachable` with a `waitSeconds`; forwarder ON; Dagster SUCCESS; log has >= 2 attempts, first failed, last succeeded; Bronze has 3 rows; the run record ends `succeeded` with >= 2 attempts. Prints the attempts.
- b. PostgreSQL connector (`postgres:5432`, user `lakehouse`) with a wrong password; Dagster FAILURE; exactly one attempt, `sign_in_refused`, no wait.
- f. `?format=text` of b's run: 200, `text/plain`, has `attempt 1`, `the source refused the sign-in` and the run id; none of the password, `password authentication failed`, `psycopg2`, `operationalerror`, `traceback`.
- c. MySQL connector with a leap-day cron (`0 2 29 2 *`, so the stack's real schedule sensor never launches it; the due window is `2028-02-29T01:59:00Z` to `02:00:00Z`), in the due list; a `connector_disabled` rule for it to the gate's receiver; one good load (2 rows); password made wrong; 3 failing runs; polls the connector until `pausedAt`; `pausedReason == "Paused after 3 failed runs in a row"`; the webhook arrives and names the connector and "3 failed runs"; the due list leaves it out and the unfiltered list carries `paused: true`.
- d. Password made right again; run launched through Dagster `launchRun` (the API mutation shape, no tag); the run record ends `paused`, `endReason connector_paused`, trigger `other`, 0 rows; Bronze row count unchanged (the table is `append`, so a load would have doubled it); the webhook count is still exactly 1.
- c (second half). `POST .../resume` -> `resumed: true`; `pausedAt` null and `failureStreak` 0; back in the due list; a second resume answers 409; the next run succeeds and Bronze gains rows.
- e. MySQL table with `secret_note`; first load (append) carries it; a row is added; `PUT ingest-spec` with `excludedColumns: ["secret_note"]`; the schema-changes list has it in `inactiveColumns` with `reason: "excluded"`; second load: Bronze grows, the count of rows with a `secret_note` value is still the first load's 2; nothing pending, not paused, the mark still there.
- g. For every run the gate launched through `POST .../ingest/run`, `GET .../runs` has it with `trigger: "user"`.

**API calls, traced against the Rust** (`rust/crates/lakehouse-api/src/`; permission is for the bootstrap administrator's session unless said):

| Call in the gate | Route registered | Policy | Handler | Fields and statuses used |
| --- | --- | --- | --- | --- |
| `POST /api/connectors` (`matrix._create_connector`) | `routes/mod.rs:542` | `policy.rs:522` `connector:manage` | `routes/connectors.rs:699` | body as the matrix gate sends it; 200 (`ok`); `id`, `credential.primary` |
| `PUT /api/connectors/{id}/ingest-spec` | `routes/mod.rs:491` | `policy.rs:566` `connector:manage` | `routes/connectors.rs:2956`, body `IngestSpecBody` `:2764` | `adapter`, `ingestMode`, `dial`, `sourceObjects[].{name,target,loadMode,excludedColumns}`, `scheduleCron`; 200; 422/400 for `excludedColumns` (not hit) |
| `POST /api/connectors/{id}/ingest/run` | `routes/mod.rs:495` | `policy.rs:573` `connector:manage` | `routes/connectors.rs:3245` -> `launch_ingest` `:3264`+ | no body; 200 `{"runId"}`; 409 paused/active (the gate waits for each run to end first) |
| `GET /api/connectors/{id}` | `routes/mod.rs:461` | `policy.rs:523` `connector:manage` | `routes/connectors.rs:286` | `pausedAt`, `pausedReason`, `failureStreak` (`lakehouse-store/src/connectors.rs:113-123`, camelCase) |
| `GET /api/connectors/{id}/runs` | `routes/mod.rs:505` | `policy.rs:592` `connector:manage` | `routes/connector_runs.rs:102` | `{"runs":[{runId,trigger,status,endReason,rowsLoaded,attempts,...}]}` |
| `GET /api/connectors/{id}/runs/{runId}/log` (JSON, 404 until the start report arrives) | `routes/mod.rs:507` | `policy.rs:593` | `routes/connector_runs.rs:144` | `attempts[].{table,attempt,result,errorClass,waitSeconds,rowsLoaded}` |
| same with `?format=text` | same | same | same, text branch | `200`, `Content-Type: text/plain; charset=utf-8`; heading `run <id> (<trigger>), <status>`, lines `table T, attempt N: failed, <phrase>` (`lakehouse-store/src/connector_run.rs:451-460, 812-827`) |
| `POST /api/connectors/{id}/resume` | `routes/mod.rs:511` | `policy.rs:594` | `routes/connector_runs.rs:200` | no body; 200 `{"resumed": true}`; 409 when not paused |
| `GET /api/connectors/{id}/schema-changes` (`matrix._schema_changes`) | `routes/mod.rs:517` | `policy.rs:607` | `routes/schema_changes.rs:634` | `pending`, `inactiveColumns[].{objectName,columnName,reason}` |
| `GET /api/connectors/ingestible[?dueAfter&dueUntil]` with `Authorization: Bearer $INGEST_SERVICE_TOKEN` | `routes/mod.rs:553` | `policy.rs:515` `ingest:read` | `routes/connectors.rs:202`, `is_due` `:243` | both bounds RFC 3339; entries `id`, `paused` (`lakehouse-store/src/connectors.rs:2065`) |
| `POST /api/alerts` (`src17.create_rule`), `DELETE /api/alerts?id=` (`src17.delete_rule`) | `routes/mod.rs:800` | `policy.rs:305, 307` `alert:write` | `routes/alerts.rs:314` | `type: connector_disabled`, `channel: webhook`, `target`, `connector`; 200 `rule.id`; the rule scope check needs an administrator (the bootstrap account) for `*`, not for one connector |
| Dagster `launchRun` (GraphQL, not the API) | n/a | n/a | `rust/crates/lakehouse-dagster/src/lib.rs:1198-1241` (the same selector `__repository__` / `dispar_orchestrate.definitions` / `ingest_job` and `RunConfigData` object) | `LaunchRunSuccess.run.runId` |

**Commands run (all from `/home/hv/lakehouse-sec27`) and outcomes.**

- `~/.cache/rantai-dagster-venv/bin/python -m py_compile ops/g6/src11_connector_operations_test.py`: no output, exit 0 (the `__pycache__` it wrote was removed).
- A local, stackless run of the gate's helpers (a scratch script outside the repo, `g6_ingest_matrix_test` replaced by an empty stub): `Forwarder` against a local server that answers `hello`: OFF -> `ConnectionRefusedError`, ON -> `b'hello'` twice, OFF again -> refused, ON again -> `hello`, `off()` twice is harmless; `src17.Receiver` on port 18099 received a POST, `wait_for` and `arrivals` returned it (1), `src17.check_webhook_body` accepted a body naming the connector and "3 failed runs"; `attempts_phrase`, `check_log_text` (accepts an honest log; rejects the password, `psycopg2`, `Traceback`), `poll` (times out with the step text) and `auto_disable_after` behaved as written. This ran no HTTP call to the API, Dagster or ClickHouse.
- `bash scripts/ci/tests/test_required_gate.sh`: `=== Results: 35 passed, 0 failed ===` (it includes the check that every job in `ci.yml` is in `ci-required` needs and has a result variable in `required_gate.sh`).
- `python3 -c "import yaml; yaml.safe_load(open(...))"` for `.github/workflows/ci.yml`, `docker-compose.yml`, `ops/g6/docker-compose.g6.override.yml`: ok for all three; `g6-ops` is in `ci-required` `needs`, `G6_OPS_RESULT` is in its env; the new keys were counted once each in `docker-compose.yml`, and the runner service key once in the override.
- `python3 ops/lint/check_intra_package_imports.py`: OK. `python3 ops/lint/check_bare_iceberg_count.py`: OK. `python3 ops/lint/check_compose_init_readiness.py`: exit 0. `python3 ops/lint/check_connector_gate_coverage.py`: OK (11 types covered, 0 waived).

**Not verified.** `docker compose config` and `up` (no docker here): the new compose blocks were re-read for unique keys, `$` escaping (none needed) and `${X:-}` versus `${X:?}` (the two new runner variables that every g6 job would otherwise have to define are `:-`; `MYSQL_ROOT_PASSWORD` and `CONNECTOR_MYSQL_PASSWORD` are `:?` as in the rest of the file). The whole gate against a real stack, including every endpoint answer above (read from the Rust, not run), the Dagster 1.x behaviour that `RetryRequested.seconds_to_wait` is honoured by the real executor (the Python handoff noted this too), and the `ExecutionTag` input name (the tag path is exercised by step g: a wrong name would show as a failed `POST ingest/run` in every step, not only g).

**Job duration estimate.** The stack bring-up with `--build` costs what `g6-alerts` costs (the API image build dominates). The gate itself about 9 to 11 minutes: a about 1.5 min (5 s wait after the first attempt, run start-up), b + f about 0.5 min, c about 4 min (a good load, 3 failing runs of about 20 s, then up to 30 s for the run sensor to report the last failure and up to 90 s of waiting for the webhook), d about 0.5 min, resume + next run about 0.7 min, e about 1.5 min, g under 0.5 min. `timeout-minutes: 30`.

**What I think is most likely to fail in CI.**

1. The sensor path. Auto-disable counts a failed run when the run sensor reports it (up to 30 s per tick); if `PIPELINE_RUN_TOKEN` or the daemon is not effective, step c times out at "the connector to be paused" after 150 s (the `.env` has the token and the daemon is in the service list, as in `g6-alerts`).
2. Step a's timing: attempt 2 starts 5 s after attempt 1 failed. The gate polls the log every second, so it switches the forwarder ON well within that; if the first poll lands late, attempt 3 (10 s later) still succeeds, but a very slow poll could burn all three retries and fail the run (the failure would say so in the step).
3. Step b: that PostgreSQL's server really refuses the password (`pg_hba` for the Docker network is `scram-sha-256` by default), that the type `PostgreSQL` takes a `file` credential with a `password` slot as MySQL does, and that the batch Postgres path reports through the same `ingest_source_object` classification (`dlt_pipeline` path) the Python handoff tested only with hand-built errors.
4. Step c's leap-day cron: `croner` must accept `0 2 29 2 *` and find the 2028 occurrence from `2028-02-29T01:59:00Z`; if it did not, "the connector is not in the due list before it is paused" is the failure.
5. Step d: the run record `trigger` for a Dagster-launched run is `other` by the Python handoff (no tag); if the daemon or the run launcher adds a tag the assertion names it. The end report of a paused run is the orchestrator's (`paused` / `connector_paused`).
6. Step e: the second `append` load of a table whose Iceberg table still has `secret_note` but the incoming batch does not; if the sink refuses a batch with fewer columns the run fails and the step says so (a product finding, not a gate defect).
7. `INGEST_SERVICE_TOKEN` inside the runner: only for the due list; empty -> the gate's own message.

## Review

*(planner)*
