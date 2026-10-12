# `SRC-9` Test every advertised connector end to end — Implementation Plan

**Status:** ready to build once PR #101 is merged. Decisions D1–D7 are
signed (2026-10-10 and 2026-10-11). It is built on its own branch off `main`,
opened as a draft pull request from the first commit so CI proves each gate.
Written 2026-10-10 by the planner (Claude Opus) for a developer agent, under
the role split in `AGENTS.md`.

**Spec:** `docs/core/specs/src-9.md`. Backlog `SRC-9`.
**Feature page:** `docs/core/features/connector-gates.md`. Decision numbers
below (D1–D7) are its rows.

**Base and branch:** `feat/phase-1-connector-reliability` at `36f2a2a`
(`SRC-6`, `SRC-7`, `SRC-8`, with `main` `a78ad62` merged in).

**Why.** Five supported connector types have no gate, the gates that exist
prove one step of four, and the upload gate never runs.

---

## 1. What exists today (anchors verified at `36f2a2a`)

| # | Spec row | Code | Verdict |
| --- | --- | --- | --- |
| F1 | Gate coverage: some types never gate-tested | Supported types: `rust/migrations/0035_connector_type.sql`, `0043_ingest_tier2_adapters.sql`, `0062_connector_type_unsupported_reason.sql` give thirteen. `ops/g6/g6_ingest_matrix_test.py` (`step_ingest_matrix`, `:599`) registers MySQL, SQL Server, PostgreSQL, object storage, REST, MongoDB, Kafka. `ops/g4/g4_test.py` covers PostgreSQL CDC. | No gate: Oracle, SFTP, MariaDB, MySQL CDC, SQL Server CDC. |
| F2 | "connect, discover, load, re-load" | g6 calls `POST …/test` for object storage only (`SRC-6`), never `POST …/discover`, and runs each connector once except the two tables `SRC-7` and `SRC-8` added. | One step of four for most types. |
| F3 | g9 exists, not in CI | `ops/g9/upload_test.py`; no job in `.github/workflows/ci.yml` and no service in `docker-compose.yml` names it. | Confirmed. |
| F4 | Gate runtime | Measured on PR #101 and on `main`, 2026-10-10: bringing the stack up takes 11m38s to 13m47s of the g6 job; eight g6 runs on `main` took 13m02s to 18m37s; with the `SRC-7` and `SRC-8` steps the job took 16m41s to 20m02s. Its limit was raised from 20 to 25 minutes in PR #101 (`f9d488f`). | One job cannot hold five more source systems inside 20 minutes. D1. |
| F5 | *Not in the spec* | `scripts/ci/detect_change_scope.sh` (from `main`) decides which jobs a pull request runs: g6 runs for a change under `rust/`, `dagster/`, `ops/`, `scripts/`, `docker-compose.yml` or `.env.example`; a documentation-only change skips it. | "Every PR touching ingestion" already has a mechanism; g9 joins it. |
| F6 | *Not in the spec* | Change capture runs one Debezium Server process per connector (`docker-compose.yml:2565`, PostgreSQL only). `routes/connectors.rs` (`debezium_properties`) renders properties for MySQL and SQL Server that nothing has ever started. | The two CDC gates each need a source with change capture switched on and a Debezium Server of their own. |
| F7 | *Not in the spec* | Nothing compares the supported types with what the gates cover. | Coverage can fall below 100% silently. D6. |

## 2. Decisions already made by the planner

- **One registry of what is gated.** `ops/gates/coverage.json`: for each
  supported type, the gate that covers it and, per step, `real` or the
  refusal the product is expected to give (D2). The gates read it and the
  coverage check reads it; neither keeps a second list.
- **The summary table is written by the gates**, from what they ran, to the
  job summary. A step that did not run is `skipped` with its reason, never
  blank and never green.
- **Jobs.** `g6-ingest` keeps the batch sources and gains MariaDB and SFTP.
  Oracle gets its own job. `g4-cdc` gains MySQL CDC and SQL Server CDC as two
  more sources if the measured time allows; otherwise they get a job of their
  own. `g9-upload` is new. All are behind the scope gate and in
  `ci-required`.
