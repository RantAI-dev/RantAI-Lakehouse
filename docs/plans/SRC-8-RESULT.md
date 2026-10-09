# `SRC-8` result: what the loader does when a source table changes shape

Measured 2026-10-09 on the build machine, by two unit test files that run in
the Dagster suite:
`dagster/dispar_orchestrate/test_schema_drift_loader.py` (commit `01c43e8`)
and the SQL cases added in commit `45d8d28`.

Environment: `dlt` 1.30.0, `pyiceberg` 0.12.0, SQLAlchemy 2.0.36, Python
3.12.3. Destination: dlt's local filesystem destination with
`table_format="iceberg"`, columns read back from the Iceberg metadata.

**This is not the production path.** There is no object store, no Lakekeeper
catalog, no day partition and no `_ingested_at` column, and the SQL source is
SQLite, whose typing is loose. It tells us what dlt and Iceberg do; it does
not prove the same for PostgreSQL, MySQL, SQL Server or Oracle. The g6 gate
(MySQL) is the first check against a real database and has not run yet.

Every result was the same under `append` and `replace`.

## Sources whose types the loader infers from the rows

Files, REST, MongoDB, Kafka, SFTP.

| Run 1, then run 2 | Run 2 | Table afterwards |
| --- | --- | --- |
| A new column appears | loads | the column is added |
| A column stops arriving | loads | the column stays |
| Integers, then text that is not a number | loads | the integer column stays; a second column `<column>__v_text` holds the text |
| Text, then integers | loads | one text column; the integers are stored as text |
| Small integers, then integers beyond 32 bits | loads | one 64-bit column |

## SQL sources, whose types come from the database's column definitions

| Change at the source | Run 2 | Table afterwards |
| --- | --- | --- |
| Column added | loads | the column is added |
| Column dropped | loads | the column stays |
| INTEGER to TEXT, with values that are not numbers | **fails** | unchanged; no second column |
| TEXT to INTEGER | **fails** | unchanged |
| INTEGER to a wider integer, values beyond 32 bits | loads | one 64-bit column |
| VARCHAR(5) to VARCHAR(500) | loads | one text column |
| Primary key moves to another column | loads | no key is kept in the table |

After either failure the failed load stays pending in the pipeline's working
state, and every later load of that table fails too, even after the source
column is put back.

Column selection: removing columns from the reflected table before the load
(`table_adapter_callback`) keeps them out of the destination.

## What this changed

- Feature-page decision 9 said a type the column cannot hold lands in a
  second column. That holds for the first group only. It was signed on that
  wrong basis and is corrected by name here (principle 5).
- Decision 3's list of widenings included "anything to text" and "more
  decimal digits". For a SQL source the first fails; the second was not
  measured. Both are removed from the list, so they are breaking and the
  table waits before anything is loaded.
- The check before loading is therefore what keeps a SQL type change from
  breaking a table for good. Before this work nothing did.

## Not measured

- Any change of a decimal's precision or scale.
- Any of the above against PostgreSQL, MySQL, SQL Server or Oracle.
- Whether a table already broken by a past type change can be recovered.
