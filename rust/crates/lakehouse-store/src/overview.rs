//! Repository layer for alert *instances* — fired occurrences with mutable
//! ack/resolve lifecycle state — the Postgres backing for
//! `OverviewService.listAlerts`/`acknowledgeAlert`/`resolveAlert`.
//!
//! See `0011_overview_alerts.sql`'s header comment for why this lives in
//! Postgres rather than alongside `lakehouse_alerts`'s rule definitions in
//! `ClickHouse`'s `console.alert_rule`, and for what is (deliberately) not
//! wired up yet: `run_rules` firing a rule does not currently insert a row
//! here.

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

/// A fired alert instance. Mirrors `AlertItem` in `contracts/overview.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertItem {
    /// `alert_instance.id`.
    pub id: String,
    /// Alert title.
    pub title: String,
    /// Severity.
    pub severity: String,
    /// Where the alert originated.
    pub source: String,
    /// What is affected.
    pub affected: String,
    /// `"open" | "acknowledged" | "resolved"`.
    pub status: String,
    /// Who is handling this alert, if assigned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// When the alert fired, ISO 8601.
    pub at: String,
    /// Detailed description.
    pub detail: String,
    /// Resolution note, once resolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution_note: Option<String>,
    /// Deep link to the affected resource, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

#[derive(Debug, FromRow)]
struct AlertRow {
    id: String,
    title: String,
    severity: String,
    source: String,
    affected: String,
    status: String,
    assignee: Option<String>,
    at: OffsetDateTime,
    detail: String,
    resolution_note: Option<String>,
    href: Option<String>,
}

impl From<AlertRow> for AlertItem {
    fn from(row: AlertRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            severity: row.severity,
            source: row.source,
            affected: row.affected,
            status: row.status,
            assignee: row.assignee,
            at: iso_millis(row.at),
            detail: row.detail,
            resolution_note: row.resolution_note,
            href: row.href,
        }
    }
}

const ALERT_COLUMNS: &str =
    "id, title, severity, source, affected, status, assignee, at, detail, resolution_note, href";

/// List every alert instance, most recent first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_alerts(pool: &PgPool) -> Result<Vec<AlertItem>, StoreError> {
    let sql = format!("SELECT {ALERT_COLUMNS} FROM alert_instance ORDER BY at DESC");
    let rows: Vec<AlertRow> = sqlx::query_as(&sql).fetch_all(pool).await?;
    Ok(rows.into_iter().map(AlertItem::from).collect())
}

/// Mark an alert acknowledged, matching `mock/overview.ts`'s
/// `acknowledgeAlert`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn acknowledge_alert(pool: &PgPool, id: &str) -> Result<Option<AlertItem>, StoreError> {
    let sql = format!(
        "UPDATE alert_instance SET status = 'acknowledged' WHERE id = $1 RETURNING {ALERT_COLUMNS}"
    );
    let row: Option<AlertRow> = sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?;
    Ok(row.map(AlertItem::from))
}

/// Mark an alert resolved with `note`, matching `mock/overview.ts`'s
/// `resolveAlert`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn resolve_alert(
    pool: &PgPool,
    id: &str,
    note: &str,
) -> Result<Option<AlertItem>, StoreError> {
    let sql = format!(
        "UPDATE alert_instance SET status = 'resolved', resolution_note = $2 \
         WHERE id = $1 RETURNING {ALERT_COLUMNS}"
    );
    let row: Option<AlertRow> = sqlx::query_as(&sql)
        .bind(id)
        .bind(note)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(AlertItem::from))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn serialized_field_names_match_the_typescript_contract() {
        let alert = AlertItem {
            id: "al-1".to_owned(),
            title: "t".to_owned(),
            severity: "high".to_owned(),
            source: "s".to_owned(),
            affected: "a".to_owned(),
            status: "open".to_owned(),
            assignee: None,
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            detail: "d".to_owned(),
            resolution_note: None,
            href: None,
        };
        let value = serde_json::to_value(&alert).unwrap();
        for key in [
            "id", "title", "severity", "source", "affected", "status", "at", "detail",
        ] {
            assert!(value.get(key).is_some(), "AlertItem is missing `{key}`");
        }
        assert!(value.get("assignee").is_none());
        assert!(value.get("resolutionNote").is_none());
        assert!(value.get("href").is_none());
    }
}

// ── Overview summary sources ────────────────────────────────────────────
//
// The overview page used to read its pipeline numbers from Dagster and fill
// the rest with zeros written into the handler. These read what the console
// actually stores, so an unreachable orchestrator costs only the pipeline
// numbers, and "0 pending approvals" means there are none rather than
// "nobody wired this up".

/// One entry of the recent-activity feed, from the audit trail.
/// Mirrors `ActivityItem` in `contracts/overview.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    /// Stable row id for the feed; the audit event id, prefixed.
    pub id: String,
    /// When it happened, RFC 3339.
    pub at: String,
    /// Who did it, as the console should name them.
    pub actor: String,
    /// `user`, `agent` or `service` — whichever principal acted.
    pub actor_kind: String,
    /// What they did, in the console's words ("ran a query").
    pub action: String,
    /// What they did it to, e.g. a table or a pipeline.
    pub target: String,
    /// Where the console can show that target, when it has a page for it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_href: Option<String>,
    /// Grouping used for filtering the feed (`query`, `pipeline`, ...).
    pub category: String,
    /// The audit event this entry was derived from.
    pub audit_event_id: String,
}

