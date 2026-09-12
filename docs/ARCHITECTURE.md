# Architecture

This document maps the Rust workspace, traces a request end to end, explains
the Postgres/ClickHouse data-model split, and calls out the seams that make
the system extensible without a rewrite.

For environment variables and a running-system quickstart, see
[README.md](../README.md). For contribution mechanics, see
[CONTRIBUTING.md](../CONTRIBUTING.md).

## Module map

The Rust backend (`rust/crates/`) is 13 crates (`lakehouse-iceberg` was
added in P1; the table below already listed it, but the prose above it had
not been updated until P6). `lakehouse-api` is the only binary; everything
else is a library crate it depends on.

| Crate | Owns |
| --- | --- |
| `lakehouse-core` | Shared domain types: `ApiError` (the one error type every handler converts into), identifier newtypes (`Ident`, `SqlLiteral` — the SQL-injection-safety boundary), and status enums. Every other crate depends on this one; it depends on nothing else in the workspace. |
| `lakehouse-clickhouse` | HTTP client for ClickHouse's plain HTTP interface (`POST` SQL, parse `FORMAT JSON`). Every analytics-reading route goes through this. |
| `lakehouse-store` | Postgres-backed OLTP storage for Phase 2 domains (`console`/mutation state ClickHouse is a poor fit for): identity, governance, pipelines, connectors, knowledge, agents, queries, overview, storage. See "Boot behavior" below — connecting is lazy and non-fatal. |
| `lakehouse-auth` | The authentication core: `Principal`, `Credential`, the `Authenticator` trait, and four concrete authenticators (local password, session, service token, OIDC). See "The `Authenticator`/`Principal` seam" below and `rust/crates/lakehouse-auth/README.md`. |
| `lakehouse-bi` | BI/dashboarding domain: static chart specs, SQL builders, and the ClickHouse-backed board/chart store. Library only — no routes live here. |
| `lakehouse-dagster` | Dagster GraphQL client (runs, schedules, launching/triggering jobs). |
| `lakehouse-llm` | OpenAI-compatible chat-completions client, defaulting to MiniMax's endpoint but pointable at any compatible node via `LLM_URL`/`LLM_MODEL`/`LLM_KEY`. |
| `lakehouse-embed` | Signed embedding (Metabase-style): HS256 JWT carrying a dashboard resource plus locked filter params, hand-rolled (no external JWT crate), matching the original TypeScript. |
| `lakehouse-notify` | Delivery to webhook (Slack/Discord/generic incoming webhook) and email (SMTP via `lettre`), used by alerts and digests. |
| `lakehouse-alerts` | Threshold alerts and scheduled digests over `serving.*` ClickHouse marts, persisted in `console.alert_rule`, delivered via `lakehouse-notify`. |
| `lakehouse-iceberg` | P1: `object_store`-backed S3 client, Lakekeeper Iceberg REST catalog client, Bronze table create + append, and (ADR 0010) Gold export create/append/read-back. **`lakehouse-api` depends on it as of the Gold export work**: `routes::gold` (`POST`/`GET /api/gold/export/{mart}`, glued together by `gold_export.rs`) is its first real caller — Bronze itself is still surfaced in the console by reading `bronze_meta.*` (ClickHouse) directly, via `routes::catalog`/`routes::governance`/`routes::storage`, never through this crate. It is also exercised by the G1/G3a/G4/Gold-export gate test runners and (indirectly) by Debezium Server/dlt as REST-catalog clients. See its crate doc comment and `docs/adr/0002`–`0004`, `0010`. |
| `lakehouse-api` | The axum HTTP service: config resolution, middleware, routing, policy, and every handler. The only crate with `main()`. |

## Request lifecycle

```
browser
  │  fetch("/api/...")
  ▼
Next.js dev/prod server
  │  next.config.ts rewrites() — only active when RUST_API_URL is set;
  │  rewrites /api/:path* → {RUST_API_URL}/api/:path* (proxy, not a redirect —
  │  the browser still sees same-origin /api/*)
  ▼
lakehouse-api (axum)
  │  1. tower-http tracing + per-route request timeout (matches each
  │     original TS route's maxDuration)
  │  2. crate::auth — AuthenticatedPrincipal extractor: reads the session
  │     cookie or Authorization: Bearer header, dispatches by credential
  │     SHAPE (opaque 64-hex service token vs. three-segment JWT vs.
  │     session cookie) to the matching lakehouse_auth::Authenticator,
  │     producing a normalized Principal — or a 401.
  │  3. crate::policy::auth_gate — looks up (method, matched route pattern)
  │     in POLICY_TABLE (data, not scattered per-handler checks). A route
  │     missing from the table is a hard 500
  │     ("route_policy_unclassified"), never a silent allow — deny-by-
  │     default even against future routing bugs, verified by
  │     `tests/route_policy.rs`.
  │  4. the matched handler in routes/*.rs
  ▼
store / client layer
  │  lakehouse-store (sqlx → Postgres) for Phase 2 OLTP domains, or
  │  lakehouse-clickhouse (HTTP → ClickHouse) for analytics reads, or
  │  lakehouse-dagster / lakehouse-llm / lakehouse-notify for the
  │  corresponding external system
  ▼
Postgres / ClickHouse / Dagster / LLM
```

