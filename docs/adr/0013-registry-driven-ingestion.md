# ADR 0013 — Registry-driven ingestion: `dial` shapes

- **Status:** Accepted
- **Phase:** WS3
- **Date:** 2026-09-16

## Context

Before WS3, `connector` (`lakehouse-store::connectors`) was a CRUD registry
that named a source/sink system and a `secretRef`, but nothing turned a row
into a runtime configuration a Dagster job could actually run. WS3's
objective is the opposite of ADR 0007's Debezium/dlt design record: a
connector row must be able to *drive* a generated ingestion job
(`ingest_factory.py`) end to end, for five adapter kinds (`sql`, `cdc`,
`files`, `rest`, `sheets`), without inventing a bespoke config format per
adapter and without ever letting a credential travel through the same
column a plain connection parameter does.

`0033_connector_ingest_spec.sql` adds five columns to `connector`:
`adapter`, `ingest_mode`, `dial` (`JSONB NOT NULL DEFAULT '{}'::jsonb`),
`source_objects` (`JSONB NOT NULL DEFAULT '[]'::jsonb`), and
`schedule_cron`, gated by two `CHECK` constraints
(`connector_adapter_check`, `connector_ingest_mode_check`). This ADR is the
durable record of what `dial` is allowed to contain per adapter, and why.

## Decision — five `dial` shapes, one per adapter, `deny_unknown_fields`

`dial` is free-form `JSONB` at the column level, but never free-form at the
application level: `lakehouse_store::ingest_spec` parses it into one of
five `#[serde(deny_unknown_fields, rename_all = "camelCase")]` structs,
dispatched on the connector's own `adapter` column value — never a
`#[serde(untagged)]` enum, so an ambiguous or malformed `dial` fails with a
named field-path error rather than silently matching the wrong shape.

- **`sql`** — `driver` (`mysql` | `postgres` | ... , a closed
  `SqlDriver` enum), `host`, `port`, `database`, `user`, `sslMode`
  (optional).
- **`cdc`** — the same connection fields as `sql`, plus `slotName` and
  `publicationName` as **required** (not `Option`) fields and an optional
  `serverId`: a `cdc` connector's replication slot/publication come from
  `dial` with no fallback, so a `cdc` dial missing either is a
  construction-time (400) error, not a later runtime surprise.
- **`files`** — `protocol` (a closed `FilesProtocol` enum), `endpoint`
  (optional), `bucket`, `prefix` (optional), `format` (a closed
  `FilesFormat` enum), `region` (optional).
- **`rest`** — `baseUrl`, `auth` (internally tagged on `type`: `api_key` |
  `bearer` | `oauth2_client_credentials` | `basic`), `pagination`,
  `endpoints` (a list), each of `auth`/`pagination`/`endpoints`' element
  type its own `deny_unknown_fields` struct.
- **`sheets`** — `spreadsheetId`, `ranges` (a list of range strings).

**`user` on `SqlDial` and `CdcDial` is a literal, non-`secretRef`-shaped
string, not a resolver reference.** A connector's `dial.user` is written
directly (e.g. `"app_reader"`), never as `"env:CONNECTOR_<ID>_USER"` or any
other indirection scheme. This is a deliberate correction against the
grand plan's original draft, which put `user` behind the same
`secretRef`-style indirection as a password: a username is not a
credential, and the seeded `conn-pg-lakehouse` row already encodes its
username directly in its `host` (`lakehouse@postgres:5432/lakehouse`,
`0022_prune_connector_seed.sql`) rather than through any resolver — adding
a second, indirect representation of the same non-secret value would be
redundant, not safer.

## Consequences

