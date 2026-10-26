//! `GET /api/notifications` — real, honest lists of open alert instances
//! and pending approvals for the navbar bell (grand plan §13, WS5 item
//! F1).
//!
//! # No `unreadCount`
//!
//! There is no per-principal "read" cursor anywhere in this schema, so
//! "unread since when" has no honest answer. An earlier draft of this
//! route invented one anyway (WS5 plan review U11) — this route reports
//! what is currently open/pending, in full, never a derived count that
//! implies a read/unread distinction nothing here tracks.
//!
//! # Everything counted is also listed
//!
//! U11 also caught a draft that counted approvals into a total but never
//! listed them, so the navbar dot could light above an empty list — a
//! fabricated-looking notification. Both `openAlerts` and
//! `pendingApprovals` are full lists here, never bare counts.
//!
//! # Mentions are the exception (`BI-19` part E)
//!
//! `unreadMentions` lists the caller's unread mentions in dashboard comments.
//! Those do have a per-person read cursor (`dashboard_comment_mention.read_at`),
//! so "unread" is true for them. Each line is built from the live dashboard
//! and left out when the caller can no longer see it; it never carries the
//! comment's text.
//!
//! # `supported: false`, not a fabricated zero
//!
//! When `state.pg` is not configured, this route reports `supported:
//! false` with a reason, never `openAlerts: []`/`pendingApprovals: []`,
//! which would read as "genuinely nothing waiting" rather than "not
//! measured" (AGENTS.md principle 2).

use axum::Extension;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// `GET /api/notifications`.
///
/// # Errors
///
/// 500 on a database failure while reading either source table.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(ApiJson(json!({
            "supported": false,
            "reason": "no Postgres pool configured for this deployment",
        })));
    };
    let alerts = lakehouse_store::overview::list_alerts(pool).await?;
    let approvals = lakehouse_store::agents::list_pending_approvals(pool).await?;

    let open_alerts: Vec<Value> = alerts
        .iter()
        .filter(|a| a.status == "open")
        .map(|a| {
            json!({
                "id": a.id,
                "title": a.title,
                "severity": a.severity,
                "at": a.at,
            })
        })
        .collect();
    let pending_approvals: Vec<Value> = approvals
        .iter()
        .map(|a| {
            json!({
                "id": a.id,
                "title": a.action,
                "requestedAt": a.requested_at,
            })
        })
        .collect();

    // BI-19 part E (D15): unread mentions in dashboard comments, per person.
    // These have a real read cursor (`dashboard_comment_mention.read_at`), so
    // unlike the two lists above they can be called unread. A reader who cannot
    // reach the comment table's dashboards gets none; a failure to work them
    // out is reported as such, never as an empty list.
    let (mentions, mentions_note) =
        match crate::routes::dashboard_comments::bell_lines(&state, &principal).await {
            Ok(lines) => (lines, Value::Null),
            Err(ApiError::PermissionDenied(_)) => (Vec::new(), Value::Null),
            Err(_) => (Vec::new(), json!("Mentions could not be read right now.")),
        };
    Ok(ApiJson(json!({
        "supported": true,
        "openAlerts": open_alerts,
        "pendingApprovals": pending_approvals,
        "unreadMentions": mentions,
        "unreadMentionsUnavailable": mentions_note,
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;
    use crate::state::AppState;

    /// The honest-unsupported branch (WS5 plan review U11): with no
    /// `DATABASE_URL` configured, `AppState::pg` is `None` and this route
    /// must report `supported: false` with a reason, never an empty-list
    /// shape that would read as "genuinely nothing waiting".
    #[tokio::test]
    async fn list_reports_unsupported_without_postgres() {
        // A malformed `DATABASE_URL` is how `AppState` legitimately ends up
        // with `pg: None` (see `state::tests::app_state_degrades_to_no_pool_on_malformed_database_url`)
        // — the default `Config` carries a real (if unreachable) dev
        // `DATABASE_URL`, so `pg` is `Some` even with no live Postgres.
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        let cfg = Config::from_map(&env).unwrap();
        let state = AppState::new(cfg);
        assert!(state.pg.is_none());

        let principal = Principal {
            id: lakehouse_auth::PrincipalId::User(uuid::Uuid::new_v4()),
            tenant_ids: Vec::new(),
            display_name: "t".to_owned(),
            permissions: lakehouse_auth::PermissionSet::default(),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        };
        let ApiJson(body) = list(State(state), Extension(principal)).await.unwrap();

        assert_eq!(
            body,
            json!({
                "supported": false,
                "reason": "no Postgres pool configured for this deployment",
            })
        );
    }
}
