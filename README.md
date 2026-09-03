<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="public/logo-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="public/logo-light.png">
    <img alt="RantAI Lakehouse logo" src="public/logo-light.png" width="160">
  </picture>
</p>

<h1 align="center">RantAI Lakehouse</h1>

<p align="center">
  A data-lakehouse console for browsing a catalog, running pipelines,
  building dashboards, and chatting with an LLM over your data.
</p>

RantAI Lakehouse is a data-lakehouse console: a web UI for browsing a data
catalog, running and scheduling pipelines, building dashboards, managing
governance policies, and chatting with an LLM over your data. It's a
Next.js/React frontend backed by a Rust (axum) API that talks to Postgres,
ClickHouse, Dagster, and an OpenAI-compatible LLM.

The backend was originally written in TypeScript (as Next.js API routes)
and has since been fully ported to Rust; see "Status / Known limitations"
below and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for what that means
in practice.

## Architecture

```mermaid
flowchart LR
    Browser["Browser"]

    subgraph Frontend["Next.js (Bun runtime)"]
        UI["UI (App Router)"]
        Rewrite["/api/* rewrite\n(next.config.ts)"]
    end

    subgraph Backend["Rust backend"]
        API["lakehouse-api (axum)"]
    end

    Postgres[("Postgres\n(OLTP: identity, governance,\npipelines, connectors, ...\nalso the CDC source, wal_level=logical)")]
    ClickHouse[("ClickHouse 26.3\n(analytics: serving.* marts,\ncatalog, lineage, BI;\nalso reads Bronze via DataLakeCatalog)")]
    Dagster["Dagster\n(orchestration, opt-in profile:\nbatch ingest + Bronze maintenance)"]
    LLM["LLM\n(OpenAI-compatible)"]

    subgraph Lakehouse["Lakehouse layer (P1-P5)"]
        Lakekeeper["Lakekeeper\n(Iceberg REST catalog)"]
        RustFS[("RustFS / SeaweedFS\n(S3-compatible object store)")]
        Debezium["Debezium Server\n(CDC, opt-in profile)"]
        Trino["Trino-as-cron\n(Bronze compaction only,\nopt-in profile, ADR 0009)"]
    end

    Browser --> UI
    UI --> Rewrite
    Rewrite --> API
    API --> Postgres
    API --> ClickHouse
    API --> Dagster
    API --> LLM
    Dagster --> Lakekeeper
    Debezium --> Lakekeeper
    Debezium --> Postgres
    Lakekeeper --> RustFS
    ClickHouse -. reads Bronze via .-> Lakekeeper
    Trino -. compacts Bronze via .-> Lakekeeper
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full module map (13
Rust crates), the request lifecycle (browser → rewrite → axum middleware →
policy check → handler → store/client), and the Postgres-vs-ClickHouse data
model.

The lakehouse layer (Lakekeeper, RustFS/SeaweedFS, Debezium, Trino) is real
infrastructure, verified end to end (see `docs/plans/G1-RESULT.md` through
`docs/plans/P5-RESULT.md`), and is compose-profile opt-in for the
Dagster/Trino pieces (`dagster`, `trino`). **`lakehouse-iceberg` now has a
real caller**: `POST`/`GET /api/gold/export/{mart}`
(`lakehouse-api::routes::gold`, ADR 0010) reads a Gold mart from
ClickHouse `MergeTree` and appends it to its own `gold` Iceberg namespace
through Lakekeeper, and reads it back through `iceberg-rust` — the console
still reads *Bronze* through ClickHouse's `DataLakeCatalog`/the
`bronze_meta.*` registry, not through `lakehouse-iceberg` directly, since
that read path already works on ClickHouse 26.3 (see "Status / Known
limitations" below for the write-path defects that do not).

## Quickstart

Assumes you have **Docker** and **Bun** (`>= 1.3.0`) installed and nothing
else.

```bash
# 1. Install Bun, if you don't have it
curl -fsSL https://bun.sh/install | bash

# 2. Clone and install frontend dependencies
git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git
cd RantAI-Lakehouse
bun install

