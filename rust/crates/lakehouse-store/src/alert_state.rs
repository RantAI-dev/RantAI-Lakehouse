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
use crate::report_schedule::{Schedule, ScheduleError};

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

/// Claim a transition before its message is sent (`BI-19` C review SF-1): write
/// `next` only if the stored state is still `expected` (a rule with no row
/// counts as having none, so the insert wins), and say whether this call won.
/// One conditional upsert, so of two overlapping runners that read the same
/// state exactly one gets `true` and sends.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn claim(
    pool: &PgPool,
    rule_id: &str,
    expected: &str,
    next: &str,
    last_value: Option<f64>,
    now: OffsetDateTime,
) -> Result<bool, StoreError> {
    let won: Option<(String,)> = sqlx::query_as(
        "INSERT INTO alert_rule_state (rule_id, state, since, last_value, last_checked_at, reason) \
         VALUES ($1, $2, $4, $3, $4, NULL) \
         ON CONFLICT (rule_id) DO UPDATE SET \
           since = CASE WHEN alert_rule_state.state = EXCLUDED.state \
                        THEN alert_rule_state.since ELSE EXCLUDED.since END, \
           state = EXCLUDED.state, \
           last_value = EXCLUDED.last_value, \
           last_checked_at = EXCLUDED.last_checked_at, \
           reason = NULL \
         WHERE alert_rule_state.state = $5 \
         RETURNING rule_id",
    )
    .bind(rule_id)
    .bind(next)
    .bind(last_value)
    .bind(now)
    .bind(expected)
    .fetch_optional(pool)
    .await?;
    Ok(won.is_some())
}

/// Give a [`claim`] back when its message could not be delivered: `to` replaces
/// `claimed`, and only if the rule is still in `claimed` (a later claim by
/// another runner is left alone).
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn release(
    pool: &PgPool,
    rule_id: &str,
    claimed: &str,
    to: &str,
    now: OffsetDateTime,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE alert_rule_state SET state = $3, since = $4 \
         WHERE rule_id = $1 AND state = $2",
    )
    .bind(rule_id)
    .bind(claimed)
    .bind(to)
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

// ── Digests: when a dashboard digest is due (`BI-65`, migration `0152`) ────

/// Why a digest's turn could not be worked out.
#[derive(Debug, thiserror::Error)]
pub enum DigestTurnError {
    /// The clock could not be read or written.
    #[error("{0}")]
    Store(#[from] StoreError),
    /// The schedule or its zone is not usable.
    #[error("{0}")]
    Schedule(#[from] ScheduleError),
}

/// What one tick should do about one digest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigestTurn {
    /// The digest was never seen: its clock starts now and nothing is sent. Its
    /// first send is the first schedule time after this tick.
    StartedClock,
    /// The next schedule time after the clock is still ahead.
    NotYet,
    /// Due, and this caller moved the clock to `now`, so only it sends.
    /// `previous` is the clock before, for [`give_back_digest`].
    Claimed {
        /// The clock before this claim.
        previous: OffsetDateTime,
    },
    /// Due, but another runner moved the clock first.
    ClaimedElsewhere,
}

/// Whether a digest counted from `clock` is due at `now`: the first run of
/// `schedule` (in `zone`) after `clock` is at or before `now`. Pure, so the
/// whole rule is a unit test. A missed run happens once: the next time is
/// worked out from the claim, never from the time that was missed
/// (`report_schedule`, feature decision 8).
///
/// # Errors
///
/// [`ScheduleError`] from [`Schedule::next_run_after`].
pub fn digest_is_due(
    schedule: Schedule,
    zone: &str,
    clock: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<bool, ScheduleError> {
    Ok(schedule.next_run_after(zone, clock)? <= now)
}

/// The instant digest `rule_id` is counted from, or `None` when it has not been
/// seen yet.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn digest_clock(
    pool: &PgPool,
    rule_id: &str,
) -> Result<Option<OffsetDateTime>, StoreError> {
    let row: Option<Option<OffsetDateTime>> =
        sqlx::query_scalar("SELECT digest_clock_at FROM alert_rule_state WHERE rule_id = $1")
            .bind(rule_id)
            .fetch_optional(pool)
            .await?;
    Ok(row.flatten())
}

/// Move the clock of `rule_id` from `expected` to `to`, only if it is still at
/// `expected` (`None` means never seen), and say whether this call won. One
/// conditional upsert, so of two overlapping runners that read the same clock
/// exactly one gets `true` and sends (the same claim as [`claim`]).
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn move_digest_clock(
    pool: &PgPool,
    rule_id: &str,
    expected: Option<OffsetDateTime>,
    to: OffsetDateTime,
) -> Result<bool, StoreError> {
    let won: Option<(String,)> = sqlx::query_as(
        "INSERT INTO alert_rule_state (rule_id, state, since, last_checked_at, digest_clock_at) \
         VALUES ($1, 'ok', $3, $3, $3) \
         ON CONFLICT (rule_id) DO UPDATE SET digest_clock_at = EXCLUDED.digest_clock_at \
         WHERE alert_rule_state.digest_clock_at IS NOT DISTINCT FROM $2::timestamptz \
         RETURNING rule_id",
    )
    .bind(rule_id)
    .bind(expected)
    .bind(to)
    .fetch_optional(pool)
    .await?;
    Ok(won.is_some())
}

/// Decide what this tick does about digest `rule_id` and, when it is due, claim
/// the send before anything is sent (so two runners cannot both send).
///
/// # Errors
///
/// [`DigestTurnError::Store`] when the clock cannot be read or written,
/// [`DigestTurnError::Schedule`] for a zone or schedule that is not usable.
pub async fn digest_turn(
    pool: &PgPool,
    rule_id: &str,
    schedule: Schedule,
    zone: &str,
    now: OffsetDateTime,
) -> Result<DigestTurn, DigestTurnError> {
    let Some(clock) = digest_clock(pool, rule_id).await? else {
        // Losing this race changes nothing: the other runner started the clock.
        move_digest_clock(pool, rule_id, None, now).await?;
        return Ok(DigestTurn::StartedClock);
    };
    if !digest_is_due(schedule, zone, clock, now)? {
        return Ok(DigestTurn::NotYet);
    }
    if move_digest_clock(pool, rule_id, Some(clock), now).await? {
        Ok(DigestTurn::Claimed { previous: clock })
    } else {
        Ok(DigestTurn::ClaimedElsewhere)
    }
}

/// Give a claimed send back when the digest could not be delivered: the clock
/// goes from `claimed` back to `previous`, so the next tick sends again. Only
/// if the clock is still at `claimed` (a later claim is left alone).
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn give_back_digest(
    pool: &PgPool,
    rule_id: &str,
    claimed: OffsetDateTime,
    previous: OffsetDateTime,
) -> Result<(), StoreError> {
    move_digest_clock(pool, rule_id, Some(claimed), previous).await?;
    Ok(())
}

/// Start a digest's clock over: it is read as never seen again, so its next send
/// is the first schedule time after the next tick. Used when its schedule or
/// zone is edited, so a new schedule does not send at once for a time that has
/// already passed under the old one.
///
/// # Errors
///
/// [`StoreError::Database`] if the statement fails.
pub async fn clear_digest_clock(pool: &PgPool, rule_id: &str) -> Result<(), StoreError> {
    sqlx::query("UPDATE alert_rule_state SET digest_clock_at = NULL WHERE rule_id = $1")
        .bind(rule_id)
        .execute(pool)
        .await?;
    Ok(())
}
