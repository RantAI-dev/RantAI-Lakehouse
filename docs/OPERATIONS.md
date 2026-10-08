# Operations

This document covers running the backend stack locally with `docker
compose`, backing up/restoring Postgres, and the operational traps in this
codebase worth knowing about before you run it against real data.

## Dockerfile note: `rust/Dockerfile`

`docker-compose.yml` builds `lakehouse-api` from `rust/Dockerfile`. This
file used to fork into two competing versions — a committed
`rust/Dockerfile` pinned to `rust:1.85-slim` that never copied
`rust/migrations/` into the build context, and a separate
`rust/Dockerfile.api` that fixed the same clean-clone build problem —
they have since been converged onto this single file, and
`Dockerfile.api` is gone.

The old committed `rust/Dockerfile` broke a clean-clone build two ways:
`Cargo.lock` requires a toolchain new enough for `time@0.3.55` (rustc
>= 1.88 — see the `rust-toolchain.toml` `channel = "1.96.1"` pin, which
is what the workspace actually builds with), and `sqlx::migrate!
("../../migrations")` in `lakehouse-store/src/lib.rs` embeds the
migrations directory at *compile* time, so it must exist in the build
context or `cargo build` fails outright. The current `rust/Dockerfile`
pins `rust:1.96.1-slim` to match `rust-toolchain.toml`, copies
`migrations/` into the build context before the build step, installs
`sqlx-cli`, and uses `entrypoint.api.sh` to apply migrations at boot.

## Local stack: what's in `docker-compose.yml`

```
scripts/compose.sh up --build
```

brings up the stack below. `scripts/compose.sh` is `docker compose` with
`GIT_SHA` set from the checkout: the API and the Dagster images must be
built with the same `GIT_SHA`, or the pipeline page cannot show op source
(`pipeline_source.rs::check_commit` refuses rather than show a file that
may not be the code that ran). A plain `docker compose` works, but builds
with `GIT_SHA=unknown`.

The stack:

