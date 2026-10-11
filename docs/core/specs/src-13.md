# `SRC-13` Table discovery for every source type

| | |
| --- | --- |
| Backlog | `SRC-13` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Only SQL and CDC sources list their tables; for files, REST, MongoDB, Kafka, SFTP and Oracle the user types names.

## What users get

Pick tables, files or streams from a list for any source.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Discovery coverage | SQL and CDC sources | Every connector type: files and buckets (by glob), REST (declared endpoints), MongoDB (collections), Kafka (topics), SFTP (paths), Oracle (schemas and tables) |
| Column preview | Not for these types | Inferred columns and types shown before choosing |
| Large sources | Not checked | Discovery of 10,000 tables completes within 2 minutes and is searchable *(proposed)* |

## Benchmark

Airbyte, Databricks and Snowflake discover schemas for every connector.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `SRC-13-AC1` For each connector type, the table picker lists what exists at the source
- `SRC-13-AC2` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Anything not in the Target table.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
