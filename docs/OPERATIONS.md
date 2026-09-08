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
docker compose up --build
```

brings up:

| Service | Image | Purpose |
| --- | --- | --- |
| `postgres` | `postgres:16` | OLTP store (identity, governance, pipelines, connectors, alerts, ...) |
| `clickhouse` | `clickhouse/clickhouse-server:26.3` | Analytics store (catalog, overview, governance, dashboards, ...); also the Iceberg query engine once `DataLakeCatalog` is wired up in P1b |
| `lakehouse-api` | built from `rust/Dockerfile` | the axum API, port 8080 |
| `rustfs` | `rustfs/rustfs:1.0.0-rc.4` | S3-compatible object store for the lakehouse warehouse (P1 infrastructure; not yet wired into `lakehouse-api`) |
| `rustfs-bucket-init` | `amazon/aws-cli:2.36.34` | One-shot: creates the warehouse bucket via the plain S3 API (`s3api create-bucket`) — never RustFS's admin API |
| `lakekeeper-db-init` | `postgres:16` | One-shot: creates Lakekeeper's own database on the existing `postgres` service |
| `lakekeeper-migrate` | `quay.io/lakekeeper/catalog:v0.13.3` | One-shot: Lakekeeper's own `migrate` subcommand against its database |
| `lakekeeper` | `quay.io/lakekeeper/catalog:v0.13.3` | Iceberg REST catalog (Rust, Apache-2.0); the only path for Iceberg writes — no path-based `IcebergS3` tables |
| `lakekeeper-warehouse-init` | `alpine:3.20` | One-shot (P1b): bootstraps the Lakekeeper server (`/management/v1/bootstrap`) and creates the `LAKEKEEPER_WAREHOUSE` warehouse against the RustFS bucket, with `sts-enabled: true` — required for Lakekeeper to vend real S3 credentials on `X-Iceberg-Access-Delegation: vended-credentials`; see `docker-compose.yml`'s comment on this service and `lakehouse-iceberg::catalog`'s module doc |

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

### ClickHouse 24.8 → 26.3: what changed and what was verified

Bumped because Iceberg writes via ClickHouse's `DataLakeCatalog` need
`clickhouse-server >= 26.2` (for `allow_database_iceberg`); 26.3 is the
current LTS tag. Verified clean:

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

Copy `.env.example` to `.env` first (`docker compose` auto-loads `.env`
from the project root) and set at minimum `AUTH_BOOTSTRAP_EMAIL` /
`AUTH_BOOTSTRAP_PASSWORD`. Every other variable has a safe local default.

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
docker compose --profile dagster up -d --build \
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
- **A real LLM.** Needs a paid API key. AI chat / agent / text-to-SQL
  routes return `503` without `LLM_KEY` (or `MINIMAX_API_KEY`) set to a
  working key.

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
| AI chat / agent / text-to-SQL | LLM API key | `503` from `/api/ai/*`, `/api/agent/*` | Set `LLM_URL`/`LLM_MODEL`/`LLM_KEY` (or `MINIMAX_API_KEY`) to a real OpenAI-compatible provider |
| Alert digests / threshold emails | SMTP | Alerts still evaluate; email delivery silently no-ops | Set `SMTP_HOST` (and friends) to a real SMTP relay |
| Signed dashboard embeds | `EMBED_SECRET` | Embed routes unavailable | Set `EMBED_SECRET` |
| SSO / OIDC login | An OIDC provider | Local password auth only | Set `OIDC_ISSUER` + `OIDC_CLIENT_ID` (see `rust/crates/lakehouse-auth/README.md`) |

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
