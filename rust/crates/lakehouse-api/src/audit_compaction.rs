//! The audit trail records the first failure of a window and a count, not
//! every failure (`SEC-75`, plan `adm-130`, decisions 3 to 6).
//!
//! # Why
//!
//! `audit_event` is append-only (migration `0142`), so one row per failed
//! sign-in is a row an anonymous caller can grow without limit, and one
//! row per refused request at the gate is a row a signed-in caller can
//! grow without limit. Nothing trims the table.
//!
//! # The rule
//!
//! Per source (a sign-in) and per principal and route (a refusal at the
//! gate), per window of fifteen minutes:
//!
//! - the FIRST failure writes the row of today, and
//! - the later ones only count, and
//! - the sweep writes ONE summary row when the window has closed, with
//!   the count of the failures that came after the first row.
//!
//! A window with nothing after its first row writes no summary: there is
//! nothing to summarise.
//!
//! # The counters
//!
//! The sign-in counter is [`lakehouse_auth::source_throttle`]'s table
//! (`login_source_throttle`, migration `0153`); this module owns the
//! gate's counter (`gate_refusal_throttle`, migration `0154`) and the
//! sweep of both.
//!
//! # The sweep
//!
//! [`sweep`] runs every minute (see `crate::lib`'s spawn) and does one
//! thing per closed window: write the summary row, and THEN delete the
//! counter row. The delete comes after the write: a counter row deleted
//! before its summary landed would lose the count with nothing to show
//! for it. A window whose summary could not be written keeps its counter
//! row, and the next sweep tries again.
//!
//! # What a row may hold
//!
//! The summary row of a sign-in names the source the failures came from
//! and the count — never the identifier, never the password, never a hash
//! of either (decision 5). The summary row of a gate refusal names the
//! route and the count, and carries the principal the refusal was
//! written for.

use lakehouse_auth::Principal;
use lakehouse_store::StoreError;
use lakehouse_store::audit::NewAuditEvent;
use serde_json::json;
use time::{Duration, OffsetDateTime};

use crate::audit_coverage;
use crate::state::AppState;

/// How long a refusal counts toward the window it is in.
///
/// The same window the sign-in counter uses: one window, one rule.
const WINDOW: Duration = lakehouse_auth::source_throttle::WINDOW;

/// The statement that counts one refusal at the gate.
///
/// One atomic statement, like the sign-in counter's: two refusals at the
/// same time both count, and the count never splits over two windows.
/// The `RETURNING` tells the caller whether this refusal is the first of
/// its window: the count is `1` exactly when this call created the row.
const RECORD_REFUSAL_SQL: &str = "INSERT INTO gate_refusal_throttle \
     (principal_id, principal_kind, actor_label, method, route_pattern, action, resource_kind, \
      window_started_at, refusals, first_row_written) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, \
             to_timestamp(floor(extract(epoch from now()) / 900) * 900), 1, FALSE) \
     ON CONFLICT (principal_id, method, route_pattern, window_started_at) DO UPDATE \
       SET refusals = gate_refusal_throttle.refusals + 1 \
     RETURNING refusals";

/// The statement that says the first refused row of a window was written.
const MARK_FIRST_ROW_SQL: &str = "UPDATE gate_refusal_throttle SET first_row_written = TRUE \
     WHERE principal_id = $1 AND method = $2 AND route_pattern = $3 AND window_started_at = $4";

/// The statement that reads the windows that have closed.
const CLOSED_REFUSALS_SQL: &str = "SELECT principal_id, principal_kind, actor_label, method, \
     route_pattern, action, resource_kind, window_started_at, refusals, first_row_written \
     FROM gate_refusal_throttle \
     WHERE window_started_at < to_timestamp(floor(extract(epoch from now()) / 900) * 900) \
     ORDER BY window_started_at, principal_id, method, route_pattern";

/// The statement that deletes the counter row of one summarised window.
const DELETE_REFUSAL_WINDOW_SQL: &str = "DELETE FROM gate_refusal_throttle \
     WHERE principal_id = $1 AND method = $2 AND route_pattern = $3 AND window_started_at = $4";

/// One closed window of the gate-refusal counter.
#[derive(Debug, Clone, sqlx::FromRow)]
struct RefusalWindow {
    /// The principal the refusals were written for.
    principal_id: String,
    /// The kind of that principal (`user`, `service`, ...).
    principal_kind: Option<String>,
    /// The principal's display name.
    actor_label: String,
    /// The method of the route that refused.
    method: String,
    /// The pattern of the route that refused.
    route_pattern: String,
    /// The route's own audit action.
    action: String,
    /// The kind of object the route changes.
    resource_kind: Option<String>,
    /// When the window started.
    window_started_at: OffsetDateTime,
    /// How many refusals the window holds.
    refusals: i32,
    /// Whether the first refused row of the window was written.
    first_row_written: bool,
}

