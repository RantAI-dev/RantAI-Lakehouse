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

## Measured on the product's path (2026-10-12 and 2026-10-13)

The unit tests named here run the product's own loader call (`load_via_sink`)
with `dlt` 1.30.0 and `pyiceberg` 0.12.0 against a local Iceberg table; they
run in CI in the job "Dagster · loader tests". The gate runs the whole stack
in CI: a MySQL source, Lakekeeper, the object store, and ClickHouse reading
the tables back. The first measurement above stays as it was written.

| Question | Result | Where |
| --- | --- | --- |
| How many table commits is one merge? | One per 1,000 rows: the loader calls the table's upsert in 1,000-row slices. | `test_load_modes_measurement.py` |
| Can history be written through that merge? | Not safely. Above 1,000 rows it is several commits, and a failure between them leaves keys with a closed row and no current row. History is therefore written by the product in one table transaction (feature-page decision, 2026-10-12). | `test_load_modes_measurement.py`, `test_history_write_measurement.py` |
| A key repeated in one load, a null key, a table that already holds duplicates | Each fails the load. The product removes repeats before loading and refuses a null key with a plain message. | `test_load_modes_dedup_measurement.py` |
| What a failed load leaves | A pending package that the next run would finish instead of reading the source again. The product aborts pending packages at the start of a run and after a failure. | `test_sink_pending_packages.py`, `test_sink_killed_run.py` |
| Does a replacing load of an empty source empty the table? | Only when the load's resource name is the one stored for the table. "Clear" and "refresh and remove" therefore delete the rows with one table delete and check that no row is left. | `test_sink_empty_table.py` |
| Memory of deduplication | The number of distinct keys, not the number of rows (10,000 rows with 10 keys hold 10 rows). Bounded by `INGEST_DEDUP_MAX_ROWS`, default 2,000,000 keys. | `test_dedup.py` |
| The whole path, read by the query engine | Key detected from the source; one row per key after an update and an insert; an unchanged run writes nothing; history closes the old row and adds a current one; refresh and clear of one table leave the others and their last load unchanged; a non-history load into a table that holds history is rejected. Passed in CI on 2026-10-13. | `ops/g6/g6_ingest_matrix_test.py::step_load_modes`, pull request #115 at `6449c85` |

CI times on pull request #115 at `6449c85`: "G6 ingest matrix (SQL
sources)", which holds the new step, 19m54s (17m03s without it, measured the
same day on pull request #120); "Dagster · loader tests" 13m26s;
"Dagster · unit tests" 55s. The SQL job is within the 20-minute target of
`SRC-9` by six seconds: the next step added to it needs a job of its own.

## Not measured

- Trino reading a deduplicated or history table (the gate reads through
  ClickHouse).
- Wall time and memory at the 2,000,000-key bound, and the time and the
  files a merge rewrites on a large table.
- A source other than MySQL in the gate; the unit tests cover the loader,
  not each source's key detection on a real server (PostgreSQL, SQL Server
  and MariaDB key detection is covered by API tests only).
