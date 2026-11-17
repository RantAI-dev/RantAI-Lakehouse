# `OPS-1` Old versions of raw tables are trimmed by default

| | |
| --- | --- |
| Backlog | `OPS-1` in [BACKLOG.md](../BACKLOG.md) |
| Module | Operations |
| Size | S |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Every write to a raw (Bronze) table adds an Iceberg snapshot. A table with
no maintenance policy never expires one, so its history and metadata grow
without bound; with change capture writing continuously, that is every
CDC table. Expiry already exists, but only for a table someone gave a
policy (`table_maintenance_policy`, migration `0030`), and only through
Trino: ClickHouse 26.8 refuses `expire_snapshots` for catalog tables.

## Decisions (product owner, 2026-10-15)

| # | Decision |
| --- | --- |
| D1 | A table with no policy, or a policy that sets no `snapshotsToKeep`, keeps 7 days of snapshots (Trino's own floor) and older ones are expired by the daily maintenance job. |
| D2 | A per-table `snapshotsToKeep` overrides the default, as today. |
| D3 | Without the `trino` profile, expiry is recorded as skipped with the reason (as today for policies); it never pretends to run. |

## Target

| Capability | Today | Target |
| --- | --- | --- |
| A table with no policy | Snapshots kept forever | Snapshots older than 7 days expired daily |
| The outcome | ClickHouse's refusal recorded | `applied`, `refused` or `skipped` with the reason, per table, in `bronze_meta.maintenance_verb_run` |
| Evidence | None | `docs/plans/OPS-1-RESULT.md`: snapshot count and metadata size before and after on a live table |

## Acceptance checklist

- `OPS-1-AC1` On a stack with the `trino` profile, a Bronze table with no policy shows an `expire_snapshots` run with the 7-day default on the Governance maintenance view
- `OPS-1-AC2` Without the `trino` profile, the same table shows expiry skipped with "trino unreachable"
- `OPS-1-AC3` A table with `snapshotsToKeep` set still follows its own policy

## Not included

Wiring Lakekeeper's own expiry task queue (needs an admin token); a
setting to change the 7-day default; compaction.
