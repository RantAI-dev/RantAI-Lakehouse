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

export type CreateConnectorInput = {
  name: string
  type: string
  direction: Connector["direction"]
  host: string
  secretRef: string
  /**
   * Optional secondary reference, e.g. the secret-access-key half of an S3
   * connector's access-key/secret-key pair. Without this, an API-created S3
   * connector can never be tested — `connector_probe::probe_s3` requires
   * both `secretRef` and `secretRefSecondary` to be set.
   */
  secretRefSecondary?: string
  environment: string
  tenant: string
  residency: string
  capabilities: string[]
  owner?: string
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
 * (`rust/migrations/0033_connector_ingest_spec.sql`). Widened with
 * `| string` because the Rust field is a plain `Option<String>`, not a
 * closed enum — the client stays honest about a value the server might
 * send that predates this list or that a future migration adds.
 */
export type IngestAdapter = "sql" | "cdc" | "files" | "rest" | "sheets" | string

/**
 * The closed set the database enforces via `connector_ingest_mode_check`
 * (`rust/migrations/0033_connector_ingest_spec.sql`). See `IngestAdapter`
 * for why this widens with `| string`.
 */
export type IngestMode = "batch" | "cdc" | string

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

export interface ConnectorService {
  listConnectors(signal?: AbortSignal): Promise<Connector[]>
  getConnector(id: string, signal?: AbortSignal): Promise<ConnectorDetail>
  createConnector(input: CreateConnectorInput, signal?: AbortSignal): Promise<Connector>
  testConnection(id: string, signal?: AbortSignal): Promise<ConnectorTestResult>
  getIngestSpec(id: string, signal?: AbortSignal): Promise<IngestSpec>
  setIngestSpec(id: string, input: IngestSpecInput, signal?: AbortSignal): Promise<IngestSpec>
}
