# `DATA-12` Certification and governed tags

| | |
| --- | --- |
| Backlog | `DATA-12` in [BACKLOG.md](../BACKLOG.md) |
| Area | Data (Catalog) |
| Who builds it | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec corrected against the code 2026-10-09. Not planned, not built |

## Why

Nothing marks a table as trusted or deprecated, and a tag is any single word anyone with edit rights types.

## What users get

Trusted tables are marked, deprecated ones warn, and tags follow agreed values.

## Target specs

"Today" is `main` at `c338862`, re-read from the code on 2026-10-09, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Certified and deprecated marks | None | Set by a data owner or admin; shown in search, on the asset page, in the chart builder's table picker and in Query Studio |
| Deprecation | None | A note and an optional replacement table; users of a deprecated table see a warning |
| Governed tags | A table carries up to 20 tags, each one lowercase word; no keys, no allowed values. Anyone with `catalog:write` sets them | Admin-defined tag keys with allowed values; only admins create keys |
| Inheritance | None | A tag on a schema applies to its tables (Snowflake behaviour) |
| Where tags show | Only on the asset page's About card. Search matches them without showing them; the Data Explorer list, the chart builder's table picker and Query Studio show none | With the marks: in search results, the Data Explorer list, the asset page, the chart builder's table picker and Query Studio |

## Benchmark

Databricks: certified and deprecated system tags, governed tags with allowed values (GA). Snowflake: tag allowed values, tag inheritance and propagation (Ent+).

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- [ ] Every Target row above works as written
- [ ] A certified table shows its mark in search and in the chart builder
- [ ] Setting a tag value outside the allowed list is refused
- [ ] A user without the owner role cannot certify
- [ ] A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Tag-based masking (later)

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
