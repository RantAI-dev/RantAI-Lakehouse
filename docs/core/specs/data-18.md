# `DATA-18` Column types on upload

| | |
| --- | --- |
| Backlog | `DATA-18` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Upload) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Every uploaded column is stored as text, so numbers and dates cannot be summed or filtered as such.

## What users get

Numbers, dates and booleans load as numbers, dates and booleans.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Type detection | Every column is text | Integer, decimal, boolean, date, timestamp and text detected from a sample of 10,000 rows *(proposed)* |
| Edit before loading | Names cleaned automatically | Edit each column's name and type in the preview |
| Bad values | n/a | Rows that do not fit the chosen type are counted and listed (first 100), and the user chooses: load as null or stop |
| Date formats | n/a | Detects ISO, dd/mm/yyyy and mm/dd/yyyy, and asks when ambiguous |

## Benchmark

Databricks: infers types, column names and types editable, 50-row preview. Snowflake: INFER_SCHEMA with editable columns.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A file with a date column loads it as a date
- [ ] Changing a column to integer when it holds text shows the failing rows before loading
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
