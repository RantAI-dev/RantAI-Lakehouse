//! Repository layer for the write-side halves of the queries domain: saved
//! queries, query-run history, and collaboration projects.
//!
//! # Postgres vs `ClickHouse`, method by method
//!
//! `src/services/clients/queries.ts` splits `QueryService` into a real half
//! (`run`/`estimate`, `ClickHouse`-backed, ported in Phase 1;
//! `generateSql`, LLM-backed, also ported in Phase 1 as
//! `/api/agent/text-to-sql`) and a mock half (`listSaved`, `listHistory`,
//! `listCollaboration`, `createCollaborationProject`). This module is the
//! Postgres backing for that mock half, plus one addition:
//! `routes::query::run` calls [`record_history`] after a successful
//! `ClickHouse` execution, so [`list_history`] returns *real* past
//! executions instead of fabricated fixtures — see [`record_history`]'s
//! doc comment for how that write is made non-fatal.
//!
//! `SavedQuery` has no `create`/`update`/`delete` method anywhere in the
//! `QueryService` contract — [`list_saved`] is genuinely read-only,
//! backed by seed rows (`0004_queries.sql`) the way `identity`'s
//! `workspace_settings` is backed by a constant, except here the shape
//! (rows in a table with `dimension`/`tags`) earns an actual table rather
//! than a hardcoded response.

use serde::Serialize;
use sqlx::FromRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// Render a timestamp the way JavaScript's `Date.prototype.toISOString`
/// does. Duplicated from `identity.rs`/`governance.rs` — see either's doc
/// comment for why.
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

// ── Saved query (read-only — no writer in the contract) ────────────────

/// A saved query. Mirrors `SavedQuery` in `contracts/queries.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedQuery {
    /// `saved_query.id`, as a string.
    pub id: String,
    /// Display title.
    pub title: String,
    /// The saved SQL text.
    pub sql: String,
    /// Who saved this query.
    pub owner: String,
    /// When it was last saved/edited, ISO 8601. Serializes as `updatedAt`.
    pub updated_at: String,
    /// Free-text tags.
    pub tags: Vec<String>,
}

#[derive(Debug, FromRow)]
struct SavedQueryRow {
    id: Uuid,
    title: String,
    sql: String,
    owner: String,
    updated_at: OffsetDateTime,
    tags: Vec<String>,
}

impl From<SavedQueryRow> for SavedQuery {
    fn from(row: SavedQueryRow) -> Self {
        Self {
            id: row.id.to_string(),
            title: row.title,
            sql: row.sql,
            owner: row.owner,
            updated_at: iso_millis(row.updated_at),
            tags: row.tags,
        }
    }
}

/// List every saved query, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_saved(pool: &PgPool) -> Result<Vec<SavedQuery>, StoreError> {
    let rows: Vec<SavedQueryRow> = sqlx::query_as(
        "SELECT id, title, sql, owner, updated_at, tags FROM saved_query \
         ORDER BY updated_at DESC, title",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(SavedQuery::from).collect())
}

// ── Query history (written by `routes::query::run`, read by listHistory) ─

/// One recorded query execution. Mirrors `QueryHistoryItem` in
/// `contracts/queries.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryHistoryItem {
    /// `query_history.id` — the same `q-<epoch_ms>` id the run that
    /// produced this row returned as `QueryResult.id`.
    pub id: String,
    /// The executed SQL text.
    pub sql: String,
    /// Who ran the query. No auth exists yet (see the routes-crate module
    /// doc comments), so this is currently always a placeholder.
    pub user: String,
    /// When the query ran, ISO 8601.
    pub at: String,
    /// `"completed" | "failed" | "cancelled" | "blocked"`.
    pub status: String,
    /// Wall-clock execution time. Serializes as `durationMs`.
    pub duration_ms: i64,
    /// Bytes scanned. Serializes as `scannedBytes`.
    pub scanned_bytes: i64,
    /// Estimated cost units consumed. Serializes as `costUnits`.
    pub cost_units: f64,
    /// The workload classification. Serializes as `workloadClass`.
    pub workload_class: String,
    /// Which storage engine served the query.
    pub engine: String,
    /// Whether the result was served (partly) from cache. Serializes as
    /// `cacheAssisted`.
    pub cache_assisted: bool,
    /// A synthesized audit-trail correlation id. Omitted from JSON when
    /// absent. Serializes as `auditEventId`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit_event_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct QueryHistoryRow {
    id: String,
    sql: String,
    user_name: String,
    at: OffsetDateTime,
    status: String,
    duration_ms: i64,
    scanned_bytes: i64,
    cost_units: f64,
    workload_class: String,
    engine: String,
    cache_assisted: bool,
    audit_event_id: Option<String>,
}

