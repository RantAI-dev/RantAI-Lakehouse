import type { Health } from "@/lib/status"

export type Connector = {
  id: string
  name: string
  type: string
  direction: "source" | "sink" | "bidirectional"
  health: Health
  environment: string
  tenant: string
  /** `null` until a supported probe has run — see `record_test_result`. */
  lastTestAt: string | null
  /** `null`: nothing measures connector activity yet. */
  lastActivityAt: string | null
  capabilities: string[]
  owner: string
}

export type ConnectorDependent = {
  id: string
  name: string
  kind: "pipeline"
}

export type DiscoveredSchema = {
  name: string
  kind: "table" | "topic" | "prefix"
  columnsOrFields: number
}

export type ConnectorDetail = Connector & {
  discoveredAssets: number
  discoveredSchemas: DiscoveredSchema[]
  recentErrors: { at: string; message: string }[]
  dependentPipelines: ConnectorDependent[]
  auditEventId?: string
}

export type ConnectorTestResult = {
  /** Whether a real connectivity probe succeeded. Always `false` when `supported` is `false`. */
  ok: boolean
  /**
   * Whether this build knows how to dial this connector's type at all.
   * `false` for every type except PostgreSQL and S3-compatible object
   * storage today — see `rust/crates/lakehouse-api/src/connector_probe.rs`.
   */
  supported: boolean
  /** Real measured latency in milliseconds; `null` when `supported` is `false` (no attempt was made). */
  latencyMs: number | null
  message: string
  /** `null` when `supported` is `false`: an unsupported type was never actually dialed. */
  testedAt: string | null
}

/**
 * One row of `GET /api/connectors/{id}/probe-history` — a past connectivity
 * probe's outcome. Distinct from `Connector.lastTestAt`/`health` (current
 * state, unchanged by reading this): this is HISTORY, per-connector,
 * newest first, bounded to the most recent 200 rows — see
 * `rust/migrations/0044_connector_probe_result.sql`. Only ever contains
 * SUPPORTED probes (an unsupported probe never dialed anything, so it has
 * no outcome to record).
 */
export type ConnectorProbeResult = {
  /** ISO 8601. */
  testedAt: string
  ok: boolean
  /** Real measured latency in milliseconds, or `null` if the dial attempt never completed. */
  latencyMs: number | null
  message: string
}

export type ProbeHistoryResponse = {
  /** Newest first. */
  results: ConnectorProbeResult[]
}

/**
 * Which of a connector's two credential slots a
 * `PUT /api/connectors/{id}/secret` request targets. Mirrors Rust
 * `SecretSlot` (`rust/crates/lakehouse-store/src/connectors.rs`)
 * field-for-field, including its lowercase wire form
 * (`#[serde(rename_all = "lowercase")]`).
 */
export type SecretSlot = "primary" | "secondary"

/**
 * Which resolver scheme a derived connector-credential name uses. Mirrors
 * Rust `CredentialSource` (`rust/crates/lakehouse-store/src/connectors.rs`)
 * field-for-field, including its `snake_case` wire form.
 */
export type CredentialSource = "env" | "file"

/**
 * The fixed suffix a derived connector-credential name ends in. Mirrors
 * Rust `CredentialKind` — ADR 0002 Addendum 3's six allowed suffixes,
 * `snake_case` wire form. `private_key` is for the `sftp` adapter's
 * `SftpAuth`'s `public_key` auth kind (a private-key PEM, not any of the
 * other five shapes).
 */
export type CredentialKind =
  | "password"
  | "secret_key"
  | "access_key"
  | "api_key"
  | "token"
  | "private_key"

/**
 * What the client chooses for a connector's credential(s): a source scheme
 * and a kind per slot. Never a reference NAME -- the client does not know
 * the connector's id yet (the server generates it), so it cannot name a
 * ref itself. The server derives the actual name(s) from the id it
 * generates and returns them once, in `CreateConnectorResponse.credential`
 * (see `docs/adr/0002-secretref-resolution.md`'s Addendum 3).
 */
export type CredentialSpec = {
  source: CredentialSource
  primary: CredentialKind
  /**
   * Optional secondary slot, e.g. the secret-access-key half of an S3
   * connector's access-key/secret-key pair. Without this, an API-created S3
   * connector can never be tested — `connector_probe::probe_s3` requires
   * both slots to be set.
   */
  secondary?: CredentialKind
}