| Service | Image | Purpose |
| --- | --- | --- |
| `postgres` | `postgres:16` | OLTP store (identity, governance, pipelines, connectors, alerts, ...) |
| `clickhouse` | `clickhouse/clickhouse-server:26.8` | Analytics store (catalog, overview, governance, dashboards, ...); also the Iceberg query engine once `DataLakeCatalog` is wired up in P1b |
| `lakehouse-api` | built from `rust/Dockerfile` | the axum API, port 8080 |
| `rustfs` | `rustfs/rustfs:1.0.0-rc.4` | S3-compatible object store for the lakehouse warehouse (P1 infrastructure; not yet wired into `lakehouse-api`) |
| `rustfs-bucket-init` | `amazon/aws-cli:2.36.34` | One-shot: creates the warehouse bucket via the plain S3 API (`s3api create-bucket`) — never RustFS's admin API |
| `lakekeeper-db-init` | `postgres:16` | One-shot: creates Lakekeeper's own database on the existing `postgres` service |
| `lakekeeper-migrate` | `quay.io/lakekeeper/catalog:v0.13.3` | One-shot: Lakekeeper's own `migrate` subcommand against its database |
| `lakekeeper` | `quay.io/lakekeeper/catalog:v0.13.3` | Iceberg REST catalog (Rust, Apache-2.0); the only path for Iceberg writes — no path-based `IcebergS3` tables |
| `lakekeeper-warehouse-init` | `alpine:3.20` | One-shot (P1b): bootstraps the Lakekeeper server (`/management/v1/bootstrap`) and creates the `LAKEKEEPER_WAREHOUSE` warehouse against the RustFS bucket, with `sts-enabled: true` — required for Lakekeeper to vend real S3 credentials on `X-Iceberg-Access-Delegation: vended-credentials`; authenticates as the `admin` principal under R1 (see `docker-compose.yml`'s comment on this service and `lakehouse-iceberg::catalog`'s module doc) |
| `openfga-db-init` / `openfga-migrate` / `openfga` | `openfga/openfga:v1.8` | R1: Lakekeeper's authorization backend — own Postgres database, `openfga migrate` schema, `openfga run` (HTTP :8080, gRPC :8081). Distroless (no shell/curl), so it has no `healthcheck:` of its own — see `openfga-ready`, next |
| `openfga-ready` | `curlimages/curl:8.10.1` | R1: one-shot that polls `openfga`'s `/healthz` from a real shell image (see above) — everything downstream depends on THIS completing, not on `openfga` reporting healthy |
| `oidc-mock` | built from `ops/oidc-mock` | R1: mock OIDC discovery/JWKS/token issuer — Lakekeeper's only generic authentication mechanism is OIDC bearer tokens, and this stack has no other identity provider. Mints one long-lived token per principal at boot; **not a production identity provider** — see ADR 0011 |
| `lakekeeper-authz-init` (+ `-seaweedfs`) | `alpine:3.20` | R1: one-shot — self-registers every non-admin principal with Lakekeeper, then grants each the warehouse-scoped relations it needs (`docker-compose.yml`'s comment on this service has the full table; ADR 0011 has the rationale) |

Both data stores have healthchecks; `lakehouse-api` waits for both to
report healthy (`depends_on: condition: service_healthy`) before starting,
so first-boot migrations and the bootstrap-admin seed (both of which need
Postgres) get a real chance to run instead of racing container startup.
This is a start-order convenience, not a hard runtime dependency — see
"Postgres-down is quiet," below.

`rustfs`, `lakekeeper`, and their bootstrap jobs are **still not called
from any `lakehouse-api` route as of P6** — see the module map in
`docs/ARCHITECTURE.md`. The console surfaces Bronze by reading
`bronze_meta.*`/ClickHouse's `DataLakeCatalog`, not `lakehouse-iceberg`
directly. Neither RustFS nor Lakekeeper failing affects `lakehouse-api`'s
own boot or its Postgres/ClickHouse-backed routes.

`g1-test-runner`, `g3a-test-runner`, `g3-maintenance-test-runner`, and
`g4-test-runner` (plus their one-shot `*-source-init` companions) are CI
gate harnesses, not part of the running product — each proves one gate
(G1/G3a/G3/G4) from a clean stack and is invoked explicitly by name (`docker
compose run --rm <name>`), never by a plain `docker compose up`. See
`docs/plans/*-RESULT.md` for what each gate measured.

### ClickHouse 24.8 → 26.3 → 26.8: what changed and what was verified

Bumped to 26.3 because Iceberg reads via ClickHouse's `DataLakeCatalog`
need `clickhouse-server >= 26.2` (for `allow_database_iceberg`), then to
**26.8 LTS**, which materially changes what the Iceberg write and
maintenance paths can do — see
[`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md`](plans/CLICKHOUSE-26.8-REMEASUREMENT.md)
for every finding re-run on 26.8, including the one that BREAKS the P4
maintenance job as originally written (`expire_snapshots` is now refused
for catalog-backed tables). Verified clean on the 26.3 bump, and the same
checks still hold on 26.8:

- All 7 `demo/clickhouse/*.sql` files (`01_databases.sql` through
  `07_meta.sql`) apply without error against a fresh 26.3 container —
  `clickhouse-client --multiquery < demo/clickhouse/NN_*.sql` for each,
  exit code 0, no DDL/DML syntax breakage across the version jump.
- The documented password-less local-dev login
  (`CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1` + empty `CLICKHOUSE_PASSWORD`)
  still works over the HTTP interface in 26.3 — `curl --user "default:"`
  against a **freshly created** container returns `200`. (During
  verification, a botched `docker restart` left a stale server process
  holding the listening ports while a second process failed to bind
  alongside it — that stale process gave misleading `REQUIRED_PASSWORD`
  errors that had nothing to do with the version bump. Recreating the
  container cleanly resolved it. If you ever see `REQUIRED_PASSWORD` with
  an empty `CH_PASSWORD` locally, recreate the container — `docker compose
  up -d --force-recreate clickhouse` — rather than `docker restart`.)
- `SET allow_database_iceberg = 1` is accepted (`allow_database_iceberg`
  is present in `system.settings` in 26.3).
- `cargo test --all-features` (see below) passes unchanged — no test in
  the workspace runs a live ClickHouse via testcontainers;
  `lakehouse-api`'s route tests point `CH_URL` at a dead upstream on
  purpose (`rust/crates/lakehouse-api/tests/common/mod.rs`), so response-
  shape assertions are exercised against fixtures/mocks, not a real
  server, and were unaffected by the bump.

Create `.env` first (`docker compose` auto-loads `.env` from the project
root) with `sh ops/init-env.sh`, which copies `.env.example` and fills the
secrets with generated values (see "No default credentials" below), then set
`AUTH_BOOTSTRAP_EMAIL`. Every non-secret variable has a safe local default.

### No default credentials (`SEC-18`)

`docker-compose.yml` carries no credential. These variables are must-set
(`${X:?}`): `docker compose` of any kind stops with a message naming the
variable while one is empty, whatever profile is active:

| Variable | What it is |
| --- | --- |
| `POSTGRES_PASSWORD` | PostgreSQL password (also inside the connection strings compose builds for the API, Lakekeeper and OpenFGA) |
| `RUSTFS_ACCESS_KEY` / `RUSTFS_SECRET_KEY` | RustFS root key pair |
| `LAKEKEEPER_ENCRYPTION_KEY` | Encrypts what Lakekeeper stores in its own schema |

`CONNECTOR_PG_PASSWORD` and `CONNECTOR_S3_ACCESS_KEY` / `_SECRET_KEY` fall
back to those, never to a literal. `SEAWEEDFS_ACCESS_KEY` /
`SEAWEEDFS_SECRET_KEY` are used only by `--profile seaweedfs`; compose
interpolates the whole file whatever profiles are active, so `${X:?}` there
would stop a default `up` that never starts SeaweedFS. They default to
empty and `seaweedfs-iam-init` (which the whole profile waits on) exits 1
while either is empty. `AUTH_BOOTSTRAP_PASSWORD` empty means no admin is
seeded. `.env.example` holds none of these values, so copying it as it is
does not start the stack; `sh ops/init-env.sh` writes a `.env` with generated
values (mode 600, refuses to overwrite an existing `.env`, prints variable
names only). Anything that runs `docker compose` without a `.env` (a CI job,
a gate, `docker compose --profile '*' config --quiet`) must export these
variables first; the CI jobs write CI-only throwaway values.

**Upgrading an install that relied on the old defaults.** Its data was
created with the old values, so keep them first and rotate on purpose:

1. Before pulling this change, put the values the stack runs with into
   `.env`: `POSTGRES_PASSWORD=lakehouse`, `RUSTFS_ACCESS_KEY=rustfsadmin`,
   `RUSTFS_SECRET_KEY=rustfsadmin`,
   `LAKEKEEPER_ENCRYPTION_KEY=this-is-not-a-secure-key-change-me` (and the
   SeaweedFS pair `seaweedfsadmin` if you use that profile). Do not run
   `ops/init-env.sh` over an existing stack: new values do not match the
   existing volumes (PostgreSQL reads its password only when the data
   directory is first created; a new Lakekeeper key cannot decrypt what is
   stored).
2. `docker compose up -d` as before. Nothing is recreated with a new value.
3. Rotate afterwards, one at a time: change the PostgreSQL role password
   inside the database (`ALTER ROLE ... PASSWORD`) and then `.env`; for the
   object store, change the key in RustFS and `.env` together; the
   Lakekeeper encryption key is not rotated in place (re-register the
   warehouse storage credentials after changing it). Rotation is not done
   by this change.

### RustFS: failure mode

RustFS is the default self-hosted S3-compatible object store (MinIO is
explicitly not a supported target). If it's down: the `rustfs-bucket-init`
one-shot job fails and the warehouse bucket is never created (surfaces as
a non-zero exit / crash-loop on that container, visible in `docker compose
ps`); once P1b wires up `lakehouse-iceberg`, Iceberg reads/writes fail
closed with connection errors from the `object_store` client. It does not
affect `lakehouse-api`'s own boot — nothing in this phase makes RustFS a
hard startup dependency for the API.

### Lakekeeper: failure mode

If Lakekeeper is down: its own REST endpoints (`/catalog/v1/*`,
`/management/v1/*`) are unreachable, so once P1b wires up
`lakehouse-iceberg`, every Iceberg catalog operation (create table, commit
snapshot, list tables) and any ClickHouse `DataLakeCatalog` database
pointed at it fail closed. It does not affect `lakehouse-api`'s own boot
or its existing Postgres/ClickHouse-backed routes, which have no
dependency on Lakekeeper in this phase.

### OpenFGA: failure mode

OpenFGA is Lakekeeper's authorization-relation store (ADR 0011) — core,
not profile-gated. If it's down (or `openfga-migrate`/`openfga-ready`
never complete): `lakekeeper-migrate` cannot write Lakekeeper's own
authorization model into the store and fails outright, so `lakekeeper`
itself never becomes healthy (it depends on `openfga-ready`, not just
`openfga` reporting started) and nothing downstream of it — every
`lakekeeper-*-init` job, every Bronze writer, ClickHouse's and Trino's
Iceberg read/write paths — starts either. This is a hard, visible failure
at bring-up, not a degraded mode: a plain `docker compose up` will not
finish coming up with OpenFGA down. Once the stack IS up, OpenFGA going
down mid-run makes every *new* authorization check Lakekeeper performs
fail (existing, already-open connections and already-cached decisions are
unaffected until Lakekeeper needs to check again).

