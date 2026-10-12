# `SRC-10` result: what the loader can do for merge and history on Iceberg

First measurement, 2026-10-12, by the planner, with a throwaway script (not
in the repository) run in the Dagster test environment on the build machine.
Task 1 of the `SRC-10` plan repeats it as a unit test on the product's own
path and replaces this page's "not measured" list.

Environment: `dlt` 1.30.0, `pyiceberg` 0.12.0, Python 3.12.3. A `dlt`
resource of plain rows with `primary_key="id"` and `table_format="iceberg"`,
loaded twice into dlt's local filesystem destination; the table read back
from its Iceberg metadata.

**This is not the production path.** No SQL source, no Lakekeeper catalog, no
object store, no `_ingested_at` stamp, no day partition, and the query engine
did not read the result.

Run 1 loads keys 1 and 2. Run 2 loads key 2 with new values and key 3.

| Write disposition in run 1 and 2 | Run 2 | Table afterwards |
| --- | --- | --- |
| `merge`, strategy `upsert` | loads | 3 rows; key 2 has the new values |
| `merge`, no strategy named | loads | the same: `upsert` is what it does |
| `merge`, strategy `insert-only` | loads | 3 rows; key 2 keeps its old values |
| `merge`, strategy `delete-insert` | **refused** before loading | "Available strategies: ['upsert', 'insert-only']" |
| `merge`, strategy `scd2` (history) | **refused** before loading | the same message |
| `upsert`, run 2 holds key 2 twice | **fails** at load, after five retries | unchanged |
| `upsert`, the same with a `dedup_sort` hint on the cursor column | **fails** the same way | unchanged |
| `upsert`, with a `hard_delete` column in run 2 | **fails**: the new column does not match the table's fields | unchanged |

## What this decides

- One current row per key (SCD type 1) is within the loader's reach.
- History (SCD type 2) is not: the loader refuses it for Iceberg tables.
  Feature-page decision 1.
- A repeated key inside one load must be removed before loading; the loader's
  own hint does not do it. Feature-page decision 4.
- A deleted source row cannot be removed through the loader's delete marker.

## Not measured

- A merge through `sql_database`, through Lakekeeper, with the `_ingested_at`
  stamp and the day partition.
- A composite key; a null key; a merge into a table that already holds
  duplicates.
- Whether ClickHouse and Trino read a merged table correctly.
- The time and the files a merge rewrites on a large table.
