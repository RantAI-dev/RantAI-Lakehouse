//! Repository layer for connector definitions: source/sink systems the
//! lakehouse pulls from or pushes to. Postgres backing for
//! `src/services/mock/connectors.ts`.
//!
//! # Credential handling — read this before adding a field
//!
//! This module never stores, accepts, returns, logs, or `Debug`-prints a
//! credential *value*. `CreateConnectorInput` (mirroring
//! `contracts/connectors.ts`) carries a `secret_ref: String` — a reference
//! to WHERE a credential lives (an env var name, a secret-manager path),
//! never the credential itself. See `0013_connectors.sql`'s header comment
//! for the full decision record and the product consequence.
//!
//! Three separate guarantees hold this up, each independently:
//!
//! 1. **[`Connector`] and [`ConnectorDetail`] have no `host`/`secret_ref`
//!    field at all.** They cannot serialize what they do not contain —
//!    this is a compile-time guarantee, not a runtime redaction step that
//!    a future edit could accidentally remove. [`Connector`]'s shape is
//!    exactly `contracts/connectors.ts`'s `Connector` type, which likewise
//!    has no such field.
//! 2. [`ConnectorRow`] and [`ConnectorDialInfo`] (the only two places
//!    `host`/`secret_ref` are held in-process) never print either value:
//!    `ConnectorRow` has a hand-written [`std::fmt::Debug`] that redacts
//!    both — same pattern `lakehouse-api::config::Config` already uses for
//!    `ch_password`/`llm_key`/`database_url` — and `ConnectorDialInfo` goes
//!    further, having no `Debug` impl at all (see its doc comment).
//! 3. [`create_connector`] rejects a `secret_ref` that is *shaped* like a
//!    raw secret (long hex/base64 blob, JWT, PEM block, `user:pass@host`)
//!    via [`looks_like_raw_secret`] — defense in depth against a caller
//!    who misunderstands the field and pastes an actual credential into
//!    it. Nothing downstream could exploit a stored secret today (there is
//!    no way to read `secret_ref` back out through any GET response), but
//!    rejecting it at the write is strictly better than accepting garbage
//!    that violates this module's whole reason for existing.
//!
//! As of P6, [`get_connector_dial_info`] hands `host`/`secret_ref` to
//! exactly one caller — `lakehouse-api`'s `connector_probe` module — so a
//! real connectivity test can resolve the referenced credential via
//! [`lakehouse_core::secret::SecretResolver`] (ADR 0002) and attempt a
//! bounded, timed-out dial. That module still never logs or serializes the
//! resolved value; it only ever reports whether the dial succeeded, how
//! long it took, and a human-readable message.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

fn iso_millis(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        at.year(),
        u8::from(at.month()),
        at.day(),
        at.hour(),
        at.minute(),
        at.second(),
        at.millisecond()
    )
}

fn iso_opt(at: Option<OffsetDateTime>) -> Option<String> {
    at.map(iso_millis)
}

/// A connector, as returned by every read endpoint. Mirrors `Connector` in
/// `contracts/connectors.ts` — deliberately has no `host` or `secret_ref`
/// field. See the module doc comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connector {
    /// `connector.id`.
    pub id: String,
    /// Display name; the table's natural key.
    pub name: String,
    /// Connector type label (e.g. `"PostgreSQL CDC"`, `"Kafka"`).
    #[serde(rename = "type")]
    pub kind: String,
    /// `"source" | "sink" | "bidirectional"`.
    pub direction: String,
    /// `"healthy" | "degraded" | "unhealthy" | "unknown"`.
    pub health: String,
    /// Deployment environment (e.g. `"production"`, `"staging"`).
    pub environment: String,
    /// Owning tenant's display name.
    pub tenant: String,
    /// Last connection-test time, ISO 8601. `None` until a real, supported
    /// probe has run via [`record_test_result`] -- WS1 finding J19: a
    /// freshly created connector must never claim a test that never
    /// happened.
    pub last_test_at: Option<String>,
    /// Last observed activity time, ISO 8601. Always `None` on read today:
    /// no writer anywhere sets `connector.last_activity_at` yet -- WS3's
    /// planned ingest-run tracking is the intended source.
    pub last_activity_at: Option<String>,
    /// Feature/capability labels this connector supports.
    pub capabilities: Vec<String>,
    /// Owning team or person.
    pub owner: String,
}

/// A dependent pipeline, derived (never stored) from `pipeline_definition`
/// rows whose `connector_id` names this connector. Mirrors
/// `ConnectorDependent`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDependent {
    /// Dependent's id.
    pub id: String,
    /// Dependent's display name.
    pub name: String,
    /// Always `"pipeline"` today: nothing in this schema records a
    /// streaming-job dependency yet (the `streaming` domain has no
    /// Postgres backing — see Task 2.10's report). Kept as a `String`
    /// rather than a hardcoded literal so a future streaming-job dependent
    /// can be added without a shape change here.
    pub kind: String,
}

