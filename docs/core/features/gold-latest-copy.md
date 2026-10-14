# A latest-copy-only table for outside readers

| | |
| --- | --- |
| Module | Build |
| Backlog | `DATA-6` |
| Kind | Feature |
| Spec | `docs/core/specs/data-6.md` (every *(proposed)* number is signed under Decisions) |
| Status | Planned |
| Priority | P2 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | Not written yet. **Waits for `DATA-2`**. |
| Started | |
| Shipped | |
| Evidence | |

## Problem

A published Gold table keeps several copies, one per publish. An outside
tool that does not filter on the newest export time counts every copy.

## What the user can do when this is done

1. Read `gold.<table>_latest` from any tool that reads Iceberg, with no filter, and get only the newest copy.
2. See and copy that table's name on the publish page.
3. Switch publishing off; both tables stop updating and both stay.

## Not included

- Iceberg views.

## Asking the assistant

No; publishing is not one of the assistant's dashboard tools.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Built, with "keep N copies" | Yes | Product owner, 2026-10-14 |
| 2 | Name *(proposed)* | `gold.<table>_latest` | Product owner, 2026-10-14 |
| 3 | Form | A plain Iceberg table, replaced in one commit | Planner |

## Limits to tell a customer

- The latest-only table changes only when a publish succeeds.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `DATA-6-AC1` | (operator) After a publish, count rows of the latest-only table from a second tool, no filter | Equals the console's row count | Not run |
| `DATA-6-AC2` | (operator) Read it while a publish runs | Old copy or new copy, never a mix | Not run |
| `DATA-6-AC3` | Force a publish failure | The latest-only table keeps the previous copy; the failure is shown | Not run |
| `DATA-6-AC4` | As a user without the permission, look for the switch | Visible but not changeable, with an explanation | Not run |
| `DATA-6-AC5` | Switch publishing off; publish a change | Neither table changes | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
