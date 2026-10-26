# Connector operations

| | |
| --- | --- |
| Module | Data (Sources) |
| Backlog | `SRC-11` |
| Kind | Feature |
| Spec | `docs/core/specs/src-11.md`. This page follows it row by row; anything the spec does not say is under "Beyond the spec". Three *(proposed)* numbers: decisions 1, 5 and 7 |
| Status | Released (accepted 2026-10-23). Built in #144 (the one pull request for the rest of Phase 1). Decisions signed 2026-10-14; built and proven by its gate on 2026-10-15. Product owner's browser check on the QA instance passed on 2026-10-16 (the screens only: runs, loads, secret-manager references and OAuth sign-in were not reachable there and are proven by the CI gates) |
| Plan | `docs/superpowers/plans/2026-10-14-src-11-connector-operations.md` |

## Problem

From the spec: ingest jobs have no checked retry policy, no readable log per
run, and column choice is unverified. Checked in the code on 2026-10-13
(`main` at `432a3c9`), row by row:

- **Retries.** A table that fails is tried again twice inside the same run,
  after about 30 and 60 seconds. A failed run is never started again. A
  wrong password, a refused address and an unsupported type fail at once;
  a wrong password for PostgreSQL is still retried (`SRC-19`), and so is
  every failure of a Kafka connector.
- **Auto-disable.** None. A connector counts its failed runs in a row and is
  shown as unhealthy from 3. The alert "connector disabled" can be set up as
  a rule and nothing raises it. A connector can be paused only by the
  schema-change policy (`SRC-8`); no one can pause or resume one by hand.
- **Run log.** One line per table and run: rows, times, status, and for a
  failure the kind of error, never the source's text. There is no line per
  attempt and no run number on a line, so the console matches lines to runs
  by time. The step-by-step log exists only on the Pipelines page, for a
  user with pipeline access. Nothing is ever deleted.
- **Run history.** The last 20 runs, found by searching the newest 100 runs
  of all connectors together, so with many connectors a connector's list is
  shorter than 20 or empty. Status and times only; rows and duration are
  added up in the browser.
- **Column selection.** Not possible. The picker shows tables and a column
  count.

## What the user can do when this is done

| Spec row | Spec target | What they can do |
| --- | --- | --- |
| Retries | Up to 3 retries per run with backoff 1, 5 and 15 minutes *(proposed)*; transient errors only | 1. Leave a connector alone when its source is briefly down: the table is tried again after 1, 5 and 15 minutes and the run succeeds without anyone acting. 2. See a failure that cannot pass by itself (wrong password, refused address, unsupported type) fail at once, with the reason. |
| Auto-disable | A connector failing 30 runs in a row is paused and an alert sent | 3. Find a connector that failed 30 runs in a row paused, with the reason on its page. 4. Receive the "connector disabled" alert. 5. Resume it with one button once the cause is fixed. |
| Run log | A log per run and per attempt, kept 30 days *(proposed)*, viewable and downloadable | 6. Open a run and read what happened, attempt by attempt and table by table. 7. Download that log as a text file. |
| Run history | A timeline of the last 50 runs with status, rows, duration *(proposed)* | 8. See a connector's last 50 runs with status, rows loaded, duration, how the run was started and how many attempts it took. |
| Column selection | Choose columns per table, not only tables | 9. Open a table in the picker and untick columns; the next load leaves them out. |

## Not included

From the spec: anything not in the Target table. In particular:

- A retry setting per connector. One policy for every connector.
- Pausing a healthy connector by hand (decision 4 adds Resume only).
- Running a failed run again by hand from the connector page: "Run now"
  already does it.
- Change-capture (CDC) connectors: they stream and have no runs (`SRC-4`).
- Filling a column that was left out, once it is ticked again, for rows
  already loaded (backfill).

## Asking the assistant

From the spec: not part of the dashboards assistant (`AI-16`).

## Decisions

