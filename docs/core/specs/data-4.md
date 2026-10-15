# `DATA-4` Publish Silver tables in open format

| | |
| --- | --- |
| Backlog | `DATA-4` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open (was: DATA-10, DATA-2, shipped) |

## Why

Raw (Bronze) tables are open format already, and Gold tables can be
published (`DATA-1`). Silver tables live only in ClickHouse, so an outside
tool cannot read the cleaned data between the two.

## What users get

The same "publish in open format" switch, status and history on a Silver
table as on a Gold table.

## Decisions (product owner, 2026-10-14)

- Built. Before `DATA-3`.

## Target specs

"Today" is `main` at `99b42812`, read from the code, not tested. A target is
either a competitor's documented number (named under Benchmark) or marked
*(proposed)*: the planner's number, which the product owner confirms or
changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Silver publishing | None | A per-table switch, as for Gold; published to the `silver` namespace in the catalog |
| Status and history | n/a | Same four states and history as Gold ("Up to date", "Out of date", "Not measured", "Never published") |
| Skip when unchanged | n/a | Same content fingerprint as Gold (`DATA-10`) |
| Copies kept | n/a | Same rule as Gold (`DATA-2`) |
| Masking | n/a | Applied when the copy is made, as for Gold |
| Row limit | n/a | Same limit as Gold, 5,000,000 rows by default; larger tables refused, not cut short |

## Benchmark

Databricks and Snowflake: every layer can be an open-format table. Gold
publishing (`DATA-1`) is the in-house reference.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-4-AC1` Switch publishing on for a Silver table; it is published after its pipeline succeeds
- `DATA-4-AC2` (operator) A second tool reads the Silver table and counts the same rows as the console
- `DATA-4-AC3` The Gold acceptance rows that do not name Gold pass for Silver too
- `DATA-4-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Storing Silver in open format first (`DATA-3` covers Gold).

## Asking the assistant

Not part of the dashboards assistant.

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
