//! Repository layer for `audit_event` — the append-only sink every copilot
//! gate decision and executed tool call writes to (plan section 3.3 of
//! `docs/superpowers/plans/2026-09-08-copilot-operations-handover.md`).
//!
//! # Append-only, by convention
//!
//! There is deliberately no `update`/`delete` function in this module. A
//! row records one thing that happened at one point in time (a gate
//! allowed a call, a call executed, an approval was requested, an approver
//! decided); a later development in the same story (the approval being
//! decided, the tool then actually running) is a NEW row, not an edit of
//! this one. This matches the migration's header comment and keeps the
//! table trustworthy as a record of what happened, not what the current
//! state is.
//!
//! # Redaction is the caller's job
//!
//! [`insert`] writes `args` verbatim. It does not — and cannot, without
//! knowing what a given tool's arguments mean — strip secrets or bulky
//! query results out of them. Every call site MUST redact before building
//! a [`NewAuditEvent`]: never pass a `secretRef`'s resolved plaintext,
//! never pass raw SQL result rows, only the shape of what was asked for.

use serde::Serialize;
use serde_json::Value;
use sqlx::FromRow;
use sqlx::types::Json;
use time::OffsetDateTime;
use uuid::Uuid;

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

/// One row of `audit_event`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    /// `audit_event.id` — a generated `audit-<uuid>` string, not caller
    /// supplied (see [`insert`]).
    pub id: String,
    /// When this event was recorded, ISO 8601.
    pub at: String,
    /// Who/what performed the action, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_id: Option<String>,
    /// `"user" | "service" | "copilot" | "schedule"`, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub principal_kind: Option<String>,
    /// Human-readable display name for the principal, if known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actor_label: Option<String>,
    /// The tool name or route verb this event is about.
    pub action: String,
    /// The kind of resource the action targets, if any (e.g.
    /// `"connector"`, `"alert_rule"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_kind: Option<String>,
    /// The specific resource id the action targets, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<String>,
    /// The (caller-redacted) arguments the action was invoked with.
    pub args: Value,
    /// `"allowed" | "executed" | "refused" | "needs_confirmation" |
    /// "needs_approval" | "approved" | "rejected" | "failed"`.
    pub outcome: String,
    /// Free-text detail (e.g. a refusal reason), if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The `agent_run` this event is connected to, if any. `NULL`s out if
    /// that run is later deleted (`ON DELETE SET NULL`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// The `approval_item` this event is connected to, if any. `NULL`s out
    /// if that approval is later deleted (`ON DELETE SET NULL`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    /// The chat session this event happened within, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(FromRow)]
struct AuditRow {
    id: String,
    at: OffsetDateTime,
    principal_id: Option<String>,
    principal_kind: Option<String>,
    actor_label: Option<String>,
    action: String,
    resource_kind: Option<String>,
    resource_id: Option<String>,
    args: Json<Value>,
    outcome: String,
    detail: Option<String>,
    run_id: Option<String>,
    approval_id: Option<String>,
    session_id: Option<String>,
}

impl From<AuditRow> for AuditEvent {
    fn from(row: AuditRow) -> Self {
        Self {
            id: row.id,
            at: iso_millis(row.at),
            principal_id: row.principal_id,
            principal_kind: row.principal_kind,
            actor_label: row.actor_label,
            action: row.action,
            resource_kind: row.resource_kind,
            resource_id: row.resource_id,
            args: row.args.0,
            outcome: row.outcome,
            detail: row.detail,
            run_id: row.run_id,
            approval_id: row.approval_id,
            session_id: row.session_id,
        }
    }
}

/// The column list shared by every audit read/write. No `FROM`/`WHERE`, so
/// it is reusable after both a `SELECT` ([`list`]) and a `RETURNING`
/// ([`insert`]).
const AUDIT_COLUMNS: &str = "id, at, principal_id, principal_kind, actor_label, action, \
     resource_kind, resource_id, args, outcome, detail, run_id, approval_id, session_id";

