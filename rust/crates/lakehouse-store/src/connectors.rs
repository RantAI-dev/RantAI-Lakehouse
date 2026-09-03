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
    /// Last connection-test time, ISO 8601.
    pub last_test_at: String,
    /// Last observed activity time, ISO 8601.
    pub last_activity_at: String,
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
    /// A deterministic, purely label-shaped audit-event reference (matches
    /// `mock/connectors.ts`'s `aud-conn-<id>` convention) — not a real
    /// audit-log lookup.
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
    last_test_at: OffsetDateTime,
    last_activity_at: OffsetDateTime,
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
            .field("last_activity_at", &self.last_activity_at)
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
            last_test_at: iso_millis(row.last_test_at),
            last_activity_at: iso_millis(row.last_activity_at),
            capabilities: row.capabilities,
            owner: row.owner,
        }
    }
}

const CONNECTOR_COLUMNS: &str = "id, name, type, direction, health, environment, tenant, host, \
     secret_ref, last_test_at, last_activity_at, capabilities, owner";

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

    Ok(Some(ConnectorDetail {
        audit_event_id: Some(format!("aud-conn-{}", connector.id)),
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

/// Create a connector. `health` always starts `"healthy"`, `lastTestAt`/
/// `lastActivityAt` start at "now" — same as `mock/connectors.ts`'s
/// `createConnector`.
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
    let sql = format!(
        "INSERT INTO connector (id, name, type, direction, health, environment, tenant, host, \
         secret_ref, residency, capabilities, owner) \
         VALUES ($1, $2, $3, $4, 'healthy', $5, $6, $7, $8, $9, $10, $11) \
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
    /// When the test ran, ISO 8601.
    pub tested_at: String,
}

/// Fetch the connectivity-relevant fields (`type`, `host`, `secret_ref`,
/// `secret_ref_secondary`) needed to attempt a real dial. Returns
/// `Ok(None)` if `id` does not name a connector.
///
/// Deliberately returns a dedicated [`ConnectorDialInfo`] rather than
/// [`Connector`] or [`ConnectorRow`] — see that type's doc comment for why
/// it has no `Debug` impl at all.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_connector_dial_info(
    pool: &PgPool,
    id: &str,
) -> Result<Option<ConnectorDialInfo>, StoreError> {
    let row: Option<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT type, host, secret_ref, secret_ref_secondary FROM connector WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(
        |(kind, host, secret_ref, secret_ref_secondary)| ConnectorDialInfo {
            kind,
            host,
            secret_ref,
            secret_ref_secondary,
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
}

/// Persist the outcome of a real connectivity probe and stamp
/// `lastTestAt`. Called by `lakehouse-api`'s `connector_probe` module
/// AFTER it has actually attempted (or declined to attempt, for an
/// unsupported type) a dial — this function never decides `ok`/`supported`
/// itself, only records what the caller measured.
///
/// `health` is updated to `"healthy"`/`"unhealthy"` only when `supported`
/// is `true` — an unsupported type's last-known health is left untouched,
/// since declining to test a connector is not evidence about whether it is
/// healthy.
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
    let sql = "UPDATE connector SET last_test_at = now(), health = CASE WHEN $2 THEN (CASE WHEN \
               $3 THEN 'healthy' ELSE 'unhealthy' END) ELSE health END WHERE id = $1 RETURNING \
               last_test_at";
    let row: Option<(OffsetDateTime,)> = sqlx::query_as(sql)
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
        tested_at: iso_millis(tested_at),
    })
}

/// Delete a connector by id. Returns `Ok(false)` (not an error) if `id`
/// does not name a connector — matching the idempotent-delete convention
/// most of this codebase's `DELETE` handlers already use.
///
/// # What this does NOT do
///
/// This does NOT connect to the connector's `host` and drop any
/// replication slot/publication a P5 CDC connector may have created there.
/// Doing so would mean this crate resolving `secret_ref` to a live
/// credential and originating an outbound connection to an
/// operator-configured external system — the same category of operation
/// `lakehouse-api`'s `connector_probe` module now does, but only for
/// PostgreSQL/S3 connectivity tests, and only there, not here. Slot/WAL
/// cleanup for a removed CDC connector is an operational
/// step performed against the source database directly (see
/// `ops/debezium/deprovision_connector.sh` and the P5 report's G4 section)
/// — a real, told-straight gap, not a silent one: deleting a connector row
/// here does not, by itself, stop WAL from being pinned if a Debezium
/// Server process for it is still running.
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
            last_test_at: "2026-01-01T00:00:00.000Z".to_owned(),
            last_activity_at: "2026-01-01T00:00:00.000Z".to_owned(),
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
                last_test_at: "2026-01-01T00:00:00.000Z".to_owned(),
                last_activity_at: "2026-01-01T00:00:00.000Z".to_owned(),
                capabilities: vec![],
                owner: "o".to_owned(),
            },
            discovered_assets: 0,
            discovered_schemas: vec![],
            recent_errors: vec![],
            dependent_pipelines: vec![],
            audit_event_id: Some("aud-conn-conn-x".to_owned()),
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
            last_test_at: OffsetDateTime::now_utc(),
            last_activity_at: OffsetDateTime::now_utc(),
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