/// [`ConnectorDetail`] enrichment this repository does not fabricate.
/// `discoveredAssets`/`discoveredSchemas`/`recentErrors` all require a
/// schema-discovery engine or an error-log store that does not exist
/// anywhere in this repository (see `AI_PROJECT_INSIGHTS.md`: "There is no
/// real ... streaming engine ... in this repository"), so they are always
/// empty/zero here rather than invented — the same "report honestly, don't
/// fabricate" call the task brief makes for the `streaming` domain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDetail {
    /// The base connector fields, flattened onto this struct's JSON.
    #[serde(flatten)]
    pub connector: Connector,
    /// Always `0` today — see the struct doc comment.
    pub discovered_assets: i64,
    /// Always empty today — see the struct doc comment.
    pub discovered_schemas: Vec<DiscoveredSchema>,
    /// Always empty today — see the struct doc comment.
    pub recent_errors: Vec<RecentError>,
    /// Pipelines whose `connector_id` names this connector.
    pub dependent_pipelines: Vec<ConnectorDependent>,
    /// The newest real `audit_event` row recorded against this connector
    /// (`resource_kind = 'connector'`, `resource_id = id`), resolved on
    /// read by [`get_connector`] — never stored. `None` until WS5 starts
    /// recording connector audit events; see `get_connector`'s doc comment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
}

/// Mirrors `DiscoveredSchema`. Never populated today — see
/// [`ConnectorDetail`]'s doc comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredSchema {
    /// Schema/topic/prefix name.
    pub name: String,
    /// `"table" | "topic" | "prefix"`.
    pub kind: String,
    /// Column or field count.
    pub columns_or_fields: i64,
}

/// Mirrors the anonymous `{ at, message }` shape of `recentErrors`. Never
/// populated today — see [`ConnectorDetail`]'s doc comment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentError {
    /// When the error occurred, ISO 8601.
    pub at: String,
    /// Human-readable error message.
    pub message: String,
}

/// The row [`list_connectors`]/[`get_connector`]/[`create_connector`]
/// select. Holds `host`/`secret_ref` — hence the hand-written
/// [`std::fmt::Debug`] below that redacts both. See the module doc
/// comment, guarantee 2. [`get_connector_dial_info`] deliberately does NOT
/// go through this type — it runs its own narrower query into
/// [`ConnectorDialInfo`], which has no `Debug` impl at all.
#[derive(FromRow)]
struct ConnectorRow {
    id: String,
    name: String,
    #[sqlx(rename = "type")]
    kind: String,
    direction: String,
    health: String,
    environment: String,
    tenant: String,
    #[allow(
        dead_code,
        reason = "selected so it round-trips through UPDATE...RETURNING, but never read: no \
                  code in this crate resolves a connector to a live connection, and the \
                  Debug impl below deliberately does not print it either — see the module \
                  doc comment"
    )]
    host: String,
    #[allow(
        dead_code,
        reason = "selected so it round-trips through UPDATE...RETURNING, but never read: it \
                  is a reference name a future connectivity-resolving component would read, \
                  not a value this crate consumes or the Debug impl prints"
    )]
    secret_ref: String,
    last_test_at: Option<OffsetDateTime>,
    capabilities: Vec<String>,
    owner: String,
}

const REDACTED: &str = "<redacted>";

impl std::fmt::Debug for ConnectorRow {
    /// Redacts `host` and `secret_ref` unconditionally — see the module
    /// doc comment, guarantee 2. Every other field is either a display
    /// label or already public via [`Connector`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectorRow")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("direction", &self.direction)
            .field("health", &self.health)
            .field("environment", &self.environment)
            .field("tenant", &self.tenant)
            .field("host", &REDACTED)
            .field("secret_ref", &REDACTED)
            .field("last_test_at", &self.last_test_at)
            .field("capabilities", &self.capabilities)
            .field("owner", &self.owner)
            .finish()
    }
}

impl From<ConnectorRow> for Connector {
    fn from(row: ConnectorRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            kind: row.kind,
            direction: row.direction,
            health: row.health,
            environment: row.environment,
            tenant: row.tenant,
            last_test_at: iso_opt(row.last_test_at),
            // No writer sets this column yet -- see the field's doc comment
            // on `Connector` and the module doc comment.
            last_activity_at: None,
            capabilities: row.capabilities,
            owner: row.owner,
        }
    }
}

// `last_activity_at` is deliberately NOT selected here: nothing writes it
// (see `Connector::last_activity_at`'s doc comment), so every read maps it
// to `None` rather than selecting a column this crate never populates.
const CONNECTOR_COLUMNS: &str = "id, name, type, direction, health, environment, tenant, host, \
     secret_ref, last_test_at, capabilities, owner";

/// List every connector, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_connectors(pool: &PgPool) -> Result<Vec<Connector>, StoreError> {
    let sql = format!("SELECT {CONNECTOR_COLUMNS} FROM connector ORDER BY created_at DESC");
    let rows: Vec<ConnectorRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(Connector::from).collect())
}