/// Everything [`insert`] needs. Mirrors the migration's columns, minus the
/// generated `id` and defaulted `at`.
///
/// A dedicated input struct (rather than passing `AuditEvent` and ignoring
/// its `id`/`at`) makes "the id and timestamp are generated, not caller
/// input" a compile-time fact instead of a documented-but-unenforced
/// convention — there is no `AuditEvent` value a caller could construct
/// with a bogus id and hand to `insert` by mistake.
#[derive(Debug, Clone, Default)]
pub struct NewAuditEvent {
    /// Who/what performed the action, if known.
    pub principal_id: Option<String>,
    /// `"user" | "service" | "copilot" | "schedule"`, if known.
    pub principal_kind: Option<String>,
    /// Human-readable display name for the principal, if known.
    pub actor_label: Option<String>,
    /// The tool name or route verb this event is about.
    pub action: String,
    /// The kind of resource the action targets, if any.
    pub resource_kind: Option<String>,
    /// The specific resource id the action targets, if any.
    pub resource_id: Option<String>,
    /// The (caller-redacted) arguments the action was invoked with.
    /// Defaults to `{}` when not given.
    pub args: Option<Value>,
    /// `"allowed" | "executed" | "refused" | "needs_confirmation" |
    /// "needs_approval" | "approved" | "rejected" | "failed"`.
    pub outcome: String,
    /// Free-text detail (e.g. a refusal reason), if any.
    pub detail: Option<String>,
    /// The `agent_run` this event is connected to, if any. Must reference
    /// an existing row or [`StoreError::ForeignKeyViolation`] is returned.
    pub run_id: Option<String>,
    /// The `approval_item` this event is connected to, if any. Must
    /// reference an existing row or [`StoreError::ForeignKeyViolation`] is
    /// returned.
    pub approval_id: Option<String>,
    /// The chat session this event happened within, if any.
    pub session_id: Option<String>,
}

/// Append one audit row and return it as written.
///
/// # Redaction
///
/// This function writes `event.args` VERBATIM. It performs no redaction of
/// its own — the caller must have already stripped secrets (never a
/// resolved `secretRef` value) and bulky/sensitive payloads (never raw SQL
/// result rows) before constructing [`NewAuditEvent`]. See the module doc
/// comment.
///
/// # Errors
///
/// Returns [`StoreError::ForeignKeyViolation`] if `run_id` or
/// `approval_id` is set but does not reference an existing row.
/// Returns [`StoreError::Database`] for any other failure, including a
/// `principal_kind`/`outcome` value outside the set the migration's
/// `CHECK` constraints allow.
pub async fn insert(pool: &PgPool, event: NewAuditEvent) -> Result<AuditEvent, StoreError> {
    let id = format!("audit-{}", Uuid::new_v4());
    let args = event
        .args
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));
    let sql = format!(
        "INSERT INTO audit_event (id, principal_id, principal_kind, actor_label, action, \
         resource_kind, resource_id, args, outcome, detail, run_id, approval_id, session_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13) \
         RETURNING {AUDIT_COLUMNS}"
    );
    let row: AuditRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(&event.principal_id)
        .bind(&event.principal_kind)
        .bind(&event.actor_label)
        .bind(&event.action)
        .bind(&event.resource_kind)
        .bind(&event.resource_id)
        .bind(Json(args))
        .bind(&event.outcome)
        .bind(&event.detail)
        .bind(&event.run_id)
        .bind(&event.approval_id)
        .bind(&event.session_id)
        .fetch_one(pool)
        .await?;
    Ok(row.into())
}

/// Filters accepted by [`list`]. All fields are optional narrowing; `limit`
/// always applies.
#[derive(Debug, Clone)]
pub struct AuditFilter {
    /// Maximum number of rows to return.
    pub limit: i64,
    /// Restrict to this resource kind, if given.
    pub resource_kind: Option<String>,
    /// Restrict to this resource id, if given. Callers that mean "one
    /// specific resource" should set both `resource_kind` and
    /// `resource_id`.
    pub resource_id: Option<String>,
    /// Restrict to this principal id, if given.
    pub principal_id: Option<String>,
    /// Restrict to events at or after this time, if given.
    pub since: Option<OffsetDateTime>,
}

impl Default for AuditFilter {
    /// A generous but bounded default `limit`, matching the 50/500-row caps
    /// used elsewhere in this crate's list queries.
    fn default() -> Self {
        Self {
            limit: 200,
            resource_kind: None,
            resource_id: None,
            principal_id: None,
            since: None,
        }
    }
}

/// List audit events, newest first, narrowed by `filter`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list(pool: &PgPool, filter: AuditFilter) -> Result<Vec<AuditEvent>, StoreError> {
    let sql = format!(
        "SELECT {AUDIT_COLUMNS} FROM audit_event \
         WHERE ($1::text IS NULL OR resource_kind = $1) \
         AND ($2::text IS NULL OR resource_id = $2) \
         AND ($3::text IS NULL OR principal_id = $3) \
         AND ($4::timestamptz IS NULL OR at >= $4) \
         ORDER BY at DESC \
         LIMIT $5"
    );
    let rows: Vec<AuditRow> = sqlx::query_as(&sql)
        .bind(&filter.resource_kind)
        .bind(&filter.resource_id)
        .bind(&filter.principal_id)
        .bind(filter.since)
        .bind(filter.limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(AuditEvent::from).collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn default_filter_has_a_bounded_limit_and_no_narrowing() {
        let filter = AuditFilter::default();
        assert_eq!(filter.limit, 200);
        assert!(filter.resource_kind.is_none());
        assert!(filter.resource_id.is_none());
        assert!(filter.principal_id.is_none());
        assert!(filter.since.is_none());
    }
}
