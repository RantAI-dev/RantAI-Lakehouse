# Iceberg sync per table: on every update, on a schedule, or by hand

| | |
| --- | --- |
| Module | Build |
| Backlog | `DATA-66` |
| Kind | Feature |
| Spec | `docs/core/specs/data-66.md` (every *(proposed)* number is listed under Decisions) |
| Status | Building |
| Priority | P1 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-11-27-data-66-iceberg-sync.md` |
| Started | 2026-11-27 |
| Shipped | |
| Evidence | |

## Problem

A Silver or Gold table switched on for publishing is copied to Iceberg after
*any* authored pipeline succeeds (every switched-on table, one after another,
in one run) and every night at 04:00. A change made outside a pipeline waits
for the night. Nobody can choose how closely the Iceberg copy follows
ClickHouse, or keep a table from being copied until they say so.

## What the user can do when this is done

1. On a Silver or Gold table's "Publish in open format (Iceberg)" card, with
   publishing on, choose **Sync**: "On every update", "On a schedule" or
   "Manual only".
2. With "On every update": run the table's pipeline and see the Iceberg tables
   (`<zone>.<t>` and `<zone>.<t>_latest`) follow without pressing anything; a
   change made to the table outside a pipeline is followed too.
3. With "On a schedule": pick hourly (at a minute) or daily (at a time) and a
   time zone; see the next sync time on the card.
4. With "Manual only": publish only with "Publish now".
5. See a failed sync as "Last publish failed" with its reason, while the
   pipeline and the dashboards carry on with the new data.
6. See each table's sync setting on the "All published marts" page.

A user without `gold:export` sees the setting but cannot change it.

## Not included

- Streaming row by row into Iceberg.
- A sync setting on the pipeline page.
- Cron expressions.
- Storing Silver or Gold in Iceberg first (`DATA-3`, killed 2026-11-27).

## Asking the assistant

No; publishing is not one of the assistant's dashboard tools.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | ClickHouse stays the primary store for Silver and Gold; `DATA-3` killed | — | Product owner, 2026-11-27 |
| 2 | Three sync modes per `(zone, table)`, stored in Postgres, shown and changed on the card, in the API | — | Product owner, 2026-11-27 |
| 3 | "On every update" covers pipeline writes and changes made outside a pipeline; a failed sync never fails the ClickHouse write | — | Product owner, 2026-11-27 |
| 4 | Mode of a table switched on **before** this release | "On every update" (the nearest to today's "after each pipeline"); today's nightly 04:00 run for console tables ends | Planner *(proposed)* |
| 5 | Mode when a table is switched on | "On every update" | Planner *(proposed)* |
| 6 | Schedule when "On a schedule" is first picked | Daily at 04:00 in the browser's time zone (today's nightly time) | Planner *(proposed)* |
| 7 | Schedule choices | Hourly at :MM, or daily at HH:MM, with an IANA time zone. No cron | Planner *(proposed)* |
| 8 | Watcher interval for changes outside a pipeline | 15 seconds | Planner *(proposed)* |
| 9 | Retry after a failed sync ("On every update") | Every 5 minutes while the newest run failed | Planner *(proposed)* |
| 10 | Targets | Pipeline write to Iceberg p95 < 60 s; outside change p95 < 90 s; 100,000-row table on the compose stack | Planner *(proposed)*; measured in `docs/plans/DATA-66-RESULT.md` |
| 11 | Who may change the mode | Holders of `gold:export`, as for the on/off switch | Planner (unchanged from `DATA-1`) |
| 12 | Tables named in the operator's `GOLD_EXPORT_MARTS` | Still exported nightly at 04:00, **unless** the table has a console setting, which then decides | Planner |
| 13 | "Publish now" | Works in every mode while publishing is on | Planner |

## Limits to tell a customer

- A sync is a full copy of the table (plus the latest-only table), so a large
  table takes as long to sync as it takes to copy. Several changes made while
  a sync runs are covered by the next sync, not one sync each.
- A background merge in ClickHouse can start a sync that then finds nothing
  changed and skips (one content check, no copy).
- "On every update" is not instantaneous: the measured delay is in
  `docs/plans/DATA-66-RESULT.md`, and is *not measured* until that file says
  otherwise.
- Times on a schedule are wall-clock times in the chosen time zone; a daily
  time that does not exist on a daylight-saving change day runs at the next
  valid time (the same rule as report subscriptions).
- Automatic syncs need the operator to set the export run token. Without it,
  only "Publish now" works, as today.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-66-AC1` | Set a Gold table to "On every update"; run its pipeline with new rows | The card says "Up to date"; an outside tool reads the new rows in `gold.<t>` and `gold.<t>_latest`; nothing pressed | Not run |
| `DATA-66-AC2` | (operator) Change the table in ClickHouse outside any pipeline | Synced without pressing anything | Not run |
| `DATA-66-AC3` | Set a table to "On a schedule", hourly; run its pipeline | Nothing published until the slot; then published once | Not run |
| `DATA-66-AC4` | Set a table to "Manual only"; run its pipeline; wait a minute | Nothing published until "Publish now" | Not run |
| `DATA-66-AC5` | (operator) Stop the Iceberg catalog; run a pipeline on an "On every update" table; start the catalog | Pipeline succeeds; dashboard shows the rows; card says "Last publish failed"; synced within 5 minutes after the catalog is back | Not run |
| `DATA-66-AC6` | As a user without `gold:export`, look at the setting | Visible, not changeable, with an explanation | Not run |
| `DATA-66-AC7` | AC1 and AC4 on a Silver table | Same results in `silver.<t>` | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
