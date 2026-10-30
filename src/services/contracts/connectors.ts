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
  /** `null` until the orchestrator has reported a successful run (`SRC-7`). */
  lastRunSuccessAt: string | null
  /** `null` until the orchestrator has reported a failed run (`SRC-7`). */
  lastRunFailureAt: string | null
  /** Failed runs since the last success; 0 after a success (`SRC-7`). */
  failureStreak: number
  /** What a non-breaking source schema change does (`SRC-8`, D1). */
  schemaChangePolicy: SchemaChangePolicy
  /** Why the connector is held back from loading; `null` when not paused (`SRC-8`). */
  pausedReason: string | null
  /** When the pause began; `null` when not paused. */
  pausedAt: string | null
}

export type SchemaChangePolicy =
  | "apply_non_breaking"
  | "apply_all"
  | "ask_first"
  | "pause"

/**
 * `GET /api/connectors/{id}/schema-changes` and the approve answer mirror
 * Rust `lakehouse_store::schema_change::SchemaChange` (`SRC-8`). `kind` and
 * `status` are widened with `| string` so a value this build does not know
 * still renders.
 */
export type SchemaChangeKind =
  | "column_added"
  | "column_removed"
  | "type_changed"
  | "primary_key_changed"
  | "table_added"

export type SchemaChangeStatus = "applied" | "pending" | "approved"

export type SchemaChange = {
  id: string
  /** The source table (`<schema>.<table>` for SQL sources). */
  objectName: string
  kind: SchemaChangeKind | string
  /** Empty for `primary_key_changed` and `table_added`. */
  columnName: string
  /** For `table_added`: `null`. For `type_changed`: the type before. */
  beforeValue: string | null
  /**
   * For `type_changed`: the type after. For `table_added`: the target Bronze
   * table when status is `applied`, or the fixed "Not added: ..." reason
   * when `pending`.
   */
  afterValue: string | null
  breaking: boolean
  status: SchemaChangeStatus | string
  runId: string | null
  detectedAt: string
  decidedBy: string | null
  decidedAt: string | null
  /**
   * Whether the approve route will take this change. `false` for a change
   * that does not wait and for every waiting change of a table that has a
   * SQL type change its column cannot hold (decision 10).
   */
  canApprove: boolean
}

/** A column kept in the Bronze table after the source dropped it. */
export type InactiveColumn = {
  objectName: string
  columnName: string
  inactiveSince: string
  /** The name the Bronze table gives the column; `null` when it was not reported, and then the Schema tab marks nothing. */
  loadedName: string | null
  /**
   * `SRC-11` decision 10: why the column is inactive. `removed_at_source`: the source dropped it (`SRC-8`).
   * `excluded`: the user left it out of the load. Widened: a newer API may add a reason.
   */
  reason: "removed_at_source" | "excluded" | string
}

export type SchemaChangeList = {
  /** What waits for a decision, oldest first. */
  pending: SchemaChange[]
  /** The newest changes that no longer wait. */
  recent: SchemaChange[]
  inactiveColumns: InactiveColumn[]
}

/** `POST .../schema-changes/approve` body. For a `table_added` notice, `object` is the table's name and approving only marks it seen. */
export type ApproveSchemaChangeRequest = { object: string }

