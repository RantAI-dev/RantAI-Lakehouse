# P5 report — CDC, connector-registry config, slot-lag metrics; Lakekeeper authz NOT enabled

Companion to `docs/plans/P5-RESULT.md` (the (A)/(B) measurements). This
records what P5 actually shipped against the plan's six deliverables, and
is explicit about the one deliverable (R1/Lakekeeper authz) that is not
done — per the task brief's own scope-control instruction, CDC work was
finished first and this is the honest partial for what remains.

## Delivered

1. **Debezium Server + `debezium-server-iceberg`** — `docker-compose.yml`'s
   `debezium-server` service (`dagster` profile), pinned by digest
   (`ghcr.io/memiiso/debezium-server-iceberg@sha256:c49eb...`). Upsert
   mode, initial snapshot then streaming (ADR 0008). Schema evolution
   constraint (R7) implemented as a pure, unit-tested function
   (`lakehouse_store::cdc::reject_unsupported_column_types`) — not yet
   wired into `POST /api/connectors`'s public contract, per ADR 0007's
   explicit scope note (that contract is frontend-consumed and P5 does not
   change it).
2. **ADR 0007** — connector registry -> runtime config.
   `lakehouse_store::cdc::render_debezium_properties` renders the exact
   properties shape measured working in `docs/plans/P5-RESULT.md`, over
   already-resolved `SecretValue`s (ADR 0002's call-site-resolution
   pattern, never weakened — `Connector`/`ConnectorDetail` still have no
   `host`/`secretRef` field, `delete_connector` still never resolves a
   `secretRef`). No dynamic per-connector provisioning ships this phase —
   documented as a deliberate, bounded gap, not a silent one.
3. **ADR 0008** — initial snapshot/backfill. Decision: use Debezium's own
   snapshot mechanism (measured working end-to-end), not a bespoke
   Dagster/dlt backfill job. `snapshot.mode=incremental` is the documented
   extension point for a real large-table connector, not built
   speculatively.