# 3. Configure the backend stack. Copy the example env file and set (at
#    minimum) AUTH_BOOTSTRAP_EMAIL / AUTH_BOOTSTRAP_PASSWORD — without
#    those there is deliberately no way to log in. Every other variable
#    has a safe local default; see .env.example and the Configuration
#    table below.
cp .env.example .env
$EDITOR .env

# 4. Bring up Postgres, ClickHouse, and the Rust API (built from
#    rust/Dockerfile). Dagster and a real LLM are NOT part of this stack —
#    see docs/OPERATIONS.md for what that means and how to add them.
docker compose up --build
# lakehouse-api listens on :8080 once postgres and clickhouse report
# healthy. Check with: curl -sf localhost:8080/health

# 5. In a separate terminal, point the frontend at the backend and run it
RUST_API_URL=http://localhost:8080 bun --bun next dev
```

Open [http://localhost:3000](http://localhost:3000). Sign in with the
bootstrap admin account you configured via `AUTH_BOOTSTRAP_EMAIL` /
`AUTH_BOOTSTRAP_PASSWORD` (see the Configuration table) — without those set,
no admin account is created and you'll need to seed one directly in
Postgres.

See [docs/OPERATIONS.md](docs/OPERATIONS.md) for what's in/out of the
compose stack, what `/health` actually checks, the Postgres backup/restore
procedure, and the biggest operational trap in this codebase (Postgres
being down fails quietly, not loudly).

Prefer running the Rust API directly instead of in a container (e.g. for
faster iteration)? The old path still works:

```bash
# Bring up just the data stores from compose, then run the API on the host
docker compose up postgres clickhouse
cd rust && cargo run -p lakehouse-api
```

### Runtime: Bun

This project runs on **Bun**, not Node.js. You need Bun `>= 1.3.0`
([install](https://bun.sh/docs/installation)):

```bash
curl -fsSL https://bun.sh/install | bash
```

All scripts (`dev`, `build`, `start`, `lint`, `typecheck`) use the `--bun`
flag so Next.js executes under the Bun runtime, not Node. The lockfile is
`bun.lock` — don't use `npm install`, it will create an out-of-sync
`package-lock.json`.

> Note: in `ps`, the server process shows up as `node` because Bun
> intentionally masquerades as Node for tooling compatibility. To confirm
> you're actually running Bun, check `readlink /proc/<pid>/exe` — it
> resolves to the `bun` binary.

```bash
# Development
bun run dev

