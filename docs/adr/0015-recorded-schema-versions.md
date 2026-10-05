# ADR 0015 — Schema versions of Silver and Gold tables are recorded, not read

- **Status:** Accepted (product owner, 2026-10-05); implementation in progress
- **Phase:** plan `docs/superpowers/plans/2026-10-05-schema-versions-silver-gold.md`
- **Date:** 2026-10-05

## Context

A raw (Bronze) table's page shows its schema versions: when columns were
added, renamed, retyped. A Silver or Gold table's page shows none, and the
product owner asked why, and whether it can.

The difference is the format. An Iceberg table carries every schema it has
had in its own metadata, with an id per column that survives a rename, and
each snapshot names the schema it was written with. The console reads that
list from the catalog (`lakehouse-iceberg::rest`, `schema_versions`).

A Silver or Gold table lives in the analytics engine. The engine keeps one
schema, the current one (`system.columns`). When a table is altered, or
rebuilt with `CREATE OR REPLACE`, the definition before it is gone. There
is nothing to read a history from.

## Decision

**The console records it.** Each time it sees that the ordered list of
`(column name, type)` of a table in the Silver or Gold database differs from
the last list it recorded for that table, it records a new version.

- **Where:** `console.table_schema_version` in the analytics engine,
  created on first use, the arrangement `console.quality_run` and
  `console.gold_export_run` already have. Not Postgres: it is an
  observation about engine tables, it needs no join with console state, and
  it adds no migration to a branch whose migration numbers already collide
  with `main`'s.
- **What a version holds:** the table's key (`silver.orders`), a number
  from 1, the column list, and when it was **observed**.
- **When the console looks:** when the API starts; on the alerts tick,
  which the orchestrator already calls on a schedule; and when the
  orchestrator reports a run as finished. Each look is one pass over both
  databases: two reads, and one insert per table that changed.
- **What the page shows:** the versions newest first, each with what
  changed from the one before, in the places a raw table's versions appear.

## What this is not

Stated on the page, not only here:

- **It starts when it starts.** The first version is "first recorded with N
  columns", dated when the console first saw the table, not when the table
  was made. Nothing before that can be recovered.
- **It is an observation.** Two changes between two looks are recorded as
  one. A change undone before the next look is not recorded at all.
- **Columns are matched by name.** A renamed column reads as one dropped
  and one added. The engine gives a column no identity that outlives its
  name.
- **It is the definition only.** The rows as they were under an earlier
  version are gone; there is no "query this version".

## Alternatives considered

**Read the engine's query log for `CREATE` and `ALTER` statements.** It is
how "built from" lineage edges are found. Rejected here: a statement is not
a schema (a `CREATE … AS SELECT` names no column types), and the log is
kept for a period an operator chooses.

**Record a version from the pipeline definition.** `main` keeps a version
history of authored pipeline definitions. Rejected as the source: a table
can change without its pipeline changing, tables built outside the builder
have no definition, and a definition is intent, not what the engine holds.

**Record on every page view.** It would make the page always current.
Rejected: a read should not write.

## Consequences

- A new table in the engine's `console` database, written by the API.
- The asset detail of a Silver or Gold table carries `schemaVersions`,
  which was always empty for them.
- A version's time is when it was seen, up to one tick after the change.
- Views are recorded as tables are: their columns can change too.

## Verification

None yet. Written with the plan, before the code.