export type ApproveSchemaChangeResponse = {
  object: string
  approved: SchemaChange[]
  pauseLifted: boolean
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
  /**
   * Whether the connector's credential is one lakehouse stores itself
   * (`source: "managed"`, ADR 0002 Addendum 4) — so the UI can offer
   * "replace credential". A boolean only; the reference is never sent.
   */
  credentialManaged: boolean
  /**
   * How the primary credential is held (`SRC-12` decision 10): `managed` is
   * sealed on lakehouse's credentials volume, `env` is an operator's
   * environment variable (read at start-up, so a change needs a restart),
   * `file` an operator's file. `null` for any other reference scheme. A
   * scheme only; the reference is never sent.
   */
  credentialSource: CredentialSource | null
  /**
   * The stored primary credential's kind (password, access key, …), read
   * from its reference name — so the edit page asks for the right thing.
   * `null` for a hand-seeded reference with no recognizable suffix. A kind
   * only; neither the reference nor the value is ever sent.
   */
  credentialKind: CredentialKind | null
  /** The secondary slot's kind; `null` when the connector has none (e.g. PostgreSQL). */
  credentialSecondaryKind: CredentialKind | null
  /** Residency label as stored — the edit page's current value. */
  residency: string
  /** Owning tenant's id; `null` for an unassigned connector. */
  tenantId: string | null
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
  /**
   * `true` only while a test that runs in the orchestrator has not finished (`SRC-4` T1.1: Oracle). Nothing was
   * dialled yet, so `ok: false` is not a failure; read the end of it with `getDiscovery(id, "test")` and the
   * probe history, as `runConnectionTest` does. Absent otherwise.
   */
  pending?: boolean
  /**
   * For a change-capture connector (`SRC-4` decision S4-f): what the source must have switched on. Absent for every
   * other connector. Never part of `ok`: a connector can be saved whatever these say.
   */
  setupChecks?: SetupCheck[]
}

/**
 * One setting a change-capture source needs, as `POST /api/connectors/{id}/test` reports it (`SRC-4` S4-f). Every
 * word is the API's fixed text, never the server's. `unchecked` means it could not be read (usually a permission):
 * nothing is claimed about it. `command` is the statement or line that sets a `missing` one.
 */
export type SetupCheck = {
  setting: string
  status: "ok" | "missing" | "unchecked" | string
  message: string
  command?: string
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
 * field-for-field, including its `snake_case` wire form. `reference` is a
 * reference to a secret in the installation's secret manager (`SRC-12` part
 * B): only the reference is stored, and the value is read at every use. The
 * client never chooses it at create time; it is set with
 * `PUT /api/connectors/{id}/credential`.
 */
export type CredentialSource = "env" | "file" | "managed" | "reference"

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
  /**
   * The credential VALUES, write-only — only with `source: "managed"` (ADR
   * 0002 Addendum 4). The server writes them to its credential store and
   * never returns them; the create response only says `credentialStored`.
   */
  values?: CredentialValues
}

/** Write-only credential values, one per declared slot. */
export type CredentialValues = {
  primary: string
  secondary?: string
}

/**
 * One slot's new credential in a `PUT /api/connectors/{id}/credential` body:
 * a value to store, or (when the installation has a secret manager) a
 * reference to a secret in it. A reference names a place (`vault:...`,
 * `aws-sm:...`, `gcp-sm:...`, `azure-kv:...`), never a value; the server
 * reads it once and tests the connection before saving it.
 */
export type CredentialSlotValue = { kind: CredentialKind; value: string } | { reference: string }

/**
 * `GET /api/connectors/credential-settings`: what this installation offers
 * for a credential (`SRC-12` part B, decision 2). `secretManager` is the one
 * secret manager the operator set up, or `null`; `typedCredentials` is
 * `false` when the operator turned typing a value off
 * (`CONNECTOR_TYPED_CREDENTIALS=false`).
 */
export type CredentialSettings = {
  secretManager: "vault" | "aws" | "gcp" | "azure" | null
  typedCredentials: boolean
  /**
   * This product's OAuth callback address, to enter in the provider's app
   * settings (`SRC-12` part C). `null` when the operator has not set
   * `CONNECTOR_OAUTH_REDIRECT_URI`: signing in is then refused.
   */
  oauthCallbackUrl: string | null
}

/** `GET /api/connectors/{id}/oauth` (`SRC-12` part C). Holds no token. */
export type OAuthStatus = {
  /** Whether the connector holds a refresh token it can read. */
  signedIn: boolean
  /** The address to enter in the provider's app settings; `null` when the operator has not set it. */
  callbackUrl: string | null
  /** Whether the dial names a revocation address, so signing out can tell the provider. */
  revocationConfigured: boolean
}