Signed by the product owner on 2026-10-14 ("setuju semua", on the planner's recommendation). Rows 1, 2, 3, 5, 7 and 9 settle how a spec row is met; the
others are beyond the spec. The planner's recommendation is the Default.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Spec *(proposed)*: up to 3 retries per run, after 1, 5 and 15 minutes. | Accept the numbers. A retry repeats only the table that failed, not the tables of the run that already loaded, as today. It replaces today's two retries after about 30 and 60 seconds. A table that keeps failing therefore holds its run open for about 21 minutes before the run fails. || 2026-10-14 |
| 2 | Spec "transient errors only": which failures are retried. | Retried: the source or the network cannot be reached, a timeout, a source that answers "busy" or a server error, the product's own API not reachable. Not retried, failing at once with the reason: a refused sign-in, a refused address, an unsupported type or load mode, a setting the product rejects, a table waiting for a schema decision. This item also makes a PostgreSQL wrong password fail at once (`SRC-19`) and gives Kafka connectors the same rule. || 2026-10-14 |
| 3 | Spec "failing 30 runs in a row is paused and an alert sent". | A run counts once, after its retries are used up. At 30 the connector is paused with the reason "Paused after 30 failed runs in a row" and the existing "connector disabled" alert is raised, once. A cancelled run does not count; a successful run resets the count, as today. The schedule skips a paused connector and "Run now" refuses it, as for a schema-change pause. || 2026-10-14 |
| 4 | Beyond the spec: how a connector paused this way starts again. | A **Resume** button on the connector's page, for a user with `connector:manage`. It lifts the pause and sets the count back to 0, so the connector gets 30 new tries. It is written to the audit trail. A pause set by the schema-change policy is not lifted by it; that one is lifted by approving the change. || 2026-10-14 |
| 5 | Spec *(proposed)*: a log per run and per attempt, kept 30 days, viewable and downloadable. What the log holds. | The product's own lines: run started and by what, each table's attempt with its number, start, end, rows and result, the kind of error, the wait before the next attempt, why the run ended. Never the source's or the driver's error text (principle 4). Kept 30 days, then removed by a daily job. Shown on the connector's page under the run; downloadable as a text file. The raw orchestrator log stays on the Pipelines page for users with pipeline access. || 2026-10-14 |
| 6 | Beyond the spec: what else the 30 days remove. | Only the log lines and attempts. The per-table results and the "last loaded" facts of a table are kept as today. || 2026-10-14 |
| 7 | Spec *(proposed)*: a timeline of the last 50 runs with status, rows, duration. | Accept 50. Each run is recorded by the product when it starts and ends, with its run number on every line, so the list is exact for every connector however many there are, and the console no longer matches lines to runs by time. Also shown: how the run was started (schedule, a user, refresh or clear) and the number of attempts. || 2026-10-14 |
| 8 | Beyond the spec: who sees the history and the log. | A user with `connector:manage` on a connector of their own tenant, as for the rest of the connector page. || 2026-10-14 |
| 9 | Spec "choose columns per table": for which sources. | PostgreSQL, MySQL, MariaDB and SQL Server, the sources whose columns the product can list before loading. For the others (Oracle, files, REST, MongoDB, Kafka, SFTP) the picker says column choice is not available for this source type. Oracle follows when it can list its tables (`SRC-13`). || 2026-10-14 |
| 10 | Beyond the spec: what leaving a column out does to the table. | The next load does not read the column from the source. In a table that is replaced on every load the column is gone after that load. In a table that keeps its rows the column stays, with no new values, and is marked inactive on the connector's page and in the table's column list, as a removed column is (`SRC-8`). A column left out is not a schema change: nothing waits and no alert is sent. || 2026-10-14 |
| 11 | Beyond the spec: columns that cannot be left out. | The table's key columns (when it deduplicates or keeps history, `SRC-10`) and its cursor column (when it loads incrementally). The picker shows them ticked and locked, with the reason. || 2026-10-14 |
| 12 | Beyond the spec: a column that appears at the source later. | It follows the connector's schema-change policy, as today: loaded when the policy applies it, waiting when the policy asks first. It is never left out silently. || 2026-10-14 |
| 13 | Beyond the spec: a paused connector must not load. Today a run that was already queued, or one started from the orchestrator's own screen, still loads a paused connector (`SRC-18`, first half). | Closed here: the orchestrator checks the pause before it loads and ends the run as "paused", loading nothing. Without it, pausing after 30 failures does not stop every load. || 2026-10-14 |
| 14 | Beyond the spec: where the gate steps run. The SQL connector gate job takes 18m52s to 19m54s against a 20-minute target (`SRC-9` decision 1). | The gate steps of this item run in a new parallel job, so no existing job grows. || 2026-10-14 |

## Limits to tell a customer

- Retries cover a source that is briefly unreachable. A table that fails
  for 21 minutes fails its run.
- The log is the product's account of the run. It does not show the
  source's own error text.
- Column choice works for PostgreSQL, MySQL, MariaDB and SQL Server.
- A column left out and ticked again is filled from the next load; older
  rows stay empty in it.
- Logs older than 30 days are gone.

## Acceptance checklist

Run on a real deployment by the product owner. Mark each Pass, Fail, or Not
run with the reason. Rows 1 to 4 are the spec's checklist, in its order.

| # | Do this | Expect | Result |
| --- | --- | --- | --- |
| 1 | Spec `SRC-11-AC1`: stop the source database, start a run, start the database again within a few minutes | The run succeeds on a retry without anyone acting | Pass 2026-10-23 (product owner) |
| 2 | Spec `SRC-11-AC2`: open that run's log | Each attempt is shown, with its number, times and result | Pass 2026-10-23 (product owner) |
| 3 | Spec `SRC-11-AC3`: untick a column of a table and run | The next load does not carry the column | Pass 2026-10-23 (product owner) |
| 4 | Spec `SRC-11-AC4`: as a user without `connector:manage`, try to resume a connector, open a log, change the columns; and make a load fail | Refused each time; the failure shows an honest message with no source error text | Pass 2026-10-23 (product owner) |
| 5 | Give a connector a wrong password and run | It fails at once, with no retry, for every SQL source | Pass 2026-10-23 (product owner) |
| 6 | Let a connector fail 30 runs in a row (a test setting may lower the number; say so in the result) | It is paused with the reason; the "connector disabled" alert arrives once; the schedule no longer starts it | Pass 2026-10-23 (product owner) |
| 7 | Press Resume | The pause is lifted, the count is 0, the next run starts | Pass 2026-10-23 (product owner) |
| 8 | Download a run's log | A text file with the same lines as the screen | Pass 2026-10-23 (product owner) |
| 9 | Open the history of a connector with more than 50 runs, on an install with several connectors | 50 runs, each with status, rows, duration, start and attempts | Pass 2026-10-23 (product owner) |
| 10 | Try to untick a key column or the cursor column | It is locked, with the reason | Pass 2026-10-23 (product owner) |
| 11 | Start a run of a paused connector from the orchestrator's own screen | The run ends as "paused" and loads nothing | Pass 2026-10-23 (product owner) |

**Accepted by:** product owner (held in the base) **Date:** 2026-10-23 **Build:** main 0e5498d2

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [x] `BACKLOG.md` item moved to Released; follow-ups added
- [ ] `CHANGELOG.md` entry a customer can read