/**
 * The `PUT /api/connectors/{id}/secret` body. No free-text ref any more
 * (ADR 0002 Addendum 3): the caller chooses a source/kind for the slot
 * being rotated, and the server derives the new ref from the CONNECTOR'S
 * OWN id (a derived `env:` name always begins `CONNECTOR_CONN_`, so a
 * rotation can never target one of the deployment's reserved, seeded
 * patterns). The server runs a real connectivity probe against a
 * candidate built with the derived ref BEFORE writing anything -- see
 * `rust/crates/lakehouse-api/src/routes/connectors.rs::rotate_secret`'s
 * doc comment for the full probe-first contract, including why an
 * unverifiable rotation is refused (422) rather than applied
 * unverified.
 */
export type RotateConnectorSecretRequest = {
  slot: SecretSlot
  source: CredentialSource
  kind: CredentialKind
}

/**
 * The `PUT /api/connectors/{id}/secret` response body. Mirrors Rust
 * `RotateSecretResponse` exactly.
 */
export type RotateConnectorSecretResponse = {
  /** Always `true` on a successful (2xx) response -- a refused rotation
   * is a non-2xx `ServiceError`, never this shape with `rotated: false`. */
  rotated: boolean
  slot: SecretSlot
}

export type CreateConnectorInput = {
  name: string
  type: string
  direction: Connector["direction"]
  host: string
  /** No `secretRef`/`secretRefSecondary` field any more (ADR 0002
   * Addendum 3) -- see `CredentialSpec`'s doc comment. */
  credential: CredentialSpec
  environment: string
  tenant: string
  residency: string
  capabilities: string[]
  owner?: string
}

/**
 * The credential reference NAMES a newly created connector's operator
 * must provision -- returned ONCE, by `POST /api/connectors`, and never
 * again (no GET response for this connector repeats them). Mirrors Rust
 * `ConnectorCredentialNames`.
 */
export type ConnectorCredentialNames = {
  primary: string
  secondary: string | null
}

/**
 * The `POST /api/connectors` response body: the created `Connector` plus
 * the names the operator must provision. Mirrors Rust
 * `CreateConnectorResponse`.
 */
export type CreateConnectorResponse = Connector & {
  credential: ConnectorCredentialNames
}

/**
 * The `dial` payload for a connector's ingest spec. Mirrors the opaque
 * `serde_json::Value` field on Rust's `IngestSpec`/`IngestSpecInput`
 * (`rust/crates/lakehouse-store/src/connectors.rs`): its actual shape is
 * one of five adapter-specific structs in
 * `rust/crates/lakehouse-store/src/ingest_spec.rs`, dispatched on
 * `adapter` and validated there by `Dial::parse` on every `PUT`. The
 * browser never re-validates `dial`'s contents — it is deliberately
 * loose JSON on this side.
 */
export type Dial = Record<string, unknown>

/**
 * Per-adapter `dial` shapes, mirroring
 * `rust/crates/lakehouse-store/src/ingest_spec.rs`'s five
 * `#[serde(deny_unknown_fields, rename_all = "camelCase")]` structs
 * field-for-field. Added alongside `src/features/connectors/dial-forms/`,
 * whose components narrow the wire-level `Dial` blob above to exactly one
 * of these before handing it back to their `onChange` — a form typed
 * against `Dial` alone could not otherwise be checked against its own
 * adapter's real fields.
 */
export type SqlDriver = "mysql" | "postgres" | "mssql" | "oracle"

export type SqlDial = {
  driver: SqlDriver
  host: string
  port: number
  database: string
  /** A literal username, never a `secretRef` — see `ingest_spec.rs`'s
   * `SqlDial::user` doc comment. */
  user: string
  sslMode: string | null
  /**
   * An operator-typed Distinguished Name, required whenever `sslMode`
   * implies TLS for `driver: "oracle"` — never derived from `host` (a
   * bare `CN=<hostname>` would not match a real certificate's full DN).
   * Ignored for every other driver. Mirrors `SqlDial::ssl_server_cert_dn`.
   */
  sslServerCertDn?: string | null
}

/** `CdcDial` = `SqlDial`'s fields plus `slotName`/`publicationName`,
 * both required with no fallback (`ingest_spec.rs`'s `CdcDial`). */
export type CdcDial = {
  driver: SqlDriver
  host: string
  port: number
  database: string
  user: string
  slotName: string
  publicationName: string
  serverId: number | null
}

