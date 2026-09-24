//! Repository layer for alert *instances* — fired occurrences with mutable
//! ack/resolve lifecycle state — the Postgres backing for
//! `OverviewService.listAlerts`/`acknowledgeAlert`/`resolveAlert`.
//!
//! See `0011_overview_alerts.sql`'s header comment for why this lives in
//! Postgres rather than alongside `lakehouse_alerts`'s rule definitions in
//! `ClickHouse`'s `console.alert_rule`. `0038_alert_instance_rule_link.sql`
//! adds the `rule_id`/`fired_at`/`silenced_until` columns and widens
//! `severity` to nullable this module now uses: [`insert_from_fired_rule`]
//! is what `routes::alerts::run` (WS5 item C1) calls for every rule that
//! actually fires, and [`silence_alert`]/[`is_rule_silenced`] back a real
//! `POST /api/overview/alerts/{id}/silence`.

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
    /// Severity, copied verbatim from the firing rule's own `severity`
    /// column — `None` when the rule was saved without one (WS5 plan
    /// review Y3: never invented from the rule's `kind`). Serializes as
    /// `null`, never omitted, so the frontend renders `—` explicitly
    /// rather than silently dropping the field.
    pub severity: Option<String>,
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
    /// The `console.alert_rule` id that fired this instance, when it was
    /// created by [`insert_from_fired_rule`] rather than seeded/hand-made.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule_id: Option<String>,
    /// When the rule actually fired (as opposed to [`Self::at`], which a
    /// hand-made row can set independently), ISO 8601.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fired_at: Option<String>,
    /// Set by [`silence_alert`]; the instance (and its rule, via
    /// [`is_rule_silenced`]) is silenced until this time, ISO 8601.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub silenced_until: Option<String>,
}

#[derive(Debug, FromRow)]
struct AlertRow {
    id: String,
    title: String,
    severity: Option<String>,
    source: String,
    affected: String,
    status: String,
    assignee: Option<String>,
    at: OffsetDateTime,
    detail: String,
    resolution_note: Option<String>,
    href: Option<String>,
    rule_id: Option<String>,
    fired_at: Option<OffsetDateTime>,
    silenced_until: Option<OffsetDateTime>,
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
            rule_id: row.rule_id,
            fired_at: row.fired_at.map(iso_millis),
            silenced_until: row.silenced_until.map(iso_millis),
        }
    }
}

const ALERT_COLUMNS: &str = "id, title, severity, source, affected, status, assignee, at, \
    detail, resolution_note, href, rule_id, fired_at, silenced_until";

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

/// Everything a fired `lakehouse_alerts::RunResult` carries that
/// [`insert_from_fired_rule`] needs, plus the rule's own `severity`
/// (copied verbatim, including `None` — WS5 plan review Y3: never invented
/// from `kind`). A dedicated struct, not `RunResult` itself, so this crate
/// has no dependency on `lakehouse_alerts` — the caller
/// (`routes::alerts::run`) does the mapping.
#[derive(Debug, Clone, Copy)]
pub struct FiredRule<'a> {
    /// `console.alert_rule.id` — the rule that fired.
    pub rule_id: &'a str,
    /// The rule's display name, used as the instance's title.
    pub title: &'a str,
    /// The rule's own severity, copied verbatim.
    pub severity: Option<&'a str>,
    /// Where this alert originated (a fixed, kind-derived label).
    pub source: &'a str,
    /// What is affected (the rule's `mart`/table target).
    pub affected: &'a str,
    /// A human-readable description of the breach.
    pub detail: &'a str,
}