/// What one sweep wrote and deleted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepCounts {
    /// How many summary rows the sweep wrote.
    pub summaries: u32,
    /// How many counter rows the sweep deleted.
    pub deleted: u32,
}

/// What one refusal at the gate did to the counter.
#[derive(Debug, Clone, Copy)]
pub struct RefusalCount {
    /// Whether this refusal is the FIRST of its window: the one that has
    /// to write the refused row.
    pub first: bool,
    /// When the window started.
    ///
    /// The caller marks the window with THIS value, not with a window it
    /// computes again: the window that closed between the count and the
    /// mark would leave the counter row unmarked, and the summary would
    /// count one refusal too many.
    pub window_started_at: OffsetDateTime,
}

/// Count one refusal at the gate, and say whether it is the FIRST refusal
/// of this principal, this route and this window.
///
/// `Some` means the refusal was counted; `first` says whether the caller
/// has to write the refused row. `None` means the counter could not be
/// written: the caller then writes the row, which is what the trail
/// looked like before the compaction, and never for fewer.
#[must_use]
pub async fn count_refusal(
    state: &AppState,
    principal: &Principal,
    method: &str,
    pattern: &str,
    action: &str,
    resource_kind: Option<&str>,
) -> Option<RefusalCount> {
    let pool = state.pg.as_deref()?;
    let (principal_id, principal_kind, actor_label) = audit_coverage::actor(Some(principal));
    let principal_id = principal_id?;
    let (refusals, window_started_at): (i32, OffsetDateTime) =
        match sqlx::query_as(RECORD_REFUSAL_SQL)
            .bind(&principal_id)
            .bind(&principal_kind)
            .bind(&actor_label)
            .bind(method)
            .bind(pattern)
            .bind(action)
            .bind(resource_kind)
            .fetch_one(pool)
            .await
        {
            Ok(row) => row,
            Err(err) => {
                tracing::warn!(%err, "failed to count a refusal at the gate");
                return None;
            }
        };
    Some(RefusalCount {
        first: refusals == 1,
        window_started_at,
    })
}

/// Say that the first refused row of this principal, route and window has
/// been written to the audit trail.
///
/// The sweep then knows the window holds a first row, and can count the
/// refusals that came after it.
pub async fn mark_first_row_written(
    state: &AppState,
    principal: &Principal,
    method: &str,
    pattern: &str,
    window_started_at: OffsetDateTime,
) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let (principal_id, _, _) = audit_coverage::actor(Some(principal));
    let Some(principal_id) = principal_id else {
        return;
    };
    if let Err(err) = sqlx::query(MARK_FIRST_ROW_SQL)
        .bind(&principal_id)
        .bind(method)
        .bind(pattern)
        .bind(window_started_at)
        .execute(pool)
        .await
    {
        tracing::warn!(
            %err,
            "failed to mark the first refused row of a window as written"
        );
    }
}

/// Write the summary row of every closed window and delete the counter
/// rows it has summarised.
///
/// The counter row is deleted only after its summary landed: a window
/// whose summary could not be written keeps its counter, and the next
/// sweep writes the summary.
///
/// # Errors
///
/// Returns the storage error when the counters cannot be read or the
/// summarised rows cannot be deleted. A summary row that cannot be
/// written is logged and its window keeps its counter row.
pub async fn sweep(state: &AppState) -> Result<SweepCounts, StoreError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(SweepCounts::default());
    };
    let mut counts = SweepCounts::default();

    for window in lakehouse_auth::source_throttle::closed_windows(pool).await? {
        let count = summarised_count(window.failures, window.first_row_written);
        if count > 0 {
            let event = sign_in_summary_event(&window.source, count, window.window_started_at);
            if !audit_coverage::record_best_effort_checked(state, event).await {
                // The counter row stays: the next sweep writes this
                // summary again.
                continue;
            }
            counts.summaries += 1;
        }
        lakehouse_auth::source_throttle::delete_window(
            pool,
            &window.source,
            window.window_started_at,
        )
        .await?;
        counts.deleted += 1;
    }

    for window in sqlx::query_as::<_, RefusalWindow>(CLOSED_REFUSALS_SQL)
        .fetch_all(pool)
        .await?
    {
        let refusals = u32::try_from(window.refusals).unwrap_or(u32::MAX);
        let count = summarised_count(refusals, window.first_row_written);
        if count > 0 {
            let event = refusal_summary_event(&window, count);
            if !audit_coverage::record_best_effort_checked(state, event).await {
                continue;
            }
            counts.summaries += 1;
        }
        sqlx::query(DELETE_REFUSAL_WINDOW_SQL)
            .bind(&window.principal_id)
            .bind(&window.method)
            .bind(&window.route_pattern)
            .bind(window.window_started_at)
            .execute(pool)
            .await?;
        counts.deleted += 1;
    }

    Ok(counts)
}

