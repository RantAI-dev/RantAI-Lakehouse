# OPS-1 result — measured, default snapshot retention

OPS-1 (spec `docs/core/specs/ops-1.md`, decisions D1–D3): a Bronze table with
no maintenance policy — or a policy that sets no `snapshotsToKeep` — keeps 7
days of snapshots, and the daily `bronze_maintenance_job
(dagster/dispar_orchestrate/maintenance.py)` expires the rest through Trino's
`ALTER TABLE … EXECUTE expire_snapshots(retention_threshold => '7d')`, because
ClickHouse 26.8 refuses `expire_snapshots` on catalog-backed tables
(`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`).

This document is the evidence the spec asks for — "snapshot count and metadata
size before and after on a live table" — measured on a dedicated compose
project against a table created for the measurement and dropped at the end.
Every number below comes from a command quoted here.

## Environment

A dedicated project, never the default one:

```
$ docker compose -p ops1check --profile dagster --profile trino up -d \
    lakehouse-api dagster-code-location dagster-webserver dagster-daemon \
    trino trino-maintenance-cron
```

(`up -d` with no service names also tries to start `debezium-server`, whose
digest-pinned image this host's registry refuses:
`Image ghcr.io/memiiso/debezium-server-iceberg:1.1.1.Final@sha256:2ad14b15… Error error from registry: denied`.
The named set above leaves `debezium-server` and its `g4-*` dependents out;
nothing in the measurement touches them.)

Image versions in that project, from `docker compose -p ops1check images`:

| Service | Image | Tag / ID |
| --- | --- | --- |
| `trino`, `trino-maintenance-cron` | `trinodb/trino` | `483` (`db58cc93e593`) |
| `lakekeeper`, `lakekeeper-migrate` | `quay.io/lakekeeper/catalog` | `v0.13.3` (`db2ba6168eb1`) |
| `clickhouse` | `clickhouse/clickhouse-server` | `26.8` (`8a1589a2dd9a`) |
| `rustfs` | `rustfs/rustfs` | `1.0.0-rc.4` (`a9fbb5e5bfce`) |
| `postgres` (app, Dagster, OpenFGA) | `postgres` | `16` (`a3b7f434b2dc`) |
| `openfga` | `openfga/openfga` | `v1.8` (`1e84deddc7c8`) |
| `dagster-*` | `rantai-lakehouse-dagster` | `local` (`f0e7d9e7c6e0`), built from this branch's `dagster/` |
| `lakehouse-api` | `rantai-lakehouse-api` | `local` (`b7881cf77639`) |

`.env` came from `sh ops/init-env.sh` (SEC-18: no default credentials), with
three values added for a Dagster-profiled stack and never committed:

```
DAGSTER_URL=http://dagster-webserver:3000/graphql
LAKEKEEPER_BASE_URI=http://lakekeeper:8181
LAKEHOUSE_MAINTENANCE_TOKEN=<fake, for this project only>
```

Four host ports were remapped (`CH_HTTP_HOST_PORT=18123`,
`CH_NATIVE_HOST_PORT=19000`, `RUSTFS_HOST_PORT=19010`,
`RUSTFS_CONSOLE_HOST_PORT=19011`) because another project's stack was holding
the defaults (`ops7drill-clickhouse-1 0.0.0.0:8123->8123/tcp`). Nothing
published by this stack was reached from the host except `lakehouse-api`'s
8080.

## The measurement table

`bronze.ops1_measure`, created and written only for this measurement, with the
same shape Bronze tables take (ADR 0004's `_ingested_at` day partition,
format-version 2), inside `dagster-code-location` through the `dlt` principal —
the principal `lakekeeper-authz-init` grants `create modify select` and the
bronze ingest itself writes as:

```
$ docker compose -p ops1check exec -T dagster-code-location python -   # pyiceberg
  load_catalog("x", type="rest", uri="http://lakekeeper:8181/catalog",
               warehouse="default", token=open("/tokens/dlt.jwt").read(),
               s3.endpoint="http://rustfs:9000")
  create_namespace_if_not_exists("bronze")
  create_table(("bronze", "ops1_measure"),
               schema=(_ingested_at timestamp, id long),          # both required
               partition_spec=day(_ingested_at),
               properties={"format-version": "2"})
→ namespace bronze exists: False
→ created table: ('bronze', 'ops1_measure')
→ format-version: 2
→ location: s3://lakehouse-warehouse/01a14074-8443-79c1-963d-202748cd283b
```

Two things this says:

- **The table is not created through Trino.** Lakekeeper's `trino` principal
  holds `select modify` and never `create` (`lakekeeper-authz-init`:
  `grant trino select modify`), and the `trino` service's own comment records
  the reproduced `ForbiddenException` for both `trino-maintenance` and
  `console-reader` attempting `CREATE TABLE`. That is the repo's own prior
  measurement, re-used here rather than re-run.
- **The table's location is a UUID prefix, not `bronze/ops1_measure/`.** The
  warehouse is created with no `default-prefix` or storage layout
  (`lakekeeper-warehouse-init`), so Lakekeeper assigns
  `s3://lakehouse-warehouse/<uuid>/`. "The table's metadata directory" below
  means `…/<uuid>/metadata/`, measured with
  `aws s3 ls --recursive --summarize` (`amazon/aws-cli:2.36.34`, the same
  image `rustfs-bucket-init` uses) against `http://rustfs:9000`.

## How the maintenance job was run

The existing way, used unchanged: the API route the console's pipeline page
calls and `ops/g3/g3_test.py` (the maintenance gate runner, its
`_post_trigger`) calls.

```
$ curl -b <session> -X POST \
    http://localhost:8080/api/pipelines/bronze_maintenance_job/trigger
→ {"id":"033c8b95-08c9-42d0-934a-c9786bfdd236","pipelineId":"bronze_maintenance_job",
   "status":"running","startedAt":"2026-10-16T00:01:09.259Z",…}
```

The session is the bootstrap admin's (`POST /api/auth/login`, then the forced
`POST /api/auth/change-password` rotation the seeded account requires), which
holds `pipeline:write` through the seeded `Platform Admin` role.

## Step 1 — ten snapshots

Ten single-row commits through Trino, each one a snapshot:

```
$ for i in $(seq 1 10); do
    docker compose -p ops1check exec -T trino trino --server http://trino:8080 \
      --user trino-maintenance \
      --execute "INSERT INTO iceberg.bronze.ops1_measure VALUES (current_timestamp, $i)"
  done
→ INSERT#1: INSERT: 1 row
  INSERT#2: INSERT: 1 row
  …
  INSERT#10: INSERT: 1 row

$ docker compose -p ops1check exec -T trino trino --server http://trino:8080 \
    --user trino-maintenance \
    --execute 'SELECT count(*) FROM iceberg.bronze."ops1_measure$snapshots"'
→ "10"
```

| Before any maintenance run | Snapshots | Metadata dir | Whole table prefix |
| --- | --- | --- | --- |
| 2026-10-15T16:51:32Z | 10 | 41 objects, 145,264 B | 51 objects, 148,944 B |

## Step 2 / `OPS-1-AC1` — the default, with no policy

Run `033c8b95…`, one Bronze table discovered, no policy configured:

```
$ curl -b <session> "…/api/pipelines/bronze_maintenance_job/runs/033c8b95-…/logs" | jq
→ INFO [list_bronze_tables] discovered 1 Bronze table(s): ['bronze.ops1_measure']
→ DEBUG [maintain_bronze_table[bronze_ops1_measure]] Finished execution of step … in 2.22s
→ steps: list_bronze_tables completed; maintain_bronze_table[bronze_ops1_measure] completed; summarize_bronze_maintenance completed
```

What the job wrote into `bronze_meta.maintenance_verb_run` — the recorded
outcome the spec asks for:

```
$ docker compose -p ops1check exec -T clickhouse clickhouse-client --query \
    "SELECT run_at, verb, engine, outcome, detail FROM lake.\`bronze_meta.maintenance_verb_run\`"
2026-10-16T00:01:23Z	expire_snapshots	trino	applied	default retention 7d (no snapshotsToKeep policy)
```

and into `bronze_meta.maintenance_run`:

```
2026-10-16T00:01:23Z	11	11	0	0	0        # run_at, snapshot_count, metadata_log_count, applied data/manifest/manifest-list files
```

**`applied`, with the 7-day default and no policy — AC1's outcome is met.** The
maintenance-view surface shows it too:

```
$ curl -b <session> "…/api/governance/maintenance" | jq
→ {"tableName":"bronze.ops1_measure","runAt":"2026-10-16T00:01:23Z"}
```

The ClickHouse probe still refuses, exactly as the module says, and is visible
in ClickHouse's own query log for the same run:

```
$ … system.query_log WHERE query LIKE '%icecat_maintenance%'
2026-10-16 00:01:21 ExceptionBeforeStart  ALTER TABLE icecat_maintenance.`bronze.ops1_measure` EXECUTE expire_snapshots(dry_run=1) SETTINGS …
```

### Snapshot count before and after: 10 → 11, and why

