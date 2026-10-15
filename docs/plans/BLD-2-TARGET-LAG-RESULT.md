# BLD-2 Target Lag Result — Measured Bronze-to-Silver Lag Gate

P8/D11 measurement gate for backlog item BLD-2 (pipelines that refresh themselves within a target lag).

Measured against a clean `docker compose -p bld2gate` stack (RustFS, Lakekeeper, ClickHouse 26.8, Dagster 1.13.20 daemon + code-location + webserver, and `lakehouse-api`).

## Test Configuration

- **Pipeline**: Authored pipeline `g7_dedupe_select` (`pl-g7-dedupe-select-...`), status `ready`
- **Source**: Iceberg table `bronze.g3a_orders` (Lakekeeper REST catalog)
- **Target**: ClickHouse MergeTree table `silver.g7_dedupe_select_out`
- **Transforms**: `["dedupe(id)", "select(id,customer)"]`
- **Target Lag**: 1 minute (`{ "kind": "lag", "minutes": 1 }`)
- **Sensor**: `authored__pl_g7_dedupe_select_..._lag`, interval 30s (`minimum_interval_seconds=30`)

## 1. Measured Lag (Time-to-Silver)

Rows were inserted into Postgres `ingest_demo.orders` and loaded into Bronze via `bronze_ingest_job`. Elapsed time was measured from the Bronze ingest commit until the Silver table reached the expected distinct row count.

| Sample | Initial Count | New Rows | Bronze Commit -> Silver Reached | Target (60s) Met? |
| :---: | :---: | :---: | :---: | :---: |
| 1 | 2000 | 1 (2001) | **52.65s** | YES (<= 60s) |
| 2 | 2001 | 1 (2002) | **10.27s** | YES (<= 60s) |
| 3 | 2002 | 1 (2003) | **16.33s** | YES (<= 60s) |
| 4 | 2003 | 1 (2004) | **8.30s** | YES (<= 60s) |
| 5 | 2004 | 1 (2005) | **6.23s** | YES (<= 60s) |

- **Average Elapsed Time**: **18.75s**
- **Result**: **ALL 5 SAMPLES MET 1-MINUTE TARGET (<= 60s)**.

> [!NOTE]
> Per plan Section 6.1: Lag is measured from the first check that saw an input change, so it can read up to one check interval (30s) low. The total wall-clock time from Bronze commit to Silver materialization stayed strictly within the 60s window across all samples.

## 2. Unchanged Inputs Assertion (Batching / Idle Preservation)

After the 5 write samples, inputs were left untouched. Every 30 seconds for 5 consecutive checks, the pipeline status and run count were probed.

| Check | Elapsed Wait | Reported Status | Reason | Run Count | New Runs |
| :---: | :---: | :---: | :---: | :---: | :---: |
| Baseline | 0s | `met` | within target lag | 7 | — |
| 1 / 5 | 30s | `met` | within target lag | 7 | 0 (None) |
| 2 / 5 | 60s | `met` | within target lag | 7 | 0 (None) |
| 3 / 5 | 90s | `met` | within target lag | 7 | 0 (None) |
| 4 / 5 | 120s | `met` | within target lag | 7 | 0 (None) |
| 5 / 5 | 150s | `met` | within target lag | 7 | 0 (None) |

- **Result**: Exactly 0 runs launched while inputs remained unchanged; pipeline status remained continuously `met`.

## 3. Conclusion

The pipeline target lag sensor and decision engine satisfy BLD-2 D11:
1. Target lag of 1 minute refreshes the pipeline within the target window upon source modifications.
2. Unchanged inputs trigger 0 unnecessary runs.
3. Pipeline status reports `met` accurately throughout.
