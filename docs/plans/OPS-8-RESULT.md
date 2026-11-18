# `OPS-8` result (part C): storage per layer, measured against the object store

The daily job `storage_snapshot_job` measured all four layers on a local
compose stack and wrote one row per layer for 2026-11-17. The two layers
that live in the object store (`bronze`, `gold`) match the store's own data
files exactly and the store's whole table prefix within 1 %, which is what
`OPS-8-AC4` needs. The whole-prefix gap has a cause, written down below,
not hidden.

## How it was measured

Measured on 2026-11-17 on a local compose project named `ops110`, torn down
afterwards (`docker compose -p ops110 down -v`). Versions: ClickHouse
`26.8.36`; RustFS for the object store; Lakekeeper for the Iceberg REST
catalog. The stack held four Iceberg tables with real rows, created through
the same REST catalog the job reads.

## What a layer is, and how its bytes are measured

| Layer | Owner | How its bytes are measured |
| --- | --- | --- |
| `bronze` | Iceberg namespace | the `total-files-size` of each table's CURRENT snapshot, from the table's own metadata document in the REST catalog |
| `gold` | Iceberg namespace | the same number, the same way |
| `silver` | ClickHouse database | `sum(bytes_on_disk)` of the database's active `system.parts` |
| `serving` | ClickHouse database | the same number, the same way |

`silver` and `serving` are ClickHouse databases in this stack, not Iceberg
namespaces, so they are not in the object store at all: their bytes are
ClickHouse's own on-disk bytes, and the object-store comparison of the next
section does not apply to them.

## The job's rows for 2026-11-17

The rows the job wrote to `lake.bronze_meta.storage_layer_snapshot`, read
back with `FINAL`:

| Layer | Tables | Bytes | Tables skipped |
| --- | --- | --- | --- |
| `bronze` | 2 | 5,550,367 | 0 |
| `gold` | 2 | 2,523,024 | 0 |
| `silver` | 3 | 9,619 | 0 |
| `serving` | 6 | 13,803 | 0 |

Four rows, no table skipped.

## Against the object store (`OPS-8-AC4`)

`OPS-8-AC4` needs the job's `total_bytes` within 1 % of the object store's
own size of the layer's prefix. The store's numbers are a RustFS listing:
the size of every object under a table's `data/` prefix, and the size of
every object under the table's whole prefix.

| Table | Job (`total-files-size`) | Store: data files | Store: whole prefix | Gap, data | Gap, whole prefix |
| --- | --- | --- | --- | --- | --- |
| `bronze.measurement_orders` | 981,245 | 981,245 | 988,317 | 0.0000 % | 0.7156 % |
| `bronze.measurement_orders_large` | 4,569,122 | 4,569,122 | 4,576,207 | 0.0000 % | 0.1548 % |
| `gold.measurement_mart` | 256,747 | 256,747 | 263,823 | 0.0000 % | 2.6821 % |
| `gold.measurement_mart_large` | 2,266,277 | 2,266,277 | 2,273,356 | 0.0000 % | 0.3114 % |
| **layer `bronze`** | **5,550,367** | 5,550,367 | 5,564,524 | 0.0000 % | **0.2544 %** |
| **layer `gold`** | **2,523,024** | 2,523,024 | 2,537,179 | 0.0000 % | **0.5579 %** |

Both layers are inside the 1 % of `OPS-8-AC4` on the whole-prefix reading,
and exact on the data files.

### The cause of the whole-prefix gap

`total-files-size` counts the DATA files a snapshot references. The object
store also holds, per table, the metadata JSON documents, the manifest list
and the manifest files: about 7,000 bytes per table on this stack (7,072 /
7,085 / 7,076 / 7,079 bytes measured, in the table order above). Those
bytes are real and belong to the table, but no snapshot summary counts
them, so the gap is a fixed per-table overhead rather than a drift: it
shrinks as the tables grow, which is why the smallest table shows 2.6821 %
and the largest 0.1548 %.

There are no orphan data files: every object under a table's `data/`
prefix is referenced by its current snapshot, and the data-file column
matches the job's number exactly for every table.

## Deviations from the plan

1. **The destination is a ClickHouse table, not an Iceberg table.** The
   rows go into `lake.bronze_meta.storage_layer_snapshot` (a
   `ReplacingMergeTree` ordered by `(layer, day)`), the way
   `capacity_snapshot_job` writes its own table, because the job already
   writes ClickHouse and the console reads the table through the same
   `lakehouse-api` route as every other measurement.
2. **The schema lives outside `EXPECTED_SCHEMAS`.**
   `_STORAGE_LAYER_SNAPSHOT_SCHEMA` sits beside `_CAPACITY_SNAPSHOT_SCHEMA`
   in `dagster/dispar_orchestrate/bronze_catalog.py`, and like it takes the
   "table does not exist yet" branch of `_assert_or_create_schema` on every
   deployment. `EXPECTED_SCHEMAS` stays the single owner of the registry
   schema (R10), and this new table is not part of the registry.
3. **The column is `layer`, not `namespace`.** Two of the four layers are
   not namespaces at all (they are ClickHouse databases), so one word
   covers all four.
4. **`silver` and `serving` are measured in ClickHouse.** The plan called
   all four layers Iceberg namespaces; only `bronze` and `gold` are.

## What the run also proved

- On a stack that had never run the bronze-maintenance job, the storage job
  created the catalog database it lists from (`icecat_maintenance`) itself,
  then listed the two namespaces and read all four tables: no table was
  skipped.
- The ClickHouse layers are counted with `system.tables.engine`
  (`engine NOT LIKE '%View'`): ClickHouse 26.8 has NO `type` column in
  `system.tables`, and a statement naming it is refused with
  `UNKNOWN_IDENTIFIER`. The engine names on this stack are `MergeTree` for
  all three `silver` and all six `serving` tables — no view, so no view is
  counted as a table.
- `docker compose up -d dagster-code-location` on the clean project started
  the Dagster code server with the new definitions, and the container
  carried `DAGSTER_RUN_LIMIT_STORAGE_LAYER_SNAPSHOT_JOB_SECONDS` (empty, so
  the code's default of 1800 seconds applies).

## What was not measured

- Per-tenant storage, and the storage history before the job's first run:
  out of scope in the plan.
- The object store's own total, for the layers that are not in it
  (`silver`, `serving`).
