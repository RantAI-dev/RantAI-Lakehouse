# `SEC-13` Check the staging file pushed to main

| | |
| --- | --- |
| Backlog | `SEC-13` in [BACKLOG.md](../BACKLOG.md) |
| Area | Security |
| Who builds it | Whoever pushed 7ec3a81 |
| When | Now |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Commit 7ec3a81 added .env.staging straight to main, without a pull request (an earlier note named df14b78, which is not on main). The planner could not read it, so nobody has confirmed it holds no real credentials or hostnames.

## What users get

Confidence that the repository holds no real password or server address.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Contents of .env.staging | Not checked | Placeholders only, or the file removed and ignored |
| Any real value found | Unknown | Rotated, and the rotation recorded |

## Benchmark

Commit 7ec3a81; AGENTS.md rule 12.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] The person who pushed it confirms in writing what it contains
- [ ] gitleaks (working tree) passes
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Rewriting history: that decision is SEC-1

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