`openfga` publishes no host port by default (PR #33 review, blocker 2) —
it has no authentication of its own, and its Management-API-shaped grant
interface is meant to be reached through Lakekeeper's own
`/management/v1/permissions/...`, not directly. If you need to inspect it
directly for debugging, use `docker compose exec` or attach a one-off
container to the compose network rather than republishing
`OPENFGA_HTTP_HOST_PORT`/`OPENFGA_GRPC_HOST_PORT`.

### oidc-mock: failure mode

`ops/oidc-mock` is the mock OIDC issuer every principal in this stack
authenticates through (ADR 0011) — also core, not profile-gated. If it's
down: `lakekeeper` never becomes healthy (it depends on `oidc-mock`
reporting healthy, since `LAKEKEEPER__OPENID_PROVIDER_URI` points at it
and Lakekeeper needs its discovery/JWKS documents to validate any bearer
token at all), so the same downstream failure as OpenFGA-down applies.
Once up, if `oidc-mock` goes down mid-run: every pre-minted token already
on the `lakehouse_oidc_tokens` volume keeps working (Lakekeeper validates
signatures against JWKS it already fetched; nothing re-fetches per
request), but ClickHouse's `oauth_server_uri` calls to `/token` start
failing, which breaks ClickHouse's `DataLakeCatalog` Iceberg read path the
next time it needs a fresh token — this is the one caller in this build
that actually depends on `oidc-mock` staying up, not just having started
once (see ADR 0011's "what a real IdP swap has to account for" for why
this is a real gap, not just a mock-specific one).

`oidc-mock` publishes no host port by default (PR #33 review, blockers 1
and 2) — it holds the private key that everything else's token
verification trusts, and (before this review) its `/token` endpoint could
mint an admin-bypass token for anyone who could reach it with no
credential. Reach it via `docker compose exec` or a container on the
compose network for debugging, not by republishing `OIDC_MOCK_HOST_PORT`.

### SeaweedFS (opt-in, P2 — storage-compatibility matrix only)

`seaweedfs`, `seaweedfs-bucket-init`, and
`lakekeeper-warehouse-init-seaweedfs` are the P2 proof that the RustFS
dependency is a genuine boundary, not an assumption: the exact same G1
integration suite runs against SeaweedFS by env/config change only, no
code diff (`docs/STORAGE-COMPATIBILITY.md`). None of these three services
start on a plain `docker compose up` — they exist purely so the matrix can
be re-run; RustFS remains the default target for every other profile
(`dagster`, `trino`). **Failure mode:** if SeaweedFS is down while its
services are the active target, the same failure shape as RustFS applies —
`seaweedfs-bucket-init` fails to create the warehouse bucket, and any
Iceberg client pointed at it (currently only the G2 test runner; no
`lakehouse-api` route uses either object store directly as of P6) gets
connection errors from `object_store`. It does not affect
`lakehouse-api`'s own boot.

### Trino-as-cron (opt-in, `trino` profile — P4, ADR 0009)

`trino` (a single-node coordinator with exactly one catalog, `iceberg`,
pointed at the same Lakekeeper/RustFS-or-SeaweedFS backend every other
Bronze consumer uses) and `trino-maintenance-cron` (a loop running `ALTER
TABLE iceberg.bronze."<table>" EXECUTE optimize` against every Bronze table
on a `TRINO_CRON_INTERVAL_SECONDS` cadence, default 6h). Added because
measurement showed **zero working in-engine small-file compaction exists
on ClickHouse 26.3** — `remove_orphan_files` doesn't exist for Iceberg
tables and `OPTIMIZE` fails at runtime with an HTTP 403 against a
catalog-registered table (`docs/plans/G3-RESULT.md`). Neither service
starts on a plain `docker compose up`; both are behind the `trino` profile,
matching every other opt-in profile in this stack (`dagster`, `seaweedfs`).
**Failure mode:** if `trino`/`trino-maintenance-cron` are down (or the
`trino` profile is simply never enabled), Bronze small-file compaction does
not happen at all — files accumulate unbounded from CDC/dlt writes, and
query planning time over Bronze degrades (measured ~15-20x at a 20-file/
partition synthetic load vs. a 1-file/partition control). This is a real
operational requirement, not a nicety, for any deployment taking CDC-rate
or dlt-batch writes into Bronze at meaningful volume — see ADR 0009. It
does not affect `lakehouse-api`'s own boot or any of its existing routes;
`dagster/dispar_orchestrate/maintenance.py`'s `expire_snapshots` chain is
independent of Trino and keeps running either way (it does not compact
data files, only aged snapshot/manifest metadata).

### Debezium Server (opt-in, `dagster` profile — P5, CDC)

`debezium-server`, pinned by **digest** (not `:latest` — no versioned tag
is published upstream for `ghcr.io/memiiso/debezium-server-iceberg`, see R4
in the risk register). Captures Postgres logical-replication changes
(initial snapshot, then streaming; ADR 0008) and writes them into Bronze
Iceberg through the same Lakekeeper REST catalog every other writer uses —
upsert mode with merge-on-read equality deletes. Config is rendered from
the connector registry (ADR 0007) into
`ops/debezium/application.properties.tmpl`, mounted at
`/debezium/config/application.properties` (this is a Quarkus app, not a
classic Kafka Connect worker — `conf/` is the wrong path and does not exist
in the image). Needs `postgres` running with `wal_level=logical` (already
the compose default, unconditionally, not profile-gated) and a replication
slot on the source (see `g4-source-init`/`ops/debezium/
deprovision_connector.sh` for provision/deprovision). **Failure mode:** if
`debezium-server` is down, CDC simply stops flowing — no new rows land in
the affected Bronze tables — while its Postgres replication slot keeps
existing and pinning WAL at its last `restart_lsn` regardless (R5); this is
exactly why slot-lag/WAL-retention are first-class metrics
(`dagster/dispar_orchestrate/replication_metrics.py`, surfaced via `GET
/api/governance/replication`, console page Governance → "Ingestion
(CDC)") rather than something only discovered when the source disk fills
up. Does not affect `lakehouse-api`'s own boot.

**DNS gotcha found during verification:** on a host whose own
`/etc/resolv.conf` carries a DNS search domain (VPN/corporate DNS,
Tailscale MagicDNS, etc.), that suffix leaks into every container's
`resolv.conf`, including `lakekeeper`'s. Lakekeeper's Rust DNS resolver
does not fall back to the bare name the way glibc-based images (e.g. the
`postgres:16` image used by `lakekeeper-db-init`/`lakekeeper-migrate`) do,
so `postgres` gets expanded to `postgres.<search-suffix>`, NXDOMAINs, and
the `lakekeeper` container exits immediately with `error communicating
with database: failed to lookup address information: Temporary failure in
name resolution` — then restart-loops. `docker-compose.yml` sets
`dns_search: ["."]` on the `lakekeeper` service specifically to prevent
search-domain expansion; this was reproduced and confirmed as the fix
during P1a verification.

### Dagster (opt-in, P3)

Behind the `dagster` compose profile — never starts on a plain `docker
compose up`. Brings up `dagster-db-init` (one-shot: creates Dagster's own
`dagster` database on the existing `postgres` service), `dagster-code-location`
(a gRPC server serving `dispar_orchestrate.definitions`, built from
`dagster/Dockerfile` — see ADR 0005), `dagster-webserver` (GraphQL API +
UI, port `3000`), and `dagster-daemon` (schedules/sensors/run queueing).

Put `DAGSTER_URL` in `.env` — do **not** rely on prefixing it onto a single
command:

```bash
# .env
DAGSTER_URL=http://dagster-webserver:3000/graphql
```
```bash
scripts/compose.sh --profile dagster up -d --build \
  lakehouse-api dagster-code-location dagster-webserver dagster-daemon
```

`lakehouse-api`'s own default (`http://dagster.invalid:13030/graphql`) is a
deliberately-unreachable placeholder (see the service definition's comment),
so pipeline routes stay `503` unless an operator opts in explicitly.

**Why `.env` and not a one-off `VAR=… docker compose up`:** a later
`docker compose run` (for example the `g3a-test-runner`) re-creates
`lakehouse-api` as part of resolving its `depends_on` chain. A value that
existed only in the environment of the earlier `up` invocation is not
present for that re-creation, so the container comes back on the
`dagster.invalid` default and every Dagster-backed route silently reverts to
`503`. This was observed as `GET /api/governance/audit` returning
`503 {"error":"Error: fetch failed"}` mid-way through an otherwise-passing
G3a run — the ingest itself succeeded, so the symptom appears only on the
routes that reach Dagster, not on the ones that read ClickHouse.

**Failure mode:** if `dagster-webserver`/`dagster-code-location` are down,
`lakehouse-dagster::DgClient` calls fail exactly the way they already do
when `DAGSTER_URL` points nowhere — `GET /api/pipelines` returns `503`,
`POST /api/pipelines/{id}/trigger` returns `503`. Nothing in this phase
makes Dagster a hard dependency for `lakehouse-api`'s own boot.

**The G3a acceptance test** (`ops/g3a/g3a_test.py`, `docs/plans/
LAKEHOUSE-FOUNDATION-PLAN.md` §3) runs inside the compose network via the
`g3a-test-runner` service, the same reason `g1-test-runner` does: the dlt
pipeline (`dagster/dispar_orchestrate/dlt_pipeline.py`) resolves
`rustfs`/`lakekeeper` by their compose-internal names, which only resolve
from inside this network.

```bash
# From a clean stack, project name your own scratch value:
DAGSTER_URL=http://dagster-webserver:3000/graphql \
  docker compose -p <project> --profile dagster up -d --build \
    lakehouse-api dagster-code-location dagster-webserver dagster-daemon \
    g3a-source-init
docker compose -p <project> --profile dagster run --rm g3a-test-runner
docker compose -p <project> down -v   # tear down when done
```

**`LAKEKEEPER_BASE_URI` gotcha, specific to dlt/pyiceberg (not just
ClickHouse's DNS quirk above).** pyiceberg's `RestCatalog` honors the
canonical catalog URI Lakekeeper reports in its own `/v1/config` response
(driven by `LAKEKEEPER__BASE_URI`) for every subsequent call. If
`LAKEKEEPER_BASE_URI` is left at its host-facing default
(`http://localhost:8181`), a catalog client running **inside** the
compose network (the dlt pipeline, in `dagster-code-location`) times out
resolving/connecting to that address — this was reproduced directly
during P3 verification. `docker-compose.yml`'s own defaults are
unaffected in the default (non-`dagster`) stack; when bringing up the
`dagster` profile, set `LAKEKEEPER_BASE_URI=http://lakekeeper:8181`
(matching the compose-internal name), exactly as `.github/workflows/
ci.yml`'s `g1-rustfs`/`g2-seaweedfs`/`g3a-dagster` jobs already do for
the same underlying reason.

### Pipeline-failure alerts (plan 1e)

`dagster/dispar_orchestrate/pipeline_events.py`'s `run_failure_sensor`
fires on every failed run in this code location (including
`alerts_run_job`, `bronze_ingest_job`, etc.) and POSTs
`{"runId": "...", "jobName": "authored__<id>"}` to
`POST /api/pipelines/events/run-failed`. The handler resolves
`jobName` through the runnable-pipeline list, asks Dagster to confirm
`status == "FAILURE"`, dedupes by `(run_id, kind="failure")` in the new
`pipeline_run_event` table (migration `0049`), then evaluates
`pipeline_failure` alert rules — each scoped to one pipeline id or `*`.

**Both `lakehouse-api` and the Dagster code location must have
`PIPELINE_RUN_TOKEN` set to the same value.** Without it the sensor
posts nothing and no failure alert fires (degraded-honest: the sensor
logs through `context.log.info` instead). Without it on the API side
the orchestrator service identity is never seeded, and the handler
refuses the request as unauthorized. This is the same
`PIPELINE_RUN_TOKEN` gating `GET /api/pipelines/runnable` for
`authored_factory.py` (already required for the dagster profile).
`docker-compose.yml`'s `dagster` profile passes the same `${PIPELINE_RUN_TOKEN:?}`
to both services — leaving it unset in `.env` makes the stack refuse to start.

**Dedup guarantees** rest on the `(run_id, kind)` primary key in
`pipeline_run_event`: a sensor retry sees `false` from
`record_pipeline_run_event` and the handler short-circuits with
`{"matched": 0, "reason": "run already alerted; sensor retry ignored"}`,
so no rule fires twice. The same table is reused by the kinds plan 1f
adds (`slow`, `volume_drop`, `late`); `record_pipeline_run_event` is
the only writer for them.

### Pipeline SLA, Late, and Volume-drop alerts (plan 1f)

`dagster/dispar_orchestrate/pipeline_events.py`'s `run_status_sensor`
(filtered to `DagsterRunStatus.SUCCESS`, `default_status=RUNNING`,
named `pipeline_run_finished_sensor`) fires on every successful run
in this code location and POSTs the same `{"runId": ..., "jobName":
...}` shape to `POST /api/pipelines/events/run-finished`. The handler
resolves `jobName` through the runnable-pipeline list, asks Dagster
to confirm `status == "SUCCESS"` and to return the run's
`endTime`, dedupes by `(run_id, kind="slow")` / `(run_id,
kind="volume_drop")`, then computes:

* **slow** — `overDuration(run.duration_seconds, sla.max_duration_seconds)`.
  `None` when the SLA is unset or the run is still in flight; `true`
  only when the duration is strictly over `maxDurationSeconds`. Each
  pipeline's runs are exposed as `overDuration` on the run payload.
* **volume_drop** — for each completed run, look at the median
  `materialization_rows` across the previous 5+ completed runs of the
  same job. `None` when the prior sample is below 5 (the rule is
  `skipped`, never fired or silent). `true` when the current rows are
  strictly under half the median.

Each rule of kind `pipeline_slow` / `pipeline_volume_drop` with
`pipeline = <id>` (or `*` for the wildcard rule) is then evaluated,
with `record_pipeline_run_event` doing the same `(run_id, kind)`
dedupe as the failure alert above.

**Per-pipeline SLA** lives in the new `pipeline_sla` table (migration
`0050`), one row per `pipeline_id`, with optional
`max_duration_seconds` and `late_after_seconds`. The `PUT
/api/pipelines/{id}/sla` route is the only writer; `GET` returns the
row or `404` (so the UI can distinguish "no SLA yet" from a degraded
backend). Both fields are validated positive at the handler (the
database CHECK is defense in depth). Each pipeline row's payload
gains an `sla` block (the full record with `null` for unset
thresholds) and a `slaOk` flag derived from `overDuration` on the
latest run.

**Late pipeline alerts** are evaluated inside the `/api/alerts/run`
op (not the sensor path) because "now" depends on the clock at the
moment of evaluation, not at run time. For each `pipeline_late` rule
whose `pipeline` matches `pipeline_id`, the op loads the SLA's
`late_after_seconds` and the pipeline's last successful run time, and
fires `true` only when the gap is strictly over the threshold. With
no SLA or no last success, the rule is `unsupported` and is recorded
in the alerts run's per-rule output. Like the run-finished path,
`record_pipeline_run_event` dedupes by `(run_id, kind="late")` — but
"run id" here is a per-rule invariant for now (we always pass the
latest known run id), so dedupe is conservative; future work may
attach a clock-based key.

### Pipeline dependencies (plan 2a)

An authored pipeline can list upstreams in its `dependsOn` field on create
and update. Each entry must be an existing authored pipeline id or a
`Dagster` job name the orchestrator currently lists (for example
`ingest_job`); the list holds at most 10 entries. A self-reference or a
cycle across authored pipelines is refused with `400`, and the message
names the offending id. `Dagster` jobs are excluded from the cycle walk —
they declare no `dependsOn`, so they cannot close one.

Semantics are **ALL**, not any: the sensor fires after a `SUCCESS` run of
one upstream, but the downstream is requested only when *every* upstream
has a `SUCCESS` run that finished after the downstream's own most-recent
run started. A downstream whose latest run is `QUEUED` or `NOT_STARTED`
is NOT the same as "no run yet" -- the queued run is the previous chain
launch still in flight, so the sensor waits for it to actually start before
firing again (no double-launch). A downstream with NO run history yet
(first-ever chain tick) does fire on a fresh upstream success, but the
ALL check STILL runs -- an upstream that has never succeeded is
`SkipReason`-named, never silently bypassed. Each downstream gets a
run-status sensor named `authored__<id>_after`, `default_status=RUNNING`
so a chain never ships silently stopped, and its `run_key` is the SORTED
tuple of every upstream's latest SUCCESS run id
(`"authored-deps:run-a-1,run-b-1"`), so two ticks that see the same set
of upstream successes ask the daemon for the same downstream launch
exactly once (the daemon's own dedup keeps the second tick from
launching a second downstream run). A `paused` pipeline contributes NO
sensor; `routes::pipelines::authored_status` also stops the stored
sensor next to the schedule when pausing a chained pipeline, so the
chain truly goes quiet (Dagster keeps sensor state across a reload).

**Where the skip reason shows.** `GET /api/pipelines/{id}/schedule-ticks`
merges the pipeline's schedule ticks with the `authored__<id>_after`
sensor's ticks. Every tick carries `kind: "schedule"` or `kind: "sensor"`;
a sensor tick that did not launch carries the `SkipReason`, naming the
upstream that is still behind. This is the surface for "why hasn't my
downstream run."

**When it takes effect.** Editing `dependsOn` is picked up when the code
location reloads — the authored update asks the orchestrator to reload, and
the factory rebuilds the sensors from `GET /api/pipelines/runnable`. A
draft has no job and is not rebuilt; its chain arms when it goes `ready`.

**Editing the chain (PR #57 review F1.8).** A save that does NOT include
`dependsOn` keeps the stored chain (the write uses `COALESCE`); an
explicit `dependsOn: []` clears it. Pre-fix every save wrote `[]`,
erasing every author-wired upstream in one round trip. The route-level
pair (`routes::authored_pipelines::update`) proves both directions
against a real `sqlx::test` Postgres.

### Uploaded files (DATA-9, ADR 0014)

A person uploads a CSV or TSV file from the console (`/connectors/upload`);
the API stores it, and a Dagster run (`file_ingest_job`) loads it into a raw
Iceberg table. What that asks of the operator:

- **The files are kept and they grow the bucket.** Each original file is
  stored in the warehouse bucket (`LAKEHOUSE_WAREHOUSE_BUCKET`, default
  `lakehouse-warehouse`) under `uploads/<tenant id>/<upload id>`, with a short
  extension when the file name had one. It stays until someone deletes the
  upload from "Uploaded files"; deleting an upload removes the file and its
  entry, never the table it became. Nothing else removes them: there is no
  expiry and no quota, so storage grows with every upload (up to 50 MB each).
- **The API needs a storage credential, and nothing else.** On the compose
  stack the `lakehouse-api` service passes the three settings the upload
  store and the RustFS health probe read. You set two: `UPLOAD_S3_ACCESS_KEY`
  and `UPLOAD_S3_SECRET_KEY`, dedicated names that are never RustFS's own
  `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY` (ADR 0002 Addendum 2) and have no
  default. `CH_RUSTFS_S3_ENDPOINT` (default `http://rustfs:9000`),
  `RUSTFS_ACCESS_KEY_SECRET_REF` (default `env:UPLOAD_S3_ACCESS_KEY`) and
  `RUSTFS_SECRET_KEY_SECRET_REF` (default `env:UPLOAD_S3_SECRET_KEY`) only
  need setting to override those defaults. Left empty, `POST /api/uploads`
  answers 503 "Upload storage is not configured.", nothing is stored, and
  the RustFS health tile reads "unknown". The API container takes its
  endpoint from `CH_RUSTFS_S3_ENDPOINT`, not from the host-facing
  `RUSTFS_S3_ENDPOINT` of `.env`.
  Least privilege: the credential needs read, write and delete on the
  warehouse bucket's `uploads/` prefix and nothing else of the API's; where
  your store supports per-prefix policies, restrict it to that prefix, and
  otherwise to the warehouse bucket. The RustFS health probe lists the
  bucket root (`client.list_with_delimiter(None)`), so a credential limited
  to `uploads/` can store files but would show that tile as failing; that is
  the cost of the narrower grant, not a fault. Whether the pinned RustFS can mint
  such an identity is the open question the `CONNECTOR_S3_*` note in
  `.env.example` records; for a throwaway local stack you may set the two
  to the same value as `RUSTFS_ACCESS_KEY`/`RUSTFS_SECRET_KEY`. A
  `docker compose up` with these settings was not run for this document.
- **`ICEBERG_QUERY_DB` must be set**, because a load into a table name that
  does not exist yet first asks ClickHouse whether the name is free. Compose
  defaults it to `icecat_api`. Unset, the load is refused with 503 "Uploads
  need ICEBERG_QUERY_DB to be set, so the API can check that a table name is
  free. Nothing was loaded."
- **The load needs the orchestrator.** `file_ingest_job` is registered in
  the code location (`dispar_orchestrate.definitions`), which runs under the
  `dagster` compose profile (see "Dagster (opt-in, P3)"). With the orchestrator
  down, "Load" answers 503 "The orchestrator could not be reached, so the load
  was not started." The job reads the stored file and writes the table
  with the settings the code location already has for Bronze ingest; uploads
  add no setting to `docker-compose.yml`, `.env.example` or `config.rs`
  (`git diff 98aaa64 -- docker-compose.yml .env.example
  rust/crates/lakehouse-api/src/config.rs` is empty on this branch).
- **A reverse proxy in front of the console must allow a request body of at
  least 51 MB** (the 50 MB file plus the multipart form around it; the API
  accepts 50 MiB + 1 MiB on this one route and keeps axum's 2 MB default on
  every other). `POST /api/uploads` has a five-minute request deadline, a
  bound for a slow link and not a measurement. A 50 MB upload through the
  console's `/api` rewrite and through a proxy was **not verified**.
- **Who can use it.** `connector:manage` on every upload route (Data Engineer,
  Platform Admin). No role was added or changed. In an install with several
  tenants, an upload and its table name belong to the uploader's tenant; the
  table appears in the shared catalog, which only the tenant that owns the
  catalog sees.

### Schema versions of Silver and Gold tables (ADR 0015)

A raw table's page shows its schema versions from its Iceberg metadata. The
analytics engine keeps only a table's current columns, so for a table in
`silver` or `serving` (the two databases the catalog serves) the API records
the versions itself, in a `ClickHouse` table it creates on first use:

- **`console.table_schema_version`** holds `table_key` (`<database>.<table>`),
  `version` (from 1), `columns` (a JSON array of `[name, type]` pairs in the
  table's column order) and `observed_at` (`DateTime64(3, 'UTC')`), as a
  `MergeTree` ordered by `(table_key, version)`. The DDL is owned by
  `rust/crates/lakehouse-api/src/routes/schema_versions.rs`, like
  `console.quality_run`; there is no `PostgreSQL` migration, and nothing
  prunes it. A row is added only when a table's ordered `(name, type)` list
  differs from the last one recorded for it.
- **When it is recorded.** One pass reads every table's columns in `silver`
  and in `serving` and compares them with the newest recorded list of each;
  it is two reads and, when something changed, one `INSERT`. A pass runs when the API starts, on `POST /api/alerts/run`
  (the orchestrator's `alerts_run_schedule`, every 15 minutes; the answer
  carries `schemaPassStarted`), and when the orchestrator reports a finished
  run (`POST /api/pipelines/events/run-finished`, from Dagster's
  `pipeline_run_finished_sensor`, which needs `PIPELINE_RUN_TOKEN`). **An
  install without the orchestrator records versions only at start-up**: a
  table that changes later is not noticed until the API restarts. One pass
  runs at a time; a start while one runs is refused, not queued.
- **A failed pass changes nothing.** If the engine is not reachable (it may
  not be yet when the API starts) the log line `schema versions: pass failed`
  says so and the next trigger tries again. An answer from the engine that is
  not a result (a `200` whose body is not `FORMAT JSON`, which `ClickHouse`
  can send before failing mid-stream) counts as a failed read, never as an
  empty store. The asset page reads the store only: a store that does not
  exist yet, or any failed read, shows as "No schema version recorded yet".
- **One API per engine is assumed.** One pass runs at a time within an API
  process, not across processes: two APIs against one engine could each
  record the same number for a table. The page folds two equal consecutive
  versions into one.
- **The page reads every version of a table, with no cap.** A table whose
  columns differ at every look adds one row per look. It is not capped on
  purpose: a cut list would label its oldest shown version "First recorded".
- **At most 2,000 tables are looked at per pass**, the first by name; when
  there are more, `schema versions: more tables than a pass looks at` is
  logged with how many were left out.
- **Limits to tell a customer.** Versions start the day this is deployed and
  the first one is dated by the console's first look at the table, not by
  the table's creation. A version's time is when the console saw the change,
  up to one trigger after it; two changes between two looks are recorded as
  one, and a change undone before the next look is not recorded. A renamed
  column reads as one dropped and one added. Only the definition is kept,
  not the rows. A table that is dropped records nothing, and its rows stay.
- **Who can see it.** The versions are a field of the asset detail
  (`GET /api/catalog/{id}`), behind that route's permission and tenant gate;
  no route was added.
- **What was run.** On the dev stack on 2026-10-05 (`ClickHouse` 26.8): the
  pass at start-up recorded one version for each of the seven tables in
  `silver` and `serving`; on a demo table an added column, a retyped plus a
  dropped column and a moved column each gave the next version, a look with
  nothing changed recorded nothing, and a column added before the
  orchestrator's 15-minute schedule was recorded at that schedule. Before
  that, the engine refused one statement the unit tests' fake engine had
  accepted (the aliases of the newest-version read, `Code: 184`), which is
  why that statement carries a test on its text. Not run: more than 2,000
  tables, two APIs against one engine, an install without the orchestrator.

### What's deliberately NOT in the stack

- **The Next.js frontend.** Its Dockerfile is untracked, ad hoc work in
  progress on this branch and does not exist in a clean clone — building a
  competing one here would fork that effort. Run it directly instead:
  ```bash
  bun install
  RUST_API_URL=http://localhost:8080 bun --bun next dev
  ```
  against the backend stack `docker compose up` brought up.
- **Dagster, by default.** Heavy (webserver + daemon + a code-location
  container) and out of scope for the *default* local dev loop — a plain
  `docker compose up` still doesn't start it, and pipeline trigger/run-
  status routes still return `503` without it. **P3 adds it behind an
  opt-in `dagster` compose profile** (mirroring how P2 gated `seaweedfs`
  behind its own profile) — see "Dagster (opt-in, P3)" below and
  `docs/adr/0005-dagster-code-location-ownership-and-packaging.md`.
- **A real LLM.** Needs a paid API key. AI chat routes return `503` without
  `LLM_KEY` (or `MINIMAX_API_KEY`) set to a working key.

See "Features unavailable locally," below, for the full list and how to
turn each one on.

## Healthcheck: what `GET /health` actually checks

`GET /health` is a **plain liveness check** — it returns `200 ok`
unconditionally, with no dependency on Postgres, ClickHouse, Dagster, or
the LLM (`rust/crates/lakehouse-api/src/routes/mod.rs`, `async fn
health()`). It answers "is the process up and serving," not "are its
dependencies reachable." That's sufficient for compose's own
`depends_on`/orchestration needs (which target `postgres` and
`clickhouse`'s own healthchecks directly, not this endpoint), and it's
what the existing `#[tokio::test] health_returns_200_ok` locks in.

**We did not add a `GET /health/ready` in this pass.** It would be
genuinely useful (a real k8s/compose readiness probe should reflect
dependency health, not just liveness), but every route file that would
need touching to add one — `main.rs`, `routes/mod.rs` is fine, but wiring
in a new handler that calls out to ClickHouse/Postgres crosses into
territory this phase was told not to touch casually — plus the DoS
consideration (a readiness probe must do a *bounded*, cheap check —
e.g. `SELECT 1` / `ChClient::query("SELECT 1")` with a short timeout, not
a full dependency traversal, and definitely not hitting Dagster or the LLM
on every probe) needs its own review, not a drive-by addition. Proposed
shape, for whoever picks this up:

```
GET /health/ready
  -> 200 {"postgres": "ok"|"unavailable", "clickhouse": "ok"|"unavailable"}
     if Postgres is configured (state.pg.is_some()), a bounded SELECT 1
     against it (short timeout, e.g. 1s) — degrade to "unavailable" on
     error rather than 500
     same for ClickHouse: a cheap `SELECT 1` / `/ping`, bounded timeout
     never hit Dagster or the LLM here — they're already excluded from
     "core" health by design (see Config's doc comments) and calling out
     to them on every probe is exactly the unbounded-upstream-call
     DoS shape to avoid
  -> always 200 (a probe endpoint shouldn't itself flap the process's
     perceived liveness); readiness is communicated in the body, not the
     status code, unless the orchestrator specifically wants a non-200 to
     pull the pod from a load balancer — pick one and document it
```

## Bootstrap admin: there is no default credential

`AUTH_BOOTSTRAP_EMAIL` / `AUTH_BOOTSTRAP_PASSWORD` seed exactly one admin
account, idempotently, on every boot (`main::bootstrap_admin`). **If you
don't set both, no account is created and there is no way to log in** —
this is deliberate (see the doc comment on `bootstrap_admin`): a
hardcoded fallback credential would be a standing backdoor. If you forget
to set them before first boot, set them now and restart the container —
the seed re-runs and succeeds (it only no-ops if that *specific* email is
already taken).

The created identity has `must_change_password = true`; the login
response includes `mustChangePassword: true` and the client is expected to
force a password change before continuing (see
`routes::auth::change_password`, which enforces this server-side, not
just as a UI hint).

## Postgres-down is quiet — this is the biggest operational trap here

**The service does not refuse to boot when Postgres is unreachable.**
`lakehouse_store::connect_lazy` never does network I/O at startup; a
misconfigured or dead `DATABASE_URL` is discovered lazily, at first use.
Dependent (Phase 2: identity, auth, governance-writes, pipelines,
connectors, alerts, ...) routes degrade to `503` one request at a time
instead of the process failing to start or a healthcheck failing loudly.

**In practice this means:** a broken `DATABASE_URL` looks, from the
outside, like "the process is up and `/health` is green" while every
login attempt and every Phase 2 route silently 503s. Watch the `503` rate
and the structured logs (`tracing::error!` on migration/bootstrap
failures), not just process uptime or the liveness check, when diagnosing
"nothing works" reports. The same applies to ClickHouse-backed routes,
which 503 the same way if ClickHouse is down — the difference is only
that in the local compose stack, ClickHouse *is* wired into
`depends_on: condition: service_healthy`, so this specific failure mode is
less likely to bite you locally than in a deployment that skips that
check.

## Features unavailable in the local stack

| Feature | Needs | Symptom without it | To enable |
| --- | --- | --- | --- |
| Pipeline trigger / run status | Dagster | `503` from `/api/pipelines/*` | Bring up the `dagster` compose profile (see "Dagster (opt-in, P3)" above) and point `DAGSTER_URL`/`DAGSTER_REPO`/`DAGSTER_LOCATION` at it |
| AI chat (Copilot and Query Studio's Natural language box) | LLM API key | `503` from `/api/ai/*` | Set `LLM_URL`/`LLM_MODEL`/`LLM_KEY` (or `MINIMAX_API_KEY`) to a real OpenAI-compatible provider |
| Alert digests / threshold emails | SMTP | Alerts still evaluate; email delivery silently no-ops | Set `SMTP_HOST` (and friends) to a real SMTP relay |
| Signed dashboard embeds | `EMBED_SECRET` | Embed routes unavailable | Set `EMBED_SECRET` |
| SSO / OIDC login | An OIDC provider | Local password auth only | Set `OIDC_ISSUER` + `OIDC_CLIENT_ID` (see `rust/crates/lakehouse-auth/README.md`) |

## Login throttling and session cleanup

`POST /api/auth/login` is throttled per email. After
`LOGIN_MAX_FAILURES` failures (default 5) inside a
`LOGIN_FAILURE_WINDOW_SECS` rolling window (default 900 s), that email is
locked out for `LOGIN_LOCKOUT_SECS` (default 300 s); every further attempt
gets `429` with a `Retry-After` header, regardless of whether the password
is right, and regardless of whether the account exists. A failed attempt
only counts when the password was actually rejected — a database error
mid-login renders as a 500 and does not count toward the lock.

There is deliberately no off switch: `LOGIN_MAX_FAILURES=0`, negative,
and unparseable values all fall back to the default (5), so an operator
cannot silence the throttle by typo. The same fallback applies to the
other two settings and to `AUTH_RETENTION_DAYS` — invalid values never
fail boot and never disable the feature.

Notes an operator should know:

- The throttle key is a SHA-256 of the trimmed, lower-cased email — the
  API never stores raw addresses in `login_throttle`. The account lookup
  itself is case-sensitive (`WHERE u.email = $1`), so two spellings that
  differ only by case are different accounts but one throttle key.
- A successful login clears the failure count for that email, and a
  failure after a lock has expired starts the count fresh — one typo
  after a lockout ends does not immediately re-lock.
- The first (bootstrap) admin can be locked out like anyone else; see
  "Unlocking a lockout by hand" below.
- The counter lives in Postgres, so a horizontally-scaled deployment
  shares one limit per deployment, not per replica (rows are upserted
  atomically, so concurrent attempts all count).
- The console surfaces `429` as a lockout message with the wait time
  from `Retry-After`; nothing else on the login page changes.

### Unlocking a lockout by hand

A lockout clears itself after `LOGIN_LOCKOUT_SECS`. To clear one early,
delete the key's row (as the database owner, e.g. the `lakehouse` role):
the API's tables live unqualified in the app database's `public` schema
(migrations create them without a schema prefix and the pool sets no
`search_path` override).

```sql
DELETE FROM public.login_throttle
WHERE key_hash = encode(sha256(lower(trim(' ' from 'User@Example.Com'))::bytea), 'hex');
```

`lower(trim(...))` matches how the API derives the key for an ordinary
address (`throttle::key_for` — exact for the common case, though Rust's
`str::trim` strips all whitespace where SQL `trim(' ' …)` strips only
spaces, and `lower()` follows the database collation), and `sha256()` is
a built-in since Postgres 11 — no extension needed. Deleting the row
clears both the count and the lock; there is no separate "unlock" flag.

### Session cleanup background job

A task spawned at API boot purges, roughly hourly:

- auth sessions whose `expires_at` or `revoked_at` is more than
  `AUTH_RETENTION_DAYS` days old (default 30);
- service credentials revoked more than `AUTH_RETENTION_DAYS` days ago;
- `login_throttle` rows that are no longer locked AND whose failure
  window has passed — this one is NOT gated by `AUTH_RETENTION_DAYS`:
  such a row can never lock again (the next failure starts a fresh
  window), so it goes as soon as its window lapses, not 30 days later.

It logs purge counts at each run and skips a tick if the previous run
has not finished (`MissedTickBehavior::Skip`). It is best-effort, not a
guarantee:
nothing promises a session is gone within any particular bound. The job
only deletes — it does not rotate anything, and an ACTIVE (non-revoked)
service token never expires or gets cleaned up no matter how old it is;
rotation is a manual, operator-driven act.

## Proposal: `GET /api/auth/providers` (not built)

**SSO is currently gated by a build-time flag, not a runtime one.** The
frontend can't read the Rust process's environment directly, so whether
the SSO login button shows up is controlled by `NEXT_PUBLIC_SSO_ENABLED`
at *Next.js build time* — it can't react to whether `OIDC_ISSUER` /
`OIDC_CLIENT_ID` are actually configured on the backend at runtime. That
means a deployment can build with SSO UI enabled but the backend
unconfigured (dead button), or the reverse (backend ready, but the UI
never shows it without a rebuild).

**Proposal:** add `GET /api/auth/providers`, unauthenticated, returning:

```json
{ "local": true, "oidc": { "enabled": true, "providerName": "okta" } }
```

derived directly from `AppState::auth.oidc.is_some()` (already computed at
startup from `OIDC_ISSUER`/`OIDC_CLIENT_ID` — see `state.rs`) and
`Config::oidc_provider_name`. The frontend would call this once (e.g. on
the login page mount, or server-side in the login route) instead of
reading a build-time env var, and show/hide the SSO button based on the
live response.

**Rationale:**
- Removes the "backend truth, frontend build flag" split — one source of
  truth, read at request time.
- Zero new attack surface: this endpoint answers "is SSO configured," not
  "here are the secrets" — no issuer secrets, client secrets, or JWKS
  contents belong in the response, only booleans/labels already meant to
  be public UI copy (the provider name shows up on the login button
  either way).
- Lets ops flip OIDC on/off (e.g. during an incident, or a provider
  migration) by restarting the Rust process with new env vars, without a
  frontend rebuild+redeploy.

**Not built in this phase** because it touches `routes/auth.rs` and
`routes/mod.rs` router wiring, which is out of scope here — left as a
proposal for whoever owns the auth surface next.

## Postgres backup / restore

Scripts: `scripts/backup-postgres.sh`, `scripts/restore-postgres.sh`. Both
assume the `docker-compose.yml` `postgres` service is running and shell
out to it via `docker compose exec`.

### Backup

```bash
scripts/backup-postgres.sh [output-dir]   # default: ./backups
```

Runs `pg_dump -Fc` (custom format — compressed, supports selective
`pg_restore`) inside the `postgres` container and writes
`<db>-<UTC-timestamp>.dump` into the output directory (git-ignored;
`backups/` is not meant to be committed). Prunes dumps for that database
older than `RETENTION_DAYS` (default 14) after each run.

### Restore

```bash
scripts/restore-postgres.sh <dump-file> [target-db]
```

Restores via `pg_restore --clean --if-exists --no-owner`. **Always
restore into a scratch database first** (`[target-db]`) to verify a dump
before trusting it — restoring into the live database name is destructive
(`--clean` drops existing objects before recreating them). The script
creates `[target-db]` if it doesn't already exist.

### Retention and location

Backups land in `./backups/` by default (override with `BACKUP_DIR` or
the first positional argument). This directory is local to whatever host
runs the script — for anything beyond local dev, point `BACKUP_DIR` at
durable, off-host storage (a mounted volume synced elsewhere, object
storage, etc.) and run the backup script on a schedule (cron/systemd
timer/CI job), not just ad hoc. `RETENTION_DAYS` (default 14) controls how
long local dumps are kept before the backup script prunes them on its own
next run — it does not proactively delete on a timer by itself.

This procedure was tested end-to-end as part of this phase: a backup was
taken, a scratch database was restored from it, and the restore was
verified by querying the restored data. See the phase report for the
actual command transcript.

## Dedicated-bucket backup job (WS5 item G2)

The script-based procedure above covers only `${POSTGRES_DB:-lakehouse}`
(the app database) and writes to local disk. `dagster/dispar_orchestrate/
backup_job.py`, run via the `backups` compose profile, covers all four
compose-created Postgres databases (the app database, plus Lakekeeper's,
OpenFGA's, and Dagster's own) and uploads each dump to an **S3-compatible
bucket that is dedicated to backups**, never the shared warehouse bucket.

**Why a dedicated bucket, not the warehouse bucket — security, not
preference.** One of the four databases is the identity database
(`app_user`/`auth_identity`, holding password and service-token hashes).
Writing that dump into the same bucket every warehouse-scoped credential
(every ingestion connector, every Iceberg reader) can already read would
turn compromise of ANY ONE of those credentials into compromise of every
user's password hash — a privilege-escalation path the warehouse bucket's
access domain was never meant to grant. `backup_job.py` therefore reads
its own `BACKUP_S3_*` endpoint, bucket, and credentials, entirely
separate from `RUSTFS_*`/`CONNECTOR_S3_*`.

### Configure

Set `BACKUP_S3_ENDPOINT`, `BACKUP_S3_BUCKET`, `BACKUP_S3_ACCESS_KEY`,
`BACKUP_S3_SECRET_KEY` (and leave `BACKUP_S3_ACCESS_KEY_SECRET_REF=env:
BACKUP_S3_ACCESS_KEY` / `BACKUP_S3_SECRET_KEY_SECRET_REF=env:
BACKUP_S3_SECRET_KEY` at their `.env.example` values) in your `.env`.
`BACKUP_RETENTION_DAYS` (default `30`) is the one optional field — it is
a retention window, not a credential.

These four bucket/credential values are **required** (`${VAR:?}`, no
default) but deliberately do **not** live in the default
`docker-compose.yml` — they live in
`ops/backups/docker-compose.backups.override.yml`, added with `-f` only
when a backup actually runs, same precedent
`ops/g6/docker-compose.g6.override.yml` already establishes for its own
gate-only credentials: a `${VAR:?}` in the default file is interpolated
for every service regardless of profile, so a must-set-only-under-one-
profile credential placed there would break `docker compose --profile
'*' config --quiet` (the repo-wide check AGENTS.md prescribes) for every
checkout, backups configured or not.

### Run

```bash
docker compose -f docker-compose.yml \
  -f ops/backups/docker-compose.backups.override.yml \
  --profile backups run --rm backup-job
```

Without the `-f ops/backups/...` override (or with any of its four vars
unset), this refuses to start — a `backups` profile enabled with no
bucket configured is a clear startup failure, never a silent write into
the wrong place, and never a silently-skipped backup reported as
successful. A failed `pg_dump` for any one database raises immediately
(`dagster.Failure`) and stops the run before uploading anything for the
databases after it — a partial run is never reported as a complete one.

**Scheduling:** this is not (yet) wired as an automatic in-process
Dagster job/schedule — see `backup_job.py`'s module doc for why (in
short: doing so safely would require threading these same bucket/DB
credentials into the always-on `dagster-code-location` service, which
would make every `dagster`-profile deployment require backup
configuration too). Run the command above from an external trigger (host
cron, a systemd timer, or a CI scheduled job) on whatever cadence your
retention policy assumes (nightly, matching `BACKUP_RETENTION_DAYS`'s
default 30-day window, is a reasonable starting point).

### Restore

Download the dump for the database/timestamp you want from
`<BACKUP_S3_BUCKET>/<db-name>/<timestamp>.dump` (via any S3-compatible
client pointed at `BACKUP_S3_ENDPOINT` with the same credentials), then:

```bash
pg_restore --dbname=<target-db> --clean --if-exists --no-owner <dump-file>
```

Same "restore into a scratch database first" caution as the script-based
procedure above applies here — `--clean` drops existing objects before
recreating them, so verify a dump against a throwaway database name
before trusting it against the live one.

## Upgrade note: set `CATALOG_TENANT_ID` before deploying multi-tenant admin support

Migration `0042_tenant_provisioning.sql` and its route-level tenant scoping
make every tenant-scoped list fail closed. Two surfaces cannot be
filtered per tenant, because nothing in the schema associates them with one:

- **the catalog** — `bronze_meta.dataset_catalog` has six columns (`slug`,
  `title`, `description`, `tier`, `updated_at`, `table_name`) and no tenant or
  connector reference;
- **the `Dagster`-job half of `GET /api/pipelines`** — a code location is one
  per deployment, and migration `0042` adds `tenant_id` to `connector` and
  `pipeline_definition` only.

Rather than show every tenant the same shared list and call it scoped, both
are **refused** for any principal without the unrestricted (`*:*`) grant once
the deployment has more than one tenant.

**What this means for an existing deployment.** `0002_seed_identity.sql` seeds
four tenants, and its users belong to several of them — so on upgrade,
Analysts, Data Engineers and Governance Admins lose the catalog and the job
list until you say who owns them:

```
CATALOG_TENANT_ID=<the id of the tenant that owns this deployment's shared catalog>
```

For a stack that still runs the seeded identities, that is the group tenant in
`0002_seed_identity.sql`. Members of that tenant then see both surfaces exactly
as before; everyone else keeps getting `{"supported": false, "reason": …}`,
which names this setting.

A **single-tenant** deployment is unaffected — there is no other tenant's data
to leak into a shared list, so nothing is refused whether or not the setting is
present.

A malformed value fails startup rather than quietly disabling the check. With
the setting unset, the API logs one warning at boot naming it.

## Gold export: growth and background merges

Every publish of a changed mart appends one full copy — an append-only
Iceberg table — and nothing expires old copies yet. Storage grows by one
copy per publish. Outside tools must select the latest export time
(`SELECT … WHERE _exported_at = (SELECT max(_exported_at) …)`) when
they read a published mart, or they will see every past copy.

`GOLD_EXPORT_MARTS` now defaults to empty: the console owns the mart list
through `GET /api/gold/publications`, and a fresh deployment publishes
only what is switched on there. Keep the env var as an operator override
for emergencies, not as the daily driver.

A background `MergeTree` merge can advance `max(modification_time)` on a
mart even when no data changed, causing one extra export. The error
direction is a spurious "changed" → one extra copy, never a missed one.
A merge-proof freshness signal tracks as backlog `DATA-10`.

A mart over the row cap (`GOLD_EXPORT_MAX_ROWS`, default 5,000,000) is
refused outright — the export names the mart and the cap in its error,
never silently truncating. Raise the cap only when you have measured the
memory pressure of a true full re-copy at the new size.