/// Fetch one connector's detail: base fields plus dependent pipelines
/// derived from `pipeline_definition.connector_id`.
///
/// # `auditEventId` resolution (WS1 task 1.14, judge finding J12)
///
/// This used to compute `aud-conn-<id>` on every read — a label-shaped
/// string that named no `audit_event` row, so "View audit" always 404ed.
/// It now looks up the newest real `audit_event` row with
/// `resource_kind = 'connector'` and `resource_id = id` and returns `None`
/// when no such event exists. The composite index
/// `audit_event_resource_idx (resource_kind, resource_id)`
/// (`0024_audit_event.sql`) covers this lookup. Nothing inserts a
/// `connector` audit event yet (WS5), so this is `None` today for every
/// connector — that is honest, not a bug.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a query fails.
pub async fn get_connector(pool: &PgPool, id: &str) -> Result<Option<ConnectorDetail>, StoreError> {
    let sql = format!("SELECT {CONNECTOR_COLUMNS} FROM connector WHERE id = $1");
    let row: Option<ConnectorRow> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let connector = Connector::from(row);

    let dependents: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, name FROM pipeline_definition WHERE connector_id = $1 ORDER BY name",
    )
    .bind(id)
    .fetch_all(pool)
    .await?;
    let dependent_pipelines = dependents
        .into_iter()
        .map(|(id, name)| ConnectorDependent {
            id,
            name,
            kind: "pipeline".to_owned(),
        })
        .collect();

    let audit_event_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM audit_event WHERE resource_kind = 'connector' AND resource_id = $1 \
         ORDER BY at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(Some(ConnectorDetail {
        audit_event_id,
        connector,
        discovered_assets: 0,
        discovered_schemas: Vec::new(),
        recent_errors: Vec::new(),
        dependent_pipelines,
    }))
}

/// Everything [`create_connector`] needs. Mirrors `CreateConnectorInput`.
///
/// `secret_ref` is a REFERENCE NAME, never a credential value — see the
/// module doc comment.
#[derive(Debug, Clone)]
pub struct CreateConnectorInput {
    /// Display name; must not collide with an existing connector.
    pub name: String,
    /// Connector type label.
    pub kind: String,
    /// `"source" | "sink" | "bidirectional"`.
    pub direction: String,
    /// Connection target (hostname/endpoint label). Never returned by any
    /// GET response — see the module doc comment.
    pub host: String,
    /// A REFERENCE NAME to where a credential lives, never the credential
    /// itself. See the module doc comment.
    pub secret_ref: String,
    /// An optional second REFERENCE NAME (e.g. the secret-access-key half
    /// of an S3 connector's access-key/secret-key pair — see
    /// [`ConnectorDialInfo::secret_ref_secondary`]). `None` for connector
    /// types that need only one credential (e.g. `PostgreSQL`). Without
    /// this, an S3 connector created through the API could never be
    /// tested: [`get_connector_dial_info`] always reads this column, and
    /// `lakehouse-api::connector_probe::probe_s3` requires it.
    pub secret_ref_secondary: Option<String>,
    /// Deployment environment.
    pub environment: String,
    /// Owning tenant's display name.
    pub tenant: String,
    #[allow(
        dead_code,
        reason = "accepted for contract compatibility; no residency column is read back today"
    )]
    /// Residency policy label, accepted for contract compatibility.
    pub residency: String,
    /// Feature/capability labels this connector supports.
    pub capabilities: Vec<String>,
    /// Owner; defaults to [`DEFAULT_OWNER`] when absent.
    pub owner: Option<String>,
}

const DEFAULT_OWNER: &str = "Current user";

