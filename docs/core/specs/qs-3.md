# `QS-3` Saved queries done properly

| | |
| --- | --- |
| Backlog | `QS-3` in [BACKLOG.md](../BACKLOG.md) |
| Module | Query Studio |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Waits for | [`SEC-20`](sec-20.md) (security fix) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Saved queries can only be created and listed, for everyone, with no folders, sharing or history.

## What users get

Organise, share and recover saved queries.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Manage | Create and list only | Edit, rename, duplicate, move, delete (to a trash kept 30 days, proposed) |
| Folders | None | Folders and a personal space |
| Sharing | Everyone | Four levels per query or folder: view, run, edit, manage (Databricks) |
| History | None | Last 15 versions with author and time, revert to any (Metabase) |
| Search | Not checked | Search saved queries by name and SQL text |

## Benchmark

Databricks: folders, sharing CAN VIEW / CAN RUN / CAN EDIT / CAN MANAGE. Metabase: collections with permissions; 15 versions of history with revert.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `QS-3-AC1` A query shared as 'run' can be run but not changed
- `QS-3-AC2` Reverting to version 3 restores its SQL
- `QS-3-AC3` A deleted query is restored from the trash
- `QS-3-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Help inside the SQL editor is [`AI-12`](ai-12.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