/// Insert a fired alert occurrence, deduped 15 minutes per `rule_id`,
/// atomically under concurrent callers (WS5 plan review Y7): a
/// transaction-scoped `pg_advisory_xact_lock` keyed on a hash of
/// `rule_id` serializes any two calls racing the same rule, so the
/// "already fired recently" check and the insert can never both pass for
/// two overlapping callers — the lock is released automatically when the
/// transaction commits or rolls back.
///
/// This primitive has no silence awareness by design: it always inserts
/// once the dedup window has lapsed, silenced or not. Whether to call it
/// at all while a rule is silenced is a decision the caller
/// (`routes::alerts::run`, via `lakehouse_alerts::SilenceSource`) makes
/// one level up — see that module's doc comment.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn insert_from_fired_rule(
    pool: &PgPool,
    fired: &FiredRule<'_>,
    now: OffsetDateTime,
) -> Result<Option<AlertItem>, StoreError> {
    let mut tx = pool.begin().await?;
    // `hashtext()` is Postgres's built-in string hash — stable within one
    // session, which is all an advisory lock scoped to this transaction's
    // lifetime needs.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(fired.rule_id)
        .execute(&mut *tx)
        .await?;
    let recent: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM alert_instance WHERE rule_id = $1 AND fired_at > $2 LIMIT 1",
    )
    .bind(fired.rule_id)
    .bind(now - time::Duration::minutes(15))
    .fetch_optional(&mut *tx)
    .await?;
    if recent.is_some() {
        tx.commit().await?;
        return Ok(None);
    }
    let id = format!("ai-{}", uuid::Uuid::new_v4());
    let sql = format!(
        "INSERT INTO alert_instance (id, title, severity, source, affected, status, at, \
         detail, rule_id, fired_at) VALUES ($1,$2,$3,$4,$5,'open',$6,$7,$8,$6) \
         RETURNING {ALERT_COLUMNS}"
    );
    let row: AlertRow = sqlx::query_as(&sql)
        .bind(&id)
        .bind(fired.title)
        .bind(fired.severity)
        .bind(fired.source)
        .bind(fired.affected)
        .bind(now)
        .bind(fired.detail)
        .bind(fired.rule_id)
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(Some(row.into()))
}

/// `POST /api/overview/alerts/{id}/silence` — sets `silenced_until` and
/// marks the row `acknowledged` so it stops appearing as `open` without
/// resolving it (a silence is temporary, a resolve is terminal).
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any failure.
pub async fn silence_alert(
    pool: &PgPool,
    id: &str,
    until: OffsetDateTime,
) -> Result<Option<AlertItem>, StoreError> {
    let sql = format!(
        "UPDATE alert_instance SET status = 'acknowledged', silenced_until = $2 \
         WHERE id = $1 RETURNING {ALERT_COLUMNS}"
    );
    let row: Option<AlertRow> = sqlx::query_as(&sql)
        .bind(id)
        .bind(until)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(AlertItem::from))
}

/// Whether `rule_id` has any `alert_instance` row whose `silenced_until`
/// is still in the future, as of `now`. Backs
/// `lakehouse_alerts::SilenceSource` (via `ApiSilenceSource` in
/// `lakehouse-api`) so a silenced rule's delivery is suppressed as well as
/// its dedup insert — see `routes::alerts::run`'s doc comment.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn is_rule_silenced(
    pool: &PgPool,
    rule_id: &str,
    now: OffsetDateTime,
) -> Result<bool, StoreError> {
    let silenced: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM alert_instance WHERE rule_id = $1 AND silenced_until > $2)",
    )
    .bind(rule_id)
    .bind(now)
    .fetch_one(pool)
    .await?;
    Ok(silenced)
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
            severity: Some("high".to_owned()),
            source: "s".to_owned(),
            affected: "a".to_owned(),
            status: "open".to_owned(),
            assignee: None,
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            detail: "d".to_owned(),
            resolution_note: None,
            href: None,
            rule_id: None,
            fired_at: None,
            silenced_until: None,
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
        assert!(value.get("ruleId").is_none());
        assert!(value.get("firedAt").is_none());
        assert!(value.get("silencedUntil").is_none());
    }

    /// `severity` is `Option<String>` — a rule fired with no severity
    /// serializes the field as `null`, never omitted (WS5 plan review Y3),
    /// so the frontend renders `—` explicitly rather than treating a
    /// missing key as "not yet loaded."
    #[test]
    fn a_none_severity_serializes_as_null_not_omitted() {
        let alert = AlertItem {
            id: "al-2".to_owned(),
            title: "t".to_owned(),
            severity: None,
            source: "s".to_owned(),
            affected: "a".to_owned(),
            status: "open".to_owned(),
            assignee: None,
            at: "2026-01-01T00:00:00.000Z".to_owned(),
            detail: "d".to_owned(),
            resolution_note: None,
            href: None,
            rule_id: Some("rule-1".to_owned()),
            fired_at: Some("2026-01-01T00:00:00.000Z".to_owned()),
            silenced_until: Some("2026-01-01T00:10:00.000Z".to_owned()),
        };
        let value = serde_json::to_value(&alert).unwrap();
        assert!(
            value.get("severity").is_some(),
            "severity key must be present"
        );
        assert!(value["severity"].is_null());
        assert_eq!(value["ruleId"], "rule-1");
        assert_eq!(value["firedAt"], "2026-01-01T00:00:00.000Z");
        assert_eq!(value["silencedUntil"], "2026-01-01T00:10:00.000Z");
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
