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

use serde::{Deserialize, Serialize};
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
/// There is no `secret_ref`/`secret_ref_secondary` field here any more
/// (ADR 0002 Addendum 3): a client does not choose a credential reference
/// NAME, only a [`CredentialSpec`] (source + kind per slot). The server
/// generates the connector's id (`slug_id`, below) and derives the actual
/// reference names from it, in [`create_connector`], after the id is
/// known.
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
    /// What to derive this connector's credential reference name(s) from —
    /// see [`derive_secret_ref`] and ADR 0002 Addendum 3.
    pub credential: CredentialSpec,
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

/// Where a derived connector-credential reference name should be looked up
/// at resolve time. Mirrors `CredentialSource` in `contracts/connectors.ts`
/// — ADR 0002 Addendum 3's two schemes, `env:` and `file:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialSource {
    /// `env:CONNECTOR_<ID>_<SUFFIX>`, resolved by `EnvSecretResolver`.
    Env,
    /// `file:/run/secrets/connector_<id>_<suffix>`, resolved by
    /// `FileSecretResolver`.
    File,
}

/// Which fixed credential-name suffix a slot derives. Mirrors
/// `CredentialKind` in `contracts/connectors.ts` — exactly the six
/// suffixes ADR 0002 Addendum 3 and
/// `lakehouse_api::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS` both
/// name; adding a seventh here without adding it there would derive a name
/// the resolver never admits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// `_PASSWORD` suffix.
    Password,
    /// `_SECRET_KEY` suffix.
    SecretKey,
    /// `_ACCESS_KEY` suffix.
    AccessKey,
    /// `_API_KEY` suffix.
    ApiKey,
    /// `_TOKEN` suffix.
    Token,
    /// `_PRIVATE_KEY` suffix — the `sftp` adapter's `SftpAuth::PublicKey`
    /// auth kind needs this (a private-key PEM is not honestly any of the
    /// five kinds above); added for that case rather than overloading
    /// `SecretKey`, whose suffix an S3 connector's secondary slot already
    /// uses for an unrelated shape.
    PrivateKey,
}

impl CredentialKind {
    /// The upper-case suffix ADR 0002 Addendum 3 names, e.g. `"PASSWORD"`.
    /// [`derive_secret_ref`]'s `file:` form lower-cases this itself, rather
    /// than this method offering a second casing — one source of truth for
    /// the suffix text.
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Password => "PASSWORD",
            Self::SecretKey => "SECRET_KEY",
            Self::AccessKey => "ACCESS_KEY",
            Self::ApiKey => "API_KEY",
            Self::Token => "TOKEN",
            Self::PrivateKey => "PRIVATE_KEY",
        }
    }
}

/// What a client chooses for a connector's credential(s): a source scheme
/// and a kind per slot. Never a reference NAME — the client does not know
/// the connector's id yet (the server generates it), so it cannot name a
/// ref itself. See [`derive_secret_ref`] and ADR 0002 Addendum 3.
#[derive(Debug, Clone)]
pub struct CredentialSpec {
    /// `env:` or `file:` — which resolver scheme the derived name(s) use.
    pub source: CredentialSource,
    /// The primary slot's kind (`connector.secret_ref`).
    pub primary: CredentialKind,
    /// `None` for a connector type that needs only one credential (e.g.
    /// `PostgreSQL`). `Some` for e.g. an S3 connector's access-key/
    /// secret-key pair — see
    /// [`ConnectorDialInfo::secret_ref_secondary`].
    pub secondary: Option<CredentialKind>,
}

/// The credential reference NAMES a newly created connector's operator
/// must provision — returned ONCE, by [`create_connector`], and never
/// again: no GET response for this connector repeats them (this does not
/// weaken the module doc comment's guarantees 1/2 — [`Connector`] and
/// [`ConnectorDetail`] gain no field; this is a distinct, create-only
/// return value). Mirrors the `credential` field of `CreateConnectorResponse`
/// in `contracts/connectors.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorCredentialNames {
    /// The primary slot's derived reference name.
    pub primary: String,
    /// The secondary slot's derived reference name, or `None` when
    /// [`CredentialSpec::secondary`] was `None`.
    pub secondary: Option<String>,
}