export type FilesProtocol = "s3" | "sftp"
export type FilesFormat = "csv" | "json" | "parquet"

export type FilesDial = {
  protocol: FilesProtocol
  endpoint: string | null
  bucket: string
  prefix: string | null
  format: FilesFormat
  region: string | null
}

/**
 * Internally tagged on `type`, mirroring `RestAuth`
 * (`#[serde(tag = "type")]`) exactly — the same four `type` values
 * `RestAuth::type_tag` names.
 */
export type RestAuth =
  | { type: "api_key"; header: string }
  | { type: "bearer" }
  | { type: "oauth2_client_credentials"; tokenUrl: string }
  | { type: "basic" }

export type RestPagination =
  | { type: "none" }
  | { type: "page"; param: string }
  | { type: "cursor"; cursorField: string }

export type RestEndpoint = {
  path: string
  recordsPath: string | null
}

export type RestDial = {
  baseUrl: string
  auth: RestAuth
  pagination: RestPagination
  endpoints: RestEndpoint[]
}

export type SheetsDial = {
  spreadsheetId: string
  ranges: string[]
}

/**
 * `dial` for the `mongodb` adapter. Mirrors `MongoDial`
 * (`ingest_spec.rs`, `#[serde(deny_unknown_fields, rename_all =
 * "camelCase")]`) exactly. `directConnection` MUST be `true` — this build
 * refuses `mongodb+srv` and replica-set discovery outright; there is no
 * `srvUri` field at all (never a toggle for something the server
 * refuses). `username` is a literal, never a `secretRef` picker, same
 * reasoning as `SqlDial.user`.
 */
export type MongoDial = {
  hosts: string[]
  database: string
  username: string
  directConnection: true
}

/**
 * `dial` for the `kafka` adapter. Mirrors `KafkaDial` (`ingest_spec.rs`)
 * exactly.
 */
export type KafkaDial = {
  bootstrapServers: string[]
  topic: string
  auth: KafkaAuth
  groupId: string
  microBatchSeconds: number
}

/**
 * Internally tagged on `type`, mirroring `KafkaAuth`
 * (`#[serde(tag = "type")]`) exactly — the two variants
 * `KafkaAuth::type_tag` names. `sasl_plain`'s `username` is dial
 * configuration, not a secret (only the password is, via the connector's
 * `secretRef`) — see `KafkaAuth::SaslPlain`'s doc comment.
 */
export type KafkaAuth = { type: "sasl_plain"; username: string } | { type: "none" }

/**
 * `dial` for the `sftp` adapter. Mirrors `SftpDial` (`ingest_spec.rs`)
 * exactly. `hostKeyFingerprint` is REQUIRED with no fallback — host-key
 * verification, never `paramiko.AutoAddPolicy`.
 */
export type SftpDial = {
  host: string
  port: number
  user: string
  hostKeyFingerprint: string
  path: string
  fileFormat: string
  auth: SftpAuth
}

/**
 * Internally tagged on `type`, mirroring `SftpAuth`
 * (`#[serde(tag = "type")]`) exactly.
 */
export type SftpAuth = { type: "password" } | { type: "public_key" }

/**
 * One object (table, endpoint, sheet range) an ingest job targets.
 * Mirrors `SourceObject` in
 * `rust/crates/lakehouse-store/src/ingest_spec.rs` (`#[serde(deny_unknown_fields,
 * rename_all = "camelCase")]`).
 */
export type SourceObject = {
  name: string
  incrementalKey?: string
  target: string
}

/**
 * Credential reference NAMES only (`env:FOO`, `file:/…`), never a
 * resolved value — mirrors `IngestSecretRefs` in
 * `rust/crates/lakehouse-store/src/connectors.rs` exactly, including its
 * `Option<String>` nullability on `secondary`.
 */
export type IngestSecretRefs = {
  primary: string
  secondary: string | null
}

/**
 * The closed set the database enforces via `connector_adapter_check`
 * (`rust/migrations/0033_connector_ingest_spec.sql`, widened by
 * `0043_ingest_tier2_adapters.sql` to add the three Tier 2 values below).
 * Widened with `| string` because the Rust field is a plain
 * `Option<String>`, not a closed enum — the client stays honest about a
 * value the server might send that predates this list or that a future
 * migration adds.
 */
export type IngestAdapter =
  | "sql"
  | "cdc"
  | "files"
  | "rest"
  | "sheets"
  | "mongodb"
  | "kafka"
  | "sftp"
  | string