The count after the run is **11**, not 10. This measurement table grew one
snapshot between the two measurements, and it was not the maintenance job:

```
$ docker compose -p ops1check logs trino-maintenance-cron
[trino-maintenance-cron] 2026-10-15T16:43:01Z starting, interval=21600s
[trino-maintenance-cron] 2026-10-15T16:43:03Z WARNING: could not list Bronze tables (trino unavailable?): … Trino server is still initializing
[trino-maintenance-cron] 2026-10-15T22:43:05Z optimize iceberg.bronze."ops1_measure"
ALTER TABLE EXECUTE
"rewritten_data_files_count","10"
"removed_delete_files_count","0"
"added_data_files_count","1"
```

`trino-maintenance-cron` (ADR 0009's 6-hourly compaction loop, which starts
with the `trino` profile) woke six hours after the stack came up and rewrote
the measurement table — one `replace` snapshot with `deleted-data-files=10,
added-data-files=1, manifests-replaced=10` (read back from
`iceberg.bronze."ops1_measure$snapshots"`). So 10 appends + 1 optimize = 11.

The 7-day default itself expired **nothing**, which is the correct outcome for
this table: every one of those snapshots was minutes to hours old, so nothing
was older than the 7-day threshold, and `expire_snapshots('7d')` is right to
leave them all in place. AC1's "snapshot count unchanged" is therefore
measured as *unchanged by the expiry verb*, with the one intervening snapshot
accounted for above by name.

The cron was then stopped, so that no further `optimize` commit could
confound the remaining steps (`docker compose -p ops1check stop
trino-maintenance-cron` → `Container ops1check-trino-maintenance-cron-1
Stopped`).

## Step 3 / `OPS-1-AC2` — no `trino` profile

```
$ docker compose -p ops1check stop trino
→ Container ops1check-trino-1 Stopped
$ curl -b <session> -X POST …/api/pipelines/bronze_maintenance_job/trigger
→ {"id":"90e38e5a-2b07-4b4d-b828-126c2a486a1e","status":"running"}
… run completed …

$ … "SELECT run_at, verb, engine, outcome, detail FROM lake.\`bronze_meta.maintenance_verb_run\`"
2026-10-16T00:21:09Z	expire_snapshots	trino	skipped	trino unreachable

$ … "SELECT run_at, skipped_verbs FROM lake.\`bronze_meta.maintenance_run\` ORDER BY run_at DESC LIMIT 1"
2026-10-16T00:21:08Z	expire_snapshots/optimize skipped: trino unreachable; OPTIMIZE (returns OK on 26.8 but does not bin-pack — not a compaction remedy; Trino owns that, see ADR 0009)
```

| State | Snapshots | Metadata dir | Whole prefix |
| --- | --- | --- | --- |
| AC2 before (00:19:03Z) | 11 | 56 objects, 237,422 B | 67 objects, 241,558 B |
| AC2 after (00:21:41Z) | 11 | 56 objects, 237,422 B | 67 objects, 241,558 B |

Nothing was written by the run: identical counts and identical bytes, with the
Trino verb recorded `skipped` and `trino unreachable` — **AC2 is met**, and it
never pretends to have expired anything (D3). (The metadata directory grew
from 41 to 56 objects between the baseline and here — that is the same
22:43:05 `optimize` writing 11 manifests, a manifest list, a stats file and a
metadata file for its `replace` snapshot.)

## Step 4 — `snapshotsToKeep: 2`, with the floor lowered

The policy, set the same way the console sets it:

```
$ curl -b <session> -X POST …/api/lakehouse/tables/bronze/ops1_measure/maintenance \
    -H 'content-type: application/json' \
    -d '{"snapshotsToKeep":2,"orphanAgeHours":null,"compactSmallFiles":false,"schedule":null}'
→ {"namespace":"bronze","tableName":"ops1_measure","configured":true,
    "snapshotsToKeep":2,"orphanAgeHours":null,"compactSmallFiles":false,"schedule":null}
```

### Trino's 7-day floor refuses this table's threshold

`_compute_retention_threshold` keeps the 2 newest snapshots, so the threshold
is the AGE of the 2nd-newest snapshot — hours, not days. Trino's
`iceberg.expire-snapshots.min-retention` default (`7d`) refuses anything
shorter, and the job records that refusal verbatim instead of retrying:

```
$ … "SELECT run_at, verb, engine, outcome, detail FROM lake.\`bronze_meta.maintenance_verb_run\`"
2026-10-16T00:23:55Z	expire_snapshots	trino	refused	Retention specified (27363.00s) is shorter than the minimum retention configured in the system (7.00d). Minimum retention can be changed with iceberg.expire-snapshots.min-retention configuration property or iceberg.expire_snapshots_min_retention session property
```

