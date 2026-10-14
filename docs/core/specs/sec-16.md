# `SEC-16` No cross-tenant leaks in the Data module

| | |
| --- | --- |
| Backlog | `SEC-16` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Also involves | Data |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

One connector route lists every tenant's connectors; all uploads share one namespace; one message names another tenant's upload table; catalog annotation edits skip the tenant gate.

## What users get

One organisation never sees another's connectors, uploads or table names.

## Target specs

"Today" is the 2026-10-07 audits; items marked re-checked were confirmed in the code by the planner. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| GET /api/connectors/ingestible | All tenants, with hosts and usernames | Filtered by the caller's tenant |
| Upload namespace | One shared Bronze namespace | A namespace per tenant |
| Claimed-name message | Names another tenant's table | Fixed message without the name |
| Catalog annotation GET/PUT | Skip the tenant gate | Pass the tenant gate and check the asset exists |

## Benchmark

From the Data audit; not re-checked.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SEC-16-AC1` A cross-tenant test for each of the four cases fails before the fix and passes after
- `SEC-16-AC2` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not an assistant feature. Where a fix touches the assistant's endpoints, the AI team reviews it.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
