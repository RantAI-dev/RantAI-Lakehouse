# P5 result — measured before building: (A) equality deletes, (B) REST catalog writes

Both of P5's stop-the-build measurements were run against a fresh, disposable
`docker compose -p p5check` stack (RustFS, Lakekeeper `0.13.3`, ClickHouse
`26.3.26.3`, Postgres 16 with `wal_level=logical`), volumes destroyed
afterward. `LAKEKEEPER_BASE_URI` was set to the in-network name
(`http://lakekeeper:8181`) for this stack — the same gotcha the task brief
warns about for `lakehouse-api`, and it manifests identically for any REST
client of Lakekeeper: leaving Lakekeeper's default (`http://localhost:8181`)
means Lakekeeper self-reports that as its own base URI, and REST clients
that honor server-returned config end up trying to reach `localhost` instead
of `lakekeeper`. This cost real debugging time and is worth restating
because Debezium Server's Iceberg sink and offset/schema-history stores are
three MORE REST-catalog clients hitting the exact same trap.

Versions: `ghcr.io/memiiso/debezium-server-iceberg@sha256:c49ebdaae01762a55
09804926710d6a831e45d56f70ce98cf69bac57cc6a6bf9` (tagged `latest` upstream —
no versioned tag is published; pin by digest, see "What this means for the
compose service" below), bundling Debezium `3.6.0.Final` and Iceberg
`1.11.0`. `quay.io/debezium/server:2.7` (the official, non-iceberg image)
was also pulled for comparison but not used — the memiiso image already
bundles a compatible Debezium core plus the Iceberg sink and all
`iceberg-*-1.11.0.jar`s needed for `io-impl=org.apache.iceberg.aws.s3.S3FileIO`
and `type=rest`, so there is no reason to hand-assemble the two images
ourselves.

## (B) Does `debezium-server-iceberg` write through the Lakekeeper REST catalog? — YES, confirmed via Lakekeeper's own API

A Postgres table (`p5_cdc.orders`, `REPLICA IDENTITY FULL`, one `CREATE
PUBLICATION`) was captured end-to-end: initial snapshot (3 rows), then an
`UPDATE`, a `DELETE`, and an `INSERT` streamed live. Debezium Server's own
commit-metrics log line for the *first* (snapshot) batch:

```
CommitReport{tableName=lakekeeper.default.p5cdc_p5_cdc_orders, ...
  addedDataFiles=1, addedEqualityDeleteFiles=1, addedEqualityDeletes=3, ...}
```

Every one of the four Iceberg tables Debezium's Iceberg sink can create
(the data table, plus — in the default configuration — an
`IcebergOffsetBackingStore` table and a `FileSchemaHistory`-equivalent
table for offsets/schema history) is a genuine REST-catalog operation, not
a local/path-based catalog: **verified by querying Lakekeeper's own
`GET /management/v1/warehouse` and `GET /catalog/v1/{warehouse}/namespaces/
default/tables` endpoints directly**, which listed `p5cdc_p5_cdc_orders`
under Lakekeeper's `default` warehouse and namespace — this is the same
check P3 (dlt) and G1 (Rust) used to rule out the "silent SQLite catalog"
trap, applied here a third time. The table's actual data landed under
`s3://lakehouse-warehouse/...` on RustFS, per Debezium's own commit log
(`Committed 3 events to table! s3://lakehouse-warehouse/01a05b96-...`).

**Configuration that got this working, and the traps along the way** (all
now baked into `docker-compose.yml`'s `debezium-server` service and
`ops/debezium/application.properties.tmpl`):

1. **Config file location.** This is a Quarkus application, not a Kafka
   Connect worker with an INI file — external config must be mounted at
   `/debezium/config/application.properties` (Quarkus's own "config beside
   the runner jar" convention), **not** `/debezium/conf/application.properties`
   (a directory that does not even exist in the image by default). Every
   Debezium Server tutorial that shows `conf/` is describing a different,
   older packaging; this memiiso image is Quarkus-native.
2. **`debezium.sink.iceberg.table-prefix` must be omitted entirely if
   empty**, not set to `""` — Debezium Server's config validation rejects
   an explicitly-empty string as "considered null by the converter" and
   refuses to start. Any optional Iceberg sink property follows this rule.
3. **The Iceberg-backed offset store and schema-history store default ON**
   whenever `debezium.sink.type=iceberg`, and by default they reuse the
   SAME REST-catalog properties as the data sink — meaning THREE Iceberg
   tables get created per connector (`_debezium_offset_storage`, a schema-
   history table, and the actual Bronze table) unless overridden. This
   build forces `debezium.source.offset.storage=
   org.apache.kafka.connect.storage.FileOffsetBackingStore` and
   `debezium.source.schema.history.internal=
   io.debezium.storage.file.history.FileSchemaHistory` (file-based, on a
   volume) so only the Bronze data table itself goes through Lakekeeper —
   avoiding two extra catalog tables per connector that have nothing to do
   with Bronze and would otherwise show up in `SHOW TABLES FROM
   icecat.bronze` alongside real data tables.
4. **`LAKEKEEPER_BASE_URI`** — see above; without it, catalog config
   fetches succeed (they hit the URI given at startup) but the client
   picks up Lakekeeper's self-reported base URI for later calls and starts
   failing with `Connection refused` to `localhost:8181` from inside a
   container that has no such listener.

### What this means for the compose service

`docker-compose.yml`'s new `debezium-server` service pins the image by
**digest**, not `:latest` — `ghcr.io/memiiso/debezium-server-iceberg` has no
versioned tag upstream (confirmed: only `latest` resolves), which is
already flagged as R4 ("community-maintained, not Debezium-official"). A
digest pin is the strongest version lock available here; bumping it is a
deliberate, reviewed action, not something that happens silently on a
`docker compose pull`.

## (A) Can ClickHouse read Iceberg merge-on-read EQUALITY deletes? — YES, with one specific, reproducible exception

Querying the same table above through ClickHouse's `DataLakeCatalog`
(`icecat.\`default.p5cdc_p5_cdc_orders\``, ClickHouse `26.3.26.3`) after the
snapshot + update + delete + insert sequence (6 physical CDC-event rows
across 2 commits, but only 4 *logical* current rows: id 1 updated, id 2
soft-deleted via a `__deleted=true` tombstone row per memiiso's
`upsert-keep-deletes=true`, id 3 unchanged, id 4 new):

| Query | Result | Correct? |
| --- | --- | --- |
| `SELECT id, customer, amount, __op, __deleted FROM ...` | Exactly 4 rows, one per id, each showing the LATEST value (id=1 → 99.99, id=2 → tombstone) | **Correct** — equality deletes were applied, superseded physical rows for id 1 and id 2 are not returned |
| `SELECT id, count() FROM ... GROUP BY id` | 4 rows, each `count()=1` | **Correct** |
| `SELECT count() FROM ... WHERE id > 0` | `4` | **Correct** — a `WHERE` clause forces the row-scan path |
| `SELECT count()` / `SELECT count(*)` / `SELECT count(id)` (no `WHERE`, no `GROUP BY`) | **`6`** | **WRONG** — counts every physical row Debezium ever wrote, including the two rows the equality-delete files mark as superseded |

This is a specific, narrow, and reproducible ClickHouse defect on 26.3, not
a wholesale failure of equality-delete support: **any query that returns
rows (`SELECT *`, `WHERE`-qualified, `GROUP BY`) correctly applies
merge-on-read equality deletes; a bare, unqualified `count()`/`count(*)`/
`count(<col>)` takes a metadata-only fast path that does not subtract
deleted rows and overcounts by exactly the superseded-row total.** This is
the same *shape* of bug class this build has hit before (G1's segfault, G3's
`OPTIMIZE` 403 — narrow, verb-specific defects in ClickHouse's Iceberg
integration, not blanket unreadability) and is corrected here rather than
assumed away, per this repo's standing rule of measuring rather than
trusting the brief.

### Consequence for the design

**Bronze CDC updates/deletes ARE readable by ClickHouse — the design is not
invalidated.** The one required workaround, recorded as an operating rule
for every row-count metric this build (or a future one) computes over a
Debezium-fed Bronze table: **never use a bare `count()`/`count(*)` against
a CDC Iceberg table; use `count() WHERE 1` (or any real predicate) or
`GROUP BY` instead.** `dagster/dispar_orchestrate/bronze_catalog.py`'s
`register_bronze_table` (`dataset_sync.total`, a row count) and any G4 test
assertion on row counts follow this rule from the start rather than
inheriting the bug silently.

## Replication slot behavior (R5), confirmed directly

```
$ psql ... -c "SELECT slot_name, active, wal_status, restart_lsn FROM pg_replication_slots;"
 slot_name  | active | wal_status | restart_lsn
------------+--------+------------+-------------
 p5cdc_slot | t      | reserved   | 0/22D3130
```

Confirms R5's premise directly: an active logical replication slot pins WAL
at `restart_lsn` (`wal_status=reserved`) for as long as the slot exists,
whether or not the consumer is caught up — exactly the "stuck slot fills
the customer's disk" failure mode P5's slot-lag/WAL-retention metrics exist
to catch. See the P5 deliverables report for the metrics surface and the
slot-cleanup-on-delete verification.