/// A caller-supplied `secret_ref` is a REFERENCE NAME (`"env:FOO"`,
/// `"vault:secret/data/x"`), never a credential value. This heuristically
/// rejects the shapes an actual secret tends to take, as defense in depth
/// against a caller who misunderstands the field — see the module doc
/// comment, guarantee 3. Deliberately loose (a false positive just means a
/// legitimate reference name has to be reworded) rather than an attempt at
/// exhaustive secret detection; `tests/parity/check-no-secrets.sh` is the
/// system's actual defense-in-depth layer for corpus data, this is the
/// analogous check for what a client can persist through this API.
#[must_use]
pub fn looks_like_raw_secret(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.contains("://") && trimmed.contains('@') {
        // e.g. `postgres://user:pass@host:5432/db`.
        return true;
    }
    if trimmed.contains("BEGIN") && trimmed.contains("PRIVATE KEY") {
        return true;
    }
    // A signed JWT: three dot-separated base64url segments.
    let dot_parts: Vec<&str> = trimmed.split('.').collect();
    if dot_parts.len() == 3
        && dot_parts.iter().all(|p| {
            !p.is_empty()
                && p.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
    {
        return true;
    }
    // A long unbroken run of hex or base64-alphabet characters with no
    // `:`/`/` structure at all looks like a raw key/token rather than a
    // reference name (every reference name in this codebase's convention
    // is `scheme:path`, which contains `:`).
    let alnum_run = trimmed.chars().all(|c| {
        c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
    });
    if alnum_run && trimmed.len() >= 32 && !trimmed.contains(':') {
        return true;
    }
    false
}

/// Create a connector. `health` always starts `"unknown"` and `lastTestAt`/
/// `lastActivityAt` start `null` — WS1 finding J19: no probe has run yet, so
/// nothing about this connector's health or test history has been measured.
/// `record_test_result` is the only function that ever moves `health` off
/// `"unknown"` or sets `lastTestAt`.
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken. Returns
/// `Err` wrapping a validation failure (via [`StoreError::Database`]'s
/// sibling — see `routes::connectors::create` for how this is actually
/// surfaced as a 400) is NOT done here: shape validation belongs to the API
/// layer, which calls [`looks_like_raw_secret`] itself before invoking
/// this function, matching the `identity`/`pipelines` modules' split
/// (repository does persistence, route does request validation).
pub async fn create_connector(
    pool: &PgPool,
    input: &CreateConnectorInput,
) -> Result<Connector, StoreError> {
    let id = slug_id(&input.name);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    // `last_test_at`/`last_activity_at` are omitted: neither column has a
    // default any more (`0028_connector_health_unknown_until_tested.sql`),
    // so both come back `NULL` -- no test has run and nothing measures
    // activity, so nothing is claimed.
    let sql = format!(
        "INSERT INTO connector (id, name, type, direction, health, environment, tenant, host, \
         secret_ref, secret_ref_secondary, residency, capabilities, owner) \
         VALUES ($1, $2, $3, $4, 'unknown', $5, $6, $7, $8, $9, $10, $11, $12) \
         RETURNING {CONNECTOR_COLUMNS}"
    );
    let row: ConnectorRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&input.name)
        .bind(&input.kind)
        .bind(&input.direction)
        .bind(&input.environment)
        .bind(&input.tenant)
        .bind(&input.host)
        .bind(&input.secret_ref)
        .bind(&input.secret_ref_secondary)
        .bind(&input.residency)
        .bind(&input.capabilities)
        .bind(owner)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// The outcome of a connectivity test. Mirrors `ConnectorTestResult`.
///
/// As of P6, this is only ever built from a REAL probe result
/// (`lakehouse-api`'s `connector_probe` module) via
/// [`record_test_result`] — never fabricated here. `latency_ms` is `None`
/// exactly when `supported` is `false`: an untested connector type must
/// never report a latency it did not measure.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorTestResult {
    /// Whether a real connectivity probe succeeded. Always `false` when
    /// `supported` is `false` — an unsupported type is never reported as a
    /// success.
    pub ok: bool,
    /// Whether this build knows how to dial this connector's type at all.
    /// `false` for every type besides `PostgreSQL` and S3-compatible object
    /// storage today — see `connector_probe`'s module doc comment for the
    /// full list and why.
    pub supported: bool,
    /// Real measured latency in milliseconds, or `None` when `supported`
    /// is `false` (no attempt was made, so no latency exists to report).
    pub latency_ms: Option<i64>,
    /// Human-readable result message. For an unsupported type, states
    /// plainly that this build cannot test it — never a fabricated
    /// success/failure message.
    pub message: String,
    /// When the test ran, ISO 8601, or `None` when `supported` is `false`:
    /// an unsupported type was never actually dialed, so no test time was
    /// stamped — see [`record_test_result`].
    pub tested_at: Option<String>,
}

/// Fetch the connectivity-relevant fields (`type`, `host`, `secret_ref`,
/// `secret_ref_secondary`) needed to attempt a real dial. Returns
/// `Ok(None)` if `id` does not name a connector.
///
/// Deliberately returns a dedicated [`ConnectorDialInfo`] rather than
/// [`Connector`] or [`ConnectorRow`] — see that type's doc comment for why
/// it has no `Debug` impl at all.
///
/// The raw row shape [`get_connector_dial_info`]'s query returns, before
/// it's reshaped into [`ConnectorDialInfo`] — named purely to satisfy
/// `clippy::type_complexity`, not used anywhere else.
type ConnectorDialInfoRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    serde_json::Value,
);

/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_connector_dial_info(
    pool: &PgPool,
    id: &str,
) -> Result<Option<ConnectorDialInfo>, StoreError> {
    let row: Option<ConnectorDialInfoRow> = sqlx::query_as(
        "SELECT type, host, secret_ref, secret_ref_secondary, adapter, dial FROM connector \
             WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(kind, host, secret_ref, secret_ref_secondary, adapter, dial)| ConnectorDialInfo {
            kind,
            host,
            secret_ref,
            secret_ref_secondary,
            adapter,
            dial,
        },
    ))
}

