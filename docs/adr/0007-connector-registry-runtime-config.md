# ADR 0007 — connector registry -> Debezium/dlt runtime config

- **Status:** Accepted, partially implemented
- **Phase:** P5
- **Date:** 2026-09-01

## Context

`connector` (`lakehouse-store::connectors`) has been a CRUD registry since
Phase 2: a row names a source/sink system, a `secretRef` (never a
credential value — see that module's doc comment), and metadata. Nothing
has ever turned a row into a runtime configuration for anything that
actually connects. P5 needs exactly that, twice over: a connector row must
be able to generate (a) a Debezium Server `application.properties` body for
CDC connectors, and (b) a dlt pipeline config for batch connectors (P3's
existing shape, `dagster/dispar_orchestrate/dlt_pipeline.py`, which today
takes its Postgres source from `BRONZE_SOURCE_DATABASE_URL` — a compose env
var, not a registry row — this ADR's dlt half is therefore a design record
for the mapping, not a code change to a hardcoded demo pipeline).

## Decision — where the mapping lives, and in what shape

**A pure Rust function in `lakehouse-store::cdc`
(`render_debezium_properties`), over already-resolved inputs, plus a pure
schema validator (`reject_unsupported_column_types`) enforcing R7/ADR
0006's registration-time gate.** Concretely:

- `render_debezium_properties(source: &DebeziumSourceSpec, sink:
  &IcebergSinkSpec, database_password: &SecretValue, s3_access_key:
  &SecretValue, s3_secret_key: &SecretValue) -> String` renders a complete
  `application.properties` body: upsert mode, initial snapshot then
  streaming, file-based offset/schema-history storage (not the
  Iceberg-backed default — see `docs/plans/P5-RESULT.md`'s measured trap:
  those default ON and silently create two extra catalog tables per
  connector that have nothing to do with Bronze), `pgoutput` plugin, a
  slot/publication name derived from the connector's own slug
  (`<slug>_slot` / `<slug>_pub`), and the SAME Iceberg-sink shape
  `docs/plans/P5-RESULT.md` measured working end-to-end against Lakekeeper.
- **Secret resolution happens at the call site**, exactly like ADR 0002
  designed `IcebergClientConfig`'s `catalog_credential`: this function
  takes an already-resolved `SecretValue` for the database password and
  the two S3 credentials, never a `secretRef` string and never a
  `SecretResolver` — it does not need to know which resolver scheme
  produced the value, and stays free of any resolver-implementation
  dependency.
- **`reject_unsupported_column_types(columns: &[SourceColumn]) ->
  Result<(), UnsupportedColumnType>`** is R7's mitigation, made concrete:
  given a source schema (column name + source-reported type name), it
  rejects the first nested struct/array/map column found. This is the
  validator ADR 0006 said would exist "at registration, not at read time"
  and left unbuilt, pending this ADR.
- **Both are pure functions, deliberately with no I/O.** Neither opens a
  network connection, queries a database, or writes a file — matching
  `connectors::test_connection`'s existing "this service does not
  originate outbound connections to operator-configured external systems"
  posture. A future schema-discovery step (connecting to a real source to
  list its columns) is out of scope here, same as it always has been for
  `test_connection`.

## What P5 does NOT do

This is the honest boundary, stated plainly rather than discovered later:

- **No dynamic per-connector provisioning.** Creating a connector row via
  `POST /api/connectors` does not spin up a new `debezium-server` container,
  and deleting one (`DELETE /api/connectors/{id}`, new in P5) does not tear
  one down. `docker-compose.yml`'s `debezium-server` service is a single,
  statically-configured instance for one demo CDC source (mirroring
  `dagster-code-location`'s own static `BRONZE_SOURCE_*` env vars from P3)
  — proving the config-rendering mechanism and the read path (P5-RESULT.md,
  G4) without building a container-orchestration control plane this phase
  has no room for. Wiring `render_debezium_properties`'s output to an
  actually-provisioned container per registry row is a P6-or-later console
  concern, the same deferral shape ADR 0003 used for warehouse creation
  ("deployment tooling ... tracked separately").
- **`reject_unsupported_column_types` is not wired into `POST
  /api/connectors`.** The existing `CreateConnectorInput`/
  `CreateConnectorBody` contract (mirrored in
  `src/services/contracts/connectors.ts`, a frontend-consumed type) has no
  column-schema field — adding one is a public contract change this phase
  did not make, to avoid touching frontend contract/mock code outside this
  phase's scope. The validator function exists, is unit-tested, and is
  exactly what a future schema-discovery/registration flow should call
  before persisting a connector; wiring the call site is left for whoever
  adds that flow (naturally a P6 console concern, alongside the dynamic
  provisioning gap above).
- **The dlt half is a design record, not new code.** `dlt_pipeline.py`'s
  existing shape (source DSN + table name -> Bronze Iceberg) already IS the
  registry-row-to-config mapping for batch connectors, conceptually — it
  just reads its inputs from compose env vars today rather than a
  `connector` row. Making it read from the registry is the same kind of
  "P6 wires it into the UI" deferral ADR 0003 used for
  `lakekeeper_warehouse`/`TENANT_ID`, for the identical reason: no route
  exists yet that would call it with a real row.

## Consequences

- `lakehouse-store::cdc` gains `SourceColumn`, `UnsupportedColumnType`,
  `reject_unsupported_column_types`, `DebeziumSourceSpec`,
  `IcebergSinkSpec`, `render_debezium_properties`.
- `docker-compose.yml`'s `debezium-server` service's
  `ops/debezium/application.properties.tmpl` is hand-written to the SAME
  shape `render_debezium_properties` produces — this is deliberate parity,
  not two independent implementations: the compose template is what a
  human operator writes today; the Rust function is what a future
  automated path would render. Divergence between them would be exactly
  the kind of drift R10 already flags for `bronze_meta.*`'s two owners, so
  any future change to one should be checked against the other by hand
  until an automated path make the compose template obsolete.
- `DELETE /api/connectors/{id}` (new in P5) deletes the registry row only;
  see `lakehouse_store::connectors::delete_connector`'s doc comment for why
  it does not attempt to drop a source's replication slot, and
  `ops/debezium/deprovision_connector.sh` for the operational mechanism
  that does (verified by G4).

## Verification

`cargo test -p lakehouse-store cdc::` — the type-rejection cases (arrays,
structs, records, maps, scalars including `jsonb`) and a rendered-config
test asserting every secret lands in exactly the field it belongs in
(`database.password`, `s3.access-key-id`, `s3.secret-access-key`) and
nowhere else, plus the derived slot/publication/topic-prefix naming. See
`docs/plans/P5-RESULT.md` for the end-to-end proof that a config in this
exact shape, run against Lakekeeper, produces a genuine REST-catalog
write.