4. **Slot lag / WAL retention metrics (R5)** —
   `dagster/dispar_orchestrate/replication_metrics.py`'s
   `replication_slot_check_job`, every 15 minutes, querying
   `pg_replication_slots` directly against the source and writing
   `lake.bronze_meta.replication_slot` (DDL owned in exactly one place,
   matching `maintenance.py`'s R10-safe precedent). Surfaced via
   `GET /api/governance/replication` — reuses the P4 maintenance surface's
   mechanism (`routes::governance`, `bronze_meta.*`), not a parallel one.
5. **Gate G4** — `ops/g4/g4_test.py` + `debezium-server`/`g4-source-init`/
   `g4-test-runner` compose services, wired into `.github/workflows/
   ci.yml` as `g4-cdc`. See "G4 results" below.
6. **Lakekeeper authorization (R1)** — **NOT enabled.** See below.

## G4 results

Run twice from a clean `docker compose` stack (`p5g4` and `g4ci` project
names), volumes destroyed both times.

| Check | Result |
| --- | --- |
| INSERT visible in ClickHouse | ~1.1s |
| UPDATE visible in ClickHouse | ~1.1s |
| DELETE visible in ClickHouse | ~1.1–2.1s |
| Agreed latency budget | **20 seconds** — a 10–20x margin over the measured commit latency above, justified in `ops/g4/g4_test.py`'s own docstring |
| Replication slot cleanup on connector delete | **Verified.** Slot `p5cdc_slot` exists and reports a non-zero `wal_retained_bytes` before `ops/debezium/deprovision_connector.sh` runs; `pg_replication_slots` has zero rows matching it afterward |

Exact commands to run G4 from clean:

```
cat > .env <<'EOF'
AUTH_BOOTSTRAP_EMAIL=ci@example.com
AUTH_BOOTSTRAP_PASSWORD=ci-password-not-real-123
LAKEKEEPER_BASE_URI=http://lakekeeper:8181
EOF

docker compose -p g4ci --profile dagster up -d \
  postgres clickhouse rustfs rustfs-bucket-init \
  lakekeeper-db-init lakekeeper-migrate lakekeeper lakekeeper-warehouse-init \
  g4-source-init debezium-server

docker compose -p g4ci --profile dagster run --rm g4-test-runner

docker compose -p g4ci down -v
rm .env
```

(Locally, behind another stack already holding the default ports, add
`POSTGRES_HOST_PORT`/`CH_HTTP_HOST_PORT`/`CH_NATIVE_HOST_PORT`/
`RUSTFS_HOST_PORT`/`RUSTFS_CONSOLE_HOST_PORT`/`LAKEKEEPER_HOST_PORT`
overrides to the `.env` — CI itself runs on a clean runner and needs none
of these.)

## No regression

- **`postgres` service now runs with `wal_level=logical`** (+
  `max_wal_senders=10`/`max_replication_slots=10`), unconditionally, not
  profile-gated — P5's CDC needs it and it is a server restart to change
  later. Confirmed this does not regress G1: re-ran `g1-test-runner`
  (RustFS) from clean after this change — `g1_half_a_rust_writes_
  clickhouse_reads ... ok`.
- `cargo fmt --check`, `cargo clippy --all-targets --all-features --locked
  -- -D warnings`, and `cargo deny check licenses` are all clean against
  the full workspace including this phase's changes.
- G2 (SeaweedFS), G3a (dlt), and the G3 maintenance test were **not
  re-run** this session — the Postgres command-line change is additive
  (adds a capability, removes none) and none of those three gates touch
  Postgres configuration directly, but they were not independently
  re-verified against the literal current `docker-compose.yml` here due to
  time budget. Recommend running `g2-seaweedfs`, `g3a-dagster`, and
  `g3-maintenance` (the existing CI job names) before merge, as a final
  check rather than an assumption.

## R1 (Lakekeeper authorization) — not retired, here is what remains

Lakekeeper still runs `"authz-backend":"allow-all"` — confirmed via
`GET /management/v1/info` against the very stack G4 used
(`docs/plans/G1-RESULT.md` first observed this; nothing in P5 changed it).
**This was not attempted this phase.** Precise scope assessment, so the
next phase (or a follow-up task) does not have to re-derive it:

1. **Stand up OpenFGA** (or the OPA bridge) as a new compose service,
   `openfga` profile-gated the same way `dagster`/`trino` are — a
   Postgres- or SQLite-backed datastore for OpenFGA itself (its own schema
   migration step, mirroring `lakekeeper-db-init`/`dagster-db-init`'s
   pattern: a dedicated database on the existing `postgres` service, not a
   third-party SQLite file).
2. **Define an authorization model** consistent with ADR 0003's tenant ->
   warehouse mapping: at minimum, a `warehouse` object type with `owner`
   (project) and `writer`/`reader` relations, and a `project` type
   containing warehouses — OpenFGA's own tuple/model DSL, written once and
   loaded via its `POST /stores/{id}/authorization-models` API at
   bootstrap (a new one-shot init job, same shape as
   `lakekeeper-warehouse-init`).
3. **Configure Lakekeeper to use it**: Lakekeeper's own `authz-backend`
   setting needs to move from `allow-all` to `openfga`, plus
   `LAKEKEEPER__OPENFGA__*` connection settings (store id, client
   credentials if OpenFGA's own auth is enabled) — a `lakekeeper`
   environment-block change plus depending on the new `openfga` service's
   readiness.
4. **Grant every existing writer** (Rust via `lakehouse-iceberg`, dlt,
   Debezium Server, the G1/G3a/G4 test runners) the relations the model
   requires, on the `default` warehouse — otherwise every existing
   catalog-write test (G1, G3a, G4, the manual measurements in this
   report) starts failing on its first run against an authz-enforcing
   Lakekeeper, not because anything is broken but because nothing has been
   granted permission yet. This is the step most likely to surface
   surprises: `lakekeeper-warehouse-init`'s current bootstrap flow assumes
   `allow-all` and grants nothing.
5. **Re-run G1** (the plan's own instruction: "re-run G1 against it to
   confirm catalog writes still work under enforcement") — plus, given
   this phase's own findings, G3a and G4 too, since both are catalog
   writers `allow-all` was silently covering for.
6. **Decide the tenant-scoping shape of the model**: ADR 0003 already
   settled "one warehouse per tenant, named `TENANT_ID` verbatim" — the
   OpenFGA model's `warehouse` objects should be keyed the same way, so a
   tenant's authorization boundary and its Lakekeeper warehouse boundary
   are the same boundary, not two that could drift.

**Rough sizing:** steps 1–3 are a compose/config exercise, roughly the
same shape and size as this phase's `debezium-server` addition (a day or
two of focused work). Step 4 is where real time goes — every existing
writer needs a grant, and getting the grant tuples right against a real
enforcing backend (not assumed) requires the same "measure, don't trust
the brief" discipline this whole build has used, applied to OpenFGA's own
tuple semantics. Step 5 (re-running three gates under enforcement) is
mechanical once 1–4 land but is not instantaneous — G1/G3a/G4 all need a
second clean run each. Total: large enough that attempting it inside this
already-large P5 session, after the CDC measurements and deliverables
above, would have meant either rushing the CDC work (which is the
riskier, harder-to-reverse half of this phase) or leaving both halves
half-done. Per the task brief's own scope-control instruction, CDC was
finished properly and this is the honest partial for authz.

**R1 is therefore still open**, exactly as `docs/plans/G1-RESULT.md` left
it: "Enabling authz means standing up OpenFGA or the OPA bridge and
defining an authorization model: a distinct task, not a config flag." That
sentence remains true after P5.
