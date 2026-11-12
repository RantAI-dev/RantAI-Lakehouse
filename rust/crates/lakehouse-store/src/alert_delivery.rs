//! A rule's latest delivery attempt (`SRC-40`, `TC-1695`; migration `0139`;
//! feature page `docs/core/features/load-failure-alerts.md`, decision 12).
//!
//! A "run succeeded" alert is delivered but opens no incident — an
//! `alert_instance` row is an incident (it is listed under "Active Incidents",
//! born `open`, and every open row lights the bell), so a success rule on a
//! 15-minute schedule would add 96 of them a day. This module stores the one
//! fact the console can then show on the rule itself: when it last sent, or
//! that it did not and why. Only the latest attempt is kept, and an older
//! attempt never replaces a newer one.
//!
//! What is written, and when, is decided by
//! `lakehouse-api/src/routes/load_alerts.rs::deliver_with_rules`: after the
//! deliveries of a success event, best effort — a write that fails is logged
//! and does not turn a delivered alert into an error. The reason is one of the
//! fixed sentences of `lakehouse_notify::DeliveryFailure::message`, never the
//! other side's text (principle 4).
//!
//! Only the success kind writes here; every other kind's delivery outcome is on
//! its incident, as before.

use std::collections::HashMap;

use serde::Serialize;
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

use crate::StoreError;
use crate::overview::{DELIVERED, DeliveryRecord, NOT_DELIVERED, iso_millis};

/// The latest delivery attempt of one rule, as `GET /api/alerts` reports it
/// under `lastDelivery`. Mirrors `AlertRule["lastDelivery"]` in
/// `contracts/alerts.ts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastDelivery {
    /// When the attempt ended, ISO 8601 with milliseconds.
    pub at: String,
    /// `true` when the message arrived.
    pub delivered: bool,
    /// Why it did not, one of the fixed sentences of
    /// `lakehouse_notify::DeliveryFailure::message`. Absent when delivered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The columns `last_for` reads, matching [`Row`].
const COLUMNS: &str = "rule_id, at, delivery_status, delivery_reason";

#[derive(FromRow)]
struct Row {
    rule_id: String,
    at: OffsetDateTime,
    delivery_status: String,
    delivery_reason: Option<String>,
}

/// Record the latest delivery attempt of `rule_id`.
///
/// One statement, so two attempts racing cannot interleave a read and a write;
/// the `WHERE` keeps an older attempt from replacing a newer one.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails, including a status the
/// table's `CHECK` does not admit.
pub async fn record(
    pool: &PgPool,
    rule_id: &str,
    delivery: DeliveryRecord<'_>,
    at: OffsetDateTime,
) -> Result<(), StoreError> {
    let (status, reason) = match delivery {
        DeliveryRecord::Delivered => (DELIVERED, None),
        DeliveryRecord::NotDelivered(reason) => (NOT_DELIVERED, Some(reason)),
    };
    sqlx::query(
        "INSERT INTO alert_rule_delivery (rule_id, at, delivery_status, delivery_reason) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (rule_id) DO UPDATE SET at = EXCLUDED.at, \
           delivery_status = EXCLUDED.delivery_status, \
           delivery_reason = EXCLUDED.delivery_reason \
         WHERE alert_rule_delivery.at <= EXCLUDED.at",
    )
    .bind(rule_id)
    .bind(at)
    .bind(status)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

/// The latest delivery attempts of `rule_ids`, by rule id. A rule with no row
/// is absent from the map.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn last_for(
    pool: &PgPool,
    rule_ids: &[String],
) -> Result<HashMap<String, LastDelivery>, StoreError> {
    let sql = format!("SELECT {COLUMNS} FROM alert_rule_delivery WHERE rule_id = ANY($1)");
    // safe: only the constant `COLUMNS` is formatted in; values are bound.
    let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(rule_ids)
        .fetch_all(pool)
        .await?;
    let mut out = HashMap::with_capacity(rows.len());
    for row in rows {
        // A stored status other than the two words the `0139` CHECK admits is
        // "not known", not a guess: the rule keeps no key, exactly as
        // `overview.rs` reads an unknown `alert_instance.delivery_status`.
        let (delivered, reason) = match row.delivery_status.as_str() {
            DELIVERED => (true, None),
            NOT_DELIVERED => (false, row.delivery_reason),
            _ => continue,
        };
        out.insert(
            row.rule_id,
            LastDelivery {
                at: iso_millis(row.at),
                delivered,
                reason,
            },
        );
    }
    Ok(out)
}

/// Forget a deleted rule's delivery row.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn delete(pool: &PgPool, rule_id: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM alert_rule_delivery WHERE rule_id = $1")
        .bind(rule_id)
        .execute(pool)
        .await?;
    Ok(())
}