# Production build, then run
bun run build
bun start
```

### Test, lint, typecheck

```bash
bun run test        # bun test (unit tests under src/lib)
bun run lint         # eslint
bun run typecheck    # tsc --noEmit
```

Rust backend, from `rust/`:

```bash
cargo test --all-features
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo deny check licenses
```

### Rust toolchain / MSRV

The workspace declares `rust-version = "1.88"` in `rust/Cargo.toml` — that's
the real floor this code has been verified to compile on (`Cargo.lock`
pulls in `time@0.3.55`, which needs rustc >= 1.88, plus
`testcontainers`/`etcetera`/`ferroid` transitively pushing past the raw
edition-2024 minimum of `1.85`; see `rust/Cargo.toml`'s comment and
`docs/CI.md` for how that was verified). `rust/rust-toolchain.toml` pins
the toolchain actually used for local dev/CI (currently `1.96.1`, newer
than the MSRV floor) so contributors and CI build with the same compiler;
`rustup` will fetch it automatically the first time you run `cargo` inside
`rust/`.

## Configuration

Every environment variable below is read in
[`rust/crates/lakehouse-api/src/config.rs`](rust/crates/lakehouse-api/src/config.rs),
which is the authoritative source — this table was generated from it, not
guessed.

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `CH_URL` | ClickHouse HTTP interface URL | `http://localhost:18123` | No |
| `CH_USER` | ClickHouse basic-auth user | `default` | No |
| `CH_PASSWORD` | ClickHouse basic-auth password | `""` | No |
| `DAGSTER_URL` | Dagster GraphQL endpoint | `http://localhost:13030/graphql` | No |
| `DAGSTER_REPO` | Dagster repository name | `__repository__` | No |
| `DAGSTER_LOCATION` | Dagster repository location | `dispar_orchestrate.definitions` | No |
| `LLM_URL` | LLM chat-completions base URL (OpenAI-compatible) | `https://api.minimax.io/v1` | No |
| `LLM_MODEL` | LLM model name | `MiniMax-M3` | No |
| `LLM_KEY` | LLM API key. Falls back to `MINIMAX_API_KEY` if unset **or empty** (`||` semantics, not `??`) | `""` | No (but AI features won't work without it) |
| `MINIMAX_API_KEY` | Fallback for `LLM_KEY` | — | No |
| `EMBED_SECRET` | HMAC signing secret for signed dashboard embeds | unset (embedding disabled) | No |
| `ALERTS_RUN_TOKEN` | Shared bearer token required to call `POST /api/alerts/run` | unset | No, but the endpoint fails closed (401) when unset — see Security notes below |
| `SMTP_HOST` | SMTP host for alert/digest email delivery | unset (email disabled) | No |
| `SMTP_PORT` | SMTP port. Invalid values log a warning and fall back to the default rather than failing boot | `587` | No |
| `SMTP_SECURE` | Force implicit TLS (`"true"`). Effective value is also `true` whenever `SMTP_PORT` is `465`, even if this is unset | `false` | No |
| `SMTP_USER` | SMTP auth username | unset (no SMTP auth) | No |
| `SMTP_PASS` | SMTP auth password | `""` | No |
| `SMTP_FROM` | `From` header for outgoing email. Falls back to `SMTP_USER`, then to `rantai-lake@localhost` | `rantai-lake@localhost` | No |
| `PORT` | Port `lakehouse-api` listens on | `8080` | No (but an invalid value fails to boot — the one field that does) |
| `APP_ENV` | `"development"` or `"local"` relaxes the session cookie's `Secure` attribute for local HTTP dev. Falls back to `NODE_ENV`. Never bypasses authentication | unset (defaults closed/`Secure`) | No |
| `NODE_ENV` | Fallback for `APP_ENV` | — | No |
| `AUTH_BOOTSTRAP_EMAIL` | Email for the idempotent bootstrap admin account created at startup | unset (no bootstrap admin) | No, but recommended for first run |
| `AUTH_BOOTSTRAP_PASSWORD` | Password for the bootstrap admin account | unset | No, but required alongside `AUTH_BOOTSTRAP_EMAIL` to actually create one |
| `OIDC_ISSUER` | OIDC provider issuer URL | unset | No — OIDC requires both this and `OIDC_CLIENT_ID` |
| `OIDC_CLIENT_ID` | This app's client id as registered with the OIDC provider | unset | No — see above |
| `OIDC_CLIENT_SECRET` | Reserved for a future authorization-code exchange; not currently read by `OidcAuthenticator` | unset | No |
| `OIDC_PROVIDER_NAME` | Short label combined with `oidc:` to form `Principal::provider` / `auth_identity.provider` | `default` | No |
| `OIDC_JWKS_URL` | Explicit JWKS endpoint override | derived: `{OIDC_ISSUER}/.well-known/jwks.json` | No |
| `OIDC_JIT_PROVISIONING` | `"true"` to auto-create an `app_user` for a validating token with no linked identity yet | `false` | No |
| `OIDC_ROLE_MAP` | `"group1=Role One,group2=Role Two"` — maps an IdP group/role claim to a local role name | empty | No |
| `OIDC_GROUPS_CLAIM` | Which token claim carries the caller's groups/roles | `groups` | No |
| `OIDC_CLOCK_SKEW_SECONDS` | Clock-skew tolerance for `exp`/`nbf` validation. Invalid values fall back to the default | `60` | No |
| `DATABASE_URL` | Postgres connection string for Phase 2 OLTP storage | `postgres://lakehouse:lakehouse@localhost:5432/lakehouse` | No (but a wrong/unreachable value means every Phase 2 route returns 503 — see below) |

Two additional variables live outside `config.rs`, on the frontend side —
listed here because you need them to run the console at all:

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `RUST_API_URL` | Target the Next.js `/api/*` rewrite proxies to (`next.config.ts`) | unset (rewrite disabled — no backend reachable) | Yes, to reach the Rust backend at all |
| `NEXT_PUBLIC_SSO_ENABLED` | Build-time flag that shows/hides SSO login UI | unset (SSO UI hidden) | No — see "SSO configuration is split across two processes" below |

See `rust/crates/lakehouse-auth/README.md` for detailed, per-provider OIDC
setup instructions (Okta, Entra, Google, Keycloak).

A further set of variables live only in `docker-compose.yml` — they
configure the P1 object store (RustFS) and Iceberg REST catalog
(Lakekeeper) *services themselves* (ports, Lakekeeper's own Postgres
database, its encryption key) and are not read by `config.rs`, because
they configure the container, not the `lakehouse-iceberg` client that
talks to it:

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `RUSTFS_ACCESS_KEY` | RustFS S3 API access key. Also passed into `lakehouse-api`'s own container env, where `conn-s3-warehouse`'s `secretRef` (`env:RUSTFS_ACCESS_KEY`) resolves it for a real connectivity test | `rustfsadmin` (public, well-known) | No, but override before exposing RustFS beyond localhost |
| `RUSTFS_SECRET_KEY` | RustFS S3 API secret key. Same "also passed into `lakehouse-api`" note as `RUSTFS_ACCESS_KEY` above (`env:RUSTFS_SECRET_KEY`) | `rustfsadmin` (public, well-known) | No, but override before exposing RustFS beyond localhost |
| `RUSTFS_HOST_PORT` | Host port mapped to RustFS's S3 API (container port 9000) | `9010` | No |
| `RUSTFS_CONSOLE_HOST_PORT` | Host port mapped to RustFS's web console (container port 9001) | `9011` | No |
| `LAKEKEEPER_PG_DB` | Name of Lakekeeper's own Postgres database on the existing `postgres` service (separate from the `lakehouse` app database's `console` schema) | `lakekeeper` | No |
| `LAKEKEEPER_ENCRYPTION_KEY` | Encrypts secrets in Lakekeeper's own schema | Lakekeeper's own placeholder — **change before any non-throwaway use** | No |
| `LAKEKEEPER_BASE_URI` | Base URL Lakekeeper advertises in its own REST responses | `http://localhost:8181` | No |
| `LAKEKEEPER_HOST_PORT` | Host port mapped to Lakekeeper's REST API (container port 8181) | `8181` | No |
| `LAKEKEEPER_OPENFGA_STORE_NAME` | Name of the OpenFGA store Lakekeeper's authorization model lives in | `lakekeeper` | No |
| `OPENFGA_PG_DB` | Name of OpenFGA's own Postgres database on the existing `postgres` service | `openfga` | No |
| `OPENFGA_HTTP_HOST_PORT` | Host port mapped to OpenFGA's HTTP API (container port 8080) | `8082` | No |
| `OPENFGA_GRPC_HOST_PORT` | Host port mapped to OpenFGA's gRPC API (container port 8081) — this is the port Lakekeeper's `LAKEKEEPER__OPENFGA__ENDPOINT` actually talks to | `8083` | No |
| `OIDC_MOCK_HOST_PORT` | Host port mapped to `ops/oidc-mock`'s discovery/JWKS/token endpoints (container port 8090) | `8090` | No |