/// Derive a user-created connector's credential reference name from ITS
/// OWN id — ADR 0002 Addendum 3 (`docs/adr/0002-secretref-resolution.md`).
/// For `id = "conn-orders-k3x9"`, `source = Env`, `kind = Password`:
/// `"env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD"`.
///
/// `id` is always [`slug_id`]'s output, `[a-z0-9-]` only, so upper-casing
/// (for `env:`) or lower-casing (for `file:`) and mapping `-` to `_` is a
/// lossless, one-to-one transform: two different ids can never derive the
/// same name, and every derived name already matches
/// `lakehouse_api::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS` — both
/// resolvers (the API's `AllowlistedSecretResolver` and Dagster's
/// `secret_resolver.resolve_secret_ref`) are unchanged by this ADR; this
/// function is the only place the invariant "every derived name matches
/// the allowlist" has to be kept true.
#[must_use]
pub fn derive_secret_ref(id: &str, source: CredentialSource, kind: CredentialKind) -> String {
    match source {
        CredentialSource::Env => {
            let key = id.to_ascii_uppercase().replace('-', "_");
            format!("env:CONNECTOR_{key}_{}", kind.suffix())
        }
        CredentialSource::File => {
            let key = id.replace('-', "_");
            format!(
                "file:/run/secrets/connector_{key}_{}",
                kind.suffix().to_ascii_lowercase()
            )
        }
    }
}

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
/// # Credential names are derived here, after the id is known (ADR 0002 Addendum 3)
///
/// The id is generated FIRST ([`slug_id`]), then [`derive_secret_ref`]
/// builds `secret_ref`/`secret_ref_secondary` from it and `input.credential`
/// — never the other way around, and never from a caller-supplied name.
/// This is the one place that ordering has to hold, so it lives here
/// rather than at the API layer: a route that generated the id itself and
/// passed refs in would duplicate `slug_id`'s definition, the exact
/// "grep for the existing helper" this module already asks of a caller
/// (AGENTS.md rule 4).
///
/// The returned [`ConnectorCredentialNames`] is the ONLY time these names
/// are ever produced — `routes::connectors::create` returns them straight
/// through in its response, once; no other function in this crate
/// recomputes or re-reveals them (a GET can recompute the SAME names from
/// the connector's own `id`, since the derivation is pure, but nothing
/// does — see [`ConnectorCredentialNames`]'s doc comment).
///
/// # Errors
///
/// Returns [`StoreError::Conflict`] (409) if the name is taken. Shape
/// validation of `name`/`type`/`direction`/etc. is NOT done here — that
/// belongs to the API layer, matching the `identity`/`pipelines` modules'
/// split (repository does persistence, route does request validation).
pub async fn create_connector(
    pool: &PgPool,
    input: &CreateConnectorInput,
) -> Result<(Connector, ConnectorCredentialNames), StoreError> {
    let id = slug_id(&input.name);
    let owner = input.owner.as_deref().unwrap_or(DEFAULT_OWNER);
    let secret_ref = derive_secret_ref(&id, input.credential.source, input.credential.primary);
    let secret_ref_secondary = input
        .credential
        .secondary
        .map(|kind| derive_secret_ref(&id, input.credential.source, kind));
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
        .bind(&secret_ref)
        .bind(&secret_ref_secondary)
        .bind(&input.residency)
        .bind(&input.capabilities)
        .bind(owner)
        .fetch_one(pool)
        .await?;
    let names = ConnectorCredentialNames {
        primary: secret_ref,
        secondary: secret_ref_secondary,
    };
    Ok((row.into(), names))
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
    /// One of `sql | cdc | files | rest | sheets | mongodb | kafka | sftp`
    /// — the value [`crate::ingest_spec::Dial::parse`] dispatches on.
    pub adapter: String,
    /// `"batch" | "cdc" | "stream"`, matching `connector_ingest_mode_check`
    /// (`0033_connector_ingest_spec.sql`, widened by
    /// `0043_ingest_tier2_adapters.sql` to admit `"stream"` for the
    /// `kafka` adapter).
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
/// what `spec.adapter` (and, for `rest`/`kafka`/`sftp`, `dial.auth.type`)
/// needs —
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
/// dial's auth type (`rest`, `kafka` or `sftp` — [`crate::ingest_spec::Dial::secret_map_auth_type`]).
/// Returns [`StoreError::NotFound`] if `id` does not
/// name a connector. Returns [`StoreError::Database`] on any other
/// failure.
pub async fn set_ingest_spec(
    pool: &PgPool,
    id: &str,
    spec: &IngestSpecInput,
) -> Result<IngestSpec, StoreError> {
    let dial = crate::ingest_spec::Dial::parse(&spec.adapter, &spec.dial)
        .map_err(|err| StoreError::Validation(err.to_string()))?;

    let auth_type = dial.secret_map_auth_type();
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
/// type is supported, stamp `lastTestAt` and append a
/// [`crate::connector_probe_result::ConnectorProbeResult`] history row.
/// Called by `lakehouse-api`'s `connector_probe` module AFTER it has
/// actually attempted (or declined to attempt, for an unsupported type) a
/// dial — this function never decides `ok`/`supported` itself, only
/// records what the caller measured.
///
/// `health` is updated to `"healthy"`/`"unhealthy"` only when `supported`
/// is `true` — an unsupported type's last-known health is left untouched,
/// since declining to test a connector is not evidence about whether it is
/// healthy. `last_test_at` follows the same rule and for the same reason:
/// an unsupported probe type was never actually dialed, so it must not
/// claim a test time it does not have (WS1 finding J19). The history row
/// follows the exact same rule: an unsupported probe has no outcome to
/// record, so no row is written for one.
///
/// The current-state `connector` UPDATE and the history insert-and-trim
/// both run inside ONE transaction, committed together: the history row's
/// `tested_at` is bound to the SAME value the `UPDATE ... RETURNING`
/// produced (never a second `now()` call), so `connector.last_test_at` and
/// the newest `connector_probe_result` row can never disagree about when
/// the last supported probe ran.
///
/// `message` is stored exactly as given. `connector_probe`'s module doc
/// comment ("Error messages never echo upstream data") guarantees every
/// value this function is ever called with is one of a small set of fixed
/// failure classes, the resolver's own refusal text, or a caller-supplied
/// config-shape complaint — never raw upstream `Display` text — and
/// `POST .../test` already returns this same string to the same
/// `connector:manage` caller this history is later read back by, so
/// recording it adds no new exposure.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` does not name a connector (in
/// which case nothing is written), or [`StoreError::Database`] on any
/// other failure.
pub async fn record_test_result(
    pool: &PgPool,
    id: &str,
    ok: bool,
    supported: bool,
    latency_ms: Option<i64>,
    message: &str,
) -> Result<ConnectorTestResult, StoreError> {
    let mut tx = pool.begin().await?;
    let sql = "UPDATE connector SET last_test_at = CASE WHEN $2 THEN now() ELSE last_test_at END, \
               health = CASE WHEN $2 THEN (CASE WHEN $3 THEN 'healthy' ELSE 'unhealthy' END) ELSE \
               health END WHERE id = $1 RETURNING last_test_at";
    let row: Option<(Option<OffsetDateTime>,)> = sqlx::query_as(sql)
        .bind(id)
        .bind(supported)
        .bind(ok)
        .fetch_optional(&mut *tx)
        .await?;
    let Some((tested_at,)) = row else {
        return Err(StoreError::NotFound);
    };

    if supported {
        // `tested_at` is `Some` whenever `supported` is `true` -- the
        // `UPDATE`'s own `CASE WHEN $2 THEN now() ...` guarantees it, so
        // this `if let` never silently skips a history row for a
        // supported probe.
        if let Some(tested_at) = tested_at {
            crate::connector_probe_result::insert_and_trim(
                &mut tx, id, tested_at, ok, latency_ms, message,
            )
            .await?;
        }
    }

    tx.commit().await?;
    Ok(ConnectorTestResult {
        ok,
        supported,
        latency_ms,
        message: message.to_owned(),
        tested_at: iso_opt(tested_at),
    })
}

/// Which of a connector's two credential slots [`swap_secret_ref`]
/// targets. Mirrors `RotateConnectorSecretRequest["slot"]` in
/// `contracts/connectors.ts`.
///
/// `secret_ref` (primary) is `NOT NULL` (`0013_connectors.sql`);
/// `secret_ref_secondary` is nullable (`0021_connector_dial_columns.sql`
/// — see [`ConnectorDialInfo::secret_ref_secondary`]'s doc comment), so
/// only [`Self::Secondary`] can ever legitimately have `None` as the
/// "current" value [`swap_secret_ref`]'s `expected_old` compares against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecretSlot {
    /// `connector.secret_ref`.
    Primary,
    /// `connector.secret_ref_secondary`.
    Secondary,
}

impl SecretSlot {
    /// The lowercase label this slot is recorded under -- e.g. in an
    /// audit event's `args` (`lakehouse-api`'s `connector_audit_event`,
    /// action `connector.secret_rotate`). Matches this enum's own
    /// `#[serde(rename_all = "lowercase")]` wire form; kept as an
    /// explicit method rather than round-tripping the value through
    /// `serde_json::to_value` at every call site that only wants the
    /// label.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Secondary => "secondary",
        }
    }
}

