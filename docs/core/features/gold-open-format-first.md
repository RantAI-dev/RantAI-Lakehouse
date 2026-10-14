# Gold stored in open format first, served from a fast copy

| | |
| --- | --- |
| Module | Build |
| Backlog | `DATA-3` |
| Kind | Feature |
| Spec | `docs/core/specs/data-3.md` (every *(proposed)* number is signed under Decisions) |
| Status | Planned |
| Priority | P3 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | Not written yet. **Waits for `DATA-4`** and a new ADR. |
| Started | |
| Shipped | |
| Evidence | |

## Problem

Gold is written to ClickHouse and copied to open format afterwards (ADR 0010).
The open copy is behind and is a second system to keep in step.

## What the user can do when this is done

1. Create a Gold table that is written once, to open format.
2. See its dashboard read a fast copy that is current within 60 seconds of each write.
3. Read the same table from an outside tool with no publish step.
4. Keep existing Gold tables on today's path until each is switched.

## Not included

- Moving existing Gold tables automatically; Silver on this path.

## Asking the assistant

No change: the dashboard tools read the fast copy as they read Gold today.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Built, after `DATA-4` | Yes | Product owner, 2026-10-14 |
| 2 | Copy lag *(proposed)* | 60 seconds, measured | Product owner, 2026-10-14 |
| 3 | Dashboard speed *(proposed)* | p95 tile time within 10% of the measured baseline | Product owner, 2026-10-14 |
| 4 | Architecture | A new ADR replacing ADR 0010's direction, accepted before the plan | Planner |

## Limits to tell a customer

- A dashboard can be up to the measured copy lag behind the open table.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-3-AC1` | Create a Gold table on the new path; open its dashboard | Same numbers as on today's path | Not run |
| `DATA-3-AC2` | (operator) Read the table from a second tool | Readable with no publish step | Not run |
| `DATA-3-AC3` | Run its pipeline with new data | The dashboard shows it within 60 seconds | Not run |
| `DATA-3-AC4` | Open a dashboard and an outside read of a masked column | Masked in both | Not run |
| `DATA-3-AC5` | (operator) Stop the copy refresh | The table page says the copy is behind; never "up to date" | Not run |
| `DATA-3-AC6` | As a user without the permission, create a table on the new path | Refused with an honest message | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