- **`dial` cannot smuggle a credential.** Every one of the five structs is
  `#[serde(deny_unknown_fields)]`: a `dial` payload carrying a `password`,
  `apiSecret`, `clientSecret`, or any other field the struct does not
  declare is rejected by `serde_json::from_value` with an "unknown field"
  error before it is ever persisted, closing off the obvious attack of
  hiding a secret value inside a JSON blob that the allowlisted
  `secretRef` resolver never inspects. This is enforced per adapter, at
  parse time, not by a single shared schema that would have to enumerate
  every field every adapter might ever need.
- **A real credential still only ever travels as a `secretRef`.** `dial`
  describes *how to connect* (host, port, protocol, pagination shape); the
  connector's existing `secretRef` column (ADR 0002) still names the one
  thing that resolves to an actual secret value (a password, an API key).
  Nothing in this ADR changes how `secretRef` resolution works.
- **New columns are additive and safe against existing rows.** `adapter`
  and `ingest_mode` are nullable; `dial` and `source_objects` default to
  empty JSON. Every connector row that predates `0033`, including the two
  `0022_prune_connector_seed.sql` seeds, keeps parsing with
  `adapter = NULL` ("not ingestible yet") rather than failing to migrate.
- **A `cdc` dial's slot/publication naming is explicit, not derived.**
  Unlike ADR 0007's Debezium config renderer, which derives a slot/
  publication name from the connector's slug, a registry-driven `cdc`
  connector's `slotName`/`publicationName` come from the operator (or the
  creation wizard) at `dial`-construction time. This is a narrower,
  registry-specific decision than ADR 0007's, made because `dial` is meant
  to be the complete, self-describing input to a generated ingestion job —
  a job that has no other source of truth to derive a name from.
- **Never `#[serde(untagged)]`.** Dispatching on the stored `adapter`
  column value, then parsing against exactly one struct, means a
  malformed `dial` for a known adapter always fails with that adapter's
  own field-path error. An untagged enum would instead try every variant
  in turn and could report a confusing error from the wrong shape, or
  worse, silently accept a payload that happens to satisfy a different
  adapter's structurally-similar fields.

## Alternatives considered

**Per-adapter column families instead of one JSONB `dial` column** — five
sets of nullable columns on `connector` (e.g. `sql_host`, `sql_port`,
`cdc_slot_name`, `rest_base_url`, ...), one set per adapter, with a
`CHECK` ensuring only the set matching `adapter` is populated. Rejected:

- It multiplies `connector`'s column count by roughly the number of
  adapters (five today, more as adapters are added), most of them `NULL`
  for any given row, for no query benefit this build actually needs —
  nothing filters or joins on an individual `dial` field at the SQL level;
  every consumer (`ingest_factory.py`, `ingest_spec.rs`) reads the whole
  shape at once.
- Adding a field to one adapter's shape (e.g. `RestDial` gaining a new
  pagination style) would be a migration under the column-family approach;
  under JSONB it is a code-only change to that adapter's struct, with
  `deny_unknown_fields` still enforcing the shape at the application
  layer where the actual validation already has to live regardless of
  storage shape.
- The `CHECK` constraint needed to keep column families mutually
  exclusive by `adapter` would just reimplement, in SQL, the same
  discriminated-union logic `ingest_spec::Dial::parse` already has to
  implement in Rust to do anything useful with the value — duplicating
  the invariant in two places for no additional safety, since the Rust
  layer is the one every route path actually goes through.

JSONB with per-adapter `deny_unknown_fields` structs keeps the schema
additive and narrow (one column, `0033`'s migration) while pushing the
actual shape enforcement to the one layer that has to enforce it either
way.

## Verification

`cargo test -p lakehouse-store` for `0033`'s own regression coverage —
`tests/connector_ingest_spec.rs` — asserting the Data Engineer role gains
`ingest:read` exactly once and the five new columns default as documented
for the existing seeded rows. `Dial::parse` and its per-adapter shape
tests (`lakehouse_store::ingest_spec`, one shape-parses and one
field-rejection test per adapter) land in a later commit; this ADR is the
durable record of the shapes ahead of that code.
