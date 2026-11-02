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
`Cargo.lock` requires a toolchain matching the `rust-version` floor (see
the `rust-toolchain.toml` `channel` pin, which is what the workspace
actually builds with), and `sqlx::migrate!
("../../migrations")` in `lakehouse-store/src/lib.rs` embeds the
migrations directory at *compile* time, so it must exist in the build
context or `cargo build` fails outright. The current `rust/Dockerfile`
pins `rust:1.99.1-slim` to match `rust-toolchain.toml`, copies
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
| `console` | built from `Dockerfile.frontend` | the Next.js console, port 3000 (REL-2 S1-T2) — waits for `lakehouse-api` to report healthy, and proxies `/api/*` to it |
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
"Postgres: unreachable is loud, a mismatched schema refuses to start,"
below.

`lakehouse-api` carries a healthcheck of its own (`/health`, liveness), and
the `console` waits for it — so the console starts after the API can
answer, not while the API is still applying migrations. The console's
`/api/*` proxy target is baked into its image at build time
(`RUST_API_URL`, the `lakehouse-api` service's address on the compose
network); `CONSOLE_INTERNAL_URL` is the address the OTHER way round (the
console as the PDF renderer reaches it), and it is empty by default — set
it to `http://console:3000` to switch the PDF export on in this stack.

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
`dagster/dispar_orchestrate/maintenance.py`'s snapshot expiry chain
(default 7-day retention floor, OPS-1) requires Trino because ClickHouse 26.8
refuses `expire_snapshots` on catalog-backed tables. If the `trino` profile is
down or unconfigured, snapshot expiry logs a warning and skips cleanly, leaving
snapshots unexpired without failing the maintenance job.

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

**Two upstreams are per table, not per job (DATA-19, BLD-4).** The
newest `SUCCESS` run of the job is the right freshness anchor for every
other upstream, but it is the wrong one for the two ingestion jobs,
because both load a Bronze table and are shared by every table they
touch:

- `file_ingest_job` loads ONE uploaded file into ONE table per run, so
  the anchor is the newest successful run whose config names the
  pipeline's own source table (DATA-19).
- `ingest_job` loads EVERY selected object of ONE connector in a run,
  and is the SAME job for every connector, so its run config names a
  connector and not any table. The anchor is the newest successful run
  that MATERIALIZED the pipeline's own source table -- the asset key
  each loaded object records inside its run (BLD-4).

So a load of a different table never fires such a pipeline, however
recent it is, and the `SkipReason` names the table it was waiting for.
Both upstreams are accepted only on a pipeline whose source is `bronze`
(`routes::authored_pipelines::validate_per_table_upstream`, one rule
run from create, update and restore): on any other source the chain
would be wired and could never fire, so it is refused at save time with
the job's own message, and a row stored before the rule waits with the
same reason. The window each walk reads is bounded
(`PER_TABLE_UPSTREAM_SCAN_LIMIT`, 20 materializations/runs), so a source
table that has not loaded in its last 20 materializations (`ingest_job`)
or runs (`file_ingest_job`) waits rather than anchoring on a run older than
that.

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

### Connection tests and internal addresses (`SEC-15`)

A connector's **Test connection**, **Discover** and the clean-up that runs
when a change-capture connector is deleted dial the host the connector was
registered with. That host is chosen by whoever may manage connectors, so the
API refuses to dial an internal address unless the operator allows it:

- **A fresh install blocks every internal address.** That is loopback,
  RFC1918 (`10/8`, `172.16/12`, `192.168/16`), link-local (`169.254/16`,
  where cloud instance metadata lives), carrier-grade NAT (`100.64/10`),
  multicast, `0/8`, the reserved `240/4` block and their IPv6 counterparts,
  including IPv4 wrapped in an IPv6 address. A name is resolved once; if any
  address it resolves to is blocked the whole name is refused, and the
  address that was checked is the address that is dialled.
- **The seeded demo connectors are refused too.** `conn-pg-lakehouse` and
  `conn-s3-warehouse` point at `postgres:5432` and `http://rustfs:9000`
  inside the compose network, so on a fresh install their "Test" says the
  host "resolves to a private/internal address". That is the intended
  outcome, not a fault.
- **To allow a network**, set `CONNECTOR_PROBE_ALLOWED_CIDRS` to the networks
  the sources live on (comma-separated, e.g. `192.168.18.0/24`; a bare address
  is one host) and restart `lakehouse-api`. This can open private ranges and
  carrier-grade NAT, never loopback, link-local, multicast or `0/8`.
- **To allow every internal address** (a trusted development stack only), set
  `CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS=true`. Only the exact string `true`
  counts. It also opens loopback and link-local, so a connection test can then
  reach the API's own host and a cloud metadata service.
- **An existing `.env` keeps what it has.** `.env.example` used to set the
  variable to `true`; a `.env` copied from it still does. Check it.
- **Redirects are not followed** by the REST and S3 tests, and these two
  tests do not use a system proxy: a redirect is reported as one, with its
  HTTP status.
- **Deleting a change-capture connector** whose source is refused is a 409
  with a fixed message and the row stays; `?force=true` removes the row
  without dialling the source, and the replication slot and publication then
  have to be dropped by hand.
- The orchestrator's own dials are a separate guard with their own settings
  (`INGEST_ALLOW_INTERNAL_HOSTS`, `INGEST_ALLOWED_CIDRS`).
- **Not hidden by this:** with an address allowed, "connection refused" and
  "timed out" still differ in a test result, so a person who may test
  connectors can tell an open port from a closed one on that network.

The CI gates that save ingest specs aimed at compose-network fixtures set the
variable themselves, in their own override (`ops/g6/`), marked gate-only.

### Connector TLS and certificate verification (`SEC-14` part B)

Connector connection tests (`POST /api/connectors/{id}/test`), table discovery
(`POST /api/connectors/{id}/discover`), and CDC clean-up require TLS for
PostgreSQL and SQL Server dials:

- **PostgreSQL connection test (`pg_probe`):** Connection tests send an
  `SSLRequest` before the PostgreSQL startup packet. If the server declines TLS
  (responds `N`), the probe fails immediately without sending credentials. The
  probe only negotiates `SCRAM-SHA-256`; if the server requests clear-text
  (`AuthenticationCleartextPassword`) or MD5 (`AuthenticationMD5Password`)
  authentication, the probe refuses and terminates the connection without
  transmitting the password. Plain `SCRAM-SHA-256` without channel binding
  is used (`channel_binding=disable`); without a CA certificate configured,
  a relaying server performing a man-in-the-middle attack cannot be detected.