/** `POST /api/connectors/{id}/oauth/start`: the provider's sign-in page, to open in a new window. */
export type OAuthStart = {
  authorizeUrl: string
  expiresInSeconds: number
}

/**
 * What signing out did at the provider (`SRC-12` decision 8): `done`; `notConfigured`
 * (no revocation address, the token stays valid there until it expires); `noToken`;
 * `failed` (the provider could not be told).
 */
export type OAuthRevocation = "done" | "notConfigured" | "noToken" | "failed" | "notApplicable"

/** `POST /api/connectors/{id}/oauth/sign-out`. The local token is gone either way. */
export type OAuthSignOut = {
  signedOut: boolean
  revocation: OAuthRevocation
  message: string
}

/**
 * The `PUT /api/connectors/{id}/credential` body (ADR 0002 Addendum 4):
 * user-supplied credential VALUES, one or both slots. Both in one request
 * because an S3 access key and secret key only work as a pair — the server
 * tests them together against the source and refuses (422) values the
 * source rejects. Reference names are derived server-side, never chosen
 * here.
 */
export type SetConnectorCredentialRequest = {
  primary?: CredentialSlotValue
  secondary?: CredentialSlotValue
  /**
   * The connection settings this credential is for, when the same edit
   * changes them (e.g. REST bearer to basic auth): the server tests the new
   * credential with these instead of the saved ones. Saving them is still
   * `setIngestSpec`'s job.
   */
  dial?: Dial
}

/** The `PUT /api/connectors/{id}/credential` response. Never carries a value. */
export type SetConnectorCredentialResponse = {
  saved: boolean
  slots: SecretSlot[]
  /**
   * Whether a real probe dialed the source with the new credential(s)
   * before they were saved. `false` only for a type this build cannot
   * probe — the values are saved, but unverified.
   */
  verified: boolean
  message: string
}

/**
 * The `PATCH /api/connectors/{id}` body: the editable, non-credential
 * fields. Every field optional; an absent one is unchanged. Type is not
 * editable; tenant, credential and connection settings (`dial`) each have
 * their own route.
 */
export type UpdateConnectorInput = {
  name?: string
  direction?: Connector["direction"]
  environment?: string
  residency?: string
  host?: string
  /** What a non-breaking source schema change does (`SRC-8`, decision 1). */
  schemaChangePolicy?: SchemaChangePolicy
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
  /** Display name — the server replaces it with the tenant row's own name
   * whenever `tenantId` (or the caller's active tenant) resolves. */
  tenant: string
  /** Must be one of the caller's tenants; absent = their active tenant. */
  tenantId?: string
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
  /** Whether the server stored the credential value(s) itself — i.e.
   * nothing is left for an operator to provision. */
  credentialStored: boolean
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

/** `CdcDial` = `SqlDial`'s fields plus the change-capture settings of the
 * driver (`ingest_spec.rs`'s `CdcDial`, `SRC-4` decision S3-a): PostgreSQL
 * requires `slotName` and `publicationName` and refuses `serverId`; MySQL
 * requires `serverId` (1 to 4294967295) and refuses the other two; SQL Server
 * refuses all three. A field the driver does not use is omitted, never sent
 * empty, because the server refuses a present one. */
export type CdcDial = {
  driver: SqlDriver
  host: string
  port: number
  database: string
  user: string
  slotName?: string
  publicationName?: string
  serverId?: number | null
  /** How the change-capture worker secures its connection to the source
   * (`SRC-4` S2-d). Absent means `require`: encrypted, certificate taken as
   * presented. `disable` is not encrypted. `verify-ca` and `verify-full` use the
   * worker's own trust store. Widened with `string` as the other contract
   * enums are. */
  sslMode?: "disable" | "require" | "verify-ca" | "verify-full" | string
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
 * (`#[serde(tag = "type")]`) exactly — the same five `type` values
 * `RestAuth::type_tag` names.
 *
 * `oauth2_authorization_code` (`SRC-12` part C) is the sign-in flow with
 * PKCE: the client id and the addresses are configuration; the client secret
 * is the connector's primary credential and the refresh token (written by the
 * sign-in, never typed) its secondary one. `revocationUrl` is optional: when
 * absent, signing out cannot tell the provider.
 */
export type RestAuth =
  | { type: "api_key"; header: string }
  | { type: "bearer" }
  | { type: "oauth2_client_credentials"; tokenUrl: string }
  | {
      type: "oauth2_authorization_code"
      clientId: string
      authorizeUrl: string
      tokenUrl: string
      revocationUrl?: string
      scopes: string[]
    }
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
  /** The cursor column of an `incremental` object: the column whose value
   * only grows, such as an id or an updated-at time. */
  incrementalKey?: string
  /** How a run's rows meet the Bronze table. Absent on an object saved
   * before load modes existed, which the ingest job runs as `replace`. */
  loadMode?: LoadMode
  /** The columns that identify one row, for the deduplicating modes and
   * `history` (`SRC-10`). Kept on the object whatever the mode, so a key the console
   * detected survives a switch of mode; the API ignores it for a mode that
   * does not use it. */
  primaryKey?: string[]
  /**
   * `SRC-11` decisions 9 to 11: the columns left out of the load, stored as what is left out, not what is kept,
   * so a column that appears later follows the schema-change policy. Accepted only for `sql` connectors on
   * the `postgres`, `mysql` or `mssql` driver, and never a key column of a deduplicating or history object nor
   * the cursor column (the API answers 422). Absent or empty: none left out.
   */
  excludedColumns?: string[]
  target: string
}