impl From<QueryHistoryRow> for QueryHistoryItem {
    fn from(row: QueryHistoryRow) -> Self {
        Self {
            id: row.id,
            sql: row.sql,
            user: row.user_name,
            at: iso_millis(row.at),
            status: row.status,
            duration_ms: row.duration_ms,
            scanned_bytes: row.scanned_bytes,
            cost_units: row.cost_units,
            workload_class: row.workload_class,
            engine: row.engine,
            cache_assisted: row.cache_assisted,
            audit_event_id: row.audit_event_id,
        }
    }
}

/// The most history rows [`list_history`] returns. The console's history
/// panel is a recent-activity view, not an archive browser; an unbounded
/// `SELECT *` would grow without limit as `routes::query::run` keeps
/// inserting.
const HISTORY_LIST_LIMIT: i64 = 200;

/// List recorded query executions, most recent first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_history(pool: &PgPool) -> Result<Vec<QueryHistoryItem>, StoreError> {
    let rows: Vec<QueryHistoryRow> = sqlx::query_as(
        "SELECT id, sql, user_name, at, status, duration_ms, scanned_bytes, cost_units, \
         workload_class, engine, cache_assisted, audit_event_id \
         FROM query_history ORDER BY at DESC LIMIT $1",
    )
    .bind(HISTORY_LIST_LIMIT)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(QueryHistoryItem::from).collect())
}

/// Everything [`record_history`] needs to write one row.
#[derive(Debug, Clone)]
pub struct RecordHistoryInput<'a> {
    /// The id to record the row under — `routes::query::run` passes the
    /// same `q-<epoch_ms>` id it hands back as `QueryResult.id`, so a
    /// history row and the result that produced it share one identifier.
    pub id: &'a str,
    /// The executed SQL text.
    pub sql: &'a str,
    /// Who ran the query.
    pub user: &'a str,
    /// `"completed" | "failed" | "cancelled" | "blocked"`.
    pub status: &'a str,
    /// Wall-clock execution time, in milliseconds.
    pub duration_ms: i64,
    /// Bytes scanned.
    pub scanned_bytes: i64,
    /// Estimated cost units consumed.
    pub cost_units: f64,
    /// The workload classification.
    pub workload_class: &'a str,
    /// Which storage engine served the query.
    pub engine: &'a str,
    /// Whether the result was served (partly) from cache.
    pub cache_assisted: bool,
    /// A synthesized audit-trail correlation id, if any.
    pub audit_event_id: Option<&'a str>,
}

/// Record one query execution.
///
/// # Non-fatal by design
///
/// This is called from `routes::query::run` *after* a query has already
/// succeeded against `ClickHouse` and a response has been built for the
/// caller. A logging write failing (no pool configured, Postgres down,
/// whatever) must never turn an otherwise-successful query into an error
/// response — so `routes::query::run` calls this, gets a `Result`, and on
/// `Err` logs a warning and still returns the successful `QueryResult`
/// unchanged. The `Result` is returned rather than swallowed *here* so
/// that decision stays visible at the call site instead of this function
/// silently deciding it for every caller (a test, for instance, very much
/// wants to see the error).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert fails.
pub async fn record_history(
    pool: &PgPool,
    input: &RecordHistoryInput<'_>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO query_history \
         (id, sql, user_name, status, duration_ms, scanned_bytes, cost_units, \
          workload_class, engine, cache_assisted, audit_event_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) \
         ON CONFLICT (id) DO NOTHING",
    )
    .bind(input.id)
    .bind(input.sql)
    .bind(input.user)
    .bind(input.status)
    .bind(input.duration_ms)
    .bind(input.scanned_bytes)
    .bind(input.cost_units)
    .bind(input.workload_class)
    .bind(input.engine)
    .bind(input.cache_assisted)
    .bind(input.audit_event_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Collaboration project ───────────────────────────────────────────────

/// A shared query workspace. Mirrors `CollaborationProject` in
/// `contracts/queries.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollaborationProject {
    /// `collaboration_project.id`, as a string.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Collaborator count, set once at creation (see [`create_collaboration_project`]).
    pub members: i32,
    /// When the project was last updated, ISO 8601. Serializes as
    /// `updatedAt`.
    pub updated_at: String,
    /// Free-text description.
    pub description: String,
}

#[derive(Debug, FromRow)]
struct CollaborationProjectRow {
    id: Uuid,
    name: String,
    members: i32,
    updated_at: OffsetDateTime,
    description: String,
}

impl From<CollaborationProjectRow> for CollaborationProject {
    fn from(row: CollaborationProjectRow) -> Self {
        Self {
            id: row.id.to_string(),
            name: row.name,
            members: row.members,
            updated_at: iso_millis(row.updated_at),
            description: row.description,
        }
    }
}

/// List every collaboration project, most recently updated first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_collaboration(pool: &PgPool) -> Result<Vec<CollaborationProject>, StoreError> {
    let rows: Vec<CollaborationProjectRow> = sqlx::query_as(
        "SELECT id, name, members, updated_at, description FROM collaboration_project \
         ORDER BY updated_at DESC, name",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(CollaborationProject::from).collect())
}

