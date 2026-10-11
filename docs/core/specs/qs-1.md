# `QS-1` Editor basics

| | |
| --- | --- |
| Backlog | `QS-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Query Studio |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Autocomplete knows only table names; the schema browser, multiple statements, run-selection and tabs are not checked; formatting is minimal.

## What users get

An editor that knows your columns and handles real work.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Autocomplete | Table names | Keywords, schemas, tables, columns and aliases, ranked by context; suggestions within 200 ms *(proposed)* |
| Schema browser | Not checked | Beside the editor: layers, tables, columns with types; click to insert |
| Statements | Not checked | Several statements in one tab; run all (a result tab per statement) or run the selection |
| Tabs | Not checked | Several query tabs, kept across reloads |
| Formatting | A small formatter | Format with settings for keyword case and indentation |
| Editor | Not checked | Find and replace, comment toggling, bracket matching, folding, keyboard shortcut list |
| Autosave | Not checked | Unsaved drafts survive a reload |

## Benchmark

Databricks SQL editor: autocomplete for keywords, catalogs, tables, columns and aliases; schema browser; multi-statement with a result per statement; tabs; formatting with custom rules. Snowflake Workspaces; Metabase SQL editor.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `QS-1-AC1` Typing a table alias followed by a dot lists its columns
- `QS-1-AC2` Three statements run and show three result tabs
- `QS-1-AC3` Closing and reopening the browser restores the open tabs
- `QS-1-AC4` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Real-time co-editing and git *(proposed drop)*

## Asking the assistant

Help inside the SQL editor is [`AI-12`](ai-12.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
