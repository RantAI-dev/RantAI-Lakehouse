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