/**
 * Mirrors Rust `LoadMode` (`ingest_spec.rs`) and `LOAD_MODES` in
 * `dagster/dispar_orchestrate/adapters/sink.py`, which acts on it:
 * `replace` overwrites the table with what the source returns now,
 * `append` adds every row again, `incremental` adds only rows beyond the
 * cursor column's last seen value (SQL connectors only), `replace_dedup`
 * is `replace` keeping one row per `primaryKey`, and `incremental_dedup` is
 * `incremental` merging into the table on `primaryKey` (SQL connectors only),
 * and `history` keeps every version of a row, closing the old one and adding a
 * new one when a run sees it changed. The deduplicating modes and `history` need
 * a `primaryKey`; `history` reads the whole source every run and takes no
 * `incrementalKey` (`SRC-10`, decision 10).
 */
export type LoadMode =
  | "replace"
  | "append"
  | "incremental"
  | "replace_dedup"
  | "incremental_dedup"
  | "history"

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
  | "saas"
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
 * `GET /api/connectors/{id}/ingest-spec`. Mirrors Rust `IngestSpecResponse`
 * (`rust/crates/lakehouse-api/src/routes/connectors.rs`): the stored
 * `IngestSpec` (`rust/crates/lakehouse-store/src/connectors.rs`)
 * field-for-field, including nullability — `adapter`/`ingestMode` are
 * plain `Option<String>` there (not a closed enum), `null` for a connector
 * that has never had an ingest spec set — plus `nextRunAt`.
 */
export type IngestSpec = {
  adapter: IngestAdapter | null
  ingestMode: IngestMode | null
  dial: Dial
  sourceObjects: SourceObject[]
  /** Five-field cron, evaluated in UTC. */
  scheduleCron: string | null
  secretRefs: IngestSecretRefs
  /** When the schedule next launches an ingest (ISO, UTC); `null` with no
   * schedule, and for CDC, which streams on its own. */
  nextRunAt: string | null
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
  /**
   * The connector's credentials, sent again when this save changes where the
   * connector points (host, port, database, endpoint, bucket, base URL,
   * brokers; `SEC-14`). Every slot the new settings read must be here,
   * otherwise the server answers 409 and saves nothing. The server tests the
   * new settings with these credentials first and saves them together with
   * the spec. Write-only: never read back. Omit it for any other save.
   */
  credential?: {
    primary?: CredentialSlotValue
    secondary?: CredentialSlotValue
  }
}

