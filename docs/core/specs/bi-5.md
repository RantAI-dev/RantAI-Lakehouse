# `BI-5` Timeouts and limits you can set

| | |
| --- | --- |
| Backlog | `BI-5` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Limits are fixed at 2,000 rows and 30 or 60 seconds, and stopping cancels only the browser request.

## What users get

Admins choose limits, and stop means stop.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Timeout | Fixed 30 s or 60 s | Admin-set, per install and per role, 10 seconds to 30 minutes *(proposed)* |
| Row limit | Fixed 2,000 | Admin-set per chart type; table charts page beyond it |
| Stop | Browser only | Cancels the query on the engine within 5 seconds *(proposed)* |
| Message | Not checked | A timed-out tile says it timed out and what the limit is |

## Benchmark

Both competitors: configurable query timeouts and row limits.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BI-5-AC1` Lowering the timeout to 10 s stops a slow tile at 10 s with a clear message
- `BI-5-AC2` Stop frees the engine
- `BI-5-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

No assistant counterpart is planned yet; the planner adds one before this is built (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
