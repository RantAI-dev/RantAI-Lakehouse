# `DATA-16` Time travel proven safe

| | |
| --- | --- |
| Backlog | `DATA-16` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| Size | S (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Spec checked | Against the code and a running instance on 2026-10-09. Feature page [`time-travel.md`](../features/time-travel.md). Trino is split off as `DATA-21` |

## Why

Querying a past version works on ClickHouse, but no test proves it still applies masking and row filters. On Trino it does not work at all: the API refuses the query before it reaches Trino.

## What users get

Querying a past version of a table is trusted to apply masking.

## Target specs

"Today" is `main` at `a78ad62`, read from the code on 2026-10-09; rows marked *run* were run against a live API and ClickHouse that day. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Masking gate test | None | A gate test proves past-version queries pass the masking and row-filter rewrite. This task: ClickHouse. Trino: `DATA-21` |
| ClickHouse | *Run:* `SETTINGS iceberg_snapshot_id = <id>` returns the table as it was (14, 28, 42, 56 rows across four versions of one table), also through a subquery shaped like the masking rewrite. One setting pins every Iceberg table in the query, so a join of two tables at one version fails | Works, proven by the gate; the limit on joins is stated where the version is chosen |
| Trino | *Run:* `FOR VERSION AS OF <id>` is answered 422 "policy cannot be evaluated" before Trino is called; the SQL parser the masking rewrite uses cannot read the clause. By reading, not run: the masking rewrite writes ClickHouse-only functions, so a table with a masking policy cannot be read on Trino at any version | This task: refused with a plain message saying past versions are available on ClickHouse. Working on Trino: `DATA-21` |
| Picker | Two exist. The asset page's "Query this version" writes the ClickHouse form and lists time and operation. Query Studio's picker writes only the Trino form, whichever engine is chosen, and lists the version id and operation without the time | One behaviour: writes the form of the chosen engine, lists time and operation, and is switched off with the reason when the engine is Trino |
| Retention shown | Not stated. A "Snapshots to keep" setting exists on the Lakehouse table page; version clean-up is skipped on this ClickHouse version | Where versions are listed and chosen, the page states how many versions the table has and the date of the oldest |

## Benchmark

Supersedes DATA-7. Snowflake: Time Travel 1 day on all editions, up to 90 days on Enterprise.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-16-AC1` A masked column stays masked, and a row filter applied, when a raw table is queried at an older version on ClickHouse
- `DATA-16-AC2` A past-version query on Trino is refused with a plain message
- `DATA-16-AC3` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Past versions on Trino, and masking on Trino: `DATA-21`.
- Pinning two tables to two different versions in one query.

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
