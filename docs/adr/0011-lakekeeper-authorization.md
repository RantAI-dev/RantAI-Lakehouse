# ADR 0011 — Lakekeeper authorization: OpenFGA, principals, and the default posture

- **Status:** Accepted
- **Phase:** R1 (risk retirement, post-P6)
- **Date:** 2026-09-01

## Context

Lakekeeper ran with `"authz-backend":"allow-all"` from P1 through P5 —
confirmed via `GET /management/v1/info` in `docs/plans/G1-RESULT.md` and
restated as still-open in `docs/plans/P5-REPORT.md`. Any client that can
reach Lakekeeper can mutate any table's metadata in any warehouse. That is
R1 in `docs/plans/LAKEHOUSE-FOUNDATION-PLAN.md`'s risk register, and it is
the last open risk in the build.

## Decision 1 — OpenFGA, not the OPA bridge

Lakekeeper supports two authorization backends: OpenFGA (native) and an
OPA bridge. This build uses **OpenFGA**:

- It is Lakekeeper's first-class integration — `lakekeeper migrate` owns
  writing Lakekeeper's own authorization model into the OpenFGA store
  automatically (confirmed empirically: `serve` against a fresh OpenFGA
  store with no model fails closed with `StoreNotFound`, and `migrate`
  writes the model — nothing in this build hand-authors an OpenFGA DSL
  model). The OPA bridge would additionally require authoring and
  maintaining a Rego policy that reimplements the same relations by hand.
- OpenFGA ships its own Postgres-backed store (`openfga migrate` against a
  dedicated database), matching every other new-service convention this
  build already uses (`lakekeeper-db-init`, `dagster-db-init`,
  `openfga-db-init` below) — no new storage technology enters the stack.
- Lakekeeper's own Management API (`/management/v1/permissions/...`,
  documented below) is the day-to-day grant interface either way; OpenFGA
  vs. OPA is an internal backend choice this build never has to expose to
  an operator.

## Decision 2 — the authorization model is Lakekeeper's own, not hand-authored

The task brief anticipated defining "a `warehouse` object type with
`owner`/`writer`/`reader` relations... written once and loaded via
OpenFGA's own tuple/model DSL." That is not how Lakekeeper 0.13.3's
OpenFGA integration works, measured directly (reading
`quay.io/lakekeeper/catalog:v0.13.3`'s compiled config surface and its
Management API's OpenAPI spec, which only appears once
`authz-backend=openfga`/`openid_provider_uri` are set):

- Lakekeeper defines and owns a fixed relation set per resource level —
  **project**: `project_admin`, `security_admin`, `data_admin`,
  `role_creator`, `describe`, `select`, `create`, `modify`; **warehouse**:
  `ownership`, `pass_grants`, `manage_grants`, `describe`, `select`,
  `create`, `modify`. Namespace/table/view levels have their own narrower
  sets. `lakekeeper migrate` writes this model into the OpenFGA store; no
  code in this build authors OpenFGA tuples directly.
- Grants happen through Lakekeeper's own Management API:
  `POST /management/v1/permissions/warehouse/{warehouse_id}/assignments`
  with a body like `{"writes":[{"type":"select","user":"oidc~<sub>"}]}`.
  `user`/`role` ids are Lakekeeper's own (`oidc~<subject-claim>` for an
  OIDC-authenticated principal — confirmed by round-tripping a real grant
  and `whoami` call against a live stack).
- **This is consistent with ADR 0003, not a divergence from it.** ADR
  0003's warehouse boundary ("tenant isolation enforced once, at the
  warehouse boundary... two namespaces inside the same warehouse are
  inherently the same tenant's data") maps exactly onto granting at the
  `warehouse` object level: everything below inherits the grant, matching
  "isolation enforced once." No amendment to ADR 0003 is needed — its
  tenant→warehouse mapping is what this ADR's grants are keyed against
  (see Decision 4).
- At minimum this expresses everything the task asked for: `create` (who
  may create/drop tables and namespaces), `modify` (who may commit
  metadata updates — append, upsert, `expire_snapshots`), `select` (who
  may only read).

