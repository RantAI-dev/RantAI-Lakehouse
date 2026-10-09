# `QS-5` Parameters, snippets and charts from results

| | |
| --- | --- |
| Backlog | `QS-5` in [BACKLOG.md](../BACKLOG.md) |
| Module | Query Studio |
| Also involves | Dashboards |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

SQL cannot take parameters, reuse a snippet or another query, or become a chart without saving it as a SQL source first.

## What users get

SQL that becomes dashboard content directly.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Parameters | None | Named parameters: text, number, date, date range, dropdown, multi-select, dropdown filled from a query (Databricks' widgets), always bound, never spliced into SQL |
| On dashboards | n/a | A parameter maps to a dashboard filter (Metabase field filters) |
| Snippets | None | Shared snippets inserted by name, with folders and permissions |
| Chart from a result | Through 'save as SQL source' only | Pick any chart type from the result and add it to a dashboard in two clicks |
| Reuse a saved query | None | Reference a saved query by name as a CTE |
| Alerts on a result | Dashboard metrics only | Alert when a saved query's result meets a condition, checked on a schedule |

## Benchmark

Metabase: variables of type text, number, date and field filter that become dashboard filters; snippets; any chart from a SQL result; a saved question used as a CTE; alerts on results. Databricks: named parameters with widgets (text, dropdown, multi-select, dynamic dropdown, date range).

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `QS-5-AC1` A query with a date-range parameter added to a dashboard is driven by the dashboard's date filter
- `QS-5-AC2` A parameter value containing a quote cannot change the SQL
- `QS-5-AC3` A chart is created from a result without saving a SQL source
- `QS-5-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Help inside the SQL editor is [`AI-12`](ai-12.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
