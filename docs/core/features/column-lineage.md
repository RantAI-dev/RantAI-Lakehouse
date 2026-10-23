# Column-level lineage

<!-- PR #106 review R1 SHOULD-FIX DATA-13 5 -->
| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-13` |
| Spec | `docs/core/specs/data-13.md` |
| Kind | Feature |
| Status | Released (accepted 2026-10-23; Phase 2 PR #106) |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | 2026-10-14 |
| Shipped | 2026-10-21 |
| Evidence | The Phase 2 pull request (#106) |
| Plan | `docs/superpowers/plans/2026-10-16-data-13-column-lineage.md` |

## Problem

Column mappings are never recorded, so the Lineage tab's column view is
hidden as always empty. A reader can see which tables feed a Gold table but
not which columns, and impact analysis stops at the table.

## What the user can do

- See, on an asset's Lineage tab, which source columns feed each of its
  columns — upstream and downstream — without anyone recording them by hand.
- Start impact from a single column: every downstream column, and the
  charts and dashboards that read them, chart by chart.
- Trust that mappings cover pipelines, Gold builds and saved SQL, and that
  anything the product could not map says so instead of showing nothing.

## Decisions

**Signed 2026-10-15** — the product owner approved every row's Default as
shown below (the *(proposed)* markers are removed; these numbers are now the
decisions). The plan builds to them; a target that cannot be met is
reported, not quietly lowered.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | How mappings are recorded. | Column mappings are **persisted rows**, written by the same path that records a pipeline run's rename, cast and select steps, a Gold build, and a saved-SQL save; a failed run writes nothing. The Lineage tab reads rows — it never rebuilds the map on request. | 2026-10-15 |
| 2 | Saved SQL depth. | When a saved SQL is saved, its referenced source tables and columns are mapped for plain `SELECT` queries the parser understands. Anything else (CTEs that defeat the parser, functions returning sets, `SELECT *`) stores what was mapped and marks the mapping **partial**, naming the reason. | 2026-10-15 |
| 3 | Who may see column lineage. | The same permission the table-level Lineage tab requires today — no new permission. | 2026-10-15 |
| 4 | Retention. | 1 year of mappings, as both competitors document. Older mapping rows are dropped; the count of retained days is shown, not guessed. | 2026-10-15 |
| 5 | The lineage gate. | The gate (`ops/g3a`) must accept the `build` link kind the API emits; the plan fixes `RECORDED_LINEAGE_EDGE_KINDS` and its test in the same item (spec, "Found while re-reading"). Not a user-visible decision — recorded here so it is not lost. | 2026-10-15 |

## Limits to tell a customer

- Lineage covers what ran through this product: pipelines, Gold builds and
  saved SQL. Reads pushed from outside tools leave no column mapping.
- Lineage from external systems stays out (GOV-3).
- A mapping marked partial covers only what was mapped; the reason is shown.
- Mappings older than 1 year are gone.

## Spec rows

| Spec row | Status |
| --- | --- |
| Column mappings recorded for every pipeline, Gold build and saved SQL source | **Partly met.** Built for all three producers. Rows are written from now on: a pipeline gets rows at its next successful run, a SQL source when next saved (no backfill), builds on the next tick (first pass looks back 180 days, 200 statements per tick). A pipeline run only records if the orchestrator's sensor reports it. Whatever the parser cannot prove is stored partial with a reason; a statement that cannot be read at all clears its producer's rows and logs. |
| View: column lineage on the Lineage tab, up and down | **Met** (code and component tests; not seen in a browser). |
| Impact: from a source column, every downstream column, chart and dashboard | **Partly met.** Downstream columns and saved charts (matched on mart or SQL source plus any column the chart definition names) with their dashboards, for `dashboard:read`. Not covered: the built-in default dashboard's tiles, and legacy charts saved without a `def`. |
| Retention 1 year | **Met in code**: purge on the 15-minute tick, and the answer states `days: 365` and the oldest visible record. Purge not run. |

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Rows follow the spec's three.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-13-AC1` | Open a Gold column's lineage. | It names the Bronze source columns, from persisted mappings of the run that built it. | Pass 2026-10-23 (product owner) |
| `DATA-13-AC2` | Open a Bronze column's downstream impact. | It lists downstream columns and, chart by chart, the dashboards that read them. | Pass 2026-10-23 (product owner) |
| `DATA-13-AC3` | Read lineage as a user without the permission; force a lineage failure. | Refused like the table-level tab; a failure shows an honest message (principles 2 and 4). | Pass 2026-10-23 (product owner) |
| `DATA-13-AC4` | Run the g3a lineage gate against a project with `build` links. | The gate passes with `build` listed, not fails on the unknown kind. | Pass 2026-10-23 (product owner) |

**Acceptor** (not the Owner): product owner (held in the base) **Date:** 2026-10-23 **Build:** main 0e5498d2

Exceptions, each with an owner and a date: none.

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [x] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [x] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This page's decisions need the product owner's signature; the plan follows
(`AGENTS.md`).