- **Certificate verification (`CONNECTOR_TLS_CA_FILE`):**
  - Unset or empty: PostgreSQL and SQL Server dials require encryption
    (`Require` / `EncryptionLevel::Required`) without checking the server
    certificate against a custom root CA.
  - Set to a PEM file path: the PostgreSQL connection test (`pg_probe`) verifies
    the server certificate and the host name (`verify-full`) against that bundle
    alone. The `sqlx` dials (table discovery, table listing, CDC clean-up)
    verify the chain only (`VerifyCa`): they dial the approved address rather
    than the name, so a name check could never pass; they trust the bundle
    plus the public webpki roots `sqlx` ships with. SQL Server dials verify the
    server certificate against that CA bundle via Tiberius's `trust_cert_ca`,
    which adds the CA to the native system roots (so SQL Server accepts either
    the configured CA or any public root, unlike the PostgreSQL connection
    test, which trusts only the configured CA bundle).
  - If `CONNECTOR_TLS_CA_FILE` is set to an unreadable or non-existent path,
    or contains no valid PEM certificates, `lakehouse-api` fails fast at startup
    with an error naming the variable.
- **Docker Compose:** The default `docker-compose.yml` includes a one-shot
  `postgres-cert-init` service that generates a local CA and server certificate
  (SAN `DNS:postgres`) into the named volume `lakehouse_postgres_tls`. The
  bundled `postgres` container runs with `ssl=on`. `lakehouse-api` mounts this
  CA at `/run/certs/postgres/ca.crt` and defaults
  `CONNECTOR_TLS_CA_FILE=/run/certs/postgres/ca.crt`, allowing the seeded
  `conn-pg-lakehouse` connector test to pass over `verify-full`.
- **Operators with external databases:**
  - Because `docker-compose.yml` defaults `CONNECTOR_TLS_CA_FILE` to
    `/run/certs/postgres/ca.crt` if unset or empty via `${CONNECTOR_TLS_CA_FILE:-/run/certs/postgres/ca.crt}`,
    it cannot be cleared to empty in `.env`.
  - To connect to external databases using custom internal CAs or public system
    roots alongside the bundled PostgreSQL, create a combined CA bundle containing
    the bundled CA (`/run/certs/postgres/ca.crt`) plus your corporate internal CA
    or system root certificates. Mount the combined bundle into the `lakehouse-api`
    container and set `CONNECTOR_TLS_CA_FILE` to its mounted path.

### Connector credentials at rest (`SRC-12`, ADR 0002 addendum 5)

A credential typed in the console is stored on the `connector_secrets`
volume encrypted with AES-256-GCM. The key is `CONNECTOR_SECRETS_KEY`: 32
random bytes, base64. The API and the orchestrator both need it, and
compose refuses to start either without it.

- **Make one, once:** `openssl rand -base64 32`. Keep it with the
  installation's other secrets. **Losing it loses every typed credential**:
  they cannot be decrypted and have to be entered again.
- **First start with a key on an older install:** the API encrypts every
  plain credential file at start-up and removes the plain copies. Until the
  API has done so, the orchestrator refuses a plain file instead of reading
  it.
- **Changing the key:**
  1. Set the new key as `CONNECTOR_SECRETS_KEY` and the current one as
     `CONNECTOR_SECRETS_KEY_PREVIOUS`.
  2. Run `docker compose run --rm lakehouse-api rotate-secrets-key` while
     nobody is saving a credential. It re-encrypts every stored credential
     with the new key and lists any file it could not.
  3. Remove `CONNECTOR_SECRETS_KEY_PREVIOUS` and restart the API and the
     orchestrator. The old key no longer opens anything.
- Credentials given as an environment variable or an operator's file are
  not encrypted by the product; they are the operator's to protect.

### Alert webhooks and internal addresses (`SEC-10`)

An alert or digest sends its message to a webhook URL a user typed, so the
sender applies the same address policy as the connector tests above:

- **Internal addresses are refused.** Loopback, RFC1918, link-local (cloud
  metadata), carrier-grade NAT, multicast, `0/8`, the reserved block and their
  IPv6 forms, including IPv4 wrapped in IPv6. The host is resolved once; if
  any address it resolves to is blocked the whole name is refused, and the
  request goes to the address that was checked. A rule is checked when it is
  saved (a 400 with "webhook address is not allowed") and again at every send,
  because a name can resolve differently later.
- **Redirects are not followed.** A webhook that answers with a redirect is a
  failed delivery ("webhook redirect not followed"); give the final URL. The
  sender also ignores a system proxy (a proxy resolves the name itself, which
  defeats the check) and gives up after 10 seconds (it had no limit before).
- **To allow an internal tool** (an on-premises chat or incident server), set
  `WEBHOOK_ALLOWED_CIDRS` to its network or address (comma-separated, e.g.
  `10.1.0.0/16`; a bare address is one host) and restart `lakehouse-api`. It
  is separate from `CONNECTOR_PROBE_ALLOWED_CIDRS`, has no allow-everything
  form, and never opens loopback, link-local, multicast or `0/8`. A malformed
  entry stops the API at start.
- **Failed deliveries carry a fixed reason**, never the HTTP library's text:
  "webhook address is not allowed", "webhook host could not be resolved",
  "webhook redirect not followed", "webhook HTTP <status>", "webhook request
  timed out", "webhook connection failed", "webhook request failed". Delivery
  records written before this change may hold the old library text; they are
  not rewritten.
- **Email (`SMTP`) is a separate item** and unchanged.

### Signed dashboard embeds (`SEC-12`)

A customer's server signs a short-lived token and the embed page presents it.
What the operator needs to know:

- **`EMBED_SECRET` is the only source of the signing secret.** The API no
  longer generates one and keeps it in `console.app_kv`. Unset or empty: signed
  embedding is unavailable (`POST /api/embed/data` answers `503` "embedding is
  not configured", the Share dialog says so, and no sample token is offered),
  and the rest of the product runs. Set it to a long random value and sign with
  the same value. **An old `embed_secret` row in `console.app_kv` is no longer
  read. Remove it** (`ALTER TABLE console.app_kv DELETE WHERE k = 'embed_secret'`
  in `ClickHouse`); the API does not delete it for you. A deployment that relied
  on the generated secret must read it once from that row and set `EMBED_SECRET`
  to it, or issue new tokens with a new secret.
- **`EMBED_TOKEN_MAX_LIFETIME_SECONDS`** (default `86400`) is the longest
  `exp - iat` a token may have. It must be a positive whole number; anything
  else stops the API at start. The Share dialog shows the value in force.
- **Signing a token.** HS256, with these claims (placeholder values):

  ```json
  {
    "resource": { "dashboard": "b_0123abcd" },
    "params": { "region": "North" },
    "iat": 1800000000,
    "exp": 1800000600,
    "jti": "7d1e8f3a-0000-0000-0000-000000000000"
  }
  ```

  `iat` and `exp` are required numbers in Unix seconds; `exp - iat` must not
  exceed the limit; `iat` may be at most 60 seconds ahead of the API's clock and
  `exp` at most 60 seconds behind it. `jti` is optional (1 to 128 characters of
  `A-Za-z0-9._-`) and is what makes one token withdrawable. `params` are filters
  the viewer cannot change. Any other claim is ignored. A token that fails any
  check is refused with the one message "embed token is invalid or expired".
- **Withdrawing.** In the Share dialog (needs `dashboard:write`): "Withdraw all
  embed tokens" refuses every token of that dashboard issued at or before that
  moment; "Withdraw one token" takes a pasted token and refuses its `jti` for
  that dashboard until the token could not be accepted anyway. Both are read
  from the dashboard on every request, with no cache, so they apply to the next
  request. Withdraw all uses the API server's clock against the token's `iat`:
  a token minted by a clock more than a moment ahead of the API's survives it
  until it expires, which the lifetime limit bounds.
- **Framing.** Each dashboard lists the sites allowed to show its embed pages
  (Share dialog; at most 20; `https://host[:port]`, or `http://localhost[:port]`
  for development; no wildcards). The console's `proxy.ts` asks the API
  (`POST /api/embed/frame`, which needs no sign-in and answers only the lists)
  and sends `Content-Security-Policy: frame-ancestors <sites>`, or
  `frame-ancestors 'none'` when the list is empty, the token is not good, or
  the API cannot be reached within 3 seconds. The console reads the API address
  from `RUST_API_URL` **at run time** for this call (the image defaults it to
  `http://localhost:8080`; a deployment must pass `-e RUST_API_URL=...`, as the
  staging deploy does). If it points where nothing listens, every embed page is
  sent as `frame-ancestors 'none'` and no embed renders. The rule is enforced by the
  viewer's browser; it does not replace the token. Every other console page
  sends `frame-ancestors 'none'` (`SEC-47`, the next section).

### The console's Content-Security-Policy (`SEC-47`)

Every console page is served with a `Content-Security-Policy`, so the browser
— not only the page's own code — refuses an inline script, a script from
another origin, an `object`, a moved `base`, and an image or frame from
anywhere but the sites an administrator allows. A redirect from an allowed
site to another one is refused by the browser as well. The policy is enforced,
not report-only.

- **What it says.** `default-src 'self'`; scripts run only with the request's
  own nonce and `'strict-dynamic'`; `object-src 'none'`; `base-uri 'self'`;
  `form-action 'self'`; `connect-src 'self'`; `font-src 'self' data:`. A
  console page sends `frame-ancestors 'none'`; an embed page sends its
  dashboard's list (the `SEC-12` bullet above). `style-src 'unsafe-inline'` is
  a stated exception: React, Radix and ECharts write inline `style` attributes
  and style elements, which a nonce cannot cover. It allows styles, never
  script. `next dev` also adds `'unsafe-eval'` and the HMR websocket; a
  production build has neither.
- **The sites images and web pages may come from** are the content sites in
  Admin > Settings (`BI-16` part B). A console page asks `GET
  /api/settings/reporting` with the visitor's own cookie and reuses the answer
  for 60 seconds, per console process; an embed page asks `POST
  /api/embed/frame`, which answers the sites of *that dashboard's* image and
  web cards only, never the whole deployment list. Both fail closed: an API
  that cannot be reached within 3 seconds, a `401`, a `500` or an answer of the
  wrong shape leaves `img-src 'self' data: blob:` and `frame-src 'none'`, and
  the page still loads (its own scripts, styles and fonts are local). A site
  added in Admin > Settings takes effect within a minute. Both calls read
  `RUST_API_URL` **at run time**, as the framing call does; pointed at nothing,
  console pages have no external image or frame but still work.
- **Every page renders per request.** A nonce is per request, so no console
  page is prerendered or cached by a CDN: `src/app/layout.tsx` sets
  `dynamic = "force-dynamic"`. Cache static assets if you need to; do not cache
  the HTML.
- **A reverse proxy in front of the console must not drop the header.** The
  console sets it itself; a proxy that strips or rewrites unknown
  `Content-Security-Policy` headers leaves the console with no script at all.
  A deployment that sets its own policy in front of the console's must keep
  that header's nonce (or replace the whole policy knowingly).
- **Known limits.** There is no `report-uri`: a refused request is visible only
  in the browser's console. A public link (`/public/dashboard/<token>`) is
  opened without a session, so it is served without the content sites: its
  image and web cards are refused by the browser (an image card then says it
  could not be loaded, a web card stays blank). Only the console and the embed
  pages load them.

### Connector credentials from a secret manager (`SRC-12`, ADR 0002 addendum 6)

Instead of typing a password, a connector can reference a secret in the
installation's secret manager. One manager per installation:

- `SECRET_MANAGER` = `vault`, `aws`, `gcp` or `azure`, plus that manager's
  sign-in for the API and the orchestrator (`.env.example` lists each:
  Vault address and token or AppRole; AWS region and role or keys; a
  Google service-account file, mounted into both services, or the
  platform identity; Azure tenant and client, or a managed identity).
- **`SECRET_MANAGER_ALLOWED_PREFIXES` is required for references to work.**
  It lists the paths connectors may reference, for example
  `vault:secret/lakehouse/{tenant}/`; `{tenant}` is the connector's tenant.
  With a manager set and no prefixes, every reference is refused. Give the
  product's manager identity a policy no wider than these prefixes.
- A reference is read at every test and every run, never cached: changing
  the secret in the manager takes effect at the next run, and a manager
  that cannot be reached fails that run with a fixed message.
- `CONNECTOR_TYPED_CREDENTIALS=false` hides the "type a credential" option
  when every credential should come from the manager.

### OAuth sign-in for REST connectors (`SRC-12`, ADR 0002 addendum 7)

- Set `CONNECTOR_OAUTH_REDIRECT_URI` to the console's address of the
  product's callback (the connector form shows the path to append). Register
  exactly that address in the provider's app settings.
- The provider's token and revocation addresses must be `https`.
  `CONNECTOR_OAUTH_ALLOW_HTTP` is for tests only; never set it in a real
  installation.
- When a provider refuses a refresh (the user's access was withdrawn, the
  app's secret changed), the connector's runs fail with "sign in again"
  until someone with `connector:manage` signs in again from the connector
  page.

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

`GET /health` answers `200` with a JSON body — `{"status": "ok",
"database": "ok" | "unavailable", "version": "0.1.0"}` — and it is still a
**liveness plus application-database check**, not a full readiness probe
(`rust/crates/lakehouse-api/src/routes/mod.rs`, `async fn health`). It
does not dial ClickHouse, Dagster, Lakekeeper or the LLM: the status code
stays `200` whenever the process can answer at all, because a deployment
with no usable Postgres still serves every Phase 1 route (catalog,
overview, query, storage), and the ops gates (`ops/g*/`) and
`ops/ai_eval` poll this endpoint for a `200` to decide the API is up.

`version` is this build's version (`CARGO_PKG_VERSION`), added with the
readiness route below (REL-2 decision D7): the console compares it with
its own version and says so when the two differ.

`database` is what `OPS-4` added:

- `ok` — the start-up step connected and applied every migration, so the
  schema is at this build's version.
- `unavailable` — no pool at all (`DATABASE_URL` did not parse), the
  database could not be reached, or it refused the migrations. Phase 2
  routes (identity, auth, governance writes, pipelines, connectors,
  alerts, ...) are degraded; Phase 1 answers.

The value is the last answer the start-up step or its 60 s retry loop
gave, **not a live probe**. A database lost after boot keeps answering
`ok` until the process restarts: a per-request probe would have to be a
bounded `SELECT 1` on its own short timeout, and `/health` is polled by
the gates, so a probe that waits out a dead database's 30 s acquire
timeout would stall them. An operator diagnosing "the database died" has
to read the log (below), not only this endpoint.

## Readiness: `GET /health/ready`

`GET /health/ready` answers the other question — "can this stack serve
traffic?" — and it is the route an installer or an updater waits on (REL-2
decision D7). It answers `200` with

```json
{"ready": true, "version": "0.1.0",
 "checks": {"postgres": "ok", "clickhouse": "ok", "lakekeeper": "ok"}}
```

when all three hold, and `503` with **the same body shape** otherwise, so
a caller still learns which dependency is not ready:

- `postgres` — the database answers AND its schema is at this build's
  version: the highest applied migration equals the highest migration
  embedded in the binary. A database one migration behind (or ahead) is
  not ready for this build.
- `clickhouse` — `ClickHouse` answers `SELECT 1`.
- `lakekeeper` — the Iceberg REST catalog answers its config request
  (`GET {catalog}/v1/config`), the same call the catalog client makes
  when it connects.

Every check is only `"ok"` or `"failed"`. The reason a check failed is in
the log, never in the body: the route is public, so it must not carry
upstream text (AGENTS.md rule 4). Each check is bounded by 3 s, so the
answer does not wait on a dead dependency.

This section used to say there was no such route; `OPS-4` had extended
the one health route with the application-database state instead. REL-2
decision D7 adds the readiness probe next to it, and leaves `/health` as
the liveness probe the gates poll.

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

## Postgres: unreachable is loud, a mismatched schema refuses to start

`lakehouse_store::connect_lazy` never does network I/O at startup, so a
dead `DATABASE_URL` is still only discovered when something first uses
the pool. `OPS-4` keeps that degrade — the Phase 1 routes (catalog,
overview, query, storage) do not touch Postgres, and an unreachable
database must not take the whole console down — but makes it loud, and
draws one hard line:

- **The database is reachable but the migrations fail** (a `_sqlx_migrations`
  row this build does not have or has changed, a failed statement):
  `lakehouse-api` **refuses to start** and exits non-zero, logging the
  migration error and the line `the application database is not at this
  build's schema version`. Compose restarts it and it says the same thing
  every time. This is deliberate: before `OPS-4` the API started anyway,
  sqlx stopped at the first mismatch, and every newer route answered
  `500` while the process looked healthy (PRs #106 and #199). The fix for
  a real mismatch is to run the matching migrations, not to restart until
  it works.
- **The database cannot be reached at all** (connection refused, a
  timeout, a closed pool): the API serves the Phase 1 routes and logs an
  `error!` at start and again every 60 s until a connection succeeds, at
  which point the migrations apply and the retries stop. `GET /health`
  reports `"database": "unavailable"` meanwhile (see "Healthcheck"
  above). If the database becomes reachable later with a schema that
  still does not match this build, the retry loop logs the same
  schema-mismatch line once and stops — a mismatch does not heal by
  waiting, and a background task cannot un-start a process that is
  already serving.
- **No pool at all** (`DATABASE_URL` is not a parsable Postgres URL):
  every Phase 2 route answers `503`, `/health` says the database is
  unavailable, and the log carries the reason.

**In practice this means:** watch `/health`'s `database` field and the
structured logs (`tracing::error!` on migration/retry/bootstrap
failures), not just process uptime. With a pool but an unreachable
database, a Phase 2 query fails as `500` with the fixed text
`database error` (the driver's own text stays in the log, never in the
response); with no pool at all it is `503`. The same applies to
ClickHouse-backed routes, which 503 the same way if ClickHouse is down —
the difference is only that in the local compose stack, ClickHouse *is*
wired into `depends_on: condition: service_healthy`, so this specific
failure mode is less likely to bite you locally than in a deployment that
skips that check.

## Features unavailable in the local stack

| Feature | Needs | Symptom without it | To enable |
| --- | --- | --- | --- |
| Pipeline trigger / run status | Dagster | `503` from `/api/pipelines/*` | Bring up the `dagster` compose profile (see "Dagster (opt-in, P3)" above) and point `DAGSTER_URL`/`DAGSTER_REPO`/`DAGSTER_LOCATION` at it |
| AI chat (Copilot and Query Studio's Natural language box) | LLM API key | `503` from `/api/ai/*` | Set `LLM_URL`/`LLM_MODEL`/`LLM_KEY` (or `MINIMAX_API_KEY`) to a real OpenAI-compatible provider |
| Alert digests / threshold emails | SMTP | Alerts still evaluate; email delivery silently no-ops | Set `SMTP_HOST` (and friends) to a real SMTP relay |
| Signed dashboard embeds | `EMBED_SECRET` | `POST /api/embed/data` answers `503` "embedding is not configured"; the Share dialog says so | Set `EMBED_SECRET` (see "Signed dashboard embeds (`SEC-12`)") |
| SSO / OIDC login | An OIDC provider | Local password auth only | Set `OIDC_ISSUER` + `OIDC_CLIENT_ID` (and, for the browser flow, the four URLs); `OIDC_JWKS_URL` is optional — see "Single sign-on (OIDC)" below and `rust/crates/lakehouse-auth/README.md` |

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

## Single sign-on (OIDC)

Browser sign-in is an authorization-code flow with PKCE, served by the API
(`GET /api/auth/oidc/start` → the provider → `GET /api/auth/oidc/callback`),
which sets the ordinary `lh_session` cookie the password login also sets.
`rust/crates/lakehouse-auth/README.md` is the reference for how a token is
verified and how groups map to roles; this section is the operator's side:
which settings go with which provider, and what each provider cannot do.

OIDC turns on only when **both** `OIDC_ISSUER` and `OIDC_CLIENT_ID` are set.
With either unset, `AuthState::oidc` stays `None` and the `oidc/*` routes
answer `503` rather than misbehaving — the same posture as every other
opt-in feature here. The browser flow reads three more settings the
resource-server path does not: `OIDC_AUTHORIZE_URL`, `OIDC_TOKEN_URL` and
`OIDC_REDIRECT_URI` (all read from the API's own config, never from the
request — that is what closes the open-redirect on `next`). A confidential
client also sets `OIDC_CLIENT_SECRET`.

| Setting | Required | Default | Notes |
| --- | --- | --- | --- |
| `OIDC_ISSUER` | to enable OIDC | unset | Compared byte-for-byte with the token's `iss`. |
| `OIDC_CLIENT_ID` | to enable OIDC | unset | Must appear in the token's `aud`. |
| `OIDC_AUTHORIZE_URL` | for the browser flow | unset | The provider's authorization endpoint; unset makes `oidc/start` `503`. |
| `OIDC_TOKEN_URL` | for the browser flow | unset | The provider's token endpoint. |
| `OIDC_REDIRECT_URI` | for the browser flow | unset | This deployment's callback: `<api-origin>/api/auth/oidc/callback`. Register the same value on the provider. |
| `OIDC_CLIENT_SECRET` | for a confidential client | unset | Sent only in the code exchange; never logged, redacted in `Config`'s `Debug`. |
| `OIDC_JWKS_URL` | no | discovery | The key set's address. Unset means it is read from the provider's discovery document (`{OIDC_ISSUER}/.well-known/openid-configuration`, its `jwks_uri`), on the first token check. Set it only when that document is unavailable or names a key set this deployment cannot reach; an explicit value wins and no discovery request is made. |
| `OIDC_PROVIDER_NAME` | no | `default` | Label behind `oidc:<name>` in `auth_identity.provider`. |
| `OIDC_GROUPS_CLAIM` | no | `groups` | Which token claim carries the caller's groups. |
| `OIDC_ROLE_MAP` | no | empty | `group=Role Name,...`; a group not named here is ignored. |
| `OIDC_JIT_PROVISIONING` | no | `false` | Exactly the string `true` (not `TRUE`/`1`/`yes`) creates an `app_user` on first sign-in. |
| `OIDC_CLOCK_SKEW_SECONDS` | no | `60` | Leeway on `exp`/`nbf`; an unparseable value falls back to `60` instead of failing boot. |

**Where the signing keys come from.** With `OIDC_JWKS_URL` unset, the API
reads the provider's discovery document
(`{OIDC_ISSUER}/.well-known/openid-configuration`) and uses its `jwks_uri`
(`SEC-38`). The document is read on the first token check — not at start-up,
so an unreachable provider does not stop the API — and cached with the key
set for five minutes, or re-read the moment a token names a key id the cache
does not know. It is trusted only when its own `issuer` equals
`OIDC_ISSUER` exactly and `jwks_uri` is an absolute `https` URL (`http` only
under an `http` issuer, which is how the local test providers are reached);
otherwise every token is refused with the same generic 401 and a `warn` line
names the failed check. Set `OIDC_JWKS_URL` to skip discovery entirely, for
a provider that publishes no discovery document or one this deployment
cannot reach.

### Keycloak

Keycloak is the automated gate's provider: a real OIDC server, not the mock.
Its override (`ops/sso/docker-compose.keycloak.override.yml`, profile `sso`)
and the gate (`ops/g10/g10_sso_test.py`) run Keycloak **in the compose
network only** — no host port is published. `docs/plans/SEC-3-RESULT.md`
records the gate's measured result.

| Setting | Local gate value | Real Keycloak |
| --- | --- | --- |
| `OIDC_ISSUER` | `http://keycloak:8080/realms/lakehouse` | `https://<host>/realms/<realm>` |
| `OIDC_AUTHORIZE_URL` | `.../protocol/openid-connect/auth` | same path on the host |
| `OIDC_TOKEN_URL` | `.../protocol/openid-connect/token` | same path on the host |
| `OIDC_JWKS_URL` | `.../protocol/openid-connect/certs` | same path on the host |
| `OIDC_REDIRECT_URI` | `http://lakehouse-api:8080/api/auth/oidc/callback` | `<api-origin>/api/auth/oidc/callback` |
| `OIDC_GROUPS_CLAIM` | `groups` | `groups` |

Discovery lives at `/realms/<realm>/.well-known/openid-configuration`; the
four endpoints above are its `authorization_endpoint`, `token_endpoint` and
`jwks_uri`. Keycloak publishes groups through an `oidc-group-membership-mapper`
whose `full.path` is `true` by default (it then emits `/top/level1`, not
`top/level1`) — set it `false` for the plain name `OIDC_ROLE_MAP` expects, as
the gate's realm template does. (Keycloak docs: `securing-apps/oidc-layers`,
`admin-api/protocol-mappers`.)

To run the gate by hand, put throwaway values in `.env` for
`KEYCLOAK_ADMIN_PASSWORD`, `OIDC_CLIENT_SECRET`, `SSO_USER_A_PASSWORD` and
`SSO_USER_B_PASSWORD` (`ops/init-env.sh` writes a `.env` if you have none;
fill the empty `OIDC_CLIENT_SECRET=` line rather than adding a second), then:

```bash
docker compose -p sso -f docker-compose.yml \
  -f ops/sso/docker-compose.keycloak.override.yml --profile sso \
  up -d --build lakehouse-api keycloak
docker compose -p sso -f docker-compose.yml \
  -f ops/sso/docker-compose.keycloak.override.yml --profile sso \
  run --rm sso-test-runner
docker compose -p sso -f docker-compose.yml \
  -f ops/sso/docker-compose.keycloak.override.yml --profile sso down -v
```

### Microsoft Entra ID

Register an application in the Entra admin center (App registrations → New
registration); note the **Application (client) ID** and **Directory (tenant)
ID**, and add a **Web** redirect URI equal to `OIDC_REDIRECT_URI`. Create a
client secret under Certificates & secrets. To carry groups, add the
**groups** optional claim (Token configuration → Add groups claim) to the ID
token. Entra's issuer and endpoints are per tenant:

| Setting | Value |
| --- | --- |
| `OIDC_ISSUER` | `https://login.microsoftonline.com/<tenant-id>/v2.0` |
| `OIDC_CLIENT_ID` | the application (client) id |
| `OIDC_CLIENT_SECRET` | the client secret value |
| `OIDC_AUTHORIZE_URL` | `https://login.microsoftonline.com/<tenant-id>/oauth2/v2.0/authorize` |
| `OIDC_TOKEN_URL` | `https://login.microsoftonline.com/<tenant-id>/oauth2/v2.0/token` |
| `OIDC_JWKS_URL` | optional — discovery names `https://login.microsoftonline.com/<tenant-id>/discovery/v2.0/keys`; set it only to skip discovery |
| `OIDC_REDIRECT_URI` | `<api-origin>/api/auth/oidc/callback` |
| `OIDC_GROUPS_CLAIM` | `groups` |

**Limits.** Entra's `groups` claim carries group **object ids** (GUIDs), not
names, so `OIDC_ROLE_MAP` keys are those GUIDs. When a user is in more groups
than the claim can hold, Entra replaces the claim with an overage marker
(`_claim_names`/`_claim_sources`, or `hasgroups` in the implicit flow) and
the group list must be fetched from Microsoft Graph instead — the console
does not follow that. `SEC-39` makes the case visible rather than silent:
the code flow this deployment uses carries `_claim_names`, so the sign-in
still succeeds, a `warn` line records the user and provider (never the
token or the claim source), and the console shows: "Your account is in more
groups than the sign-in token can list, so no group-based role was applied.
Ask an administrator to assign your role directly or to limit the groups
sent in the token." Following the Graph link to apply the mapped role
remains the full fix and is not built. (Microsoft docs:
<https://learn.microsoft.com/en-us/entra/identity-platform/access-token-claims-reference>,
the `groups` and overage claims.)

### Google

Create an OAuth client (Google Cloud console → APIs & Services →
Credentials → OAuth client ID, type **Web application**) and add the
authorized redirect URI equal to `OIDC_REDIRECT_URI`. Google publishes a
discovery document at
`https://accounts.google.com/.well-known/openid-configuration`, so
`OIDC_JWKS_URL` can stay unset; the value it names is shown below for an
operator who prefers to set it explicitly:

| Setting | Value |
| --- | --- |
| `OIDC_ISSUER` | `https://accounts.google.com` |
| `OIDC_CLIENT_ID` | the OAuth client id, ending `.apps.googleusercontent.com` |
| `OIDC_CLIENT_SECRET` | the client secret |
| `OIDC_AUTHORIZE_URL` | `https://accounts.google.com/o/oauth2/v2/auth` |
| `OIDC_TOKEN_URL` | `https://oauth2.googleapis.com/token` |
| `OIDC_JWKS_URL` | optional — `https://www.googleapis.com/oauth2/v3/certs` |
| `OIDC_REDIRECT_URI` | `<api-origin>/api/auth/oidc/callback` |
| `OIDC_GROUPS_CLAIM` | not usable — see below |

**Limit, stated honestly (D3):** Google's `id_token` carries no groups
claim, and there is no supported way to add one. So `OIDC_ROLE_MAP` cannot
map anything for Google: a Google sign-in gets whatever its local
`app_user_role` rows grant, and a JIT-provisioned account starts with none
until an administrator assigns a role. Group-based authorization on Google
is therefore not supported. (Google docs:
<https://developers.google.com/identity/openid-connect/reference>.)

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

## Dedicated-bucket backup job (OPS-7 / WS5 item G2)

The script-based procedure above covers only `${POSTGRES_DB:-lakehouse}`
(the app database) and writes to local disk. `dagster/dispar_orchestrate/
backup_job.py`, run via the `backups` compose profile, covers the entire
Lakehouse state (OPS-7 D1):
1. **The four Postgres databases:** the application database (`lakehouse`),
   Lakekeeper's REST catalog metadata (`lakekeeper`), OpenFGA's authorization
   tuples (`openfga`), and Dagster's run storage/event logs (`dagster`).
2. **The warehouse object store bucket:** full incremental mirror of all
   Apache Iceberg data and metadata files (streaming via `s3fs`) into
   `<BACKUP_S3_BUCKET>/warehouse/`.
3. **ClickHouse databases:** all non-catalog, non-system databases (such as
   `lake`, holding change-capture offsets and ingest facts) backed up via
   native `BACKUP DATABASE <db> TO S3(...)` into
   `<BACKUP_S3_BUCKET>/clickhouse/<timestamp>/<db>/`.

All components land in an **S3-compatible bucket dedicated to backups**,
never inside the shared warehouse bucket.

**Why a dedicated bucket, not the warehouse bucket — security, not
preference.** One of the four databases is the identity database
(`app_user`/`auth_identity`, holding password and service-token hashes).
Writing that dump into the same bucket every warehouse-scoped credential
(every ingestion connector, every Iceberg reader) can already read would
turn compromise of ANY ONE of those credentials into compromise of every
user's password hash — a privilege-escalation path the warehouse bucket's
access domain was never meant to grant. `backup_job.py` therefore reads
its own `BACKUP_S3_*` endpoint, bucket, and credentials, entirely
separate from `RUSTFS_*`/`CONNECTOR_S3_*`. Furthermore, read access
to the warehouse bucket uses dedicated read-only credentials
(`BACKUP_WAREHOUSE_ACCESS_KEY` / `BACKUP_WAREHOUSE_SECRET_KEY`), ensuring
the backup process cannot mutate live warehouse data.

### Configure

Set in your `.env`:
- Target backup bucket: `BACKUP_S3_ENDPOINT`, `BACKUP_S3_BUCKET`,
  `BACKUP_S3_ACCESS_KEY`, `BACKUP_S3_SECRET_KEY` (and `_SECRET_REF` vars).
- Source warehouse credentials (read-only): `BACKUP_WAREHOUSE_ACCESS_KEY`,
  `BACKUP_WAREHOUSE_SECRET_KEY` (and `_SECRET_REF` vars).
- Source warehouse target: `BACKUP_WAREHOUSE_ENDPOINT` (default: `http://rustfs:9000`),
  `BACKUP_WAREHOUSE_BUCKET` (default: `lakehouse-warehouse`).
- ClickHouse connection: `CH_URL` (default: `http://clickhouse:8123`),
  `CH_USER` (default: `default`), `CH_PASSWORD` (if configured).
- Retention: `BACKUP_RETENTION_DAYS` (default `30`) controls retention pruning
  for database dumps and ClickHouse backups. Warehouse mirrored objects are
  never pruned by age (historical Iceberg snapshots may still reference them).

These must-set credential values are **required** (`${VAR:?}`, no default)
in `ops/backups/docker-compose.backups.override.yml`, added with `-f` only
when a backup runs. Placing them in the override file ensures `docker compose
--profile '*' config --quiet` continues to validate cleanly in checkouts that
do not run backups.

### Run

```bash
docker compose -f docker-compose.yml \
  -f ops/backups/docker-compose.backups.override.yml \
  --profile backups run --rm backup-job
```

Without the `-f ops/backups/...` override (or with any required variable
unset), this refuses to start — a `backups` profile enabled with no
bucket configured is a clear startup failure, never a silent write into
the wrong place, and never a silently-skipped backup reported as
successful. A failure in any component raises immediately (`dagster.Failure`)
and aborts the run.

**Scheduling:** this is not (yet) wired as an automatic in-process
Dagster job/schedule — see `backup_job.py`'s module doc for why (in
short: doing so safely would require threading these same bucket/DB
credentials into the always-on `dagster-code-location` service, which
would make every `dagster`-profile deployment require backup
configuration too). Run the command above from an external trigger (host
cron, a systemd timer, or a CI scheduled job) on whatever cadence your
retention policy assumes (nightly, matching `BACKUP_RETENTION_DAYS`'s
default 30-day window, is a reasonable starting point).

### Full restore procedure (`scripts/restore-from-backup.sh`)

A full recovery across all Lakehouse components is automated by
`scripts/restore-from-backup.sh`.

```bash
# Preview usage and options
scripts/restore-from-backup.sh --help

# Execute restore from a specific timestamp
scripts/restore-from-backup.sh 20261016T074234Z

# Force restore even if local data volumes are not empty
scripts/restore-from-backup.sh --force 20261016T074234Z
```

**Restoration order (OPS-7 E4):**
1. **Empty-volume safety check:** The script checks all compose data volumes
   (`postgres_data`, `rustfs_data`, `clickhouse_data`) and refuses to run
   if existing data is found unless `--force` is passed, preventing partial
   overwrite corruption.
2. **Step 1: PostgreSQL restore alone first:** Starts Postgres in isolation
   while other containers are down, drops and recreates the 4 databases
   (`lakehouse`, `lakekeeper`, `openfga`, `dagster`), and restores each
   from `s3://<BACKUP_S3_BUCKET>/<db>/<timestamp>.dump` via `pg_restore`.
   Restoring alone first guarantees that Lakekeeper and API initialization
   do not race with or destructively overwrite restored tables.
3. **Step 2: Warehouse bucket restore:** Starts RustFS (or object store) and
   syncs `s3://<BACKUP_S3_BUCKET>/warehouse/` back to
   `s3://<BACKUP_WAREHOUSE_BUCKET>/`.
4. **Step 3: ClickHouse restore:** Starts ClickHouse, discovers all backed-up
   databases under `s3://<BACKUP_S3_BUCKET>/clickhouse/<timestamp>/`, drops
   stale databases, and runs `RESTORE DATABASE <db> FROM S3(...)`.
5. **Step 4: Stack bring-up:** Starts the remaining services
   (`lakekeeper`, `openfga`, `lakehouse-api`, `dagster`) and waits for
   readiness.

**Verified Recovery Performance (OPS-7-AC3):**
Tested end-to-end in a timed recovery drill (`ops7drill`). Total restore
duration across all 4 Postgres databases, 9 Iceberg objects, and ClickHouse
state was **82 seconds** (94 seconds total to full API readiness). See
[docs/plans/OPS-7-RESULT.md](plans/OPS-7-RESULT.md) for the complete drill
report and component timing table.

## Local-disk backup (REL-2 S1-T4)

The bucket job above needs an S3-compatible bucket. An air-gapped host has
none, so there is a second target: the local disk. One command, no bucket,
no network.

### What one backup is

A directory `backups/<UTC timestamp>-<version>/` (the version is the
API's own, read from `GET /health`):

| Part | What it is |
| --- | --- |
| `postgres.dump` | `pg_dumpall` of the whole Postgres instance: the application database, Lakekeeper's, OpenFGA's, Dagster's, and the roles. |
| `clickhouse/<timestamp>/<db>/` | `ClickHouse`'s own `BACKUP DATABASE ... TO Disk('backups', ...)` output, per database. The disk is configured in `ops/backups/clickhouse/backups.xml` and mounted into the `clickhouse` service. |
| `connector-secrets.tar.gz` | The `lakehouse_connector_secrets` volume — every stored connector credential, **sealed**. Without it, a restored deployment has no working connector credential. |
| `env` | A copy of the deployment's `.env` — the generated secrets, including the key those credentials are sealed with. |
| `manifest.json` | Each part's size and SHA-256. |

The restore checks those checksums **before** it restores anything: a
backup that was cut short is refused, not restored as if it were whole.

**What it does not carry yet:** `release.json` and `state.json` (the
release manifest and the installed-version state) are part of the same
backup set in the plan, but they do not exist yet — REL-2 S2 publishes the
release manifest and S4 writes the state file. The backup script says so
in its output rather than pretending.

### Run

```bash
# One backup into ./backups/<timestamp>-<version>/
ops/backups/backup-local.sh

# Into another directory
ops/backups/backup-local.sh /mnt/backups

# A stack that is not named after its directory
COMPOSE_PROJECT=my-stack ops/backups/backup-local.sh
```

It needs Docker, `docker compose`, `curl`, `jq` and `sha256sum` on the
host — the PostgreSQL and ClickHouse clients are the ones in the running
containers, so the host needs no database client of its own.

### Restore

```bash
# The backup's checksums are checked first; nothing is written before they pass.
scripts/restore-from-backup.sh --local backups/20261031T090000Z-0.1.0

# Force it on top of volumes that are not empty
scripts/restore-from-backup.sh --force --local backups/20261031T090000Z-0.1.0
```

The order is: the empty-volume safety check (the same one the bucket
restore has), the manifest's checksums, Postgres, ClickHouse, the
connector secrets, the `.env` (the previous one is kept beside it as
`.env.before-restore-<timestamp>`), and the stack bring-up.

**The connector credentials only decrypt with the `.env` they were sealed
with.** A restore whose `.env` is not the one the backup carries leaves
every connector unreadable — which is why the `.env` is part of the backup
and why the restore puts it back.

**Measured** (REL-2 S1-T4's own drill, on a clean `-p rel2` project):
Postgres 8 s, ClickHouse 7 s, the connector secrets 1 s, the stack
bring-up 16 s — **32 s** in total, and the restored connector's credential
still decrypted: the API's own key-rotation command opened the restored
sealed file with the restored key and reported `1 already under the
current key, 0 failed`.

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

### Where the setting is described

`CATALOG_TENANT_ID` is one environment variable of the API service. It is
declared in `docker-compose.yml` with an empty default — an unset value
degrades to the documented refusal, not a boot failure — and listed in
`.env.example` with a comment that says the same thing as this section
and this page's "What the setting is for now" below. The API reads it
at startup (`Config::catalog_tenant_id`), and every route that answers
"may this caller read the shared catalog" asks the same question of it —
there is no second setting and no second rule.

### What the setting is for now (`SEC-50`)

The setting is no longer about two lists. Every statement that reads
the shared catalog passes one tenant gate, and the deployment's shared
catalog belongs to the tenant this setting names:

- **the authored pipelines** act as their own tenant
  (`pipeline_definition.tenant_id`), so a pipeline of tenant B can no
  longer read a `bronze.*` upload table that tenant A claimed;
- **the Gold export** is gated by whoever asked for it: a signed-in
  person acts as themselves, so their own tenant must own the shared
  catalog, while a service identity or the shared run token — the
  scheduled export, a manual trigger done as a service — has no user
  behind it and acts as the tenant that owns the shared catalog. The
  assistant's own tool is a **signed-in person's** call, so it is gated as
  that person, not as the catalog's tenant. A deployment with several
  tenants and no `CATALOG_TENANT_ID` **refuses the job-with-no-user
  export outright**, with a 422 that names this setting; a signed-in
  person of another tenant is refused too, but with the gate's own 404
  (the mart reads "not available to your organisation");
- the **dashboards, the embeds, the public links and the alerts** read the
  same shared catalog and follow in the second slice of `SEC-50`. They are
  **not** gated yet, so they still serve what they served before.

**What this means for an operator.** On a deployment with more than one
tenant, this setting is **required** for the gated paths to work at all:
without it there is no catalog tenant in the first place, so a job with
no user (the Gold export) is refused with the 422 that names this
setting, and through the paths the tenant gate covers (Query Studio, the
authored pipelines) a member reads only the uploads their own tenant
claimed. The dashboards, the embeds, the public links and the alerts are
not gated yet, so a member of another tenant still sees what those paths
show today. With the setting set, the members of that tenant read the
shared catalog as before, and the other tenants read only their own
uploads. A **single-tenant** deployment is unaffected either way.

### Authored pipelines with no tenant (`SEC-50`)

Migration `0042_tenant_provisioning.sql` added `tenant_id` to
`pipeline_definition` and **backfilled nothing**: every pipeline created
before that migration has a NULL tenant, and so has every pipeline an
operator created without a tenant. `SEC-50` refuses such a pipeline — its
enforce call (`POST /api/pipelines/runnable/{id}/enforce`, the one the
orchestrator makes) answers **409** with:

```
This pipeline belongs to no tenant. An administrator assigns one with PUT /api/pipelines/{id}/tenant.
```

A pipeline whose **author** holds the unrestricted (`*:*`) grant is
exempt: such an author reads every table anyway, so its pipeline is not
refused for a tenant it never had.

**How to find them.** `GET /api/pipelines` is scoped to the caller's own
tenant, so an unassigned pipeline is invisible there — that is the point
of the scoping, and it is also why the database is the place to look:

```sql
SELECT id, name, run_as FROM pipeline_definition WHERE tenant_id IS NULL;
```

One exception, so the sentence above is not read as "nobody can see them":
a Platform Admin (`*:*`) who belongs to no tenant of their own sees every
authored pipeline on that list, unassigned ones included (PR #281 review
round 2 NIT).

**How to assign one.** `PUT /api/pipelines/{id}/tenant` with the tenant's
id — the route requires the `identity:write` permission (PR #281 review
round 2 NIT):

```json
{ "tenantId": "11111111-1111-4111-8111-000000000001" }
```

The route answers 404 for an unknown pipeline id or an unknown tenant id
(the same status for both), and 204 on success.

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

## Outside tools read published tables (DATA-5, ADR 0016)

A published Gold mart lives in the catalog's `gold` namespace, and until
DATA-5 nobody outside this deployment could read it: the catalog vends the
storage address `http://rustfs:9000`, which resolves only inside the
deployment. This section is how an OUTSIDE tool — a BI tool, a notebook,
DuckDB — reads a published mart, and how the console counts who does.

**The API is the only door.** The tool talks to the console's own
`/api/iceberg` — the read side of the Iceberg REST catalog — and the API
forwards each allowed call to the catalog under one read-only principal of
its own. The tool's own key never reaches the catalog, so a tool cannot
widen its own reach, and every read through the door is counted.

### 1. Publish the storage address

`OUTSIDE_S3_ENDPOINT` is the address an outside tool can REACH: the
RustFS (or S3) host and port, behind TLS, on the public side of the
deployment. It is empty by default, and empty means outside reading is
OFF — every door route answers 503 naming that setting, and the API
reports the read count as not measured, with the same reason. Never a
count of zero.

```bash
# .env, or the deployment's own configuration
OUTSIDE_S3_ENDPOINT=https://storage.example.com
```

Restart `lakehouse-api` afterwards. The API swaps the vended `s3.endpoint`
for this one in a table-load and a credentials response; the vended KEYS
stay the catalog's own short-lived, table-scoped ones.

Two addresses, two situations:

- A tool running OUTSIDE the deployment needs the published address
  (`https://storage.example.com`).
- A tool running INSIDE the deployment's network — the acceptance test is
  one — needs the in-network name. In the local compose stack that is
  `http://rustfs:9000` (or `http://localhost:9010`, the host port
  RustFS publishes, for a tool on the same machine as the browser).

### 2. Create the identity and get its key

An outside tool is a service identity with the `iceberg:read` scope. On
the console's **Administration → Service Identities** page: create one,
name it after the tool (`Finance DuckDB`), add the scope `iceberg:read`,
then press **Rotate**. The key is shown ONCE — copy it then. Rotating
again issues a fresh key and kills the old one immediately; deleting the
identity stops the tool as well, and its past reads stay listed under the
name it had.

The same two calls over HTTP (what the acceptance test does):

```bash
# Create the identity. Needs the `identity:write` permission.
curl -X POST "$CONSOLE/api/identity/service-identities" \
  -H "Content-Type: application/json" \
  -b cookies.txt \
  -d '{"name":"Finance DuckDB","scopes":["iceberg:read"],"environment":"production"}'

# Rotate it for the key. The key is in `secret`, and is never served again.
curl -X POST "$CONSOLE/api/identity/service-identities/<id>/rotate" -b cookies.txt
```

### 3. Point the tool at the door

**pyiceberg.** `load_catalog("rest", …)` builds pyiceberg's
`RestCatalog`:

```python
from pyiceberg.catalog import load_catalog

catalog = load_catalog(
    "rest",
    uri="https://console.example.com/api/iceberg",  # the DOOR, not the catalog
    token="<the key>",
    warehouse="default",                            # the warehouse's name
)
table = catalog.load_table(("gold", "gold_export_smoke"))
print(table.scan().to_arrow().num_rows)
```

**DuckDB.** The `iceberg` extension attaches the REST catalog as a
database:

```sql
INSTALL iceberg;
LOAD iceberg;

ATTACH 'lakehouse' AS cat (
    TYPE iceberg,
    URI 'https://console.example.com/api/iceberg',
    AUTHORIZATION_TYPE 'OAUTH2',
    TOKEN '<the key>'
);

SELECT count(*) FROM cat.gold.gold_export_smoke;
```

Both examples were run against this stack: pyiceberg read the published
table and was refused 403 on a write and on another namespace; DuckDB
read the published table.

Two things worth knowing about the door, because they surprise clients:

- The **warehouse** in the request is the deployment's own
  (`LAKEKEEPER_WAREHOUSE`, `default` in this stack). The door serves that
  one warehouse and replaces whatever the client asked for, so a client
  cannot reach another one by naming it.
- The door answers the client's config call with the warehouse's own
  prefix, which the client then uses in every path. That is why the
  client needs no Lakekeeper knowledge at all.

### 4. What the tool may read

- Only the `gold` namespace. Every other namespace, and every write, is
  refused 403 with the Iceberg error shape.
- Only a service identity may use the door. A browser session is refused:
  the count is per tool, and a session is not a tool.

### 5. The count

The count shows up in the console on a published table as "Readers (7
days)": each tool, its reads and its last read. A table nobody read says
"No reads in 7 days"; a deployment with outside reading off says "Not
measured" with the reason. The API behind it is
`GET /api/gold/export/{mart}/consumers`, which answers the same three
states: the list, an empty list with `supported: true` (measured, and
nobody read it), or `supported: false` with the reason.

The count is the API's own: one read per table load through the door. A
read that skips the door — internal storage keys, or the catalog's own
published port — is NOT counted. Those credentials are for internal
services only.

### Limits

- Only the catalog tenant's warehouse is served. Tenant warehouses are not.
- Tools pass the key as a bearer token. The OAuth2 client-credentials
  exchange is not offered.
- A tool that skips the door is not counted.
- Each publish of a changed mart appends a full copy, so a tool must
  select the latest export time (`_exported_at`) or it sees every past
  copy — see "Gold export: growth and background merges" above.
- The read history is kept for 90 days; the page shows 7.

## Releases: where a release comes from

A release is produced by the pipeline in `release.yml`, not by hand: the four images are
built and pushed, signed, and given a CycloneDX SBOM; the Compose file an operator
deploys is RENDERED from this repository's template with every image pinned by digest;
and one signed manifest, `release.json`, names the version, every image's digest, the
Compose file's checksum and the upgrade path. Everything an installer needs is checked
through that one file.

What that means for an operator:

- the Compose file in a release is NOT this repository's `docker-compose.yml`. It is the
  rendered one, and it names published images instead of building them;
- the images are signed, and the manifest is signed with the release key. The public key
  is `ops/release/release.pub`;
- the offline bundle holds the Compose file, the manifest, its signature, one image per
  file, the SBOMs and the checksums.

The pipeline itself, the key, the draft release and the one manual step after the first
release are documented in [CI.md](CI.md#release-pipeline). The tooling is in
`ops/release/`.