/**
 * The closed set the database enforces via `connector_ingest_mode_check`
 * (`rust/migrations/0033_connector_ingest_spec.sql`, widened by
 * `0043_ingest_tier2_adapters.sql` to add `"stream"` for `kafka`). See
 * `IngestAdapter` for why this widens with `| string`.
 */
export type IngestMode = "batch" | "cdc" | "stream" | string

/**
 * A connector's ingest configuration, as returned by
 * `GET /api/connectors/{id}/ingest-spec`. Mirrors Rust `IngestSpec`
 * (`rust/crates/lakehouse-store/src/connectors.rs`) field-for-field,
 * including nullability: `adapter`/`ingestMode` are plain
 * `Option<String>` there (not a closed enum), `null` for a connector
 * that has never had an ingest spec set.
 */
export type IngestSpec = {
  adapter: IngestAdapter | null
  ingestMode: IngestMode | null
  dial: Dial
  sourceObjects: SourceObject[]
  scheduleCron: string | null
  secretRefs: IngestSecretRefs
}

/**
 * The `PUT /api/connectors/{id}/ingest-spec` body. Mirrors Rust
 * `IngestSpecInput` (`rust/crates/lakehouse-store/src/connectors.rs`):
 * `adapter`/`ingestMode` are required plain strings there too (validated
 * at runtime by `Dial::parse`, not by the type system), and
 * `scheduleCron` is omittable for a `cdc`-mode job or one that is not
 * scheduled.
 */
export type IngestSpecInput = {
  adapter: IngestAdapter
  ingestMode: IngestMode
  dial: Dial
  sourceObjects: SourceObject[]
  scheduleCron?: string
}

/**
 * One row of `connector_type` (`rust/migrations/0035_connector_type.sql`),
 * as returned by the wizard's type listing. Mirrors Rust
 * `ConnectorType` (`rust/crates/lakehouse-store/src/connector_type.rs`)
 * field-for-field. `supported: false` is a real, listed roadmap entry
 * (AGENTS.md rule 2 — never omitted, never faked as working).
 */
export type ConnectorType = {
  name: string
  adapter: IngestAdapter | null
  supported: boolean
  docsUrl: string | null
}

/**
 * One discovered column. Mirrors Rust `DiscoveredColumn`
 * (`rust/crates/lakehouse-api/src/connector_discover.rs`) exactly —
 * note the field is `typeName`, not `type`, and this array is never
 * `null` (only ever empty), matching that struct's plain `Vec`.
 */
export type DiscoveredColumn = {
  name: string
  typeName: string
}

/**
 * One discovered table (or future non-SQL equivalent object). Mirrors
 * Rust `DiscoveredObject` exactly — `columns` is a plain array, never
 * nullable, and there is no `sample` field: this route reports schema
 * only, never a data preview.
 */
export type DiscoveredObject = {
  name: string
  columns: DiscoveredColumn[]
}

/**
 * The response body for `POST /api/connectors/{id}/discover`. Mirrors
 * Rust `DiscoverResult` exactly (same module). `reason` is populated
 * whenever `supported` is `false`, omitted (not `null`) otherwise — the
 * Rust field is `Option<String>` with no `skip_serializing_if`, but every
 * other `Option<String>` reason field on this route follows the same
 * "absent means not applicable" convention as `AlertRule`'s optional
 * fields (`contracts/alerts.ts`), so this widens with `?` rather than
 * `| null` for consistency; either reads a missing key the same way.
 */
export type DiscoverResult = {
  objects: DiscoveredObject[]
  supported: boolean
  reason?: string
}

/**
 * `POST /api/connectors/{id}/ingest/run`'s response. The route returns
 * a raw `serde_json::Value`, not a typed struct
 * (`rust/crates/lakehouse-api/src/routes/connectors.rs`'s `ingest_run`),
 * and its two branches send DIFFERENT keys — never both: a successful
 * launch sends only `{ runId }` (no `supported` key at all); the
 * honest `cdc`-adapter refusal sends only `{ supported: false, reason }`
 * (no `runId`). Both fields are therefore optional here, not `supported:
 * boolean` required — a required `supported` would claim every response
 * carries it, which the real wire body does not.
 */
export type IngestRunResult = {
  runId?: string
  supported?: boolean
  reason?: string
}

/**
 * One row of `bronze_meta.ingest_run` (ClickHouse, WS3 item 25), as
 * returned by `GET /api/governance/ingest-runs?connectorId=`.
 */
