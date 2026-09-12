# ClickHouse 26.8 — re-measurement of every 26.3 finding

Every Iceberg finding in this repo was measured on `26.3.26.3`. Review of the
PR stack pointed out that 26.8 LTS ships catalog-INSERT work, and that four
documents plus two ADRs rest on 26.3 evidence. This re-runs all of them on
`26.8.2.7` against the same stack (Lakekeeper `0.13.3`, RustFS
`1.0.0-rc.4`, format-version 2, OpenFGA authorization enforced).

## Result table

| Operation | 26.3 | 26.8 | Changed? |
| --- | --- | --- | --- |
| `CREATE TABLE` inside a `DataLakeCatalog` database | `Code 79` — falls back to MergeTree, never contacts the catalog | **`Code 79`, identical** | no |
| `INSERT` into a **partitioned** catalog table | **SEGFAULT** (signal 11, `ChunkPartitioner::partitionChunk` ← `IcebergStorageSink::consume`) | **works** — rows land and read back | **YES** |
| `INSERT` into an **unpartitioned** catalog table | `Code 1000 … Can not extract empty value` | **works** | **YES** |
| `OPTIMIZE` on a catalog table | `Code 499 … HTTP 403` | **returns OK** | **YES** |
| `OPTIMIZE … MANIFEST` | `Code 62` SYNTAX_ERROR | **returns OK** | **YES** |
| `remove_orphan_files` | `Code 36` — verb does not exist for Iceberg | **works** | **YES** |
| `expire_snapshots` | worked | **`Code 48` — "not supported for Iceberg tables backed by a transactional catalog"** | **YES (removed)** |
| `allow_iceberg_remove_orphan_files` setting | absent | present | **YES** |
| Iceberg-related settings | 28 | 35 | — |

## What this changes, and what it does not

### The segfault is fixed — ADR 0010's premise is gone

ADR 0010 moved Gold export into Rust because ClickHouse **crashed the server**
on `INSERT` into a partitioned catalog-registered table. On 26.8 that INSERT
succeeds. The crash-based justification no longer holds.

**But the decision still stands, on a narrower basis:** `CREATE TABLE` inside
a `DataLakeCatalog` database is *unchanged* — it still falls back to MergeTree
and fails `Code 79`, never reaching the catalog. Verified via the catalog's
own REST API: the namespace is not created. So ClickHouse still cannot
*create* a catalog-registered table; something else must. The Rust path
already does both create and append, is proven by G1(a), and does not depend
on an experimental setting.

ADR 0010 is therefore re-based rather than reverted: the reason changes from
"ClickHouse segfaults" to "ClickHouse cannot create catalog-registered
tables", and the segfault becomes historical context.

### `OPTIMIZE` unblocked, but it was never the compaction we needed

The `403` is gone. That is *not* the same as compaction working. Measured
directly: seven Parquet data files in a single day partition, `OPTIMIZE`
returns OK, and **seven files remain**. Row count stays correct (7).

This matches what the original brief actually said — `OPTIMIZE` "merges
position-delete files into data files", and "**NOT available in ClickHouse:
bin-pack rewrite of small data files**". The G3 gate measured ~15–20×
query-planning degradation from small-file accumulation; nothing in 26.8
addresses that.

**ADR 0009's Trino escape hatch therefore stands** — but the ADR's stated
reason (an S3 403 from `OPTIMIZE`) is now wrong and must be corrected to the
real one: ClickHouse has no bin-pack rewrite at any version tested.

### `expire_snapshots` is now unsupported — this breaks the P4 job

On 26.3, `expire_snapshots` was the *only* working maintenance verb, and the
P4 Dagster job was built around it. On 26.8 it fails:

```
Code: 48. expire_snapshots is not supported for Iceberg tables backed by a
transactional catalog.
```

That is a deliberate restriction, not a bug: with a REST catalog, snapshot
expiry has to go through the catalog rather than the engine. **Bumping to 26.8
breaks the maintenance job as written**, and it must be reworked — dropping
`expire_snapshots`, adding the now-working `remove_orphan_files`, and routing
snapshot expiry through Lakekeeper.

### Net effect on the maintenance chain

| Verb | 26.3 | 26.8 |
| --- | --- | --- |
| `expire_snapshots` | only working verb | **unsupported** — must move to the catalog |
| `remove_orphan_files` | did not exist | **works** |
| `OPTIMIZE` | 403 | runs, but does not bin-pack |
| bin-pack compaction | unavailable | **still unavailable** — Trino remains required |

## Not re-measured here

**R11 (bare `count()` overcounting on equality deletes)** needs a CDC-fed
table carrying merge-on-read equality deletes, which requires the full
Debezium path rather than a hand-built table. It is unverified on 26.8. The
lint guarding it is cheap and correct regardless of version, so it stays; but
the risk register should not claim the defect is confirmed on 26.8 until it
is re-run.

**G3's planning-time numbers** (~1.05s vs ~0.067s) were measured on 26.3 and
are not re-run here. The bin-pack gap that caused them is unchanged, so the
conclusion holds even though the figures are version-stamped.