## Decision 3 — every principal authenticates via a pre-minted static bearer token, through a purpose-built mock OIDC issuer

Lakekeeper's OpenFGA authorization checks a *principal*; principals in
Lakekeeper 0.13.3 come from exactly two authentication mechanisms —
OIDC bearer tokens (`openid_provider_uri`/`openid_audience`) or Kubernetes
service-account tokens. There is no "static API key" or local credential
store. This is a real, measured gap beyond what the task brief's own
sizing anticipated (`docs/plans/P5-REPORT.md`'s "steps 1–3... a day or two
of focused work" did not account for needing an identity provider at
all) — retiring R1 for real requires *some* OIDC-compliant issuer, because
without one there are no principals to grant anything to, and enforcement
would either reject every caller or (worse) have to stay `allow-all` in
practice.

`ops/oidc-mock` is that issuer, and it is deliberately minimal:

- One RSA keypair, generated once at first boot and persisted on a named
  volume (`lakehouse_oidc_data`).
- A standard OIDC discovery document (`/.well-known/openid-configuration`)
  and JWKS (`/jwks.json`), so Lakekeeper's own OIDC client validates
  tokens exactly the way it would against a real IdP.
- One long-lived (30-day, see PR #33 review below) RS256 token pre-minted
  per principal (`ops/oidc-mock/server.py`'s `PRINCIPALS`) at boot,
  written to a shared volume (`lakehouse_oidc_tokens`) every writer mounts
  read-only (as a per-principal single-file subpath mount, not the whole
  volume — see "PR #33 review" below) and reads directly — no login flow,
  no refresh.
- A `/token` endpoint implementing an OAuth2 client-credentials grant *in
  name only* (it does not check `client_secret` against anything — there
  is nothing else in this stack to check it against, and it refuses to
  mint a token for `admin`, see below). This exists for exactly one
  caller: ClickHouse's `DataLakeCatalog` REST engine, whose
  `catalog_credential` setting only accepts the Iceberg REST spec's
  `client_id:client_secret` form and performs a real OAuth2 exchange —
  measured empirically (`catalog_credential` set to a raw token is
  rejected at parse time: "expected client_id and client_secret separated
  by `:`"). `oauth_server_uri` is pointed at this mock endpoint directly,
  bypassing Lakekeeper's own (unverified in this build) `/v1/oauth/tokens`.

This is **not a production identity provider** and is not meant to be
read as one.

**Corrected claim (PR #33 review):** an earlier version of this ADR said
"nothing else in this design changes" when `ops/oidc-mock` is swapped for
a real IdP. That is false, and worth saying plainly instead of leaving it
implied by the rest of this ADR:

- **There is no token refresh anywhere in this build.** Every writer reads
  a single pre-minted token once (at process start, or once per `/token`
  call for ClickHouse) and holds it for the rest of its process lifetime.
  A real IdP-backed deployment needs each writer's client to implement
  actual OAuth2 token refresh (or short-lived-token reissuance) before a
  30-day (or shorter, if a real IdP issues shorter-lived tokens, which
  most do) token expiring mid-run becomes a real operational failure
  mode. Nothing in this codebase does that today.
- **Trino and Debezium use static bearer tokens, not an OAuth2 client
  they own.** `trino`'s `iceberg.rest-catalog.oauth2.token` and
  `debezium-server`'s `debezium.sink.iceberg.token` are both rendered
  once, at container start, from the token file on
  `lakehouse_oidc_tokens` — see `docker-compose.yml`. Neither Trino's nor
  Debezium's Iceberg client re-fetches or refreshes it. Swapping in a
  real IdP does not fix this by itself; it just changes who signed the
  (still-static, still-never-refreshed) token these two services start
  with.
- **ClickHouse's `oauth_server_uri` points at `ops/oidc-mock` specifically
  (`http://oidc-mock:8090/token`), not at Lakekeeper or a generic OIDC
  endpoint.** A real-IdP deployment has to repoint this setting at
  whatever OAuth2 client-credentials endpoint the real IdP exposes (most
  do have one), and confirm that endpoint accepts a bare `client_id`
  naming a known principal the way this mock does — a real IdP will
  legitimately require a `client_secret`, which ClickHouse's
  `catalog_credential` setting already supports (`client_id:client_secret`)
  but this build never had reason to configure, since the mock never
  checks it.

A real deployment replacing `ops/oidc-mock` with a real IdP (Keycloak,
Dex, the customer's own OIDC provider) has to account for all three of
the above, not just repoint `openid_provider_uri`/`openid_audience`.

**Port posture (PR #33 review, blocker 2):** `oidc-mock` (container port
8090) and `openfga` (container ports 8080/8081) publish **no host ports**
by default — no `ports:` entry in `docker-compose.yml` at all, so both are
reachable only from other containers on the compose network. This was not
true before this review: both were published
(`${OIDC_MOCK_HOST_PORT:-8090}:8090`,
`${OPENFGA_HTTP_HOST_PORT:-8082}:8080`,
`${OPENFGA_GRPC_HOST_PORT:-8083}:8081`), which on a plain `docker compose
up` meant anyone who could reach the host could fetch `oidc-mock`'s
private signing key (blocker 1) and, before the `/token` fix above, mint
an `admin` token with no credential at all — the two together were a
straight path to a Lakekeeper instance-admin bypass identity from outside
the stack. Nothing in this build's own CI or test-profile services needs
host access to either: `lakekeeper`, every `lakekeeper-*-init` job, and
every writer's `CH_OAUTH_SERVER_URI` already resolve `oidc-mock`/`openfga`
by compose service name from inside the network, and the G1/G2/G3a/G4 test
runners (`g1-test-runner`, `gold-export-test-runner`, etc.) run as compose
services themselves, not from the host. An operator who genuinely needs to
inspect either from the host for debugging should use `docker compose exec`
or a one-off container attached to the compose network, not republish
these ports.

## Decision 4 — grants, per principal, on the ADR-0003-named warehouse

Every grant below targets the warehouse named by `LAKEKEEPER_WAREHOUSE`
(`default` in this compose stack, `TENANT_ID`'s value in a real
deployment — ADR 0003's exact mapping, unchanged). `lakekeeper-authz-init`
(and its `-seaweedfs` counterpart, since G2 registers a second warehouse)
performs these grants after `lakekeeper-warehouse-init` creates the
warehouse:

| Principal | Relations granted | Why |
| --- | --- | --- |
| `rust-iceberg` | `create`, `modify`, `select` | G1's Rust writer: creates the Bronze namespace/table, appends via vended credentials, and (in tests) reads back. |
| `debezium` | `create`, `modify`, `select` | P5 CDC (`debezium-server-iceberg`): creates the table on first snapshot, upserts continuously. |
| `dlt` | `create`, `modify`, `select` | P3 dlt pipeline (Dagster code location): same shape as `rust-iceberg` — a Bronze table writer. |
| `clickhouse-reader` | `select`, `modify` | Every gate's ClickHouse read path (`select`), **plus** `modify` for `maintenance.py`'s `expire_snapshots` — the one ClickHouse catalog WRITE that works on this ClickHouse version (`docs/plans/G3-RESULT.md`; `CREATE TABLE`/`INSERT` still do not, per `G1-RESULT.md`). See "Over-grants" below. |
| `trino` | `select`, `modify` | ADR 0009's compaction escape hatch (`trino-maintenance-cron`'s `ALTER TABLE ... EXECUTE optimize`): reads existing Bronze data files and commits a rewritten snapshot — a `select`+`modify` shape, not a `create` one (it never creates a namespace or table). See "Trino" below for the measured proof. |
| `unauthorized-test` | none | The negative-test principal — self-registered with Lakekeeper (it has an identity) but never granted anything. |
| `admin` | none (instance-admin bypass) | `LAKEKEEPER__INSTANCE_ADMINS=["oidc~admin"]` — bypasses authorization for control-plane actions only (confirmed via Lakekeeper's own startup log: "these principals bypass authorization for all control-plane actions (but not for `CatalogTableAction::ReadData`/`WriteData`)"). Used only by the one-shot init jobs to bootstrap Lakekeeper and grant the others; never used by a running writer, and cannot itself read or write Bronze data. **PR #33 review:** `admin.jwt` is still pre-minted to `lakehouse_oidc_tokens` at `oidc-mock` boot (the init jobs read it straight off that volume), but `ops/oidc-mock`'s `/token` endpoint refuses to mint an `admin` token on request — that endpoint has no secret check at all, so leaving `admin` mintable through it would have let any caller that could reach the port (before the same review's port-unpublishing fix) obtain the instance-admin bypass identity outright. |

No principal holds a blanket grant across all resource levels or all
warehouses — `unauthorized-test` proves this (Decision 5), and every
writer above is scoped to exactly one warehouse and exactly the relations
its own write shape needs.

**Over-grant, noted rather than silently shipped:** `clickhouse-reader`
holding `modify` (not `select`-only) is broader than "read path" as
originally scoped. A fifth principal (`clickhouse-maintenance`, `modify`
only) would be more precise, but was not built this session — the
marginal isolation benefit did not justify a fourth OIDC principal and a
second ClickHouse-side auth configuration for what is, in this build, one
shared ClickHouse identity behind one connection string. If a future
deployment wants read and maintenance identities cryptographically
separated, `ops/oidc-mock`'s `PRINCIPALS` list and
`lakekeeper-authz-init`'s grant calls are the two places to split it.

**Trino, granted (closing an earlier gap in this ADR):** an initial
version of this ADR left `trino` ungranted — ADR 0009's compaction escape
hatch (the *only* working remedy for Bronze small-file accumulation on
this ClickHouse version: `OPTIMIZE` fails `403`, `remove_orphan_files`
does not exist) would have been silently denied under R1's enforced-by-
default posture, which is worse than either problem alone. `trino` is now
granted `select`+`modify` the same way as the other writers. Trino's
`trino-iceberg` plugin (measured directly from the shipped
`IcebergRestCatalogConfig`/`OAuth2SecurityConfig` classes in
`trinodb/trino:483`'s plugin jar) supports a static bearer token exactly
like `iceberg-catalog-rest`'s and pyiceberg's `token` properties:
`iceberg.rest-catalog.security=OAUTH2` +
`iceberg.rest-catalog.oauth2.token=<token>` sends the token as-is, no
OAuth2 exchange. `docker-compose.yml`'s `trino` service reads the
`trino` principal's pre-minted token from the shared token volume
(`ops/oidc-mock`) into that property the same way `debezium-server`
does. Measured proof:

Measured against a clean stack (`bronze.g1_rust_write`, built by running
`g1-test-runner`'s `g1_half_a` five times against the same table — each
run either creates the table or appends to it):

```
$ trino --execute 'SELECT count(*) FROM iceberg.bronze."g1_rust_write$files"'
"3"
$ trino --execute "ALTER TABLE iceberg.bronze.g1_rust_write EXECUTE optimize"
ALTER TABLE EXECUTE
"rewritten_data_files_count","3"
"removed_delete_files_count","0"
"added_data_files_count","1"
$ trino --execute 'SELECT count(*) FROM iceberg.bronze."g1_rust_write$files"'
"1"
$ trino --execute "SELECT count(*) FROM iceberg.bronze.g1_rust_write"
"6"
```

**3 data files -> 1**, row count unchanged (6, matching the 3 successful
appends' 2 rows each) — the same compaction shape as the pre-R1
measurement in `docs/plans/G3-RESULT.md` (280 -> 14), run smaller here
only because this stack's own G1 fixture is a much smaller table than
G3's synthetic load. `trino --execute` ran as the `trino` principal, over
the exact `iceberg.rest-catalog.oauth2.token` wiring `docker-compose.yml`
ships — this is not a superuser/instance-admin path.

## Decision 5 — the negative test

`rust/crates/lakehouse-iceberg/tests/g1_lakekeeper.rs`'s
`g1_negative_ungranted_principal_is_denied` connects as
`unauthorized-test` (registered with Lakekeeper, zero grants) and calls
`ensure_bronze_namespace`. Measured, actual result:

```
iceberg catalog operation failed: Unexpected, context: { status: 404 Not Found,
  ..., json: {"error":{"message":"A warehouse 'default' does not exist",
  "type":"NoSuchWarehouseException","code":404, ...} } }
```

Lakekeeper denies by **information hiding**, not a `403`: an unauthorized
caller cannot even confirm the warehouse exists. This is a stronger
fail-closed shape than a bare `403` (it does not leak that the resource
exists to a principal with no relation to it), and the test asserts on
this shape (`403`/`404`/"forbidden"/"not found", not the literal message),
so it stays correct if Lakekeeper's own wording changes.

ClickHouse's read path shows a weaker but still real denial: an
unauthorized `catalog_credential` produces zero visible tables (`SHOW
TABLES` returns nothing, no error) rather than a loud error, because
ClickHouse's `DataLakeCatalog` engine does not surface catalog-level
authorization failures as query errors for `SHOW TABLES`. It is still
correct — no data is exposed — but callers relying on this path for an
audit trail should not expect ClickHouse's own error stream to show the
denial the way Lakekeeper's own REST responses do.

## Decision 6 — enforced by default, not profile-gated

`openfga`, `oidc-mock`, and their Postgres-backed migration jobs are
**core services now** — no `profiles:` entry, unlike `dagster`/`seaweedfs`/
`trino`/`test`. `lakekeeper` will not become healthy until `openfga-ready`
and `oidc-mock` are healthy; `lakekeeper-migrate` will not succeed without
reaching a live OpenFGA. A plain `docker compose up` — the shipped
default — now runs with Lakekeeper authorization enforced.

This is a deliberate reversal of this build's own precedent: `allow-all`
survived from P1 through P5 specifically because standing up
authorization was deferred as "a distinct task, not a config flag"
(`docs/plans/G1-RESULT.md`), and each deferral compounded — by P5, R1 was
still open with the same justification repeated. Leaving OpenFGA
profile-gated (`--profile authz` or similar) would have reproduced exactly
that pattern: the lightweight, insecure path stays the path of least
resistance, and the secure path is opt-in, which is how a shipped default
ends up being the insecure one. The light dev loop this preserved — no
new service, no token to manage — is real, but it is not worth
reintroducing the exact failure mode R1 exists to close.

The trade this decision makes explicit: every developer bringing up this
stack for the first time now needs `openfga`+`oidc-mock` healthy before
`lakekeeper` starts, adding on the order of 10–15 seconds and two more
containers to a cold `docker compose up`. Every existing writer this
build knows about (`rust-iceberg`, `debezium`, `dlt`, `clickhouse-reader`)
is pre-granted by `lakekeeper-authz-init`/`lakekeeper-authz-init-seaweedfs`
in the same bring-up path, so this does not require a manual grant step
for any workflow this repo's own tests exercise.

## What remains un-exercised

**R1's original framing — "ClickHouse catalog-registered writes fail
against Lakekeeper's authz enforcement on metadata updates" — is still
untestable, for the same reason `docs/plans/G1-RESULT.md` gave before any
of this work started.** ClickHouse cannot write `CREATE TABLE`/`INSERT`
through the catalog *at all* on ClickHouse 26.3 (a ClickHouse defect,
independent of authorization), so that specific interaction never reaches
Lakekeeper's authz layer to be measured. What this ADR's work *does*
newly prove is that the writers that ARE real on this stack today
(`rust-iceberg`, `debezium`, `dlt`) keep working under real enforcement,
that ClickHouse's READ path (and its one working write verb,
`expire_snapshots`) authenticate and are correctly scoped, and that an
unauthorized caller is genuinely denied — not that R1's original sentence
was fully exercised.

Also not exercised: Lakekeeper's own `/v1/oauth/tokens` endpoint (both
ClickHouse's and Trino's principals bypass it entirely via
`oauth_server_uri`/a static `oauth2.token`, pointed at `ops/oidc-mock` —
whether Lakekeeper's own endpoint works at all, with what identity
provider wiring, is unknown).
