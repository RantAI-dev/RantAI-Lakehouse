# A gate test for every listed connector

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-9` |
| Spec | `docs/core/specs/src-9.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec". One *(proposed)* number, decision 1 |
| Plan | `docs/superpowers/plans/2026-10-10-src-9-connector-gates.md` |

## Problem

From the spec: Oracle, SFTP, MySQL CDC and SQL Server CDC have never been
tested by a gate, and the upload gate does not run in CI. Checked in the code
on 2026-10-10, after `main` was merged in:

- Thirteen connector types are listed as supported. Seven have a gate that
  loads data (`ops/g6`: PostgreSQL, MySQL, SQL Server, object storage, REST,
  MongoDB, Kafka) and one has a change-capture gate (`ops/g4`: PostgreSQL
  CDC). Five have none: Oracle, SFTP, MariaDB, MySQL CDC, SQL Server CDC. The
  spec does not name MariaDB.
- The gates that exist load once. Only object storage is tested for
  "connect", nothing is tested for "discover", and only one MySQL table is
  loaded twice (added by `SRC-6` and `SRC-8` on this branch).
- `ops/g9/upload_test.py` exists and no CI job or compose service runs it.
- Nothing stops a new supported type from being listed without a gate.

## What the user can do when this is done

This item has no screen. "The user" is whoever reads a pull request's checks.

| Spec row | Spec target | What they can do |
| --- | --- | --- |
| Gate coverage | 100% of connector types shown as supported have a gate: connect, discover, load, re-load | 1. See, in one table in the CI run, every supported connector type with a result for each of the four steps. 2. See CI fail when a type is listed as supported and has no gate. |
| File-upload gate g9 | Runs in CI on every PR touching ingestion | 3. See the upload gate run on every pull request that touches the API, the orchestrator, the gates or compose. |
| Gate runtime | The connector gates together finish within 20 minutes in CI *(proposed)* | 4. Read the measured time of each connector gate job in a result document. |

## Not included

From the spec: anything not in the Target table. In particular:

- Making "connect" or "discover" work for a type where the product answers
  "cannot be tested from the console" or "discovery not supported". The gate
  checks that honest answer; `SRC-13` and `SRC-5` change the product.
- Google Sheets: listed as not supported since `SRC-6`.
- Row-filter tests: they stay in `VER-1`.

## Asking the assistant

From the spec: not part of the dashboards assistant.

## Decisions

Decisions 1 to 6 signed by the product owner on 2026-10-11 ("run it", on the planner's recommendation). Decision 7 was settled on 2026-10-10: the Phase 1 branch was pushed as draft PR #101 and the gates are proven in CI.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec *(proposed)*: the connector gates finish within 20 minutes. They cannot all run in one job in that time once five source systems are added. | The connector gates run as parallel jobs, and the slowest one finishes within 20 minutes. Measured from CI and written into `docs/plans/SRC-9-RESULT.md`. | 2026-10-11 |
| 2 | Spec "connect, discover, load, re-load" for types where the product itself says no. Oracle, MongoDB, Kafka and SFTP cannot be tested from the console; only PostgreSQL, MySQL, MariaDB and SQL Server have discovery; change-capture types have no "load". | Each step is checked for what the product promises today: a real result where it exists, the exact honest refusal where it does not. For change capture, "load" is the first copy and "re-load" is a later insert, update and delete arriving, as `ops/g4` does for PostgreSQL. | 2026-10-11 |
| 3 | Spec "CI shows one passing gate per connector type". | One table in the run's summary with a row per type and a column per step, written by the gates. Not one CI job per type: each job would start the whole stack. | 2026-10-11 |
| 4 | Beyond the spec: MariaDB is listed as supported and has no gate. | Add it. | 2026-10-11 |
| 5 | Beyond the spec: an Oracle gate needs an Oracle database in CI. | Use Oracle's free container image (Oracle Database Free), pulled in CI only, in a job of its own because it is large and slow to start. If its licence terms are not acceptable, Oracle is the one type listed as supported without a gate, and the picker must say so. | 2026-10-11 |
| 6 | Beyond the spec: keep coverage at 100%. | A check in CI compares the types listed as supported with the gates' own list and fails on a difference. | 2026-10-11 |
| 7 | Beyond the spec: none of this can be proven on the build machine. A gate starts the whole stack in containers, which that machine cannot do safely, and the branch is not pushed. | `SRC-9` is built on a branch of its own after PR #101 merges, pushed as a draft pull request from its first commit, so CI runs each gate while it is written. | 2026-10-11 |

## Limits to tell a customer

- A gate proves a connector type against one version of one source system,
  in a container, with small data.
- For Oracle, MongoDB, Kafka and SFTP the gate proves the load, not a
  connection test, because the product has none for them yet.

## Acceptance checklist

Run on the repository's CI. Mark each Pass, Fail, or Not run with the reason.
Rows 1 to 4 are the spec's checklist, in its order.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec: every Target row above works as written | Rows 5 to 8 pass | |
| 2 | Spec: open a pull request's CI run | One passing gate per connector type in the picker: thirteen rows, four steps each | |
| 3 | Spec: on a test branch, break one connector's code (for example the SFTP adapter) | That type's row turns red and CI fails; the other rows stay green | |
| 4 | Spec: the gates sign in as a user; a step that fails prints an honest message | No gate passes by skipping a step; a skipped step is shown as skipped with its reason | |
| 5 | On a test branch, list a new type as supported without a gate | The coverage check fails and names the type | |
| 6 | Open a pull request that changes only `rust/` | The upload gate runs | |
| 7 | Open a pull request that changes only documentation | No gate runs | |
| 8 | Read `docs/plans/SRC-9-RESULT.md` | Each connector gate job's measured time, the slowest within 20 minutes | |

**Accepted by:** __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` item moved to Done; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