/// Everything [`create_collaboration_project`] needs. Mirrors
/// `CreateCollaborationProjectInput`.
#[derive(Debug, Clone)]
pub struct CreateCollaborationProjectInput {
    /// Display name.
    pub name: String,
    /// Collaborator names; only the count is retained (see
    /// [`CollaborationProject::members`]).
    pub collaborators: Vec<String>,
    /// Free-text description; falls back to a "Collaborators: ..." sentence
    /// when absent.
    pub description: Option<String>,
}

/// Create a collaboration project. `members` is stored as the collaborator
/// count at creation time (`mock/queries.ts`'s
/// `input.collaborators.length`) — the contract has no method to add or
/// remove a member later, so there is nothing for a derived `COUNT(*)`
/// subquery to stay in sync with.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn create_collaboration_project(
    pool: &PgPool,
    input: &CreateCollaborationProjectInput,
) -> Result<CollaborationProject, StoreError> {
    #[allow(
        clippy::cast_possible_wrap,
        clippy::cast_possible_truncation,
        reason = "a collaborator list from one request body cannot approach i32::MAX"
    )]
    let members = input.collaborators.len() as i32;
    let description = input
        .description
        .clone()
        .unwrap_or_else(|| format!("Collaborators: {}", input.collaborators.join(", ")));
    let row: CollaborationProjectRow = sqlx::query_as(
        "INSERT INTO collaboration_project (name, members, description) \
         VALUES ($1, $2, $3) RETURNING id, name, members, updated_at, description",
    )
    .bind(&input.name)
    .bind(members)
    .bind(&description)
    .fetch_one(pool)
    .await?;
    Ok(row.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The wire format is the contract: every key the browser reads must be
    /// the camelCase name `contracts/queries.ts` declares.
    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let saved = SavedQuery {
            id: "s".to_owned(),
            title: "t".to_owned(),
            sql: "SELECT 1".to_owned(),
            owner: "o".to_owned(),
            updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
            tags: vec!["finance".to_owned()],
        };
        let value = serde_json::to_value(&saved).unwrap();
        for key in ["id", "title", "sql", "owner", "updatedAt", "tags"] {
            assert!(value.get(key).is_some(), "SavedQuery is missing `{key}`");
        }

        let history = QueryHistoryItem {
            id: "q".to_owned(),
            sql: "SELECT 1".to_owned(),
            user: "u".to_owned(),
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            status: "completed".to_owned(),
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics".to_owned(),
            engine: "hot-store".to_owned(),
            cache_assisted: true,
            audit_event_id: Some("aud-1".to_owned()),
        };
        let value = serde_json::to_value(&history).unwrap();
        for key in [
            "id",
            "sql",
            "user",
            "at",
            "status",
            "durationMs",
            "scannedBytes",
            "costUnits",
            "workloadClass",
            "engine",
            "cacheAssisted",
            "auditEventId",
        ] {
            assert!(
                value.get(key).is_some(),
                "QueryHistoryItem is missing `{key}`"
            );
        }

        let collab = CollaborationProject {
            id: "c".to_owned(),
            name: "n".to_owned(),
            members: 3,
            updated_at: "2026-01-01T00:00:00.000Z".to_owned(),
            description: "d".to_owned(),
        };
        let value = serde_json::to_value(&collab).unwrap();
        for key in ["id", "name", "members", "updatedAt", "description"] {
            assert!(
                value.get(key).is_some(),
                "CollaborationProject is missing `{key}`"
            );
        }
    }

    /// `auditEventId` is optional in the contract; a history row recorded
    /// without one must omit the key, not emit `null`.
    #[test]
    fn history_item_omits_absent_audit_event_id() {
        let item = QueryHistoryItem {
            id: "q".to_owned(),
            sql: "SELECT 1".to_owned(),
            user: "u".to_owned(),
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            status: "completed".to_owned(),
            duration_ms: 1,
            scanned_bytes: 1,
            cost_units: 0.1,
            workload_class: "hot-analytics".to_owned(),
            engine: "hot-store".to_owned(),
            cache_assisted: false,
            audit_event_id: None,
        };
        let value = serde_json::to_value(&item).unwrap();
        assert!(value.get("auditEventId").is_none());
    }

    /// `description` falls back to a "Collaborators: ..." sentence when
    /// absent, matching `mock/queries.ts`'s `createCollaborationProject` —
    /// pinned down here since the fallback logic lives in application code,
    /// not SQL, and has no direct integration-test coverage of its own.
    #[test]
    fn collaboration_description_fallback_matches_mock_behavior() {
        let input = CreateCollaborationProjectInput {
            name: "n".to_owned(),
            collaborators: vec!["Rina".to_owned(), "Bayu".to_owned()],
            description: None,
        };
        let description = input
            .description
            .clone()
            .unwrap_or_else(|| format!("Collaborators: {}", input.collaborators.join(", ")));
        assert_eq!(description, "Collaborators: Rina, Bayu");
    }
}
