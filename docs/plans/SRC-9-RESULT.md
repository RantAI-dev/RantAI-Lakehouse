# `SRC-9` result: what the connector gates cover, what they found, how long they take

Measured on pull request #114, commit `3edde86`, CI run 38233177653, three
attempts of the same commit on 2026-10-12. GitHub-hosted `ubuntu-latest`
runners.

## Coverage

Eleven connector types are listed as supported. Each has a gate with four
steps. "Refused" means the product answers an honest "not supported" for
that step today, and the gate checks exactly that answer.

| Connector type | Job | connect | discover | load | reload |
| --- | --- | --- | --- | --- | --- |
| PostgreSQL | `g6-ingest` | real | real | real | real |
| MySQL | `g6-ingest` | real | real | real | real |
| MariaDB | `g6-ingest` | real | real | real | real |
| SQL Server | `g6-ingest` | real | real | real | real |
| Object storage | `g6-ingest-other` | real | refused (`SRC-13`) | real | real |
| REST API | `g6-ingest-other` | real | refused (`SRC-13`) | real | real |
| MongoDB | `g6-ingest-other` | refused (`SRC-5`) | refused (`SRC-13`) | real | real |
| Kafka | `g6-ingest-other` | refused (`SRC-5`) | refused (`SRC-13`) | real | real |
| SFTP | `g6-ingest-other` | refused (`SRC-5`) | refused (`SRC-13`) | real | real |
| Oracle | `g6-oracle` | refused (`SRC-5`) | refused (`SRC-13`) | real | real |
| PostgreSQL CDC | `g4-cdc` | real | real | real (first copy) | real (insert, update, delete arrive) |

The file-upload gate (`ops/g9`) runs as the job `g9-upload`.
`ops/lint/check_connector_gate_coverage.py` fails CI when a type is listed as
supported without a gate; on `3edde86` it reports 11 supported, 11 covered, 0
waived.

MySQL CDC and SQL Server CDC are no longer listed as supported (migration
`0066`, feature-page decision 8). Google Sheets has not been since `SRC-6`.

## Job durations

| Job | Run 1 | Run 2 | Run 3 |
| --- | --- | --- | --- |
| `g6-ingest` (SQL sources, with the `SRC-7` and `SRC-8` steps) | 16m36s | 17m22s | 15m16s |
| `g6-ingest-other` (files, APIs, streams, and the link-local half) | 18m36s | 17m58s | 14m53s |
| `g6-oracle` | 15m51s | 12m54s | 14m55s |
| `g4-cdc` | 16m08s | 15m47s | 11m31s |
| `g9-upload` | 12m05s | 10m57s | 14m57s |

The slowest connector gate job took 18m36s. The spec's target, "the
connector gates together finish within 20 minutes" *(proposed)*, signed as
"the slowest connector gate job finishes within 20 minutes" (feature-page
decisions 1 and 9), holds in these three runs with 1m24s to spare in the
worst one.

Before the split (decision 9) the single batch job took 21m56s to 22m48s
over four runs, which is why its limit is 25 minutes. About 11.5 to 14
minutes of every job is bringing the stack up, most of it building the API
image; that time varies between runs by more than the gate steps do.

## What the gates found in the product

| # | Found by | Defect | Outcome |
| --- | --- | --- | --- |
| G1 | `g6`, MySQL discover | Listing a MySQL connector's tables always failed. MySQL 8 returns `information_schema` names in a form the driver would not read as text, and the failure was reported as "connection failed". | Fixed (`fe76d3d`); the gate passes. |
| G2 | reading, then `g6-oracle` | Oracle discovery answered an error where every other type answers "not supported". | Fixed (`a0fbc77`). |
| G3 | `g6-oracle`, load | An Oracle connector could not load from a pluggable database: the console's "service name" was sent as a SID. | Fixed (`9a6465a`); the gate loads from Oracle Database Free. |
| G4 | `g9-upload` | On an install that had never run a connector, the first upload answered 503: the catalog registry's database did not exist yet and the name check failed closed on that. | Fixed (`47517cc`); the gate passes on a stack that has never ingested. |
| G5 | reading for the CDC gates | MySQL CDC and SQL Server CDC cannot run: the API renders the PostgreSQL Debezium settings for them. | Both listed as not supported (`77677f4`); building them is `SRC-4`. |

Also fixed on the way: the gate's REST stub answered 404 on its base path,
so a REST connection test could not pass; `g3`, `g3a` and `g7` triggered a
job before the orchestrator had loaded it.

## Not measured, not covered

- "Breaking a connector's code turns its gate red" (the spec's acceptance
  line) was seen four times by accident (G1, G3, G4 and the first Oracle
  run) and not as a planned test on a branch.
- Every gate runs against one version of one source system, in a container,
  with a handful of rows.
- Oracle is loaded without TLS: the adapter requires a certificate name for
  TLS that the fixture does not have.
- `ops/g7` (the pipeline gate) is still not run by any CI job.
- The durations are three runs on one day.