#[derive(Debug, FromRow)]
struct ActivityRow {
    id: String,
    at: OffsetDateTime,
    // Nullable in `audit_event`: an action with no principal or no resource
    // (most of them) writes NULL, not an empty string.
    principal_kind: Option<String>,
    actor_label: Option<String>,
    action: String,
    resource_kind: Option<String>,
    resource_id: Option<String>,
    outcome: String,
}

/// An audited action as a sentence: what happened to what, in plain words.
fn activity_action(action: &str, outcome: &str) -> String {
    // "executed" falls through to the wildcard on purpose: it is the
    // common case, and "ran" is also the sensible reading of any outcome
    // this list has not learned yet.
    let verb = match outcome {
        "failed" => "failed to run",
        "refused" => "was refused",
        "needs_confirmation" => "asked to run",
        "needs_approval" => "requested approval for",
        _ => "ran",
    };
    format!("{verb} {}", action.replace('_', " "))
}

/// Which section of the console an audited action belongs to.
fn activity_category(principal_kind: &str, resource_kind: &str) -> &'static str {
    if principal_kind == "copilot" || principal_kind == "agent" {
        return "agent";
    }
    match resource_kind {
        "pipeline" | "pipeline_run" => "pipeline",
        "connector" => "connector",
        "policy" | "classification_rule" | "quality_rule" => "policy",
        "approval" => "approval",
        "alert" | "alert_rule" => "incident",
        "dataset" | "table" => "schema",
        // Charts, boards and saved queries fall through to the wildcard:
        // they belong to "query", and so does anything this list has not
        // learned yet.
        _ => "query",
    }
}

fn activity_href(resource_kind: &str) -> Option<String> {
    let href = match resource_kind {
        "chart" | "board" => "/dashboards",
        "pipeline" | "pipeline_run" => "/pipelines",
        "connector" => "/connectors",
        "policy" => "/governance/policies",
        "approval" => "/agents/approvals",
        "saved_query" | "query" => "/query-studio/saved",
        _ => return None,
    };
    Some(href.to_owned())
}

impl From<ActivityRow> for ActivityItem {
    fn from(r: ActivityRow) -> Self {
        let resource_kind = r.resource_kind.unwrap_or_default();
        let principal_kind = r.principal_kind.unwrap_or_default();
        let target = r
            .resource_id
            .filter(|s| !s.is_empty())
            .or_else(|| (!resource_kind.is_empty()).then(|| resource_kind.clone()))
            .unwrap_or_else(|| "—".to_owned());
        let actor_kind = match principal_kind.as_str() {
            "user" => "user",
            "service" => "service",
            _ => "agent",
        };
        Self {
            at: iso_millis(r.at),
            actor: r.actor_label.unwrap_or_else(|| "Unknown".to_owned()),
            actor_kind: actor_kind.to_owned(),
            action: activity_action(&r.action, &r.outcome),
            target,
            target_href: activity_href(&resource_kind),
            category: activity_category(&principal_kind, &resource_kind).to_owned(),
            audit_event_id: r.id.clone(),
            id: r.id,
        }
    }
}

/// The most recent audited actions, newest first.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn recent_activity(pool: &PgPool, limit: i64) -> Result<Vec<ActivityItem>, StoreError> {
    let rows: Vec<ActivityRow> = sqlx::query_as(
        "SELECT id, at, principal_kind, actor_label, action, resource_kind, resource_id, outcome \
         FROM audit_event ORDER BY at DESC LIMIT $1",
    )
    .bind(limit.clamp(1, 200))
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(ActivityItem::from).collect())
}

/// Counts the overview's cards need, in one round trip.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct OverviewCounts {
    /// Approvals waiting for a human decision.
    pub pending_approvals: i64,
    /// Audited actions the policy gate refused, last 7 days.
    pub policy_violations_7d: i64,
    /// Agent runs currently executing.
    pub active_agent_runs: i64,
    /// Pipelines currently running.
    pub pipelines_running: i64,
    /// Pipelines whose last run failed.
    pub pipelines_failed: i64,
    /// Pipelines that missed their freshness SLA.
    pub pipelines_delayed: i64,
}

/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn counts(pool: &PgPool) -> Result<OverviewCounts, StoreError> {
    let counts: OverviewCounts = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM approval_item WHERE status = 'pending') AS pending_approvals, \
           (SELECT count(*) FROM audit_event WHERE outcome = 'refused' AND at > now() - interval '7 days') AS policy_violations_7d, \
           (SELECT count(*) FROM agent_run WHERE status IN ('running', 'queued')) AS active_agent_runs, \
           (SELECT count(*) FROM pipeline_definition WHERE status = 'running') AS pipelines_running, \
           (SELECT count(*) FROM pipeline_definition WHERE status = 'failed') AS pipelines_failed, \
           (SELECT count(*) FROM pipeline_definition WHERE sla_ok = false) AS pipelines_delayed",
    )
    .fetch_one(pool)
    .await?;
    Ok(counts)
}
