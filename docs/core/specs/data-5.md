# `DATA-5` Count who reads a published table from outside

| | |
| --- | --- |
| Backlog | `DATA-5` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | M for option B below (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | The product owner's choice below |

## Why

A Gold table published in open format (`DATA-1`) exists for tools outside the console. Today
nobody can tell whether any tool reads it.

- The Gold Exports page has a "Consumers (7d)" column, and it always reads "Not measured".
- The route behind it, `GET /api/gold/export/{mart}/consumers`
  (`rust/crates/lakehouse-api/src/routes/gold.rs:694`), is an honest stub.

Without a count, an owner cannot tell which copies matter, which ones to stop publishing, or who
to warn before a change.

## What is there today

Read at `main` `6711ad65`, from the code, not tested.

| Piece | Today |
| --- | --- |
| Catalog | Lakekeeper 0.13, which hands out short-lived storage keys (`sts-enabled`). It writes an audit record in JSON for every permission check, including table reads (`read_data`), with the principal, the table and the time, but only to its own log. Nothing collects that log. |
| Object store | RustFS. It can send a record of every S3 request to a webhook, but none is configured. |
| Trino | Its query history (`system.runtime.queries`) is in memory only. It sees only reads made through Trino, and the user name comes from an unverified header. |
| Outside readers | There is no way to give an outside tool its own identity. The only catalog principals are internal (the API's reader, ClickHouse's reader, the export writer). Every outside read would look the same, or would use the static storage keys and bypass the catalog. |

## Options (the product owner chooses one)

| Option | What is counted | Misses | Size |
| --- | --- | --- | --- |
| **A. Trino history** (the stub's original idea) | Queries through Trino that name the published table | Every tool that reads the catalog or storage directly (pyiceberg, Spark, DuckDB). Counts are lost when Trino restarts. Names are unverified. | S |
| **B. Catalog audit, plus one identity per outside tool** (planner's recommendation) | Every table load through the catalog, by tool and by table, kept by the API | Readers that bypass the catalog with the static storage keys. Those keys are for internal services only, and the page says so. | M |
| **C. Storage audit** | Every file read under the published table's location | Nothing, but it counts files, not reads: one query reads many files. It maps to storage keys, not to tools, so it still needs B's identities to name anyone. | M, after B |

**Why B.** It answers the real question, which tool reads which table, with a name an admin
gave that tool. Every standard Iceberg client goes through the catalog. It also gives each
outside tool its own revocable credential, which is needed anyway: today there is no proper way
to connect one. C can be added later if bypassing the catalog becomes a concern.

## What users get (option B)

- An admin adds an outside tool (for example "Finance DuckDB") and gets a read-only catalog
  credential for it, shown once.
- The tool can read published Gold tables and nothing else.
- On a published table, "Readers (7 days)" lists each tool with its number of reads and its
  last read. A table nobody read says "No reads in 7 days".
- A table whose reads cannot be counted, for example because the audit feed is down, says "Not
  measured" with the reason. It never says zero.
- Revoking a tool stops its access, and its past reads stay listed.

## Target specs (option B)

| Capability | Today | Target |
| --- | --- | --- |
| Outside tool identity | None | One read-only catalog principal per tool, created and revoked in the console *(proposed)* |
| What a tool may read | n/a | Published Gold tables only (namespace `gold`) *(proposed)* |
| Read count | "Not measured" always | Every catalog table load by an outside tool, per table and per tool *(proposed)* |
| Delay | n/a | A read shows within 5 minutes *(proposed)* |
| History kept | n/a | 90 days of daily counts; the page shows the last 7 days *(proposed)* |
| Internal readers | n/a | Not counted as readers: the API, ClickHouse and Trino are excluded by principal *(proposed)* |
| Feed down | n/a | "Not measured" with the reason, never zero (principle 2) |

## Benchmark

- **Snowflake:** `ACCESS_HISTORY` records every read with its user and objects, kept for a
  year.
- **Databricks Unity Catalog:** system tables (`system.access.audit`) record every table access
  with its principal.

Both name the reader and keep the history, which is the bar for option B.

## Acceptance checklist (option B)

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-5-AC1` An admin adds an outside tool and gets a credential, shown once.
- `DATA-5-AC2` (operator) That tool reads a published Gold table, and within 5 minutes the
  table's "Readers (7 days)" shows the tool with one read.
- `DATA-5-AC3` The same credential cannot read a Silver or Bronze table, or write anything.
- `DATA-5-AC4` A table nobody read says "No reads in 7 days". With the audit feed stopped, it
  says "Not measured", not zero.
- `DATA-5-AC5` After the tool is revoked, its reads fail, and its past reads stay listed.

## Not included

- Counting reads that bypass the catalog (option C).
- Rows or bytes read per query.
- Silver tables (`DATA-4`), which follow when they are published.

## Asking the assistant

Not part of the dashboards assistant.

## Before anyone builds it

This is a spec, not a plan. The product owner chooses A, B or C and confirms or changes every
*(proposed)* number. Then a feature page and a plan follow (`AGENTS.md`).