/// Everything a real connectivity probe needs. Returned only to the one
/// caller that is about to attempt a dial (`lakehouse-api`'s
/// `connector_probe` module, via [`get_connector_dial_info`]) — never
/// logged, never serialized, never returned from any HTTP response.
///
/// Deliberately has NO [`std::fmt::Debug`] impl at all — not even a
/// hand-written redacting one like [`ConnectorRow`]'s. A caller that tries
/// `{:?}` on this fails to compile instead of needing to remember a
/// runtime redaction, the same compile-time-guarantee pattern
/// `lakehouse_core::secret::SecretValue` uses for a resolved credential
/// (see the module doc comment, guarantee 2, and ADR 0002).
#[derive(Clone)]
pub struct ConnectorDialInfo {
    /// Connector type label (e.g. `"PostgreSQL"`, `"Object storage"`),
    /// used to decide whether/how to dial.
    pub kind: String,
    /// Connection target. Never a credential by itself, but still handled
    /// with the same care as `secret_ref` here — see the module doc
    /// comment.
    pub host: String,
    /// Reference to the primary credential (e.g. a password, or an S3
    /// access key id secretRef).
    pub secret_ref: String,
    /// Reference to a secondary credential, for connector types that need
    /// two (e.g. S3 access key id + secret access key). `None` for types
    /// that only ever need one.
    pub secret_ref_secondary: Option<String>,
    /// One of `sql | cdc | files | rest | sheets` (`connector_adapter_check`,
    /// `0033_connector_ingest_spec.sql`), or `None` for a connector row
    /// created before that migration added the column ("not ingestible
    /// yet"). This is what `lakehouse-api`'s connector-deletion deprovision
    /// step dispatches on (WS3 plan review X4): only `adapter = "cdc"`, or
    /// the bounded `adapter IS NULL` legacy-Postgres case, ever attempts to
    /// drop a replication slot/publication — a `sql`/`files`/`rest`/
    /// `sheets` connector never had one, regardless of what its `kind`
    /// string says.
    pub adapter: Option<String>,
    /// The connector's raw ingest `dial` (`{}` until `set_ingest_spec` has
    /// ever been called — `0033`'s column default). Handed back unparsed
    /// rather than as a `crate::ingest_spec::Dial` because the one caller
    /// that needs it (the deprovision step, for `adapter = "cdc"`) is the
    /// only place that knows which shape to parse it against.
    pub dial: serde_json::Value,
}

/// Caller-supplied fields for [`set_ingest_spec`]. Mirrors `IngestSpecInput`
/// in `contracts/connectors.ts`.
///
/// `dial` is opaque `serde_json::Value` here: [`set_ingest_spec`] is what
/// actually validates its shape, via [`crate::ingest_spec::Dial::parse`],
/// dispatched on `adapter` — see that function's doc comment for why the
/// parse happens BEFORE any write.
#[derive(Debug, Clone)]
pub struct IngestSpecInput {
    /// One of `sql | cdc | files | rest | sheets` — the value
    /// [`crate::ingest_spec::Dial::parse`] dispatches on.
    pub adapter: String,
    /// `"batch" | "cdc"`, matching `connector_ingest_mode_check`
    /// (`0033_connector_ingest_spec.sql`).
    pub ingest_mode: String,
    /// Validated by [`crate::ingest_spec::Dial::parse`] against the shape
    /// `adapter` names. Never free-form at the application level, even
    /// though the `dial` column itself is plain `JSONB`.
    pub dial: serde_json::Value,
    /// The objects (tables/endpoints/sheet ranges) this ingest job
    /// targets. Not individually validated by this function — see
    /// `crate::ingest_spec::SourceObject::validate` for the per-object
    /// check, applied by the caller before this is invoked.
    pub source_objects: serde_json::Value,
    /// An optional cron schedule for a `batch`-mode ingest job.
    pub schedule_cron: Option<String>,
}

/// Names ONLY (`"env:FOO"`, `"vault:secret/data/x"`) of the credentials a
/// connector's ingest job resolves at run time — never a resolved value.
/// Mirrors `IngestSecretRefs` in `contracts/connectors.ts`. Reuses the same
/// `connector.secret_ref`/`secret_ref_secondary` columns
/// [`ConnectorDialInfo`] already names; `set_ingest_spec` never writes
/// either column, since a credential reference is set at connector-create
/// time (`create_connector`), not at ingest-spec-configuration time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSecretRefs {
    /// The primary credential's reference name.
    pub primary: String,
    /// The secondary credential's reference name, or `None` for a
    /// connector type that only ever needs one.
    pub secondary: Option<String>,
}

/// A connector's ingest configuration, as returned by [`get_ingest_spec`].
/// Mirrors `IngestSpec` in `contracts/connectors.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestSpec {
    /// `None` for a connector that has never had an ingest spec set — see
    /// `0033_connector_ingest_spec.sql`'s "additive" header comment: every
    /// pre-WS3 connector row parses with `adapter = NULL` until this is
    /// called for it.
    pub adapter: Option<String>,
    /// `None` under the same condition as `adapter`.
    pub ingest_mode: Option<String>,
    /// `{}` until [`set_ingest_spec`] is ever called — the column default.
    pub dial: serde_json::Value,
    /// `[]` until [`set_ingest_spec`] is ever called — the column default.
    pub source_objects: serde_json::Value,
    /// `None` for a `cdc`-mode ingest job, or one that has never been
    /// scheduled.
    pub schedule_cron: Option<String>,
    /// Credential reference NAMES only — see [`IngestSecretRefs`]'s doc
    /// comment.
    pub secret_refs: IngestSecretRefs,
}

