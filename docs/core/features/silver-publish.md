# Publish Silver tables in open format

| | |
| --- | --- |
| Module | Build |
| Backlog | `DATA-4` |
| Kind | Feature |
| Spec | `docs/core/specs/data-4.md` (every *(proposed)* number is signed under Decisions) |
| Status | Planned |
| Priority | P3 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | Not written yet. Nothing open blocks it (`DATA-10`, `DATA-2` shipped). |
| Started | |
| Shipped | |
| Evidence | |

## Problem

Bronze is open format and Gold can be published (`DATA-1`), but Silver lives
only in ClickHouse, so an outside tool cannot read the cleaned data.

## What the user can do when this is done

1. Switch on publishing for a Silver table, as for a Gold table.
2. See its status ("Up to date", "Out of date", "Not measured", "Never published") and history.
3. Have it published after its pipeline succeeds, and skipped when unchanged.
4. Read it from a second tool in the `silver` namespace.

## Not included

- Storing Silver in open format first.

## Asking the assistant

No; publishing is not one of the assistant's dashboard tools.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Built, before `DATA-3` | Yes | Product owner, 2026-10-14 |
| 2 | Row limit | Same as Gold, 5,000,000 by default (`GOLD_EXPORT_MAX_ROWS`, one setting for both layers) | Product owner, 2026-11-20 (the spec's defaults accepted) |
| 3 | Copies kept | Same as Gold (`DATA-2`, 7; `GOLD_KEEP_COPIES`, one setting for both layers) | Product owner, 2026-10-14 |
| 4 | Status, history, skip when unchanged, masking | Exactly Gold's: the same states (including "Last publish failed", `DATA-49`), the same content fingerprint (`DATA-10`), masking applied when the copy is made | Product owner, 2026-11-20 (the spec's defaults accepted) |
| 5 | Who may switch publishing on or off, and press "Publish now" | Holders of the existing `gold:export` permission, as for Gold; no new permission | Product owner, 2026-11-20 (the spec's "as for Gold" default) |
| 6 | Who can read the published Silver table | The tenant that owns the shared catalog, as for the Silver table itself (`SEC-20`): the copy is in that tenant's warehouse, in the `silver` namespace, under the same name, so the same tenant gate, role access and masking rules apply inside the product. Outside tools do not read it through the API's door (`/api/iceberg`, ADR 0016 serves `gold` only); that is a follow-up | Planner, 2026-11-20, from `SEC-20` and ADR 0016 |
| 7 | A Silver table built from another organisation's data | Refused: when the install holds more than one tenant, a Silver table that an upload of another tenant claims, or that a pipeline of another tenant writes, is not published (switch refused; a scheduled publish fails with the reason) | Planner, 2026-11-20, from `SEC-20` and `SEC-80` |
| 8 | The overview of all published tables | Stays Gold-only in this build; Silver tables are published and shown on their own page only | Planner, 2026-11-20 (see the plan's Mismatch section) |

## Limits to tell a customer

- Same limits as Gold publishing (`features/gold-publish-per-mart.md`).

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-4-AC1` | Switch publishing on for a Silver table; run its pipeline | Published without pressing anything; time and snapshot shown | Not run |
| `DATA-4-AC2` | (operator) Count rows from a second tool | Equals the console's row count | Not run |
| `DATA-4-AC3` | Run the pipeline with no change | Nothing new published | Not run |
| `DATA-4-AC4` | Force a failure (lower the row limit) | A failure with a reason; status does not say up to date | Not run |
| `DATA-4-AC5` | As an Analyst, look for the switch | Visible but not changeable, with an explanation | Not run |
| `DATA-4-AC6` | Switch publishing off; run a change | Nothing new published; the published table stays | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