Errors from any layer are converted to `lakehouse_core::ApiError` and
rendered as a JSON envelope (`application/json;charset=utf-8` on every
response, including errors) — there is no unhandled-panic-as-500 path by
design; see `lakehouse-api/src/error.rs`.

## Data model: Postgres vs. ClickHouse, and why

Two stores, deliberately not merged, because they solve different problems:

- **ClickHouse** holds append-heavy, query-heavy **analytical** data: the
  `serving.*` marts dashboards and charts read from, lineage/catalog
  metadata, and (historically) `console.alert_rule` and the BI board/chart
  store — data that's written rarely and read by aggregate queries at
  volume. ClickHouse has no real transactions, which is fine for this data:
  nothing here needs read-your-writes consistency across multiple rows in
  one operation.
- **Postgres** (`lakehouse-store`, Phase 2) holds **OLTP / mutation-heavy**
  `console`-schema state that genuinely needs transactions and row-level
  integrity: identity (`app_user`, `auth_identity`, `role`,
  `app_user_role`, sessions, service credentials), governance policies and
  authored rules, pipeline definitions, connector definitions, knowledge
  sources and vector jobs, digital-employee agents/tools/workflows/runs/
  approvals, saved queries and query history/collaboration, and storage/
  overview/ops mutation state (KILL QUERY, alert instances). Phase 1 (the
  original TypeScript backend) never had a database for this — every one
  of these domains was either mocked or lived only in ClickHouse in a
  shape that didn't fit. Phase 2's job was to give this state an actual
  transactional home without touching the ClickHouse-backed analytics
  routes at all.

`lakehouse-store`'s module list mirrors this split directly:
`identity`, `governance`, `pipelines`, `connectors`, `knowledge`, `agents`,
`queries`, `overview`, `storage` — one module per Postgres-backed domain,
plus shared `error` and `connectors`-adjacent plumbing.

### A third store: Bronze Iceberg on object storage (P1–P5)

P1–P5 added a third data path that is neither Postgres nor ClickHouse
`MergeTree`: **Bronze**, stored as Apache Iceberg tables on an S3-compatible
object store (RustFS by default; SeaweedFS is a verified drop-in swap, see
`docs/STORAGE-COMPATIBILITY.md`), registered in Lakekeeper's Iceberg REST
catalog. Two independent writers populate it — Dagster/dlt for batch
ingest (P3) and Debezium Server for CDC (P5) — both writing through
Lakekeeper, never through ClickHouse. ClickHouse reads Bronze back out via
a `DataLakeCatalog` database (`allow_database_iceberg = 1`) pointed at the
same Lakekeeper catalog; this read path works and is what
`routes::catalog`, `routes::governance` (`maintenance`/`replication`
kinds), and `routes::storage`'s Warm-tier estimate all rely on.

**ClickHouse cannot reliably write to this catalog itself** on 26.3:
`CREATE TABLE` inside a `DataLakeCatalog` database never reaches
Lakekeeper, and `INSERT` into a partitioned catalog-registered table
segfaults the server (see `docs/plans/G1-RESULT.md`). This is why Gold
export was moved to Rust (`lakehouse-iceberg`, ADR 0010) instead of going
through ClickHouse's Iceberg write path, and why in-engine Bronze
maintenance is limited to `expire_snapshots` (`docs/plans/G3-RESULT.md`,
ADR 0009's Trino-as-cron escape hatch for the small-file compaction
`OPTIMIZE` cannot do).

**Gold export is built** (ADR 0010): `routes::gold` reads a `serving.*`
Gold mart from ClickHouse `MergeTree`, then appends it as a new Iceberg
snapshot to its own `gold` namespace through Lakekeeper — the same
vended-credentials write path G1(a) proved, exercised from
`lakehouse-api` instead of a test runner for the first time. It is
append-only, matching `iceberg-rust` 0.10.x's capabilities: a re-export
does not replace the table's rows, it adds another snapshot, tagged by
`_exported_at` — see `lakehouse-iceberg::gold`'s module doc for the
consequence of running it on a schedule. Triggered by `dagster/dispar_orchestrate/gold_export.py` (daily schedule,
calling the Rust route over HTTP — Dagster has no `iceberg-rust` binding
of its own) or directly via the route. See ADR 0010 for the full record,
including the principal/grant and the namespace-naming decision.

A parallel Bronze *metadata registry* — `lake.bronze_meta.*` and
`lake.bronze_meta_sec.*` (dataset catalog, column, sync, plus P4/P5's
`maintenance_run`/`replication_slot` tables) — lives in ClickHouse
`MergeTree`, not Iceberg. This is what every console route above actually
queries; it is the reason the "never emit a bare `count()` against a Bronze
Iceberg table" rule (R11, see `docs/plans/P5-RESULT.md`) has not bitten any
existing product code — the registry is a `MergeTree` table, immune to the
Iceberg-specific equality-delete counting bug. Two independent owners write
this schema's DDL today — `demo/clickhouse/04_registry.sql` and
`dagster/dispar_orchestrate/bronze_catalog.py` — verified byte-identical
but not enforced as such (R10); this document does not add a third.