/// The raw tuple shape [`get_ingest_spec`]/[`set_ingest_spec`] both decode
/// from a `SELECT`/`RETURNING` naming the same seven columns, in the same
/// order: `adapter, ingest_mode, dial, source_objects, schedule_cron,
/// secret_ref, secret_ref_secondary`. A module-level alias rather than a
/// local one in each function -- `clippy::items_after_statements` forbids
/// a local `type` declared after `Dial::parse`'s validation statement in
/// [`set_ingest_spec`].
type IngestSpecRow = (
    Option<String>,
    Option<String>,
    serde_json::Value,
    serde_json::Value,
    Option<String>,
    String,
    Option<String>,
);

/// Build an [`IngestSpec`] from an [`IngestSpecRow`], shared by
/// [`get_ingest_spec`] and [`set_ingest_spec`].
fn ingest_spec_from_row(row: IngestSpecRow) -> IngestSpec {
    let (
        adapter,
        ingest_mode,
        dial,
        source_objects,
        schedule_cron,
        secret_ref,
        secret_ref_secondary,
    ) = row;
    IngestSpec {
        adapter,
        ingest_mode,
        dial,
        source_objects,
        schedule_cron,
        secret_refs: IngestSecretRefs {
            primary: secret_ref,
            secondary: secret_ref_secondary,
        },
    }
}

/// Read a connector's ingest configuration. Returns `Ok(None)` if `id` does
/// not name a connector.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_ingest_spec(pool: &PgPool, id: &str) -> Result<Option<IngestSpec>, StoreError> {
    let row: Option<IngestSpecRow> = sqlx::query_as(
        "SELECT adapter, ingest_mode, dial, source_objects, schedule_cron, secret_ref, \
         secret_ref_secondary FROM connector WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(ingest_spec_from_row))
}

