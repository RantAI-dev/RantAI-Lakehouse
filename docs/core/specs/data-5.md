# `DATA-5` Count who reads a published table from outside

| | |
| --- | --- |
| Backlog | `DATA-5` in [BACKLOG.md](../BACKLOG.md) |
| Module | Build |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open. The product owner chose option B on 2026-10-31 |

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

## Options (the product owner chose B, 2026-10-31)

| Option | What is counted | Misses | Size |
| --- | --- | --- | --- |
| **A. Trino history** (the stub's original idea) | Queries through Trino that name the published table | Every tool that reads the catalog or storage directly (pyiceberg, Spark, DuckDB). Counts are lost when Trino restarts. Names are unverified. | S |
| **B. Catalog audit, plus one identity per outside tool** (planner's recommendation) | Every table load through the catalog, by tool and by table, kept by the API | Readers that bypass the catalog with the static storage keys. Those keys are for internal services only, and the page says so. | M |
| **C. Storage audit** | Every file read under the published table's location | Nothing, but it counts files, not reads: one query reads many files. It maps to storage keys, not to tools, so it still needs B's identities to name anyone. | M, after B |

**Why B.** It answers the real question, which tool reads which table, with a name an admin
gave that tool. Every standard Iceberg client goes through the catalog. It also gives each
outside tool its own revocable credential, which is needed anyway: today there is no proper way
to connect one. C can be added later if bypassing the catalog becomes a concern.

## Decisions (product owner, 2026-10-31)

- **Option B.** Every *(proposed)* number below is signed as written.
- **How B is built** (planner, recorded in ADR 0016). The catalog's only login service is a
  development stand-in that cannot issue logins on demand (ADR 0011). So outside tools do not
  log in to the catalog. They come through the console's API instead:
  - An outside tool is a service identity with the scope `iceberg:read`. Its key is shown once
    and can be revoked, as for any service identity.
  - The API answers the Iceberg REST catalog calls a reader makes (`/api/iceberg/v1/...`). It
    allows reads of the `gold` namespace only, refuses everything else, and forwards each call to
    the catalog under one read-only principal.
  - The API counts a read when it answers a table load. The count is therefore immediate, not
    "within 5 minutes". A load the API cannot record is refused, not served uncounted
    (principle 5).
  - The catalog hands out a storage address that only works inside the deployment
    (`rustfs:9000`). The API swaps it for the address an operator sets
    (`OUTSIDE_S3_ENDPOINT`). Without that address, outside reading is off and says so.
  - **Found while designing this:** outside tools cannot read published tables today, because
    the vended address is internal. The checklist rows that say an outside tool reads a table
    (`DATA-3-AC4`, `DATA-4-AC2`) depend on this item.

## What users get

- An admin creates a service identity with the scope `iceberg:read` for an outside tool (for
  example "Finance DuckDB"), then uses **Rotate** to get its key, shown once.
- The tool connects to `<console address>/api/iceberg` with that key. It can list and read
  published Gold tables, and nothing else.
- On a published table, "Readers (7 days)" lists each tool with its number of reads and its
  last read. A table nobody read says "No reads in 7 days".
- When outside reading is not set up, the page says "Not measured", with the reason. It never
  says zero.
- Revoking or deleting the tool's key stops its access, and its past reads stay listed under
  the name it had.

## Target specs

| Capability | Today | Target |
| --- | --- | --- |
| Outside tool identity | None | A service identity with `iceberg:read`; key shown once; revocable |
| What a tool may read | Nothing works (the vended address is internal) | Published Gold tables only (namespace `gold`, the catalog tenant's warehouse) |
| Writes | n/a | Every write call refused (403) |
| Read count | "Not measured" always | Every table load through the API, per table and per tool |
| Delay | n/a | Immediate (counted when the load is answered); was proposed as within 5 minutes |
| History kept | n/a | 90 days of daily counts; the page shows the last 7 days |
| Internal readers | n/a | Not counted: the API's own reader, ClickHouse and Trino use the catalog directly, not the door |
| Not set up | n/a | "Not measured" with the reason, never zero (principle 2) |

## Benchmark

- **Snowflake:** `ACCESS_HISTORY` records every read with its user and objects, kept for a
  year.
- **Databricks Unity Catalog:** system tables (`system.access.audit`) record every table access
  with its principal.

Both name the reader and keep the history, which is the bar for option B.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-5-AC1` An admin creates a service identity with `iceberg:read` and gets its key, shown
  once.
- `DATA-5-AC2` (operator) A tool (pyiceberg or DuckDB) reads a published Gold table through
  `/api/iceberg` with that key. It counts the same rows as the console, and the table's
  "Readers (7 days)" shows the tool with one read.
- `DATA-5-AC3` The same credential cannot read a Silver or Bronze table, or write anything.
- `DATA-5-AC4` A table nobody read says "No reads in 7 days". With `OUTSIDE_S3_ENDPOINT` unset,
  it says "Not measured", not zero.
- `DATA-5-AC5` After the tool is revoked, its reads fail, and its past reads stay listed.

## Not included

- Counting reads that bypass the catalog (option C).
- Rows or bytes read per query.
- Silver tables (`DATA-4`), which follow when they are published.
- Tenant warehouses: the door serves the catalog tenant's warehouse only.
- The OAuth2 client-credentials exchange (`credential=id:secret`). Tools pass the key as a
  bearer token (`token`), which pyiceberg, Spark and DuckDB all accept.

## Asking the assistant

Not part of the dashboards assistant.

## Before anyone builds it

Decided. The feature page is [`features/outside-readers.md`](../features/outside-readers.md),
the design is ADR 0016, and the plan is
`docs/superpowers/plans/2026-10-31-data-5-outside-readers.md`.
