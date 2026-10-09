# `SRC-8` Schema changes at the source

| | |
| --- | --- |
| Backlog | `SRC-8` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

When a source table gains, loses or changes a column, the load evolves silently and nobody is told.

## What users get

When a source table changes shape, someone is told, and the choice of what happens is theirs.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Detection | Default evolution, silent | Added, removed, renamed-as-removed-and-added, and type-changed columns detected on every run |
| Per-connector policy | None | Four choices, matching Airbyte: apply non-breaking automatically (default), apply all, ask first, pause |
| Breaking changes (column removed, type narrowed, primary key changed) | Not detected | Always pause the affected table and ask, whatever the policy |
| Removed columns | Not handled | Kept in the table and marked inactive, never dropped (Databricks behaviour) |
| Notice | None | A notice on the connector page listing each change, and an alert (SRC-7) |

## Benchmark

Airbyte: propagate field changes, propagate all, approve all, stop; breaking changes always pause. Databricks Lakeflow: new columns added, removed ones marked inactive.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-8-AC1` Add a column at the source: it appears after the next run and a notice lists it
- `SRC-8-AC2` Drop a column at the source: the table pauses, the column stays, the notice asks for a decision
- `SRC-8-AC3` Set 'pause': any change stops the load until approved
- `SRC-8-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Backfilling a newly added column's history (later)

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
