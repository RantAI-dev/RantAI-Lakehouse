//! Where each threshold alert stands (`BI-19` part C, decision D11, migration
//! `0113`; feature page `docs/core/features/sharing-and-delivery.md`, C2).
//!
//! This module stores a rule's state and nothing else. What a state means and
//! when it changes is `lakehouse_alerts::transition`; the evaluator
//! (`lakehouse-alerts`) reaches this table through the `StateSource` trait that
//! `lakehouse-api` implements over these functions, so the alerts crate keeps no
//! Postgres dependency.
//!
//! A rule with no row reads as `ok`: the first check over the line sends once.

use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;

use crate::StoreError;

/// A stored state row.
#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct RuleState {
    /// `ok`, `firing` or `stopped`.
    pub state: String,
    /// When `state` last changed.
    pub since: OffsetDateTime,
    /// The last number read, when the last check read one.
    pub last_value: Option<f64>,
    /// When the rule was last checked, a failed check included.
    pub last_checked_at: OffsetDateTime,
    /// Why the rule is `stopped`, or why the last check could not run. One of
    /// the product's own sentences, never engine text.
    pub reason: Option<String>,
}

/// The state of `rule_id`, or `None` when it has never been checked.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool, rule_id: &str) -> Result<Option<RuleState>, StoreError> {
    let row = sqlx::query_as(
        "SELECT state, since, last_value, last_checked_at, reason \
         FROM alert_rule_state WHERE rule_id = $1",
    )
    .bind(rule_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Record a check that ran: the new `state` (`since` moves only when it differs
/// from the stored one), the number read, and the reason when `state` is
/// `stopped` (cleared otherwise). One statement, so two overlapping ticks cannot
/// interleave a read and a write.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails, including a `state` the
/// table's `CHECK` does not admit.
pub async fn record_check(
    pool: &PgPool,
    rule_id: &str,
    state: &str,
    last_value: Option<f64>,
    reason: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO alert_rule_state (rule_id, state, since, last_value, last_checked_at, reason) \
         VALUES ($1, $2, $5, $3, $5, $4) \
         ON CONFLICT (rule_id) DO UPDATE SET \
           since = CASE WHEN alert_rule_state.state = EXCLUDED.state \
                        THEN alert_rule_state.since ELSE EXCLUDED.since END, \
           state = EXCLUDED.state, \
           last_value = EXCLUDED.last_value, \
           last_checked_at = EXCLUDED.last_checked_at, \
           reason = EXCLUDED.reason",
    )
    .bind(rule_id)
    .bind(state)
    .bind(last_value)
    .bind(reason)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Record a check that could not run: `last_checked_at` and `reason` move, the
/// state and the last value stay (C2). A rule never checked before gets an `ok`
/// row, which is what it read as without one.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn record_failed_check(
    pool: &PgPool,
    rule_id: &str,
    reason: &str,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO alert_rule_state (rule_id, state, since, last_checked_at, reason) \
         VALUES ($1, 'ok', $3, $3, $2) \
         ON CONFLICT (rule_id) DO UPDATE SET \
           last_checked_at = EXCLUDED.last_checked_at, reason = EXCLUDED.reason",
    )
    .bind(rule_id)
    .bind(reason)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Forget a deleted rule's state.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn delete(pool: &PgPool, rule_id: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM alert_rule_state WHERE rule_id = $1")
        .bind(rule_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Resolve the open and acknowledged `alert_instance` rows of `rule_id` with
/// `note`: the rule is back under its line (`BI-19` part C, D12). Rows already
/// resolved are left as they are. Returns how many it closed.
///
/// The same `UPDATE` that `overview::resolve_alert` does for one row by id,
/// for the rows a rule opened, so the overview shows one state for one
/// condition.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn resolve_open_instances(
    pool: &PgPool,
    rule_id: &str,
    note: &str,
) -> Result<u64, StoreError> {
    let done = sqlx::query(
        "UPDATE alert_instance SET status = 'resolved', resolution_note = $2 \
         WHERE rule_id = $1 AND status <> 'resolved'",
    )
    .bind(rule_id)
    .bind(note)
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}
