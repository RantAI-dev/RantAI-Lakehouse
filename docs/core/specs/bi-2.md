# `BI-2` Reusable metrics

| | |
| --- | --- |
| Backlog | `BI-2` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Metrics are built-in cards defined once in a deployment file; users cannot define them and charts cannot pick them by name.

## What users get

Define a metric once and every chart and the assistant use the same number.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Define | Built-in cards in a deployment file | Users with the right role define a metric: name, description, formula, default time column and grain, filters |
| Segments | None | Saved named filters (Metabase segments) |
| Use | n/a | Picked by name in the builder, in KPIs, in alerts, and by the assistant (AI-7) |
| Governance | n/a | Certified metrics (with DATA-12); a change shows which charts use the metric |
| Time comparisons | n/a | Every metric offers previous-period and same-period-last-year comparison |

## Benchmark

Metabase metrics and segments (all plans). Tableau Pulse metrics.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `BI-2-AC1` A metric 'Revenue' used on three dashboards changes on all three when its formula is edited
- `BI-2-AC2` Asking the assistant for revenue uses the metric
- `BI-2-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Ships with [`AI-7`](ai-7.md) (Named metrics first), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