This is the shift the module documents, measured: on a table whose history is
younger than 7 days, a keep-count policy cannot reclaim anything until the
floor is lowered. That is also why the next run lowers it.

### With `iceberg.expire-snapshots.min-retention=0s`

Through the measurement-only override
`ops/trino/docker-compose.ops1-measure.override.yml` (never by an edit to
`docker-compose.yml`):

```
$ docker compose -f docker-compose.yml \
    -f ops/trino/docker-compose.ops1-measure.override.yml \
    -p ops1check --profile trino up -d trino
$ docker compose -p ops1check exec -T trino cat /etc/trino/catalog/iceberg.properties | grep min-retention
→ 23:iceberg.expire-snapshots.min-retention=0s
$ curl -b <session> -X POST …/api/pipelines/bronze_maintenance_job/trigger
→ {"id":"64276604-da28-4a09-abdb-49044d2544b6","status":"running"}   … completed …

$ … "SELECT run_at, verb, engine, outcome, detail FROM lake.\`bronze_meta.maintenance_verb_run\`"
2026-10-16T00:27:42Z	expire_snapshots	trino	applied	retention_threshold=27589s
```

The next measurement block, 44 seconds later (00:28:26Z), reads exactly two
snapshots — the two newest:
```
$ docker compose -p ops1check exec -T trino trino --server http://trino:8080 \
    --user trino-maintenance \
    --execute 'SELECT snapshot_id, committed_at, operation FROM iceberg.bronze."ops1_measure$snapshots" ORDER BY committed_at DESC'
→ "2564423866681085990","2026-10-15 22:43:07.605 UTC","replace"      # the trino-maintenance-cron optimize
→ "7981254286018977704","2026-10-15 16:47:52.402 UTC","append"        # the 10th INSERT
```

| State | Snapshots | Metadata dir | Whole prefix |
| --- | --- | --- | --- |
| With the 0s floor, before (00:25:18Z) | 11 | 56 objects, 237,422 B | 67 objects, 241,558 B |
| With the 0s floor, after (00:28:26Z) | **2** | **39 objects, 191,622 B** | **50 objects, 195,758 B** |

**17 metadata objects and 45,800 bytes reclaimed, 9 snapshots gone, the two
newest kept** — exactly `snapshotsToKeep: 2` (OPS-1-AC3). What was reclaimed,
file by file, from the metadata directory:

