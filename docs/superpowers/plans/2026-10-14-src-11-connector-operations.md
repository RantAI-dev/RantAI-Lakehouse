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

*(developer)*

## Review

*(planner)*
