# <Feature name>

One page. The agents build from it; the product owner accepts with it.
Copy this file to `<feature-slug>.md` and fill it in. Keep it to what a user
can observe; how it is built belongs in the engineering plan.

| | |
| --- | --- |
| Module | <one name from the base's list: Data, Dashboards, Query Studio, AI Copilot, Build, Governance, Operations, Administration & Security> |
| Backlog | <ID> |
| Kind | Feature |
| Spec | `docs/core/specs/<id>.md` (target numbers; every *(proposed)* one is signed under Decisions) |
| Status | Idea / Planned / Building / In Acceptance / Released / Killed (the base's six; "merged" is **In Acceptance**, never Released) |
| Priority | P0 / P1 / P2 / P3 |
| Owner | The module's owner, in the base. Not named here (the repo is public). |
| Acceptor | Who runs the acceptance checklist. Must not be the Owner; for a feature the owner built, a second person. Held in the base. |
| Plan | `docs/superpowers/plans/<date>-<slug>.md` |
| Started | <date work began> (base) |
| Shipped | <date merged to `main`> (base) |
| Evidence | The PR(s), and a link to the accepted build or its checklist run |

## Problem

What is wrong today, in two or three sentences, with the evidence.

## What the user can do when this is done

Short, separate, checkable statements. One per line.

1. ...
2. ...

## Not included

What this deliberately leaves out.

## Asking the assistant

Can the assistant do this too? If yes, what does the user ask? If no, why
not?

## Decisions

Anything the product owner must choose. Until signed, the default is used.
Every number marked *(proposed)* in the task's spec is listed here.

| # | Decision | Default | Signed |
| --- | --- | --- | --- |

## Limits to tell a customer

Plain statements of what it will not do. Copied into `PRODUCT.md` section 3
when accepted.

## Acceptance checklist

One row per check. Each has a stable ID (`<BACKLOG-ID>-AC<n>`, e.g. `BI-1-AC1`)
so it maps to one QA Case in the base and keeps that ID for life. Run on a real
deployment. **Result** uses the base's values: **Pass**, **Fail**, or
**Not run** (with the reason). A step not performed is never Pass. Steps needing
a terminal are marked (operator).

This list is the self-contained acceptance criteria. Do not write "every target
row works"; write the checks out in full here, so the row means something on its
own once it is a QA Case in the base.

Always include: the normal path, a failure being visible, a user without
permission, switching it off, and an unknown value shown honestly.

| ID | Do this | Expect | Result |
| --- | --- | --- | --- |
| `<ID>-AC1` | ... | ... | Not run |

**Acceptor** (not the Owner): __________ **Date:** ______ **Build:** ______

Exceptions, each with an owner and a date:

## After acceptance

- [ ] `PRODUCT.md` section 2 and 3 updated
- [ ] `BACKLOG.md` status set to **Released**, with the PR in the PR column; follow-ups added
- [ ] The base row updated to match (status, Shipped, Acceptor, QA-case results)
- [ ] `CHANGELOG.md` entry a customer can read
