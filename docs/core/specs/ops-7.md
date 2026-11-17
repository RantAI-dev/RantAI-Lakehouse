# `OPS-7` A restore from backup is complete, tested and timed

| | |
| --- | --- |
| Backlog | `OPS-7` in [BACKLOG.md](../BACKLOG.md) |
| Module | Operations |
| Size | M |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

`backup-job` (`backups` profile) dumps the four Postgres databases
(application and identity, Lakekeeper catalog, OpenFGA, Dagster) to a
dedicated bucket. It does not back up the warehouse bucket, which holds
every table's data and Iceberg metadata, nor `ClickHouse`'s own databases,
which hold ingest state such as change-capture offsets. A restore from
today's backups gets a catalog pointing at files that are gone. No restore
has ever been run end to end or timed.

## Decisions (product owner, 2026-10-15)

| # | Decision |
| --- | --- |
| D1 | Close the gaps: the backup also covers the warehouse bucket and `ClickHouse`'s non-catalog databases, into the same dedicated backup bucket. |
| D2 | Run a full restore drill on a local stack and record the times per component in `docs/plans/OPS-7-RESULT.md`. |

## Target

| Capability | Today | Target |
| --- | --- | --- |
| What a backup holds | 4 Postgres databases | The 4 databases, the warehouse bucket, `ClickHouse`'s own databases |
| Restore procedure | Postgres only, manual, documented | Every component, in a documented order, with a script |
| Restore time | Never measured | Measured per component on a stated data size (`OPS-7-RESULT.md`) |
| After a restore | Unknown | Sign-in works, tables list and read the same row counts, permissions hold, ingestion resumes from its offset |

## Acceptance checklist

- `OPS-7-AC1` A backup run lists every component it saved and its size
- `OPS-7-AC2` On an emptied stack, the documented restore brings back users, tables (same counts), permissions and run history
- `OPS-7-AC3` `OPS-7-RESULT.md` gives the time of each step and the total, on a stated data size

## Not included

Point-in-time recovery; a scheduled in-Dagster backup; backing up Kafka
offsets of a deployment that runs Kafka; off-site copies.
