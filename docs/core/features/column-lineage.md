# Column-level lineage

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-13` |
| Spec | `docs/core/specs/data-13.md` |
| Kind | Feature |
| Status | Drafting — **decisions not signed**. The product owner signs every row marked *(proposed)*; the plan is written after that |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist; not the Owner. Held in the base. |
| Started | Not yet |
| Shipped | Not yet |
| Evidence | The Phase 2 pull request (#106) |
| Plan | Not yet written; follows the signature |

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

**Not signed.** Each row's Default is what the planner proposes; the product
owner confirms or changes it. *(proposed)* numbers come from the planner, not
from a competitor's documentation.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | How mappings are recorded. | Column mappings are **persisted rows**, written by the same path that records a pipeline run's rename, cast and select steps, a Gold build, and a saved-SQL save; a failed run writes nothing. The Lineage tab reads rows — it never rebuilds the map on request *(proposed: persisted, not computed on read)*. | |
| 2 | Saved SQL depth. | When a saved SQL is saved, its referenced source tables and columns are mapped for plain `SELECT` queries the parser understands. Anything else (CTEs that defeat the parser, functions returning sets, `SELECT *`) stores what was mapped and marks the mapping **partial**, naming the reason *(proposed: partial, honestly — principle 14)*. | |
| 3 | Who may see column lineage. | The same permission the table-level Lineage tab requires today — no new permission *(proposed)*. | |
| 4 | Retention. | 1 year of mappings, as both competitors document. Older mapping rows are dropped; the count of retained days is shown, not guessed. | |
| 5 | The lineage gate. | The gate (`ops/g3a`) must accept the `build` link kind the API emits; the plan fixes `RECORDED_LINEAGE_EDGE_KINDS` and its test in the same item (spec, "Found while re-reading"). Not a user-visible decision — recorded here so it is not lost. | |

## Limits to tell a customer

- Lineage covers what ran through this product: pipelines, Gold builds and
  saved SQL. Reads pushed from outside tools leave no column mapping.
- Lineage from external systems stays out (GOV-3).
- A mapping marked partial covers only what was mapped; the reason is shown.
- Mappings older than 1 year are gone.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Rows follow the spec's three.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-13-AC1` | Open a Gold column's lineage. | It names the Bronze source columns, from persisted mappings of the run that built it. | |
| `DATA-13-AC2` | Open a Bronze column's downstream impact. | It lists downstream columns and, chart by chart, the dashboards that read them. | |
| `DATA-13-AC3` | Read lineage as a user without the permission; force a lineage failure. | Refused like the table-level tab; a failure shows an honest message (principles 2 and 4). | |
| `DATA-13-AC4` | Run the g3a lineage gate against a project with `build` links. | The gate passes with `build` listed, not fails on the unknown kind. | |

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This page's decisions need the product owner's signature; the plan follows
(`AGENTS.md`).