P1b (`lakehouse-iceberg`) adds the client-side counterparts below, read by
`config.rs` — these are what a Rust process (not the container) uses to
*connect to* RustFS/Lakekeeper. `lakehouse-api`'s Gold export route
(`routes::gold`, ADR 0010) is the first route to actually build an
`IcebergClient` from these fields; the G1 test and any manual `cargo run`
usage share the same documented source docker-compose already uses:

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `LAKEKEEPER_CATALOG_URI` | Lakekeeper's Iceberg REST catalog base URI, as reached from the Rust process | `http://localhost:8181/catalog` | No |
| `LAKEKEEPER_WAREHOUSE` | Lakekeeper warehouse this deployment writes Bronze tables into — see ADR 0003 for the `TENANT_ID` naming convention | `default` | No |
| `LAKEKEEPER_CREDENTIAL_SECRET_REF` | `secretRef` (see `lakehouse_core::secret`, ADR 0002) for Lakekeeper's OAuth2 client-credential, when Lakekeeper authorization is enabled | unset (no-auth mode assumed) | No |
| `LAKEKEEPER_GOLD_EXPORT_TOKEN_FILE` | File path to the `gold-export` Lakekeeper principal's pre-minted static bearer token (ADR 0011), read at export-request time — not a `secretRef`, see the field's doc comment for why | `/tokens/gold-export.jwt` | No |
| `GOLD_SOURCE_SCHEMA` | ClickHouse schema `routes::gold` reads Gold marts from | `serving` | No |
| `GOLD_EXPORT_RUN_TOKEN` | Shared token gating `POST`/`GET /api/gold/export/{mart}` (same D4 shape as `ALERTS_RUN_TOKEN`); unset means only a service-identity principal may call it | unset | No |
| `RUSTFS_S3_ENDPOINT` | S3-compatible endpoint the `lakehouse-iceberg` `object_store` client targets | `http://localhost:9010` | No |
| `RUSTFS_S3_REGION` | Region string sent to the S3 client (RustFS does not enforce AWS region semantics, but the S3 API requires a value) | `us-east-1` | No |
| `LAKEHOUSE_WAREHOUSE_BUCKET` | Bucket the lakehouse warehouse's Iceberg tables live under — also read by the compose `rustfs-bucket-init` job | `lakehouse-warehouse` | No |
| `RUSTFS_ACCESS_KEY_SECRET_REF` | `secretRef` for a static RustFS/S3 access key, used only as a fallback outside the vended-credentials write path (see `lakehouse-iceberg`'s crate doc) | unset | No |
| `RUSTFS_SECRET_KEY_SECRET_REF` | `secretRef` for the matching static secret key | unset | No |

A further batch of compose-only variables were added in P2–P5 for
SeaweedFS (the P2 storage-compatibility target, matrix-profile only),
Trino (the P4 small-file-compaction escape hatch, ADR 0009), and Debezium
Server (P5 CDC). None of these are read by `config.rs` — they configure the
containers themselves, and (for Trino/Debezium) are only relevant behind
their opt-in compose profiles:

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `SEAWEEDFS_ACCESS_KEY` | SeaweedFS S3 API access key | `seaweedfsadmin` (public, well-known) | No, but override before exposing SeaweedFS beyond localhost |
| `SEAWEEDFS_SECRET_KEY` | SeaweedFS S3 API secret key | `seaweedfsadmin` (public, well-known) | No, but override before exposing SeaweedFS beyond localhost |
| `TRINO_HOST_PORT` | Host port mapped to Trino's coordinator UI/API (`trino` profile). Not `8090` — `oidc-mock` already publishes that, and the two collide when the `trino` profile runs alongside the base stack | `8091` | No |
| `TRINO_CRON_INTERVAL_SECONDS` | How often `trino-maintenance-cron` runs `ALTER TABLE ... EXECUTE optimize` against every Bronze table (`trino` profile) | `21600` (6h) | No |

Debezium Server's image (`ghcr.io/memiiso/debezium-server-iceberg`) is
pinned by digest in `docker-compose.yml`, not by an env var — see the
service definition's comment and R4 in the risk register for why (no
versioned tag is published upstream). Its config
(`ops/debezium/application.properties.tmpl`) is rendered from the
connector-registry (ADR 0007), not from top-level env vars.