/**
 * `dial` for the `saas` adapter (`SRC-3`): the vendor, the one part of its
 * address a person supplies, and how the connector signs in. Mirrors Rust
 * `SaasDial` (`lakehouse-store/src/ingest_spec.rs`). No address or secret is
 * part of it: the vendor's addresses come from the vendor table.
 */
export type SaasDial = {
  vendor: string
  account: string
  /** Objects the person names where the vendor has no listing of them. */
  objects?: string[]
  auth: { type: "oauth2_authorization_code"; clientId: string } | { type: "integration" }
  /** NetSuite's integration credential only: the id of the certificate uploaded in NetSuite (the assertion's `kid`). */
  certificateId?: string
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
  /** Why a `supported: false` type is unavailable (SRC-6 F5); null when there is no specific reason. */
  unsupportedReason: string | null
}

/**
 * Table discovery (`SRC-13`): what a connector's source holds, listed and previewed
 * before anything is chosen to load. Mirrors the store's `connector_discovery::Answer`
 * (`rust/crates/lakehouse-store/src/connector_discovery.rs`, camelCase) and the
 * routes in `rust/crates/lakehouse-api/src/routes/connector_discovery.rs`.
 */
export type DiscoveryKind = "list" | "preview"

/**
 * What a discovery answer or poll can be about: the two a person starts, and `test` (`SRC-4` T1.1), the connection
 * test of a connector only the orchestrator can dial, which `POST /api/connectors/{id}/test` launches. A test is
 * never started on the discovery route (the API answers 400), only read there.
 */
export type DiscoveryReadKind = DiscoveryKind | "test"

/** `running` only for the sources the orchestrator reads; the console polls until it ends. */
export type DiscoveryState = "running" | "done" | "failed"

/**
 * Why a listing failed, from the closed list the API words (decision D10). The widening
 * keeps a class a newer API adds from breaking the type; the console words an unknown
 * one by the `reason` the API sent.
 */
export type DiscoveryErrorClass =
  | "unreachable"
  | "sign_in_refused"
  | "address_refused"
  | "no_permission"
  | "timeout"
  | "too_many_objects"
  | "not_supported"
  | "failed"
  | string

/** What a listed object is: a schema, a table, a file, a REST endpoint, a Kafka topic, a MongoDB collection, a business-app object, or (a GA4 property's fields, `schema: "fields"`) a dimension or a metric, or (a SharePoint tenant's sites, same request) a site. */
export type DiscoveryObjectKind =
  | "schema"
  | "table"
  | "file"
  | "endpoint"
  | "topic"
  | "collection"
  | "object"
  | "dimension"
  | "metric"
  | "site"
  | string

/**
 * One listed object. `name` is what a source object's `name` is in the ingest spec:
 * `schema.table` for SQL, the key for a file, the path for an endpoint. `size` (bytes)
 * and `modifiedAt` (RFC 3339) are present for files only.
 */
export type DiscoveryObject = {
  name: string
  kind: DiscoveryObjectKind
  size?: number
  modifiedAt?: string
  /**
   * A business-app object only (`SRC-3` decision 6): `incremental`, or
   * `re-read in full: <reason>`. Fixed text from the vendor module's declaration.
   */
  load?: string
  /**
   * A name for a person where `name` is an id (`SRC-3` slice 5 review BLOCKER 14: a SharePoint list is
   * `<site id>/<list id>`, labelled `Site / List`). The ingest spec keys the object by `name`, so a rename
   * at the source changes nothing stored.
   */
  label?: string
}

/** One column of a preview; `inferred` is true when `typeName` was guessed from a sample (D6). */
export type DiscoveryColumn = {
  name: string
  typeName: string
  inferred: boolean
}

/**
 * The answer of every discovery route. A finished list has `objects` (and `truncated`);
 * a finished preview has `columns` (and `primaryKey` for SQL); a failure has `errorClass`
 * and a fixed `reason`, never the source's own text. An empty `columns` is an empty
 * topic, collection or file, not a failure.
 */