| Kind | Before | After |
| --- | --- | --- |
| `.metadata.json` | 13 | 14 (the expiry's own commit adds one) |
| manifest lists (`snap-*-*.avro`) | 11 | 2 |
| manifests (`*-m*.avro`) | 21 | 21 |
| `.stats` | 11 | 2 |
| **total** | **56 objects / 237,422 B** | **39 objects / 191,622 B** |

Two things this run did NOT do, measured rather than assumed:

- **It reclaimed no data files.** The whole-prefix object count went 67 → 50 and
  the byte total dropped by exactly the 45,800 B the metadata directory lost
  (241,558 − 191,622), so nothing under `data/` was removed. Measured directly
  after the run, `aws s3 ls --recursive --summarize …/<uuid>/data/` reports
  `Total Objects: 11` / `Total Size: 4136` — the same 11 objects and 4,136 B the
  before-state allots to `data/` (67−56 objects, 241,558−237,422 B). The expired
  snapshots' Parquet files sit on as unreferenced objects; the job's own
  `remove_orphan_files` step is what reclaims those, and it runs BEFORE expiry
  in the same run, so on this measurement it had nothing to do
  (`applied_deleted_data_files=0` in all four runs above).
- **It did not change the snapshot count the job records.** The
  `bronze_meta.maintenance_run` row for this run still says
  `bronze_snapshot_count=11`, because `measure_snapshot_growth` reads the
  catalog before its own expiry step runs. The post-expiry count (2) comes
  from the reads quoted above, not from that row.

## What this decides

| Question | Answer, measured |
| --- | --- |
| Does the default run for a table with no policy, and is it recorded? | Yes — `applied`, `default retention 7d (no snapshotsToKeep policy)`, in `bronze_meta.maintenance_verb_run` at `2026-10-16T00:01:23Z` (`OPS-1-AC1`). |
| Does it refrain from acting without Trino? | Yes — `skipped` / `trino unreachable`, zero change in snapshot count and bytes (`OPS-1-AC2`, decision D3). |
| Does `snapshotsToKeep` still override the default? | Yes — `snapshotsToKeep: 2` left exactly 2 snapshots (`OPS-1-AC3`, decision D2). |
| Is the 7-day default data-loss-shaped for a young table? | No — it expired nothing on a table whose history was hours old; the reclaim was observed by lowering the floor, not by shortening the default. |
| Is the floor real? | Yes — a keep-count threshold of 27,363 s was refused with `Retention specified (27363.00s) is shorter than the minimum retention configured in the system (7.00d)`, recorded as `refused`. Lowering `iceberg.expire-snapshots.min-retention` to `0s` through an `ops/` override made the same statement `applied` (`retention_threshold=27589s`). |
| Does default expiry reclaim disk? | Snapshot METADATA only, in this measurement: 17 objects / 45,800 B. The expired snapshots' data files stayed on disk. |

Two consequences the documents should carry:

1. **A keep-count shorter than the 7-day floor cannot do anything on a young
   table** without an operator lowering Trino's floor. That is a
   deployment/product decision, and the module already refuses to make it for
   anyone — it records the refusal and never retries. The measurement proves
   both halves of that.
2. **`trino-maintenance-cron` writes to Bronze tables on a 6-hour loop** and
   commits a snapshot each time. Any snapshot-count measurement of a Bronze
   table longer than six hours has to account for that, or stop the cron, as
   this one does.

## Found but not fixed

**`GET /api/lakehouse/tables/{ns}/{table}/maintenance` cannot show the run it
just measured.** It answers:

```
$ curl -b <session> "…/api/lakehouse/tables/bronze/ops1_measure/maintenance" | jq -c .
{"configured":false,"snapshotsToKeep":null,"orphanAgeHours":null,
 "compactSmallFiles":false,"schedule":null,"lastRun":null,"lastVerbRuns":[]}
```

while the same query with the qualified name returns the row:

```
$ … "SELECT verb, engine, outcome, detail, run_at FROM lake.\`bronze_meta.maintenance_verb_run\`
       WHERE table_name = 'bronze.ops1_measure' AND run_at = (SELECT max(run_at) …)"
→ 1 row: expire_snapshots / trino / applied / default retention 7d (no snapshotsToKeep policy)
$ … the same query with table_name = 'ops1_measure'
→ 0 rows
```

Cause: the job records the catalog's qualified name (`bronze.ops1_measure`,
from `discover_bronze_tables`), while
`routes/lakehouse.rs::maintenance_verb_run_query` and
`routes/governance.rs::maintenance_run_query` filter on the bare table name
taken from the URL. ClickHouse's own query log shows the API sending the bare
form (`WHERE table_name = \'ops1_measure\'`) at 00:06:00 and 00:07:01 against
rows filed under the qualified form. Not introduced by this branch —
`git diff --stat origin/main...HEAD` lists no `rust/` file — and the aggregate
`GET /api/governance/maintenance` (the Governance maintenance view) shows the
run correctly. The per-table route's `lastRun`/`lastVerbRuns` are nevertheless
always empty for Bronze tables, which is the surface
`docs/core/features/`-style acceptance would use per table. Filed for the
planner; not fixed here, because it is outside OPS-1's scope and this branch
owns no Rust file.

## Not measured

- **The 7-day default reclaiming anything real.** No snapshot in this
  measurement was older than 7 days, so the default's only observed act is a
  correct no-op. The reclaim itself was measured through the policy path with
  the floor lowered, and that is what the numbers above show.
- **Data files reclaimed by expiry.** See above: 11 Parquet objects before and
  after, and no run removed any.
- **A Bronze table written by the real ingest.** The G3a seed (`g3a-source-init`
  → `bronze_ingest_job`) writes once and yields one snapshot, so the
  measurement table was built by hand through Trino, per OPS-1 review round 1
  BLOCKER 1.
- **The console UI.** The job was triggered and the outcomes read through the
  API the console uses; no browser was involved.
- **Lakekeeper's own expiry task queue**, the G3a demo pipeline's behaviour,
  and anything under `ops/g3/g3_test.py`'s own functional gate — out of
  scope per the plan's section 5.
- **A deployment posture.** `0s` exists only in
  `ops/trino/docker-compose.ops1-measure.override.yml`, which is never passed
  to a deployment, and whose own header says so.

The measurement table was dropped at the end
(`purge_table(("bronze", "ops1_measure"))` → `exists after drop: False`,
`tables in bronze: []`) and the project torn down with
`docker compose -p ops1check --profile dagster --profile trino down -v`.
The commands above and the logs they produced live outside the repository,
under `/home/shiro/rantai/review-logs/ops1-*`.