## Status / Known limitations

This is a young, honestly-scoped project. Please read this before filing an
issue about any of the following — they're known, not bugs:

- **There is no streaming surface.** There is no Kafka/Redpanda/Pulsar/Flink
  anywhere in this project. The console previously had a mocked
  `streaming` domain fabricating lag/throughput/checkpoint numbers; it has
  been removed rather than kept as a mock. **CDC (Debezium, P5) is not a
  streaming engine** and is not relabeled as one — it's a
  change-data-capture pipe from Postgres into Bronze Iceberg, surfaced
  instead under Governance → "Ingestion (CDC)".
- **`knowledge.search` is mocked.** There is no vector store or embeddings
  API wired up. Knowledge *sources* and *vector jobs* ARE real, backed by
  Postgres (`lakehouse-store::knowledge`) — only the search-query path
  itself is mocked.
- **Connector "Test connection" genuinely dials only PostgreSQL and
  S3-compatible object storage.** `POST /api/connectors/{id}/test`
  (`lakehouse-api`'s `connector_probe` module) opens a real, 5s-bounded
  connection and measures real latency for those two types, resolving the
  connector's `secretRef` via `EnvSecretResolver` (ADR 0002). Every other
  connector `type` (Kafka, MQTT, MongoDB, Oracle, SAP/ERP, SFTP, a vendor
  REST API, ...) has no dial implementation in this build and returns
  `{ supported: false }` with a message saying so — never a fabricated
  latency or success. The seed (`0022_prune_connector_seed.sql`) was
  shrunk to match: two connectors, `conn-pg-lakehouse` and
  `conn-s3-warehouse`, pointing at the compose stack's own Postgres and
  RustFS.
- **ClickHouse cannot write Iceberg through the catalog on 26.3.**
  `CREATE TABLE` inside a `DataLakeCatalog` database never reaches
  Lakekeeper (falls back to `MergeTree`, `Code: 79`); `INSERT` into a
  **partitioned** catalog-registered table **segfaults the server**
  (signal 11 in `IcebergStorageSink::consume`); `INSERT` into an
  unpartitioned one fails cleanly (`Code: 1000`). Bronze ingestion
  (Debezium/dlt/Rust → Iceberg → ClickHouse reads) is unaffected — only
  ClickHouse-*originated* writes are. Gold export was moved to Rust as a
  result (ADR 0010). See `docs/plans/G1-RESULT.md`.
- **`remove_orphan_files` does not exist for Iceberg tables, and `OPTIMIZE`
  fails at runtime with an HTTP 403 on a catalog-registered Iceberg
  table.** Of the maintenance chain the plan assumed, only
  `expire_snapshots` actually works on ClickHouse 26.3 — it does not
  compact small data files. Small-file compaction on Bronze runs
  out-of-band via a Trino-as-cron container instead (`trino` compose
  profile, ADR 0009); a deployment that never enables that profile
  accumulates small Bronze files unbounded. See `docs/plans/G3-RESULT.md`.
- **A bare `count()` overcounts on CDC-fed Bronze tables.** On ClickHouse
  26.3, `SELECT count()` / `count(*)` / `count(<col>)` against a Bronze
  Iceberg table with merge-on-read equality deletes (i.e. any
  Debezium-fed table) takes a metadata-only fast path that does not
  subtract deleted rows — measured returning 6 where 4 was correct. Any
  `WHERE` or `GROUP BY` forces the correct row-scan path. This is a
  silent wrong answer, not an error; no code added in this repository
  emits a bare `count()` against a Bronze Iceberg table (R11). See
  `docs/plans/P5-RESULT.md`.
- **Lakekeeper authorization is enabled by default — `authz-backend:
  "openfga"`, not `"allow-all"`.** `docker compose up` now brings up
  OpenFGA and a mock OIDC issuer (`ops/oidc-mock`) alongside Lakekeeper,
  and every writer this repo's own tests exercise (the Rust `g1-lakekeeper`
  test path, `debezium-server`, the dlt pipeline) authenticates as a
  granted principal — see `docs/adr/0011-lakekeeper-authorization.md` for
  the model, the grants, and the default-posture decision. **What is still
  open:** ClickHouse's `CREATE TABLE`/`INSERT` through the catalog do not
  work on this ClickHouse version at all (a ClickHouse defect, independent
  of authorization — see above and `docs/plans/G1-RESULT.md`), so R1's
  original framing ("ClickHouse catalog-registered
  writes fail against Lakekeeper's authz on metadata updates") is still
  untestable. The `trino` compose profile (ADR 0009's small-file-
  compaction escape hatch) is granted its own principal too — `EXECUTE
  optimize` measured working under enforcement (3 data files -> 1 on a
  live Bronze table; see ADR 0011 and `docs/plans/G3-RESULT.md`).
- **`getWorkspaceSettings` returns a fixed response.** The contract has no
  setter; workspace settings are not actually persisted or configurable
  yet.
- **No login rate limiting beyond logging.** Failed login attempts are
  logged but not throttled or locked out.
- **SSO is gated by a build-time flag, not a runtime one.** The frontend
  can't read the Rust process's environment variables directly, so whether
  the SSO login UI shows up is controlled by `NEXT_PUBLIC_SSO_ENABLED` at
  *build* time — it cannot react to whether `OIDC_ISSUER`/`OIDC_CLIENT_ID`
  are actually set on the backend at runtime. A `GET /api/auth/providers`
  endpoint (letting the frontend ask the backend what's configured) has
  been proposed but is not built.
- **The service does not refuse to boot when Postgres is down.**
  `lakehouse-store::connect_lazy` never does network I/O at startup;
  Postgres connectivity is only discovered lazily, at first use. Dependent
  (Phase 2) routes return `503` instead of the process failing to start.
  This is deliberate (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)),
  but it means a misconfigured `DATABASE_URL` degrades quietly rather than
  loudly — watch logs and the 503 rate, not just process uptime.
- **Sessions and service tokens have no rotation/cleanup job.** Nothing
  today expires or garbage-collects them beyond whatever TTL logic exists
  at issuance/verification time.
- **A previously-internal API key and internal LAN hostnames are present
  in git history** (2 and ~10 commits reachable from `main`,
  respectively), predating this repo going public. The key must be, and
  has been treated as, compromised. See the "Known exposure" section of
  [SECURITY.md](SECURITY.md) and [docs/CI.md](docs/CI.md) (the
  `gitleaks (full git history)` job is intentionally red because of this).
- **The backend was ported from TypeScript to Rust by an AI agent
  workflow**, reviewed by AI reviewers, task by task, with a parity harness
  comparing responses against the original TypeScript backend along the
  way. The commit history has not had a full human security/architecture
  review end to end — treat it accordingly, especially before production
  use.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup, test commands, commit
conventions, and PR expectations. See [SECURITY.md](SECURITY.md) to report
a vulnerability privately.

## License

AGPL-3.0-or-later — see [LICENSE](LICENSE) and [NOTICE](NOTICE) (including a
transitive LGPL-3.0 note for `sharp-libvips`, which is compatible with
AGPL-3.0). `v0.1.0` was released under Apache-2.0; the project relicensed to
AGPL-3.0-or-later afterward — see [CHANGELOG.md](CHANGELOG.md).

---

## Frontend implementation notes (Next.js scaffold)

The sections below are lower-level frontend conventions, kept from the
project's original scaffold notes.

### Tech stack

- **Bun** (runtime, package manager, test runner)
- **Next.js** (App Router)
- **TypeScript**
- **Tailwind CSS**
- **shadcn/ui** (UI components)
- **lucide-react** (icons)
- **next-themes** (dark mode)
- **clsx** (utility class names)

Styling follows the **Rantai Design System** (`design-system/`): OKLCH
blue/navy color tokens, Geist font, dark mode as the default theme.

### `src/` folder structure

```
src/
├── app/                 # App Router (layout, page, routes)
├── components/
│   ├── ui/              # shadcn/ui components
│   └── shared/          # Shared components (ThemeProvider, etc.)
├── lib/                 # Utilities (utils, config)
├── hooks/               # Custom React hooks
└── types/               # TypeScript types/interfaces
```

### Adding a shadcn component for the first time

1. Browse [shadcn/ui Components](https://ui.shadcn.com/docs/components).
2. Add a component via the CLI:
   ```bash
   bunx shadcn@latest add <component-name>
   ```
   Example:
   ```bash
   bunx shadcn@latest add card
   bunx shadcn@latest add dialog
   bunx shadcn@latest add input
   ```
3. Components land in `src/components/ui/` (per `components.json`).
4. Usage:
   ```tsx
   import { Button } from "@/components/ui/button"
   import { Card, CardContent, CardHeader } from "@/components/ui/card"

   export default function Page() {
     return (
       <Card>
         <CardHeader>Title</CardHeader>
         <CardContent>
           <Button>Click</Button>
         </CardContent>
       </Card>
     )
   }
   ```

### Path alias

- `@/*` → `./src/*` (configured in `tsconfig.json`)

### Dark mode

The project uses `next-themes` via the design system's `ThemeProvider`
(`@rantai/design-system/components/theme-provider`) in `src/app/layout.tsx`.
Per the design system, dark mode is **forced**. To enable a light/dark
toggle, remove the `forcedTheme` prop on that component.