/// Conditionally rotate one of a connector's two credential REFERENCE
/// NAMES (never a credential value -- see the module doc comment) from
/// `expected_old` to `new_ref`.
///
/// This is an optimistic-concurrency compare-and-swap: the `UPDATE` only
/// takes effect `WHERE id = $2 AND <col> IS NOT DISTINCT FROM
/// expected_old`, so a caller who read the connector's current ref, then
/// lost a race with a second rotation of the same slot before this call
/// lands, gets [`StoreError::Conflict`] instead of silently clobbering a
/// rotation it never observed.
///
/// This function does NOT verify that `new_ref` resolves to a working
/// credential -- that is the caller's job, BEFORE calling this at all.
/// `lakehouse-api`'s `routes::connectors::rotate_secret` runs a real
/// connectivity probe (the same one `POST .../test` uses, via the same
/// allowlisted resolver) against a candidate built with `new_ref` in
/// place, and only calls this function once that probe reports
/// `supported: true, ok: true` -- see that handler's doc comment for why
/// the probe result itself is never persisted as
/// `connector_probe_result` history here (it tested a credential the
/// connector was not yet using when the probe ran).
///
/// `<col>` is picked by matching [`SecretSlot`] between two constant SQL
/// string literals below -- `format!` is not used at all in this
/// function, so there is no identifier interpolation to audit against
/// AGENTS.md's "format! only for constant identifiers" rule in the first
/// place.
///
/// # Zero rows: connector gone vs. ref changed since read
///
/// A single conditional `UPDATE` cannot itself distinguish "no such
/// connector" from "the connector exists but its current ref no longer
/// equals `expected_old`" -- both leave `rows_affected() == 0`. Rather
/// than wrapping the whole call in a `SELECT ... FOR UPDATE` transaction
/// (which would need to hold a row lock across the caller's earlier
/// probe too, to actually close the race, at the cost of serializing
/// every rotation attempt against that connector behind a lock held for
/// the probe's `DIAL_TIMEOUT`), this runs a second, cheap query ONLY on
/// the zero-rows path: if the row is now absent, [`StoreError::NotFound`];
/// if it is still present (so the `IS NOT DISTINCT FROM` comparison is
/// what failed the `WHERE` clause), [`StoreError::Conflict`]. Two
/// round-trips only on the rare zero-rows path -- the common (one-row)
/// success path is a single statement.
///
/// # Errors
///
/// Returns [`StoreError::NotFound`] if `id` does not name a connector.
/// Returns [`StoreError::Conflict`] if `id` names a connector but its
/// current value in slot `slot` no longer equals `expected_old`. Returns
/// [`StoreError::Database`] on any other failure.
pub async fn swap_secret_ref(
    pool: &PgPool,
    id: &str,
    slot: SecretSlot,
    expected_old: Option<&str>,
    new_ref: &str,
) -> Result<(), StoreError> {
    let sql = match slot {
        SecretSlot::Primary => {
            "UPDATE connector SET secret_ref = $1 WHERE id = $2 AND secret_ref IS NOT DISTINCT FROM $3"
        }
        SecretSlot::Secondary => {
            "UPDATE connector SET secret_ref_secondary = $1 WHERE id = $2 AND secret_ref_secondary \
             IS NOT DISTINCT FROM $3"
        }
    };
    let result = sqlx::query(sql)
        .bind(new_ref)
        .bind(id)
        .bind(expected_old)
        .execute(pool)
        .await?;
    if result.rows_affected() > 0 {
        return Ok(());
    }
    // Zero rows: figure out which of the two honest reasons applies --
    // see the doc comment above.
    let exists: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM connector WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    match exists {
        Some(_) => Err(StoreError::Conflict),
        None => Err(StoreError::NotFound),
    }
}