export type DiscoveryAnswer = {
  kind: DiscoveryReadKind
  state: DiscoveryState
  /** What a list was narrowed by (the schema or the glob), when it was. */
  scope?: string
  /** The object a preview is about. */
  object?: string
  objects?: DiscoveryObject[]
  truncated?: boolean
  columns?: DiscoveryColumn[]
  primaryKey?: string[]
  /** A finished connection test: the milliseconds the connection took, as the orchestrator measured them. */
  latencyMs?: number
  errorClass?: DiscoveryErrorClass
  reason?: string
  startedAt?: string
  endedAt?: string
}

/**
 * The body of `POST /api/connectors/{id}/discovery/{kind}`; every field optional.
 * `schema` empty on a SQL list asks for the schemas; `object` is required for a
 * preview and refused for a list; `limit` is 1 to 10,000.
 */
export type DiscoveryRequest = {
  schema?: string
  glob?: string
  search?: string
  limit?: number
  object?: string
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
 * What a run of ONE table does (`SRC-10` decision 5), the body of
 * `POST /api/connectors/{id}/ingest/run`: refresh keeping the rows already
 * there, refresh after removing them, or clear the table and load nothing.
 * Mirrors Rust `IngestAction` (`routes/connectors.rs`).
 */
export type IngestTableAction = "refresh_keep" | "refresh_remove" | "clear"

/** The optional body of `POST .../ingest/run`: both fields, or no body at all. */
export type IngestTableRequest = {
  /** The source object's name, as listed in the ingest spec. */
  object: string
  action: IngestTableAction
}

/** Why a run was started (`SRC-11` decision 7). `other`: launched from the orchestrator's own screen. Widened. */
export type RunTrigger = "schedule" | "user" | "refresh" | "clear" | "other" | string

/** Where a run stands in the product's own record; `running` is also what a killed run keeps, see the Runs card. Widened. */
export type RunStatus = "running" | "succeeded" | "failed" | "paused" | string

/** Why a run ended (`SRC-11` decision 5). Widened. */
export type RunEndReason = "completed" | "table_failed" | "connector_paused" | "other" | string

/**
 * One run of a connector, from `GET /api/connectors/{id}/runs` (newest 50 first). Mirrors the API's run record
 * (`routes/connector_runs.rs`, `SRC-11` decision 7). Times are ISO 8601 with milliseconds.
 */
export type ConnectorRun = {
  runId: string
  trigger: RunTrigger
  /**
   * The one table a `refresh` or `clear` run is about, so the run list can say "Clear · public.orders"
   * (`SRC-10` decision 5); `null` for every other run and for a run recorded before it was kept.
   */
  table: string | null
  status: RunStatus
  startedAt: string
  /** `null` until the run reports its end; a killed run may never. */
  endedAt: string | null
  durationSeconds: number | null
  /** Rows of the attempts that succeeded. */
  rowsLoaded: number
  /** All attempts at all tables. */
  attempts: number
  endReason: RunEndReason | null
}

/** The closed list of why an attempt failed (`SRC-11` decision 2); a success carries none. Widened. */
export type AttemptErrorClass =
  | "unreachable"
  | "timeout"
  | "source_busy"
  | "api_unreachable"
  | "sign_in_refused"
  | "address_refused"
  | "unsupported"
  | "setting_rejected"
  | "schema_decision_pending"
  | "other"
  | "permission_missing"
  | "rate_limited"
  | string

/** One attempt at one table of a run. */
export type ConnectorRunAttempt = {
  table: string
  attempt: number
  startedAt: string
  endedAt: string
  rowsLoaded: number
  result: "succeeded" | "failed" | string
  errorClass: AttemptErrorClass | null
  /** The wait before the next attempt, in seconds; only on a failure that is retried. */
  waitSeconds: number | null
}

/** The closed list of lines a run's log holds. Widened. */
export type RunLogEvent =
  | "run_started"
  | "attempt"
  | "retry_wait"
  | "schema_decision_pending"
  | "columns_left_out"
  | "report_gap"
  | "run_ended"
  | string

/** One line of a run's log; `text` is the product's own sentence, never the source's or the driver's. */
export type ConnectorRunLine = {
  at: string
  event: RunLogEvent
  table: string | null
  attempt: number | null
  text: string
}

/** `GET /api/connectors/{id}/runs/{runId}/log`. Attempts and lines older than 30 days are gone (empty arrays). */
export type ConnectorRunLog = {
  run: ConnectorRun
  attempts: ConnectorRunAttempt[]
  lines: ConnectorRunLine[]
}

/** `POST /api/connectors/{id}/resume`. */
export type ResumeConnectorResponse = { resumed: boolean }

/** `POST /api/connectors/{id}/pause` (`SRC-4` decision S2-e). */
export type PauseConnectorResponse = { paused: boolean }

/**
 * `GET /api/connectors/{id}/change-capture`'s response (`SRC-4` decision S2-h). Mirrors Rust
 * `ChangeCaptureResponse` (`rust/crates/lakehouse-api/src/routes/connector_runs.rs`).
 * `supported: false` carries a fixed `reason` (a connector that captures no changes, or a
 * deployment without Kafka Connect). `state` is `running`, `paused`, `failed`, `starting`,
 * `not_created` (the capture service has no such connector yet) or `unreachable` (it could not
 * be asked); never a made-up `running`.
 */
export type ChangeCaptureState = {
  supported: boolean
  reason?: string
  state?: "running" | "paused" | "failed" | "starting" | "not_created" | "unreachable" | string
  /** How far behind the source each table is, the limit and whether its alert is open (`SRC-4` slice 4). */
  lag?: ChangeCaptureLag
}

/**
 * One table's lag, as the API words it (`SRC-4` decision S4-c). `behind` carries `seconds`; `not_known_since` carries
 * `since` (the time of the newest report); the others carry neither. A number is never invented: `not_known_yet`
 * means nothing was reported, `not_known_since` that the orchestrator stopped reporting.
 */
export type CaptureTableLag = {
  table: string
  status: "up_to_date" | "behind" | "not_known_yet" | "not_known_since" | "paused" | string
  seconds?: number
  since?: string
}

/** Mirrors Rust `LagReadout` (`rust/crates/lakehouse-api/src/routes/capture_lag.rs`). */
export type ChangeCaptureLag = {
  tables: CaptureTableLag[]
  /** The table furthest behind; absent when none is behind by a number. */
  worst?: { table: string; seconds: number }
  limitMinutes: number
  alertOpen: boolean
}

/** `PUT /api/connectors/{id}/lag-limit` (`SRC-4` S4-d). */
export type LagLimitResponse = { minutes: number }

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
  /**
   * `POST /api/connectors/{id}/discovery/{kind}` (`SRC-13`): ask for a listing or a preview.
   * SQL sources, object storage and the REST endpoint list answer in this call; the
   * sources only the orchestrator reads answer `running`, and `getDiscovery` is polled.
   */
  startDiscovery(id: string, kind: DiscoveryKind, request: DiscoveryRequest, signal?: AbortSignal): Promise<DiscoveryAnswer>
  /** `GET /api/connectors/{id}/discovery/{kind}`: the latest answer of that kind (404 when none was asked for). */
  getDiscovery(id: string, kind: DiscoveryReadKind, signal?: AbortSignal): Promise<DiscoveryAnswer>
  /** With `table`, a run of that one table with an action (`SRC-10`); without, every table. */
  runIngest(id: string, signal?: AbortSignal, table?: IngestTableRequest): Promise<IngestRunResult>
  /** `GET /api/connectors/{id}/runs` (`SRC-11`): the newest 50 runs from the product's own run record. */
  listConnectorRuns(connectorId: string, signal?: AbortSignal): Promise<ConnectorRun[]>
  /** `GET /api/connectors/{id}/runs/{runId}/log` (`SRC-11`): the attempts and lines of one run. */
  getConnectorRunLog(connectorId: string, runId: string, signal?: AbortSignal): Promise<ConnectorRunLog>
  /** The same log as text (`?format=text`), for the Download log button. */
  downloadConnectorRunLog(connectorId: string, runId: string, signal?: AbortSignal): Promise<string>
  /** `POST /api/connectors/{id}/resume` (`SRC-11` decision 4): lifts a pause set by repeated failures, or by a person (`SRC-4`); no other. */
  resumeConnector(connectorId: string, signal?: AbortSignal): Promise<ResumeConnectorResponse>
  /** `POST /api/connectors/{id}/pause` (`SRC-4`): pauses the connector until a person resumes it. */
  pauseConnector(connectorId: string, signal?: AbortSignal): Promise<PauseConnectorResponse>
  /** `GET /api/connectors/{id}/change-capture` (`SRC-4`): the connector's state in the capture service. */
  getChangeCapture(connectorId: string, signal?: AbortSignal): Promise<ChangeCaptureState>
  /** `PUT /api/connectors/{id}/lag-limit` (`SRC-4`): the lag, in minutes (1 to 1440), at which an alert is sent. */
  setLagLimit(connectorId: string, minutes: number, signal?: AbortSignal): Promise<LagLimitResponse>
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
  /** `GET /api/connectors/credential-settings` — see `CredentialSettings`. */
  getCredentialSettings(signal?: AbortSignal): Promise<CredentialSettings>
  /** `GET /api/connectors/{id}/oauth` (`SRC-12` part C) — see `OAuthStatus`. 409 for a connector that does not sign in with OAuth authorization code. */
  getOAuthStatus(id: string, signal?: AbortSignal): Promise<OAuthStatus>
  /** `POST /api/connectors/{id}/oauth/start` — see `OAuthStart`. The state and verifier stay on the server. */
  startOAuth(id: string, signal?: AbortSignal): Promise<OAuthStart>
  /** `POST /api/connectors/{id}/oauth/sign-out` — revoke at the provider when it can be told, and delete the stored token. */
  signOutOAuth(id: string, signal?: AbortSignal): Promise<OAuthSignOut>
  /**
   * `PUT /api/connectors/{id}/credential` — save a user-supplied credential
   * as the connector's lakehouse-managed credential, probe-first (ADR 0002
   * Addendum 4). See `SetConnectorCredentialRequest`.
   */
  setCredential(
    id: string,
    body: SetConnectorCredentialRequest,
    signal?: AbortSignal
  ): Promise<SetConnectorCredentialResponse>
  /** `PATCH /api/connectors/{id}` — see `UpdateConnectorInput`. */
  updateConnector(id: string, input: UpdateConnectorInput, signal?: AbortSignal): Promise<Connector>
  /** `GET /api/connectors/{id}/schema-changes` (`SRC-8`): what waits, the recent changes, the inactive columns. */
  listSchemaChanges(id: string, signal?: AbortSignal): Promise<SchemaChangeList>
  /**
   * `POST /api/connectors/{id}/schema-changes/approve` (`SRC-8`): approve
   * what waits for one table. 409 with a plain message when what waits
   * includes a type change the column cannot hold (decision 10).
   */
  approveSchemaChanges(
    id: string,
    body: ApproveSchemaChangeRequest,
    signal?: AbortSignal
  ): Promise<ApproveSchemaChangeResponse>
  /**
   * `DELETE /api/connectors/{id}` — removes the connector and its managed
   * credential files. Refused (409) while a pipeline still reads from it,
   * `force` or not. A CDC connector's replication slot/publication is
   * dropped first; when that fails the server answers 409 and keeps the
   * connector, unless `force` is set (the slot must then be cleaned up by
   * hand on the source database).
   */
  deleteConnector(id: string, options?: { force?: boolean }, signal?: AbortSignal): Promise<void>
  /**
   * `PUT /api/connectors/{id}/tenant` — move a connector to another tenant.
   * A governance action: needs `identity:write`, which a connector manager
   * may not hold (the server answers 403).
   */
  assignTenant(id: string, tenantId: string, signal?: AbortSignal): Promise<void>
}
