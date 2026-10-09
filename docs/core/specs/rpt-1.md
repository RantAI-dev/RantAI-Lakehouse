# `RPT-1` Scheduled reports with attachments

| | |
| --- | --- |
| Backlog | `RPT-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Dashboards |
| When | Next |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Only text digests exist, all on one 15-minute cycle.

## What users get

Send a dashboard on a schedule as a PDF or Excel file.

## Target specs

"Today" is `main` at `47a826d`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Schedules | None | Hourly, daily, weekly, monthly at a chosen time and time zone (Metabase) |
| Attachments | Text digests only | PDF of the dashboard; CSV or Excel per chart |
| Filters | n/a | A subscription carries its own filter values |
| Recipients | n/a | Users and outside email addresses (admin can restrict to a domain allowlist) |
| Skip when empty | None | Optional |
| Delivery record | None | Each send logged with status; failures alert the owner |

## Benchmark

Metabase dashboard subscriptions: hourly, daily, weekly, monthly; CSV and XLSX attachments, all plans; skip when empty. Tableau subscriptions: PNG and PDF.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `RPT-1-AC1` A weekly Monday 08:00 PDF arrives with the dashboard's charts
- `RPT-1-AC2` A subscription filtered to Bali shows only Bali
- `RPT-1-AC3` With 'skip when empty', no email is sent for an empty result
- `RPT-1-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Slack, Teams and Google channels (owner decision)

## Asking the assistant

Ships with [`AI-10`](ai-10.md) (Reports and exports), in the same release (assistant parity, `AGENTS.md`).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