- **No product code changes** unless a gate finds a defect. A defect is
  reported to the planner and fixed in its own commit with its own finding
  id; a gate is never bent around it.

## 2a. Spec row to tasks

| Spec Target row | Tasks |
| --- | --- |
| Gate coverage, four steps, every supported type | 1, 2, 3, 4, 5, 6, 8 |
| g9 in CI on every PR touching ingestion | 7 |
| Connector gates within 20 minutes | 9 (measure), 10 (write up) |

## 3. Tasks

One task per commit. Every task that adds a compose service is proven by a
real `up` of the affected profile from a clean project (rule 8); where that
runs is decision 7.

0. **G3's start-up race (found on PR #101).** `ops/g3/g3_test.py` triggers
   `bronze_ingest_job` before the code location has finished loading and gets
   `422 PipelineNotFoundError` (PR #101, run of 2026-10-10; a re-run passed).
   `ops/g6/g6_ingest_matrix_test.py` already waits and retries once for the
   same race (`_post_ingest_run`). Give g3 the same handling, reusing the
   helper rather than copying it, and add the backlog item for it with the
   next free ID when this branch is cut (product owner, 2026-10-11).
1. **Registry and summary.** `ops/gates/coverage.json` and a small shared
   module the gates import to record a step result and to write the summary
   table. No behaviour change to any gate yet.
2. **Four steps for the types g6 already loads.** For each of PostgreSQL,
   MySQL, SQL Server, object storage, REST, MongoDB, Kafka: connect
   (`POST …/test`), discover (`POST …/discover`), load, and a second load that
   asserts the table's rows as the load mode promises. Where the product
   refuses, assert the refusal's fixed shape (`supported: false`), not a
   string that may be reworded.
3. **MariaDB.** A `mariadb-g6` service in the g6 override; the four steps.
4. **SFTP.** An SFTP server in the g6 override with a known host key, a CSV
   fixture, and the four steps. The connector's host-key fingerprint is the
   fixture's own.
5. **Oracle.** Its own profile, service and CI job (D5); the four steps;
   `ORACLE_*` secrets as `${X:?}` in the override, never a default.
6. **MySQL CDC and SQL Server CDC.** A source with change capture on, the
   properties the API renders written where a Debezium Server reads them, and
   the steps of `ops/g4` for PostgreSQL: first copy, then an insert, an update
   and a delete arriving within the budget g4 uses, then deletion of the
   connector leaving nothing behind at the source.
7. **g9 in CI.** A `g9-test-runner` service and a `g9-upload` job, under the
   scope gate with the same condition as g6, in `ci-required`.
8. **Coverage check (D6).** `ops/lint/check_connector_gate_coverage.py`:
   reads the supported types from the migrations and the registry, fails on a
   type with no gate or a gate for a type no longer supported; a job step in
   `repo-lints`; unit-tested with fixtures.
9. **Measure.** From CI runs on the final commit: each connector gate job's
   duration, three runs. If the slowest is over 20 minutes, report it; the
   target is not lowered and the job is not shortened by removing steps.
10. **Docs (planner).** `docs/plans/SRC-9-RESULT.md`, `CHANGELOG.md`,
    `docs/OPERATIONS.md` or the gates' README, the spec's status and its
    "Today" row for MariaDB.

## 4. Build limits on this machine

No docker and no gate can run here. Python lints and unit tests run for real
in `~/.cache/rantai-dagster-venv`. Everything else in this plan is proven
only by CI (decision 7).

## 5. Out of scope

- Any change to what a connector does. Discovery for more types (`SRC-13`);
  a connection test for Oracle, MongoDB, Kafka, SFTP (`SRC-5`).
- Row-filter tests (`VER-1`). Google Sheets.
- Load, soak or version-matrix testing of a source system.

## Handoff

*(developer)*

## Review

*(planner)*
