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
- One long-lived (10-year) RS256 token pre-minted per principal
  (`ops/oidc-mock/server.py`'s `PRINCIPALS`) at boot, written to a shared
  volume (`lakehouse_oidc_tokens`) every writer mounts read-only and reads
  directly — no login flow, no refresh.
- A `/token` endpoint implementing an OAuth2 client-credentials grant *in
  name only* (it does not check `client_secret` against anything — there
  is nothing else in this stack to check it against). This exists for
  exactly one caller: ClickHouse's `DataLakeCatalog` REST engine, whose
  `catalog_credential` setting only accepts the Iceberg REST spec's
  `client_id:client_secret` form and performs a real OAuth2 exchange —
  measured empirically (`catalog_credential` set to a raw token is
  rejected at parse time: "expected client_id and client_secret separated
  by `:`"). `oauth_server_uri` is pointed at this mock endpoint directly,
  bypassing Lakekeeper's own (unverified in this build) `/v1/oauth/tokens`.

This is **not a production identity provider** and is not meant to be
read as one. A real deployment replaces `ops/oidc-mock` with a real IdP
(Keycloak, Dex, the customer's own OIDC provider) and Lakekeeper's
`openid_provider_uri`/`openid_audience` point at it instead — nothing else
in this design changes, because every writer already authenticates via
the standard OIDC bearer-token / OAuth2-client-credentials mechanisms a
real IdP also speaks.

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
| `unauthorized-test` | none | The negative-test principal — self-registered with Lakekeeper (it has an identity) but never granted anything. |
| `admin` | none (instance-admin bypass) | `LAKEKEEPER__INSTANCE_ADMINS=["oidc~admin"]` — bypasses authorization for control-plane actions only (confirmed via Lakekeeper's own startup log: "these principals bypass authorization for all control-plane actions (but not for `CatalogTableAction::ReadData`/`WriteData`)"). Used only by the one-shot init jobs to bootstrap Lakekeeper and grant the others; never used by a running writer, and cannot itself read or write Bronze data. |

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

**Not granted at all: `trino`.** The `trino`/`trino-maintenance-cron`
profile services are not part of this session's re-run list (none of the
five gates exercise them) and are not granted a principal. Using the
`trino` profile together with R1 authorization today means every catalog
call Trino makes is denied — a real, open gap, not silently papered over.

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

Also not exercised this session: the `trino` profile under enforcement
(Decision 4), and Lakekeeper's own `/v1/oauth/tokens` endpoint (this
build's ClickHouse principal bypasses it entirely via `oauth_server_uri`
pointed at `ops/oidc-mock` — whether Lakekeeper's own endpoint works at
all, with what identity provider wiring, is unknown).
