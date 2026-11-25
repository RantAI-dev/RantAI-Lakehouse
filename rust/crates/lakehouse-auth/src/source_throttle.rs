//! The per-source limit on failed sign-ins (`SEC-75`, plan `adm-130`,
//! decision 2).
//!
//! # Why a second throttle
//!
//! The per-account throttle ([`crate::throttle`]) is keyed by a hash of
//! the submitted identifier, so it protects one account. It cannot
//! protect the installation: every attempt with a NEW identifier is a
//! new key, so an anonymous caller can guess forever, and each guess
//! costs the API an `Argon2` hash and (before decision 3) a row in the
//! audit trail.
//!
//! This module counts the failures per SOURCE — the address the request
//! came from (`lakehouse_api::client_source`) — so one caller runs out
//! of budget long before it can spend the whole installation's.
//!
//! # The numbers
//!
//! [`MAX_FAILURES_PER_SOURCE`] failures per [`WINDOW`]. 50, not 5,
//! because one office behind one NAT shares one address: the per-account
//! lock (5 in 15 minutes) still guards every single account. They are
//! constants, not settings: a setting here would be a knob an operator
//! turns to "make the login form louder".
//!
//! # The window
//!
//! A fixed wall-clock window, computed IN SQL (see [`WINDOW_START_SQL`]):
//! every process of the API counts into the same window, so a caller
//! cannot slip a failure into a window of its own by landing between
//! two processes' reads.

use time::{Duration, OffsetDateTime};

use crate::{AuthError, PgPool};

/// How many failed sign-ins one source may have in one window before
/// every further sign-in from that source is refused with `429`.
pub const MAX_FAILURES_PER_SOURCE: u32 = 50;

/// How long a failure counts toward [`MAX_FAILURES_PER_SOURCE`].
pub const WINDOW: Duration = Duration::minutes(15);

/// The start of the window a failure falls into, computed by the
/// database.
///
/// The expression is the same in every statement of this module, and
/// `tests::both_statements_use_the_same_window` holds them to it: two
/// statements that disagree about the window would split one caller's
/// failures over two windows, and the limit would never be reached.
///
/// It is public because the audit sweep (`SEC-75` decision 3) has to
/// find the windows that have closed with the SAME expression: a sweep
/// that computed the window differently would summarise a window that
/// is still open, or skip one that is not.
pub const WINDOW_START_SQL: &str = "to_timestamp(floor(extract(epoch from now()) / 900) * 900)";

/// The failures counted for one source in the current window.
const CURRENT_SQL: &str = "SELECT failures, window_started_at, first_row_written \
     FROM login_source_throttle \
     WHERE source = $1 \
       AND window_started_at = to_timestamp(floor(extract(epoch from now()) / 900) * 900)";

/// The one statement that counts a failure.
const RECORD_SQL: &str = "INSERT INTO login_source_throttle \
     (source, window_started_at, failures, first_row_written) \
     VALUES ($1, to_timestamp(floor(extract(epoch from now()) / 900) * 900), 1, FALSE) \
     ON CONFLICT (source, window_started_at) DO UPDATE \
       SET failures = login_source_throttle.failures + 1 \
     RETURNING failures, window_started_at, first_row_written";

/// Count one failed sign-in for `source` in the current window.
///
/// One atomic statement: two failures at the same time both count, and
/// the count never splits over two windows (the window is the key).
///
/// The count starts at 1 in a window nobody failed in yet, and every
/// later failure in that window adds one. A caller that wants to know
/// whether it was the FIRST failure of the window (the audit step
/// writes the first refused row then) reads it off the count: it is 1
/// exactly when this call created the row.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn record_failure(pool: &PgPool, source: &str) -> Result<SourceWindow, AuthError> {
    let (failures, window_started_at, first_row_written): (i32, OffsetDateTime, bool) =
        sqlx::query_as(RECORD_SQL)
            .bind(source)
            .fetch_one(pool)
            .await?;
    Ok(SourceWindow {
        failures: u32::try_from(failures).unwrap_or(u32::MAX),
        window_started_at,
        first_row_written,
    })
}

/// What one source's counter says about the current window.
///
/// `None` when the source has no failure in the current window — either
/// it never failed, or its last failure was in an earlier window.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn current(pool: &PgPool, source: &str) -> Result<Option<SourceWindow>, AuthError> {
    let row: Option<(i32, OffsetDateTime, bool)> = sqlx::query_as(CURRENT_SQL)
        .bind(source)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(
        |(failures, window_started_at, first_row_written)| SourceWindow {
            failures: u32::try_from(failures).unwrap_or(u32::MAX),
            window_started_at,
            first_row_written,
        },
    ))
}