/// Everything `dagster/dispar_orchestrate/ingest_factory.py` needs to build
/// and run one ingest job for a connector, and nothing else — narrower
/// than [`ConnectorDialInfo`] (which also carries `host`/`kind` fields
/// this route has no use for) and a STRICT SUPERSET of what [`Connector`]
/// (the public, redacted read type) can express, since `Connector` has no
/// `adapter`/`dial`/`secretRef` fields at all (see the module doc
/// comment). Returned only by [`list_ingestible_connectors`], exposed
/// ONLY to `ingest:read`-scoped callers
/// (`GET /api/connectors/ingestible`, `0033_connector_ingest_spec.sql`'s
/// grant) — a caller with ONLY `ingest:read` and no `connector:manage`
/// still gets `secretRef` NAMES here (never resolved values), the same
/// "name, never resolve" posture `GET .../ingest-spec` already
/// established for `connector:manage` callers.
///
/// `host` still never appears as its own field — it lives inside `dial`,
/// which this type DOES expose (unlike `Connector`), since the whole
/// point of `/ingestible` is handing a trusted, `ingest:read`-scoped
/// internal caller enough to actually dial.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestibleConnector {
    /// `connector.id`.
    pub id: String,
    /// One of `sql | cdc | files | rest | sheets | mongodb | kafka | sftp`
    /// — never `NULL` here, since [`list_ingestible_connectors`] only
    /// selects rows where `adapter IS NOT NULL`.
    pub adapter: String,
    /// `"batch" | "cdc" | "stream"` (the last one only for a `kafka`
    /// adapter — see [`IngestSpecInput::ingest_mode`]). `set_ingest_spec`
    /// always writes this in the same `UPDATE` as `adapter`, so a row
    /// this query selects (`adapter IS NOT NULL`) has always had
    /// `ingest_mode` written too in practice — but see this struct's
    /// `# Note` if that invariant is ever weakened.
    pub ingest_mode: String,
    /// Validated at `set_ingest_spec` time against
    /// [`crate::ingest_spec::Dial::parse`] for this row's `adapter`.
    pub dial: serde_json::Value,
    /// The objects (tables/endpoints/sheet ranges) this ingest job
    /// targets.
    pub source_objects: serde_json::Value,
    /// `None` for a `cdc`-mode job, or a `batch`-mode job that has never
    /// been scheduled.
    pub schedule_cron: Option<String>,
    /// Reference NAME to the primary credential — never resolved here.
    pub secret_ref: String,
    /// Reference NAME to a secondary credential, for an adapter/auth-type
    /// combination that needs two (`secret_map.secret_field_names`,
    /// Dagster-side; `ingest_spec::secret_field_names`, Rust-side).
    pub secret_ref_secondary: Option<String>,
}

