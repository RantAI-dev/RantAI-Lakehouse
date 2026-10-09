# `SRC-9` Test every advertised connector end to end

| | |
| --- | --- |
| Backlog | `SRC-9` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Oracle, SFTP, MySQL CDC and SQL Server CDC have never been tested by a gate, and the upload gate does not run in CI.

## What users get

Every connector type we list has a gate test in CI, so a break is caught before release.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Gate coverage | Some connector types never gate-tested | 100% of connector types shown as supported have a gate: connect, discover, load, re-load |
| File-upload gate g9 | Exists, not in CI | Runs in CI on every PR touching ingestion |
| Gate runtime | n/a | The connector gates together finish within 20 minutes in CI *(proposed)* |

## Benchmark

Includes the connector half of VER-1.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-9-AC1` CI shows one passing gate per connector type in the picker
- `SRC-9-AC2` Breaking a connector's code turns its gate red
- `SRC-9-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