/// The count a summary row reports: the failures that came after the first
/// row of the window, or all of them when no first row was written.
fn summarised_count(failures: u32, first_row_written: bool) -> u32 {
    if first_row_written {
        failures.saturating_sub(1)
    } else {
        failures
    }
}

/// The summary row of a window of failed sign-ins.
///
/// It names the source the failures came from, the count, and the window
/// (decision 5). It names nothing the visitor typed: not the identifier,
/// not the password, not a hash of either.
fn sign_in_summary_event(source: &str, count: u32, window: OffsetDateTime) -> NewAuditEvent {
    NewAuditEvent {
        action: "auth.login".to_owned(),
        outcome: "refused".to_owned(),
        actor_label: Some(lakehouse_store::audit::ACTOR_SIGN_IN_FORM.to_owned()),
        args: Some(json!({
            "source": source,
            "count": count,
            "windowStartedAt": iso(window),
            "windowEndedAt": iso(window + WINDOW),
        })),
        detail: Some(sign_in_summary_detail(count)),
        ..Default::default()
    }
}

/// The detail of a sign-in summary row.
///
/// "15 minutes" is [`WINDOW`]: the sentence names the window the count
/// covers, so a reader of the trail knows what the count is a count of.
fn sign_in_summary_detail(count: u32) -> String {
    format!("{count} more failed sign-ins from this source in 15 minutes.")
}

/// The summary row of a window of refusals at the gate.
///
/// It names the route that refused and the count (decision 4), and
/// carries the principal the refusals were written for.
fn refusal_summary_event(window: &RefusalWindow, count: u32) -> NewAuditEvent {
    NewAuditEvent {
        principal_id: Some(window.principal_id.clone()),
        principal_kind: window.principal_kind.clone(),
        actor_label: Some(window.actor_label.clone()),
        action: window.action.clone(),
        resource_kind: window.resource_kind.clone(),
        outcome: "refused".to_owned(),
        args: Some(json!({
            "route": window.route_pattern,
            "method": window.method,
            "count": count,
            "windowStartedAt": iso(window.window_started_at),
            "windowEndedAt": iso(window.window_started_at + WINDOW),
        })),
        detail: Some(refusal_summary_detail(count, &window.method, &window.route_pattern)),
        ..Default::default()
    }
}

/// The detail of a refusal summary row.
fn refusal_summary_detail(count: u32, method: &str, route: &str) -> String {
    format!("{count} more refusals of {method} {route} in 15 minutes.")
}

/// A timestamp in ISO 8601, `None` when it cannot be written.
fn iso(at: OffsetDateTime) -> Option<String> {
    at.format(&time::format_description::well_known::Rfc3339).ok()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Every statement of this module counts into the SAME window as the
    /// sign-in counter's statements: two statements that disagree about
    /// the window would split one caller's failures over two windows, and
    /// the count would never be what the caller sees.
    ///
    /// The statement that marks the first row takes the window as a bound
    /// value instead of computing it again: the window that closed between
    /// the count and the mark would leave the counter row unmarked.
    #[test]
    fn every_statement_uses_the_same_window() {
        let window = lakehouse_auth::source_throttle::WINDOW_START_SQL;
        assert!(RECORD_REFUSAL_SQL.contains(window));
        assert!(CLOSED_REFUSALS_SQL.contains(window));
        assert!(!MARK_FIRST_ROW_SQL.contains(window));
    }

    /// The window is the same fifteen minutes the sign-in counter uses.
    #[test]
    fn the_window_is_fifteen_minutes() {
        assert_eq!(WINDOW, Duration::minutes(15));
    }

    /// A window with a first row counts the failures that came after it;
    /// a window whose first row never landed counts all of them.
    #[test]
    fn the_count_is_the_failures_after_the_first_row() {
        assert_eq!(summarised_count(60, true), 59);
        assert_eq!(summarised_count(60, false), 60);
        assert_eq!(summarised_count(1, true), 0);
    }

    /// The detail names the window the count covers.
    #[test]
    fn the_detail_names_the_window_and_the_route() {
        assert_eq!(
            sign_in_summary_detail(59),
            "59 more failed sign-ins from this source in 15 minutes."
        );
        assert_eq!(
            refusal_summary_detail(4, "POST", "/api/identity/roles"),
            "4 more refusals of POST /api/identity/roles in 15 minutes."
        );
    }
}