/// The raw tuple shape [`list_ingestible_connectors`] decodes from its
/// `SELECT`, naming the same eight columns in the same order: `id,
/// adapter, ingest_mode, dial, source_objects, schedule_cron, secret_ref,
/// secret_ref_secondary`. A module-level alias rather than an inline type,
/// same `clippy::type_complexity` reason [`IngestSpecRow`] exists for.
type IngestibleConnectorRow = (
    String,
    Option<String>,
    Option<String>,
    serde_json::Value,
    serde_json::Value,
    Option<String>,
    String,
    Option<String>,
);

/// List every connector that has ever had an ingest spec set
/// (`adapter IS NOT NULL`) — the source `ingest_factory.py`'s schedule
/// factory and `run_ingest` op both read from, via
/// `GET /api/connectors/ingestible`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_ingestible_connectors(
    pool: &PgPool,
) -> Result<Vec<IngestibleConnector>, StoreError> {
    let rows: Vec<IngestibleConnectorRow> = sqlx::query_as(
        "SELECT id, adapter, ingest_mode, dial, source_objects, schedule_cron, secret_ref, \
         secret_ref_secondary FROM connector WHERE adapter IS NOT NULL",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(
            |(
                id,
                adapter,
                ingest_mode,
                dial,
                source_objects,
                schedule_cron,
                secret_ref,
                secret_ref_secondary,
            )| {
                // `adapter IS NOT NULL` is the query's own WHERE clause, so
                // this `?` never actually short-circuits in practice — it
                // exists so a future change to the query can never turn a
                // NULL adapter into a fabricated empty string here instead
                // of silently dropping the row (AGENTS.md principle 2).
                // Same reasoning for `ingest_mode`: `set_ingest_spec` always
                // writes it alongside `adapter`, so this only guards
                // against a hand-edited or future-migration-violated row.
                Some(IngestibleConnector {
                    id,
                    adapter: adapter?,
                    ingest_mode: ingest_mode?,
                    dial,
                    source_objects,
                    schedule_cron,
                    secret_ref,
                    secret_ref_secondary,
                })
            },
        )
        .collect())
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

    // ── ADR 0002 Addendum 3: derived connector-credential names ─────────

    /// The exact example the addendum documents.
    #[test]
    fn derive_secret_ref_matches_the_documented_example() {
        assert_eq!(
            derive_secret_ref(
                "conn-orders-k3x9",
                CredentialSource::Env,
                CredentialKind::Password
            ),
            "env:CONNECTOR_CONN_ORDERS_K3X9_PASSWORD"
        );
        assert_eq!(
            derive_secret_ref(
                "conn-orders-k3x9",
                CredentialSource::File,
                CredentialKind::Password
            ),
            "file:/run/secrets/connector_conn_orders_k3x9_password"
        );
    }

    /// A pattern-match check, ported the same way
    /// `dagster/dispar_orchestrate/secret_resolver.py`'s `_pattern_matches`
    /// is: exactly one `*` splitting a fixed prefix and a fixed suffix.
    /// Test-only mirror of `lakehouse_core::secret::pattern_matches` --
    /// this crate does not depend on `lakehouse-core`'s `secret` module,
    /// so the test reimplements the one predicate it needs rather than add
    /// a dependency just for an assertion helper.
    fn test_pattern_matches(pattern: &str, value: &str) -> bool {
        let (prefix, suffix) = pattern.split_once('*').expect("every pattern has one '*'");
        value.len() >= prefix.len() + suffix.len()
            && value.starts_with(prefix)
            && value.ends_with(suffix)
    }

    /// Every derived name, for every source/kind combination, matches
    /// `lakehouse_api::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`
    /// (mirrored here as a literal, since `lakehouse-api` depends on
    /// `lakehouse-store` and not the reverse — a dependency this test
    /// cannot invert just to import the constant). If either list ever
    /// changes, this and that crate's own
    /// `connector_secret_resolver_admits_credential_suffixed_refs_...`
    /// test must be updated together.
    #[test]
    fn every_derived_name_matches_the_connector_allowlist_patterns() {
        let patterns = [
            "env:CONNECTOR_*_PASSWORD",
            "env:CONNECTOR_*_SECRET_KEY",
            "env:CONNECTOR_*_ACCESS_KEY",
            "env:CONNECTOR_*_API_KEY",
            "env:CONNECTOR_*_TOKEN",
            "env:CONNECTOR_*_PRIVATE_KEY",
            "file:/run/secrets/connector_*",
        ];
        for id in ["conn-orders-k3x9", "conn-a", "conn-pg-lakehouse-2"] {
            for source in [CredentialSource::Env, CredentialSource::File] {
                for kind in [
                    CredentialKind::Password,
                    CredentialKind::SecretKey,
                    CredentialKind::AccessKey,
                    CredentialKind::ApiKey,
                    CredentialKind::Token,
                    CredentialKind::PrivateKey,
                ] {
                    let derived = derive_secret_ref(id, source, kind);
                    assert!(
                        patterns.iter().any(|p| test_pattern_matches(p, &derived)),
                        "{derived:?} (id={id}, source={source:?}, kind={kind:?}) matches no \
                         allowlist pattern"
                    );
                }
            }
        }
    }

    /// No seeded ref (`0014_seed_connectors.sql`, `0022_prune_connector_seed.sql`,
    /// `0023_connector_dedicated_secret_refs.sql`) can ever be derived from
    /// any id beginning `conn-` — every derived `env:` name begins
    /// `CONNECTOR_CONN_`, and every seeded ref does not.
    #[test]
    fn no_seeded_ref_can_be_derived_from_any_connector_id() {
        let seeded_refs = [
            // 0014 (deleted by 0022, kept here since the assertion is
            // about the SHAPE, not which rows are currently live).
            "env:CLICKHOUSE_SERVING_PASSWORD",
            "env:ERP_FINANCE_PASSWORD",
            "env:FX_RATES_API_KEY",
            "env:GSHEETS_OAUTH_REFRESH_TOKEN",
            "env:ICEBERG_CATALOG_TOKEN",
            "env:KAFKA_CLICKSTREAM_SASL",
            "env:KAFKA_FLEET_SASL",
            "env:KAFKA_ORDERS_SASL",
            "env:MONGO_CATALOG_URI",
            "env:MQTT_WAREHOUSE_PASSWORD",
            "env:MYSQL_POS_PASSWORD",
            "env:ORACLE_GL_PASSWORD",
            "env:PG_OMS_CDC_PASSWORD",
            "env:PRICE_CRAWLER_PROXY_TOKEN",
            "env:WEATHER_API_KEY",
            // 0022/0023 (live today).
            "env:CONNECTOR_PG_PASSWORD",
            "env:CONNECTOR_S3_ACCESS_KEY",
            "env:CONNECTOR_S3_SECRET_KEY",
        ];
        for r in seeded_refs {
            assert!(
                !r.starts_with("env:CONNECTOR_CONN_"),
                "{r:?} would collide with a derived name's fixed prefix"
            );
        }
    }

    /// Two distinct ids never derive the same name, for ANY pair of kinds
    /// and either source. The collision that would matter is across kinds:
    /// `conn-a` + `SECRET_KEY` and `conn-a-secret` + a `KEY` suffix would
    /// both spell `..._CONN_A_SECRET_KEY`. It cannot happen because no
    /// allowed suffix is an `_`-separated tail of another; the cross-kind
    /// pairs below are the ids that would exploit it if one ever were.
    #[test]
    fn distinct_ids_never_derive_the_same_name() {
        const KINDS: [CredentialKind; 6] = [
            CredentialKind::Password,
            CredentialKind::SecretKey,
            CredentialKind::AccessKey,
            CredentialKind::ApiKey,
            CredentialKind::Token,
            CredentialKind::PrivateKey,
        ];
        let pairs = [
            ("conn-a-password", "conn-a"),
            ("conn-a-b", "conn-a-b-"),
            ("conn-ab", "conn-a-b"),
            ("conn-x-y-z", "conn-x-y-z-"),
            ("conn-a", "conn-a-secret"),
            ("conn-a", "conn-a-access"),
            ("conn-a", "conn-a-api"),
            ("conn-a-secret-key", "conn-a"),
        ];
        for (left, right) in pairs {
            assert_ne!(left, right, "test fixture bug: ids must differ");
            for source in [CredentialSource::Env, CredentialSource::File] {
                for left_kind in KINDS {
                    for right_kind in KINDS {
                        let left_ref = derive_secret_ref(left, source, left_kind);
                        let right_ref = derive_secret_ref(right, source, right_kind);
                        assert_ne!(
                            left_ref, right_ref,
                            "distinct ids {left:?} ({left_kind:?}) and {right:?} ({right_kind:?}) \
                             derived the same name"
                        );
                    }
                }
            }
        }
    }
}