/// Set a connector's ingest configuration.
///
/// [`crate::ingest_spec::Dial::parse`] runs FIRST, against `spec.dial`
/// dispatched on `spec.adapter` — an invalid `dial` (an unknown field, a
/// missing required field, an unsafe hostname) fails here as
/// [`StoreError::Validation`] and never reaches the `UPDATE` below, so a
/// half-written or malformed `dial` is never persisted.
///
/// Second (WS3 plan review Z6): once the `dial` shape itself is valid,
/// this checks that the connector's declared secret refs are ENOUGH for
/// what `spec.adapter` (and, for `rest`, `dial.auth.type`) needs —
/// [`crate::ingest_spec::secret_field_names`] names the ordered fields
/// (mirrored by `dagster/dispar_orchestrate/secret_map.py`'s
/// `SECRET_FIELD_NAMES`, same commit), and a two-field combination
/// (e.g. `rest`/`basic` needs `username`+`password`) REQUIRES
/// `secret_ref_secondary` to already be set on the connector row. A
/// mismatch is rejected here, at SAVE time, rather than surfacing as a
/// `KeyError` deep inside `secret_resolver.resolve_secrets`
/// (`dagster/dispar_orchestrate/secret_resolver.py`) when a Dagster
/// ingest job actually runs.
///
/// # Errors
///
/// Returns [`StoreError::Validation`] if `spec.dial` fails
/// [`crate::ingest_spec::Dial::parse`] for `spec.adapter`, or if the
/// connector's secret-ref count does not match
/// [`crate::ingest_spec::secret_field_names`] for `spec.adapter`/the
/// dial's auth type. Returns [`StoreError::NotFound`] if `id` does not
/// name a connector. Returns [`StoreError::Database`] on any other
/// failure.
pub async fn set_ingest_spec(
    pool: &PgPool,
    id: &str,
    spec: &IngestSpecInput,
) -> Result<IngestSpec, StoreError> {
    let dial = crate::ingest_spec::Dial::parse(&spec.adapter, &spec.dial)
        .map_err(|err| StoreError::Validation(err.to_string()))?;

    let auth_type = dial.rest_auth_type();
    let fields =
        crate::ingest_spec::secret_field_names(&spec.adapter, auth_type).ok_or_else(|| {
            StoreError::Validation(format!(
                "no secret field mapping for adapter {:?} auth_type {auth_type:?}",
                spec.adapter
            ))
        })?;

    // Read the connector's OWN declared secret-ref count before writing —
    // never derived from `id` (the exact bug Z6 replaces on the Dagster
    // side), only from the row this connector already carries.
    let secret_ref_secondary = sqlx::query_scalar::<_, Option<String>>(
        "SELECT secret_ref_secondary FROM connector WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;

    if fields.len() == 2 && secret_ref_secondary.is_none() {
        return Err(StoreError::Validation(format!(
            "adapter {:?} auth_type {auth_type:?} needs {} secret refs (fields: {:?}), but this \
             connector has only secretRef set, no secretRefSecondary",
            spec.adapter,
            fields.len(),
            fields
        )));
    }

    let row: Option<IngestSpecRow> = sqlx::query_as(
        "UPDATE connector SET adapter = $2, ingest_mode = $3, dial = $4, source_objects = $5, \
         schedule_cron = $6 WHERE id = $1 RETURNING adapter, ingest_mode, dial, source_objects, \
         schedule_cron, secret_ref, secret_ref_secondary",
    )
    .bind(id)
    .bind(&spec.adapter)
    .bind(&spec.ingest_mode)
    .bind(&spec.dial)
    .bind(&spec.source_objects)
    .bind(&spec.schedule_cron)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(ingest_spec_from_row(row))
}

/// Persist the outcome of a real connectivity probe and, when the probe
/// type is supported, stamp `lastTestAt`. Called by `lakehouse-api`'s
/// `connector_probe` module AFTER it has actually attempted (or declined to
/// attempt, for an unsupported type) a dial — this function never decides
/// `ok`/`supported` itself, only records what the caller measured.
///
/// `health` is updated to `"healthy"`/`"unhealthy"` only when `supported`
/// is `true` — an unsupported type's last-known health is left untouched,
/// since declining to test a connector is not evidence about whether it is
/// healthy. `last_test_at` follows the same rule and for the same reason:
/// an unsupported probe type was never actually dialed, so it must not
/// claim a test time it does not have (WS1 finding J19).
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` does not name a connector, or
/// [`StoreError::Database`] on any other failure.
pub async fn record_test_result(
    pool: &PgPool,
    id: &str,
    ok: bool,
    supported: bool,
    latency_ms: Option<i64>,
    message: &str,
) -> Result<ConnectorTestResult, StoreError> {
    let sql = "UPDATE connector SET last_test_at = CASE WHEN $2 THEN now() ELSE last_test_at END, \
               health = CASE WHEN $2 THEN (CASE WHEN $3 THEN 'healthy' ELSE 'unhealthy' END) ELSE \
               health END WHERE id = $1 RETURNING last_test_at";
    let row: Option<(Option<OffsetDateTime>,)> = sqlx::query_as(sql)
        .bind(id)
        .bind(supported)
        .bind(ok)
        .fetch_optional(pool)
        .await?;
    let Some((tested_at,)) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(ConnectorTestResult {
        ok,
        supported,
        latency_ms,
        message: message.to_owned(),
        tested_at: iso_opt(tested_at),
    })
}

/// Delete a connector by id. Returns `Ok(false)` (not an error) if `id`
/// does not name a connector — matching the idempotent-delete convention
/// most of this codebase's `DELETE` handlers already use.
///
/// # What this does NOT do
///
/// This function itself never connects to the connector's `host` and never
/// drops any replication slot/publication a `PostgreSQL` CDC connector may
/// have created there — this repository layer only ever touches the
/// registry row, same as every other function in this module. That is no
/// longer a gap in practice: `lakehouse-api`'s
/// `routes::connectors::delete` (the only caller) resolves the connector's
/// `secret_ref`, dials its `host`, and drops the slot/publication via
/// `connector_deprovision::drop_slot_and_publication` BEFORE calling this
/// function for a `PostgreSQL`-kind connector — see that handler's doc
/// comment for the full 409/`force` contract. If that deprovisioning
/// fails, the caller does not call this function at all (unless
/// `?force=true`), which is what keeps the registry row around as the
/// visible record of an orphaned slot instead of this function silently
/// deleting it anyway.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn delete_connector(pool: &PgPool, id: &str) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM connector WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// A slug-based id, same shape `pipelines::slug_id` uses
/// (`"conn-<slug>-<base36 millis>"`).
fn slug_id(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug.trim_matches('-');
    let slug: String = slug.chars().take(32).collect();
    let slug = slug.trim_matches('-');
    let millis = OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000;
    #[allow(
        clippy::cast_sign_loss,
        reason = "unix millis since epoch is always positive"
    )]
    let millis = millis as u128;
    format!(
        "conn-{}-{}",
        if slug.is_empty() { "new" } else { slug },
        radix36(millis)
    )
}

/// Render `n` in base 36 lowercase, matching JavaScript's
/// `n.toString(36)`. Duplicated from `pipelines::radix36` rather than
/// shared: both are private to their module and the duplication is three
/// lines, not worth a new shared module for.
fn radix36(mut n: u128) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".to_owned();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use uuid::Uuid;

    #[test]
    fn connector_serializes_without_host_or_secret_ref() {
        let connector = Connector {
            id: "conn-x".to_owned(),
            name: "n".to_owned(),
            kind: "PostgreSQL CDC".to_owned(),
            direction: "source".to_owned(),
            health: "healthy".to_owned(),
            environment: "production".to_owned(),
            tenant: "Meridian Group".to_owned(),
            // Exercises both states: a test that has run, and activity that
            // (today) never gets measured -- see the field doc comments.
            last_test_at: Some("2026-01-01T00:00:00.000Z".to_owned()),
            last_activity_at: None,
            capabilities: vec!["CDC".to_owned()],
            owner: "o".to_owned(),
        };
        let value = serde_json::to_value(&connector).unwrap();
        for key in [
            "id",
            "name",
            "type",
            "direction",
            "health",
            "environment",
            "tenant",
            "lastTestAt",
            "lastActivityAt",
            "capabilities",
            "owner",
        ] {
            assert!(value.get(key).is_some(), "Connector is missing `{key}`");
        }
        assert_eq!(value.get("lastActivityAt"), Some(&serde_json::Value::Null));
        // The whole point of this domain: no credential-adjacent field
        // exists on the wire type at all.
        assert!(value.get("host").is_none());
        assert!(value.get("secretRef").is_none());
        assert!(value.get("secret_ref").is_none());
    }

    #[test]
    fn connector_detail_serializes_without_host_or_secret_ref() {
        let detail = ConnectorDetail {
            connector: Connector {
                id: "conn-x".to_owned(),
                name: "n".to_owned(),
                kind: "Kafka".to_owned(),
                direction: "source".to_owned(),
                health: "healthy".to_owned(),
                environment: "production".to_owned(),
                tenant: "Meridian Group".to_owned(),
                last_test_at: Some("2026-01-01T00:00:00.000Z".to_owned()),
                last_activity_at: None,
                capabilities: vec![],
                owner: "o".to_owned(),
            },
            discovered_assets: 0,
            discovered_schemas: vec![],
            recent_errors: vec![],
            dependent_pipelines: vec![],
            // This is a serialization test, not a resolution test — the
            // exact value doesn't matter, but it stands for a real
            // `audit_event.id` (see `audit::insert`'s `audit-<uuid>`
            // format), not the `aud-conn-<id>` label this crate used to
            // fabricate.
            audit_event_id: Some(format!("audit-{}", Uuid::new_v4())),
        };
        let value = serde_json::to_value(&detail).unwrap();
        assert!(value.get("host").is_none());
        assert!(value.get("secretRef").is_none());
        assert!(value.get("discoveredAssets").is_some());
        assert!(value.get("dependentPipelines").is_some());
    }

    /// The `ConnectorRow::Debug` impl — the one type in this crate that
    /// actually holds `host`/`secret_ref` in memory — must never print
    /// either value, however it is constructed.
    #[test]
    fn connector_row_debug_redacts_host_and_secret_ref() {
        let row = ConnectorRow {
            id: "conn-x".to_owned(),
            name: "n".to_owned(),
            kind: "PostgreSQL CDC".to_owned(),
            direction: "source".to_owned(),
            health: "healthy".to_owned(),
            environment: "production".to_owned(),
            tenant: "Meridian Group".to_owned(),
            host: "super-secret-internal-host.example:5432".to_owned(),
            secret_ref: "env:VERY_SENSITIVE_LOOKING_NAME".to_owned(),
            last_test_at: Some(OffsetDateTime::now_utc()),
            capabilities: vec![],
            owner: "o".to_owned(),
        };
        let debug = format!("{row:?}");
        assert!(!debug.contains("super-secret-internal-host"));
        assert!(!debug.contains("VERY_SENSITIVE_LOOKING_NAME"));
        assert!(debug.contains(REDACTED));
    }

    #[test]
    fn looks_like_raw_secret_flags_url_with_embedded_credentials() {
        assert!(looks_like_raw_secret(
            "postgres://admin:hunter2@db.internal:5432/oms"
        ));
    }

    #[test]
    fn looks_like_raw_secret_flags_pem_block() {
        assert!(looks_like_raw_secret(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIB...\n-----END RSA PRIVATE KEY-----"
        ));
    }

    #[test]
    fn looks_like_raw_secret_flags_jwt() {
        assert!(looks_like_raw_secret(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"
        ));
    }

    #[test]
    fn looks_like_raw_secret_flags_long_unstructured_hex() {
        assert!(looks_like_raw_secret(
            "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
        ));
    }

    #[test]
    fn looks_like_raw_secret_accepts_reference_names() {
        assert!(!looks_like_raw_secret("env:PG_OMS_PASSWORD"));
        assert!(!looks_like_raw_secret(
            "vault:secret/data/connectors/pg-oms"
        ));
        assert!(!looks_like_raw_secret("aws-secrets-manager:oms/cdc/creds"));
    }

    #[test]
    fn slug_id_lowercases_and_strips_punctuation() {
        let id = slug_id("Order Events Stream!!");
        assert!(id.starts_with("conn-order-events-stream-"));
    }

    #[test]
    fn radix36_matches_js_to_string_36() {
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
    }
}
