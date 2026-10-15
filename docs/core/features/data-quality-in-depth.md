# Data quality in depth

| | |
| --- | --- |
| Module | Data |
| Backlog | `DATA-14` |
| Spec | `docs/core/specs/data-14.md` |
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

Four ready checks run hourly and a page lists their latest results, but a
failed check tells nobody, nothing is learned from run history, and there is
no list of what is wrong now. Editing a rule even deletes its past runs.

## What the user can do

- Pick from a fuller library of ready checks — the Snowflake set — and set
  a per-table schedule, including on every load.
- See open incidents grouped by table, with each rule's run history kept.
- Let the product learn volume and freshness anomalies from history and
  open incidents when a load breaks them.
- Get alerted on a failed check through SRC-7's channels.

## Decisions

**Not signed.** Each row's Default is what the planner proposes; the product
owner confirms or changes it. *(proposed)* numbers come from the planner, not
from a competitor's documentation.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | The check library in this item. | The spec's named set, all of it: null count and percent, duplicate count, row count, freshness, accepted values, min/max/average, unique count — the four existing checks fold in, no check is removed *(proposed: all eight in this item, not staged)*. | |
| 2 | Threshold levels. | Two levels stay, as today: **warning** (recorded, feeds nothing) and **failure** (feeds the asset's health and opens an incident). A check configures each level separately *(proposed)*. | |
| 3 | Incident lifecycle. | A failure opens an incident; it is **resolved by a person**, with actor and reason recorded; the next passing run marks it stale but does not auto-resolve it *(proposed)*. Editing a rule **keeps** its past runs — today's delete-on-edit goes away. | |
| 4 | Schedules. | Per table: on every load, or on an interval of 1 hour to 1 day (the spec's range). On-load checks run **after** the load commits and never block or fail it; a schedule change takes effect at the next tick *(proposed: async after-commit, both schedule kinds)*. | |
| 5 | Anomaly detection. | Volume and freshness anomalies are learned per table from at least **14 days** of history (the spec's proposed number). Before that, the page says "not learned yet — collecting history" and detects nothing, rather than guessing. A volume anomaly is a load far under the recent median *(proposed: under half, today's per-pipeline factor, generalized per table)*; a freshness anomaly is a load later than the learned interval allows *(proposed: twice the learned median interval)*. Alerts go through SRC-7's channels. | |
| 6 | Permissions. | Unchanged from today's quality feature: viewing needs what the Data Quality page needs, defining rules and resolving incidents need `governance:write` *(proposed: no new permission)*. | |

## Limits to tell a customer

- Anomalies need 14 days of history per table; new tables detect nothing at
  first, and the page says so.
- On-load checks never block a load; a failing check opens an incident, the
  load itself stays committed.
- Alerts need SRC-7's channels configured; without them an incident still
  opens, only nothing is sent.

## Acceptance checklist

Run on a real deployment. Mark each Pass, Fail, or Not run with the reason.
A step not performed is never Pass. Rows follow the spec's four.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-14-AC1` | Add a no-nulls check on a column with nulls and run it. | The run fails and an incident opens. | |
| `DATA-14-AC2` | Load a table at 10% of its usual rows with 14+ days of history. | It is flagged as a volume anomaly. | |
| `DATA-14-AC3` | Open the quality page. | The incident is listed by table until a person resolves it; the rule's past runs survive an edit. | |
| `DATA-14-AC4` | Define a rule and resolve an incident as a user without `governance:write`; force a failure. | Refused; a failure shows an honest message (principles 2 and 4). | |
| `DATA-14-AC5` | Set a table's checks to on-every-load and load it. | Checks run after the load commits; a check failure does not fail the load. | |

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed
to the AI team in [`AI-16`](ai-16.md); AI-suggested checks are out of scope.

## Before anyone builds it

This page's decisions need the product owner's signature; the plan follows
(`AGENTS.md`).