/// Say that the first refused row of this source and window has been
/// written to the audit trail.
///
/// The audit step (decision 3) calls this after writing that row; the
/// sweep then knows the window holds a first row and can write its
/// summary with the count of the failures that came after it.
///
/// # Errors
///
/// Returns [`AuthError::Database`] on a storage failure.
pub async fn mark_first_row_written(
    pool: &PgPool,
    source: &str,
    window_started_at: OffsetDateTime,
) -> Result<(), AuthError> {
    sqlx::query(
        "UPDATE login_source_throttle SET first_row_written = TRUE \
         WHERE source = $1 AND window_started_at = $2",
    )
    .bind(source)
    .bind(window_started_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// One closed window of the per-source counter.
///
/// The sweep (`SEC-75` decision 3) reads the windows that have closed,
/// writes one summary row for each of them, and deletes the counter
/// row it has summarised.
#[derive(Debug, Clone)]
pub struct ClosedWindow {
    /// The source the failures came from.
    pub source: String,
    /// When the window started (a wall-clock boundary, computed by the
    /// database).
    pub window_started_at: OffsetDateTime,
    /// How many failed sign-ins the window holds.
    pub failures: u32,
    /// Whether the first refused row of the window was written to the
    /// audit trail.
    pub first_row_written: bool,
}

/// The windows that have closed: the ones that started before the
/// current window started.
///
/// # Errors
///
/// Returns the storage error on a failed read. The sweep reads the
/// windows it has to summarise; a read that fails leaves every window
/// in place, and the next sweep tries again.
pub async fn closed_windows(pool: &PgPool) -> Result<Vec<ClosedWindow>, sqlx::Error> {
    let rows: Vec<(String, i32, OffsetDateTime, bool)> = sqlx::query_as(
        "SELECT source, failures, window_started_at, first_row_written \
         FROM login_source_throttle \
         WHERE window_started_at < to_timestamp(floor(extract(epoch from now()) / 900) * 900) \
         ORDER BY window_started_at, source",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(source, failures, window_started_at, first_row_written)| ClosedWindow {
                source,
                failures: u32::try_from(failures).unwrap_or(u32::MAX),
                window_started_at,
                first_row_written,
            },
        )
        .collect())
}

/// Delete the counter row of one window.
///
/// The sweep calls this AFTER the summary row landed: a counter row
/// that is deleted before its summary is written loses the count with
/// nothing to show for it.
///
/// # Errors
///
/// Returns the storage error on a failed delete.
pub async fn delete_window(
    pool: &PgPool,
    source: &str,
    window_started_at: OffsetDateTime,
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM login_source_throttle WHERE source = $1 AND window_started_at = $2")
        .bind(source)
        .bind(window_started_at)
        .execute(pool)
        .await?;
    Ok(())
}

/// One source's counter, as of the current window.
#[derive(Debug, Clone, Copy)]
pub struct SourceWindow {
    /// How many failed sign-ins this source has in this window.
    pub failures: u32,
    /// When the window started (a wall-clock boundary, computed by the
    /// database).
    pub window_started_at: OffsetDateTime,
    /// Whether the first refused row of this source and window has been
    /// written to the audit trail.
    pub first_row_written: bool,
}

impl SourceWindow {
    /// How many seconds until this window ends — the `Retry-After` of a
    /// sign-in refused because the source is over its limit.
    ///
    /// Never below one second: a window that ends in less than a second
    /// still has to be waited out, and a `Retry-After: 0` would send the
    /// caller straight back into the refusal.
    #[must_use]
    pub fn retry_after_secs(&self) -> u64 {
        let ends_at = self.window_started_at + WINDOW;
        let remaining = ends_at - OffsetDateTime::now_utc();
        u64::try_from(remaining.whole_seconds()).unwrap_or(1).max(1)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn both_statements_use_the_same_window() {
        // The window expression is written out in both statements, so
        // they can drift; this test is what holds them together.
        assert!(CURRENT_SQL.contains(WINDOW_START_SQL));
        assert!(RECORD_SQL.contains(WINDOW_START_SQL));
    }

    #[test]
    fn the_window_is_fifteen_minutes() {
        assert_eq!(WINDOW, Duration::minutes(15));
        assert_eq!(WINDOW.whole_seconds(), 900);
    }

    #[test]
    fn the_limit_is_fifty_failures_per_window() {
        assert_eq!(MAX_FAILURES_PER_SOURCE, 50);
    }
}
