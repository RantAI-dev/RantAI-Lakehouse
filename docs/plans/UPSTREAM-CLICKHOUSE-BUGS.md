# Upstream ClickHouse defects — ready to file, NOT yet filed

Four distinct defects in ClickHouse's Iceberg support, all found and
reproduced during this build. **Nothing here has been reported upstream.**
Filing is a public, outward-facing action and is the repository owner's call
— this document exists so that filing is a copy-paste job when they decide to.

Report at <https://github.com/ClickHouse/ClickHouse/issues>.

## Common environment (paste into every report)

```
ClickHouse: 26.3.26.3 (official build)
build id:   6EFC606D87BD3BB970628AD59E4DE56B7118904B
git hash:   4545d1a94b01a8c0ad70326c028ea2998617b960
image:      clickhouse/clickhouse-server:26.3
Catalog:    Lakekeeper 0.13.3 (Iceberg REST), Postgres-backed
Storage:    RustFS 1.0.0-rc.4 (S3-compatible), also reproduced on SeaweedFS 4.44
Format:     Iceberg format-version 2
```

Common setup for all four:

```sql
SET allow_database_iceberg = 1;
CREATE DATABASE icecat ENGINE = DataLakeCatalog(
  'http://lakekeeper:8181/catalog', '<key>', '<secret>')
SETTINGS catalog_type='rest',
         storage_endpoint='http://rustfs:9000/lakehouse-warehouse',
         warehouse='default';
```

Reading through this database works correctly — `SHOW TABLES`, `DESCRIBE`,
and `SELECT` all behave. The defects below are all on the write side, plus
one read-side aggregation bug.

---

## 1. SEGFAULT — `INSERT` into a partitioned catalog-registered Iceberg table

**Severity: crash.** Remote-triggerable by any user permitted to run `INSERT`.

Given a format-version-2 Iceberg table registered in the REST catalog and
partitioned by `day(_ingested_at)`:

```sql
SET allow_database_iceberg=1, allow_insert_into_iceberg=1,
    allow_experimental_insert_into_iceberg=1;
INSERT INTO icecat.`bronze.my_table` (id, label) VALUES (1, 'probe');
```

The server dies. Client sees `Code: 32 … ATTEMPT_TO_READ_AFTER_EOF`; the
container restarts.

```
Received signal 11 (Segmentation fault)
Address: 0x3f5b5b60. Access: read. Address not mapped to object.
  3. DB::ColumnString::get(unsigned long, DB::Field&) const
  4. DB::ChunkPartitioner::partitionChunk(DB::Chunk const&)
  5. DB::IcebergStorageSink::consume(DB::Chunk&)
  6. DB::SinkToStorage::onConsume(DB::Chunk)
 14. DB::AsynchronousInsertQueue::processData(...)
```

Note the `INSERT` supplies only the non-partition columns. An
**unpartitioned** table does not crash — see defect 2 — so the partition
path is implicated.

## 2. `INSERT` into an unpartitioned catalog table fails with a POCO exception

Same setup, but the table has an empty `partition-spec` (created directly via
the Iceberg REST API):

```
Code: 1000. DB::Exception: Exception: Can not extract empty value. (POCO_EXCEPTION)
```

No crash — the server stays up. Taken with defect 1, **`INSERT` into a
catalog-registered Iceberg table does not work in either shape**; partitioning
only determines whether the failure is a crash or an exception.

## 3. `CREATE TABLE` inside a `DataLakeCatalog` database never reaches the catalog

```sql
SET allow_database_iceberg = 1;
CREATE TABLE icecat.`bronze.probe` (id Int64, label String);
-- Code: 79. DB::Exception: MergeTree storages require data path. (INCORRECT_FILE_NAME)
```

Lakekeeper is never contacted (verified by its request log). ClickHouse
applies the default engine instead of routing the DDL to the catalog, then
fails on the missing data path. Explicit `ENGINE = Iceberg` still demands a
literal S3 URL, i.e. path-based only, which defeats using a catalog at all.

**Expected:** `CREATE TABLE` in a `DataLakeCatalog` database should create the
table *through the catalog*.

## 4. `ALTER TABLE … EXECUTE optimize` on an Iceberg table fails 403

```sql
SET allow_database_iceberg=1, allow_insert_into_iceberg=1,
    allow_experimental_iceberg_compaction=1;
ALTER TABLE icecat.`bronze.my_table` EXECUTE optimize;
-- Code: 499. DB::Exception: Failed to get object info: No response body..
--            HTTP response code: 403.
```

**Not a credentials problem** — reproduced identically with (a) Lakekeeper
vended credentials and (b) static object-store admin credentials with full
bucket access, where `aws s3api head-object` against the *same object* with
the *same* static credentials succeeds.

## 5. Read-side: bare `count()` overcounts on tables with equality deletes

**Severity: silent wrong answer.** This one returns a plausible number rather
than an error, so nothing catches it.

On a CDC-fed table (Debezium upsert mode) carrying merge-on-read **equality
delete** files:

| Query | Returns | Correct? |
| --- | --- | --- |
| `SELECT count() FROM t` | 8 | **No** |
| `SELECT count(*) FROM t` | 8 | **No** |
| `SELECT count() FROM t WHERE id > 0` | 6 | Yes |
| `SELECT count() FROM (SELECT id FROM t GROUP BY id)` | 6 | Yes |
| `SELECT id, … FROM t` | 6 rows, latest values | Yes |

A bare, unqualified `count()` appears to take a metadata-only fast path that
sums row counts from data-file metadata **without subtracting equality
deletes**. Any `WHERE` or `GROUP BY` forces the row-scan path and is correct.

**Expected:** `count()` should agree with `count() WHERE <always true>`.

Equality deletes are otherwise applied correctly on row-returning queries, so
this is specifically an aggregation fast-path bug, not a lack of
equality-delete support.

---

## Our workarounds (context for maintainers)

- Defects 1–3: we do not write Iceberg from ClickHouse. Rust
  (`iceberg-rust`), Debezium, and dlt write through the catalog; ClickHouse
  only reads. See `docs/adr/0010-gold-export-to-iceberg-from-rust.md`.
- Defect 4: compaction runs via a Trino cron container scoped to Bronze. See
  `docs/adr/0009-small-file-compaction-trino-escape-hatch.md`.
- Defect 5: never emit an unqualified `count()` against a Bronze Iceberg
  table. Tracked as R11 in `LAKEHOUSE-FOUNDATION-PLAN.md`.

Full evidence: [`G1-RESULT.md`](G1-RESULT.md),
[`G3-RESULT.md`](G3-RESULT.md), [`P5-RESULT.md`](P5-RESULT.md),
[`CLICKHOUSE-MAINTENANCE-FINDINGS.md`](CLICKHOUSE-MAINTENANCE-FINDINGS.md).