export type IngestRun = {
  connectorId: string
  job: string
  object: string
  /** `null` means "not measured" (dlt's normalize row count was
   * unavailable), never a fabricated `0` (WS3 plan review Z9). */
  rows: number | null
  startedAt: string
  endedAt: string
  status: string
  error: string
}

/**
 * `GET /api/connectors/{id}/debezium-properties?table=`'s response.
 * Mirrors Rust `DebeziumPropertiesResponse`
 * (`rust/crates/lakehouse-api/src/routes/connectors.rs`). `properties`
 * contains ONLY `${ENV_VAR_NAME}` references for every credential-shaped
 * field, never a resolved secret — rendered as literal text, labeled with
 * `note`, and never resolved client-side.
 */
export type DebeziumProperties = {
  properties: string
  table: string
  note: string
}

/**
 * One connector `dagster/dispar_orchestrate/ingest_factory.py` can build
 * and run a job for. Mirrors Rust `IngestibleConnector`
 * (`rust/crates/lakehouse-store/src/connectors.rs`) — NOT `Connector`:
 * this is a materially different, wider shape (exposes `adapter`/
 * `dial`/`secretRef` names, which the redacted `Connector` type never
 * does) returned by `GET /api/connectors/ingestible`, scoped to
 * `ingest:read` callers.
 */
export type IngestibleConnector = {
  id: string
  adapter: IngestAdapter
  ingestMode: IngestMode
  dial: Dial
  sourceObjects: unknown
  scheduleCron: string | null
  secretRef: string
  secretRefSecondary: string | null
}

export interface ConnectorService {
  listConnectors(signal?: AbortSignal): Promise<Connector[]>
  getConnector(id: string, signal?: AbortSignal): Promise<ConnectorDetail>
  createConnector(input: CreateConnectorInput, signal?: AbortSignal): Promise<CreateConnectorResponse>
  testConnection(id: string, signal?: AbortSignal): Promise<ConnectorTestResult>
  getIngestSpec(id: string, signal?: AbortSignal): Promise<IngestSpec>
  setIngestSpec(id: string, input: IngestSpecInput, signal?: AbortSignal): Promise<IngestSpec>
  /**
   * `GET /api/connectors/types` — every row of `connector_type`, used by
   * the create wizard to offer a type (or list it disabled, honestly,
   * when `supported` is `false`). WS3 item 33 mounted this route
   * (`rust/crates/lakehouse-api/src/routes/connectors.rs::list_types`,
   * gated `connector:manage`, matching this wizard's other connector
   * reads) — the store function and migration already existed with no
   * HTTP surface between them; this client method 404d until that gap
   * fix closed it.
   */
  listTypes(signal?: AbortSignal): Promise<ConnectorType[]>
  listIngestible(signal?: AbortSignal): Promise<IngestibleConnector[]>
  discoverConnector(id: string, signal?: AbortSignal): Promise<DiscoverResult>
  runIngest(id: string, signal?: AbortSignal): Promise<IngestRunResult>
  listIngestRuns(connectorId: string, signal?: AbortSignal): Promise<IngestRun[]>
  /**
   * `GET /api/connectors/{id}/debezium-properties?table=` — a read-only
   * rendering of the `Debezium` `.properties` file body for a `cdc`
   * adapter's captured table, `${ENV_VAR_NAME}` references only, never a
   * resolved secret. `table` is required: no registry column stores which
   * table alone until an ingest spec names one via `sourceObjects`.
   */
  getDebeziumProperties(
    id: string,
    table: string,
    signal?: AbortSignal
  ): Promise<DebeziumProperties>
  /**
   * `GET /api/connectors/{id}/probe-history?limit=` — the connector's most
   * recent connectivity-probe results, newest first. `limit` defaults to
   * 50 server-side when omitted; the server rejects (never silently
   * clamps) a `limit` outside `1..=200`.
   */
  listProbeHistory(id: string, limit?: number, signal?: AbortSignal): Promise<ProbeHistoryResponse>
  /**
   * `PUT /api/connectors/{id}/secret` — probe-first credential-reference
   * rotation. Rejects (never applies) a rotation this build cannot verify
   * -- see `RotateConnectorSecretRequest`'s doc comment.
   */
  rotateSecret(
    id: string,
    body: RotateConnectorSecretRequest,
    signal?: AbortSignal
  ): Promise<RotateConnectorSecretResponse>
}
