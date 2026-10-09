# `DATA-17` More formats, more files, bigger files

| | |
| --- | --- |
| Backlog | `DATA-17` in [BACKLOG.md](../BACKLOG.md) |
| Module | Data |
| When | Next |
| Size | M (planner's estimate: S = days, M = one to two weeks, L = several weeks) |
| Status | Spec. Not planned, not built |

## Why

Upload accepts one delimited text file of up to 50 MB.

## What users get

Upload Excel, JSON and Parquet files, up to 10 at once, up to 2 GB in total.

## Target specs

"Today" is `main` at `f3a3196`, read from the code, not tested. A target is either a competitor's documented number (named under Benchmark) or marked *(proposed)*: the planner's number, which the product owner confirms or changes on the feature page before the plan is written.

| Capability | Today | Target |
| --- | --- | --- |
| Formats | CSV and TSV | CSV, TSV, JSON and JSON Lines, Avro, Parquet, Excel (.xlsx: one chosen sheet) |
| Files per upload | 1 | Up to 10 files, uploaded in parallel (Databricks) |
| Size | 50 MB and 2,000,000 rows | Up to 2 GB per upload (Databricks), with the memory of the ingestion process measured in a RESULT document before the number ships |
| Upload method | Single request | Resumable chunked upload with a progress bar per file; a dropped connection resumes |
| Compressed files | Not supported | gzip and zip |
| Several files into one table | n/a | Files with the same columns load into one table; mismatches are listed before loading |

## Benchmark

Databricks upload UI: CSV, TSV, JSON, Avro, Parquet, text; up to 10 files, under 2 GB total. Snowflake load wizard: CSV, TSV, JSON, Avro, ORC, Parquet, XML; up to 250 files of 250 MB.

## Acceptance checklist

Run on a running console by the product owner. A step not performed is never a pass.

- `DATA-17-AC1` 10 Parquet files totalling 1.9 GB upload in parallel and load
- `DATA-17-AC2` An .xlsx with three sheets asks which sheet
- `DATA-17-AC3` Killing the network mid-upload and reconnecting resumes
- `DATA-17-AC4` A 2.1 GB upload is refused with a clear message
- `DATA-17-AC5` A user without the permission is refused, and a failure shows an honest message (principles 2 and 4)

## Not included

- Files arriving on a schedule (bucket or SFTP connector)

## Asking the assistant

Not part of the dashboards assistant. AI work for the Data module is handed to the AI team in [`AI-16`](ai-16.md).

## Before anyone builds it

This is a spec, not a plan. It gets a feature page in [`features/`](../features/) (decisions signed by the product owner, including every *(proposed)* number) and a plan in `docs/superpowers/plans/` first (`AGENTS.md`).
