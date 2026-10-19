# Tables that refresh themselves within a target lag

| | |
| --- | --- |
| Module | Build |
| Backlog | `BLD-2` |
| Kind | Feature |
| Spec | `docs/core/specs/bld-2.md` (every *(proposed)* number is signed under Decisions) |
| Status | Planned |
| Priority | P3 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Held in the base; not the Owner. |
| Plan | `docs/superpowers/plans/2026-10-15-bld-2-target-lag.md` (branch `feat/bld-2-target-lag`) |
| Started | |
| Shipped | |
| Evidence | |

## Problem

A pipeline runs on a schedule or after its upstream pipeline. A table fed by
a connector or an upload is only as fresh as that schedule, and a frequent
schedule wastes runs when nothing changed.

## What the user can do when this is done

1. Set a target lag on a pipeline, from 1 minute to 7 days, or "Downstream".
2. See the pipeline run only when an input changed and the target would be missed.
3. See the current lag, the target, and "met" or "behind", on the pipeline page and in the list.
4. Get an alert when a pipeline stays behind its target for 3 checks in a row.
5. Ask the assistant to set or read a target lag.

## Not included

- Refreshing only the changed rows; each refresh rebuilds the output.
- ClickHouse materialized views.

## Asking the assistant

Yes, in the same PR: "Keep silver.orders_clean at most 10 minutes behind."
The AI team reviews that part.

## Decisions

| # | Decision | Default | Signed |
| --- | --- | --- | --- |
| 1 | Approach | A target lag per pipeline, on the existing pipelines | Product owner, 2026-10-14 |
| 2 | Smallest target lag | 1 minute (Snowflake's); measured before it is claimed, raised if not met | Product owner, 2026-10-14 |
| 3 | Alert after behind *(proposed)* | 3 checks in a row | Product owner, 2026-10-14 |
| 4 | Assistant | Same PR | Product owner, 2026-10-14 |
| 5 | How `BLD-2-AC4` is tested | Runs that fail while inputs change, not a stopped orchestrator: the orchestrator makes the lag checks, so with it stopped nothing is checked and the page shows "not measured" (`BLD-2-AC5`) | Product owner, 2026-10-15 |

## Limits to tell a customer

- The smallest lag is 1 minute, the target, and it was measured: in the final 1-minute gate run every sample was within 60 s (worst 32.70 s, average 25.64 s; `docs/plans/BLD-2-TARGET-LAG-RESULT.md`, run on `29d889340b1a`).
- Each refresh rebuilds the whole output table.

## Acceptance checklist

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `BLD-2-AC1` | Set a 5-minute target on a Bronze-to-Silver pipeline; load new rows into the Bronze table | The Silver table has them within 5 minutes | Not run |
| `BLD-2-AC2` | Leave the inputs unchanged for an hour | No run happens; status shows "met" | Not run |
| `BLD-2-AC3` | Set a downstream pipeline to "Downstream" | Its upstream refreshes only when the downstream needs it | Not run |
| `BLD-2-AC4` | Make the pipeline's runs fail (for example, drop its source table) while its inputs keep changing | After 3 checks in a row an alert is raised; the page shows "behind" | Not run |
| `BLD-2-AC5` | Open a pipeline whose lag cannot be measured (orchestrator down) | Shown as not measured, never as met | Not run |
| `BLD-2-AC6` | Ask the assistant to keep a table at most 10 minutes behind | The target lag is set | Not run |
| `BLD-2-AC7` | As a user without `pipeline:write`, set a target lag | Refused with an honest message | Not run |
| `BLD-2-AC8` | Remove the target lag | The pipeline goes back to its schedule | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] `BACKLOG.md` Dates has the Shipped date
- [ ] The base row updated to match the repo (status, PR, dates, QA-case results); the Acceptor is recorded in the base
- [ ] `CHANGELOG.md` entry a customer can read