### Boot behavior: Postgres down is not fatal

`lakehouse_store::connect_lazy` performs no network I/O and only fails if
`DATABASE_URL` doesn't parse. This is deliberate: `lakehouse-api` serves the
full Phase 1 route surface (ClickHouse/Dagster/LLM — no Postgres involved)
regardless of Postgres's availability. If pool construction blocked on, or
failed because of, an unreachable Postgres, losing the Postgres container
would take down routes that have nothing to do with it — a regression
relative to the original TypeScript backend. So the pool is always
constructed lazily; connectivity is only discovered, and only ever fails,
at the first Phase 2 query, surfacing as an ordinary `503`
(`StoreError::Unavailable`/`Database` → `ApiError`) instead of a boot-time
panic. See the `lakehouse-store` crate doc comment for the full reasoning.
**Trade-off, stated plainly:** because this failure mode is quiet by
design, a misconfigured or down Postgres in production will not announce
itself at startup — only request-time 503s (and logs) surface it. There is
no separate health/readiness signal today that distinguishes "Postgres
never connected" from "Postgres connected fine."

## Seams

### The `Authenticator` / `Principal` / `auth_identity` seam

This is the seam that makes identity providers pluggable without touching
a handler or the schema. Three pieces, all in `lakehouse-auth`
(full detail in `rust/crates/lakehouse-auth/README.md`):

1. **`Principal`** — the normalized shape every `Authenticator` produces
   (`id`, `display_name`, `permissions`, `tenant_ids`, `provider`), never
   secret material. A handler reads `principal.has("catalog:write")` and
   never learns whether the caller typed a password, presented a session
   cookie, or arrived via an OIDC id token.
2. **`Authenticator`** (trait) — `provider_id(&self) -> &str` and
   `async fn authenticate(&self, credential: &Credential) -> Result<Principal, AuthError>`.
   Four implementations exist today: `password::LocalPasswordAuthenticator`,
   `session::SessionAuthenticator`, `service_token::ServiceTokenAuthenticator`,
   and `oidc::OidcAuthenticator`. `lakehouse-api/src/auth.rs`'s
   `AuthenticatedPrincipal` extractor disambiguates which one to try by
   credential *shape* (session cookie vs. opaque 64-hex bearer token vs.
   three-segment JWT bearer token) — never by trying every authenticator
   blindly.
3. **`auth_identity`** (`rust/migrations/0019_auth.sql`) — a local password
   is not special-cased on `app_user`; it's one row with
   `provider = 'local'`. Adding a new IdP (Okta, Entra, Google, Keycloak,
   or any OIDC issuer) means inserting rows with
   `provider = 'oidc:<name>'` into the same table — no migration, no new
   column, no handler change.

`OidcAuthenticator` is a **resource server**, not a full OIDC client: it
verifies an already-issued bearer `id_token` against the provider's JWKS.
It does not perform the authorization-code exchange or serve a `/callback`
route — that redirect/login flow is a frontend concern layered on top.

Route-level authorization is a separate, equally explicit seam:
`lakehouse-api/src/policy.rs`'s `POLICY_TABLE` maps every
`(method, route pattern)` to `Public` / `RequiresAuth` /
`RequiresPermission(...)` as data, checked once by the `auth_gate`
middleware before any handler runs — not scattered per-handler `if`
checks that are easy to forget on a new route.

### The `services/contracts` ↔ Rust route contract boundary

`src/services/contracts/*.ts` defines the TypeScript-side request/response
shapes the frontend codebase was originally written against (one file per
domain: `governance.ts`, `pipelines.ts`, `identity.ts`, `overview.ts`,
`queries.ts`, `storage.ts`, `connectors.ts`, `agents.ts`, `knowledge.ts`,
`assets.ts`, ...). Frontend components call these
contracts, not `fetch` directly.

After the Rust cutover, `src/services/clients/*` implementations of these
contracts call `lakehouse-api` over `/api/*` (proxied by the
`next.config.ts` rewrite — see the request lifecycle above) instead of
Next.js's own (now-deleted) API routes. The contract — the TypeScript
type shape — is the thing a Rust route handler's JSON response must match
byte-for-byte; a parity harness (`rust/crates/lakehouse-api/tests/`,
referenced in project history as the "corpus parity harness") compared
golden responses captured from the original TypeScript backend against the
new Rust handlers during the port, specifically to keep this boundary
honest. Changing a Rust handler's response shape without updating the
matching `services/contracts/*.ts` type (or vice versa) is the one class of
change most likely to break the frontend silently — there is no shared
schema/codegen enforcing this today; it is convention plus the (now
historical) parity harness.

`src/services/mock/*` remains the implementation for domains that are
still mocked rather than backed by a real Rust route — see README's
"Status / Known limitations" for exactly which ones (`knowledge.search`).
The `streaming` domain was removed outright rather than kept mocked —
there is no streaming engine anywhere in this stack, so faking one in the
UI was a locked-decision violation, not a limitation to document.
