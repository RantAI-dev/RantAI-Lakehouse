# Gold stored in Iceberg first, served from a fast copy

| | |
| --- | --- |
| Module | Build |
| Backlog | `DATA-3` |
| Kind | Feature |
| Spec | `docs/core/specs/data-3.md` (every *(proposed)* number is listed under Decisions) |
| Status | Planned. **Blocked by `DATA-4`** and **`DATA-6`** (#612); ADR 0019 awaits the owner's signature |
| Priority | P3 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| ADR | `docs/adr/0019-gold-in-iceberg-first.md` (Proposed) |
| Plan | `docs/superpowers/plans/2026-11-20-data-3-gold-iceberg-first.md` |
| Measurements | `docs/plans/DATA-3-RESULT.md` (written by the plan's T0 gate) |
| Started | |
| Shipped | |
| Evidence | |

Renamed on 2026-11-20 from `gold-open-format-first.md` (no other page linked to it).

## Problem

A Gold table is written to ClickHouse, and an open-format (Iceberg) copy is made afterwards
when publishing is on (ADR 0010, `DATA-1`). The open copy is behind, holds several copies an
outside tool must filter, and is a second system to keep in step.

## What the user can do when this is done

1. When creating a pipeline whose output is a Gold table, choose "Store in open format
   (Iceberg), with a fast copy for dashboards". Today's path stays the default.
2. Run the pipeline: the table is written once, to Iceberg, and its dashboards read a fast
   ClickHouse copy refreshed from it within the copy-lag target.
3. See on the table's page: the open table's name (`gold.<table>`, with a copy button), its
   last write time, and the copy's state: "Copy current", "Copy behind since <time>",
   "Last copy refresh failed" (with the reason), "Not measured" (with the reason) or
   "Never written". Never "current" when it is not.
4. Read the same table from an outside tool through the console's door, with no publish step.
5. Keep existing Gold tables on today's path; they are not moved.

A user without `pipeline:write` cannot create a table on the new path.

## Not included

- Moving existing Gold tables automatically, or switching an existing table (a follow-up).
- Silver on this path.
- Outside reads of a table that has a masking or row-filter policy (refused; decision 7).
- Writing only the changed rows (each run replaces the table).

## Asking the assistant

No change to the dashboards assistant: its tools read the fast copy as they read Gold today.
The assistant's `create_pipeline` tool keeps creating tables on today's path (decision 14);
its schema does not change.

## Decisions

Numbers marked *(proposed)* are the planner's; the owner signs or changes each one before the
build starts. The plan builds to them; a target that cannot be met is reported, not lowered.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Built, after `DATA-4` | Yes | Product owner, 2026-10-14; go-ahead 2026-11-20 |
| 2 | Copy lag | 60 seconds from the Iceberg commit to the copy swap, measured | Product owner, 2026-10-14 |
| 2a | How the lag is measured *(proposed)* | p95 of 20 runs on a reference table of 1,000,000 rows and 20 columns; also measured and recorded at 5,000,000 rows | |
| 2b | Lag above the reference size *(proposed)* | Measured and written here as a limit; not a target | |
| 3 | Dashboard speed | p95 within 10 % of the measured baseline | Product owner, 2026-10-14 |
| 3a | How speed is measured *(proposed)* | A 12-tile board on the reference table, 20 opens with remembered results off, today's table vs the copy, same data and machine | |
| 4 | Architecture | ADR 0019: Iceberg is the source of truth; ClickHouse holds a copy refreshed from it | |
| 5 | Default for new Gold tables *(proposed)* | Today's path; the new path is chosen per table. Changing the default is a follow-up after acceptance | |
| 6 | When the choice is made *(proposed)* | At creation, then fixed. Switching an existing table is a follow-up | |
| 7 | Masking for outside readers *(proposed)* | An open-format-first table that any masking or row-filter policy names is refused at the door, with a fixed message. Dashboards mask on every read, as today | |
| 8 | Old versions kept *(proposed)* | The newest 7 Iceberg snapshots (same number as `DATA-2`) | |
| 9 | A stale copy *(proposed)* | Labelled on the table page with its age; dashboards still show it (not blocked) | |
| 10 | Who may choose the new path *(proposed)* | Holders of `pipeline:write`; no new permission | |
| 11 | Several organisations *(proposed)* | Only a pipeline of the catalog's organisation may choose the new path; others are refused | |
| 12 | Row limit *(proposed)* | 5,000,000 rows, the publish limit (`GOLD_EXPORT_MAX_ROWS`); over it nothing is written. Lowered only with the owner's signature if T0 shows it does not fit in memory | |
| 13 | Publishing such a table *(proposed)* | Refused with "This table is stored in open format; there is nothing to publish." | |
| 14 | Tables the assistant creates *(proposed)* | Today's path; no tool change | |

## Limits to tell a customer

- A dashboard can be up to one copy refresh behind the open table: within 60 seconds at the
  reference size; the measured lag at 5,000,000 rows is written here after T0.
- Each run replaces the whole table; there is no incremental write on this path.
- A table with a masking or row-filter policy cannot be read by outside tools on this path.
- An existing Gold table stays on today's path.
- If the catalog or storage is down, the pipeline run fails and dashboards keep the last good
  copy; the page says freshness is not measured.

## Acceptance checklist

Run on a running console. A step not performed is never a pass.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-3-AC1` | Create a Gold table on the new path from the same definition as an existing one; open a dashboard on each | Same numbers | Not run |
| `DATA-3-AC2` | (operator) Read `gold.<table>` from a second tool through the door, no filter | Row count equals the console's; no publish step was pressed | Not run |
| `DATA-3-AC3` | Run its pipeline with new data; reload the dashboard | New numbers within 60 seconds of the run's commit time shown on the table page | Not run |
| `DATA-3-AC4` | Add a masking policy on a column; open the dashboard as a masked role, then read from the second tool | Masked on the dashboard; the outside read is refused with a clear message | Not run |
| `DATA-3-AC5` | (operator) Stop the copy refresh, run the pipeline | The table page says the copy is behind, with the time; never "current" | Not run |
| `DATA-3-AC6` | As a user without `pipeline:write`, try to create a table on the new path | Refused with an honest message | Not run |
| `DATA-3-AC7` | Press "Publish now" on a table on the new path (or call the route) | Refused: "This table is stored in open format; there is nothing to publish." | Not run |
| `DATA-3-AC8` | (operator) Drop the ClickHouse copy; wait one minute | The copy is rebuilt from Iceberg; the dashboard shows the same numbers | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] ADR 0010's status line: "Accepted; superseded in part by ADR 0019"
- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added (switch an existing table, masked outside reads, default path)
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
