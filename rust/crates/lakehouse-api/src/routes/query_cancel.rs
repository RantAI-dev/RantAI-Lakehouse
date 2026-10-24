//! `POST /api/query/cancel { runId }` (`BI-5` plan D5): Stop. Ends, on the
//! engine, the queries the caller started under one run (`X-Run-Id`).
//!
//! # Why a caller can only stop their own queries
//!
//! The statement is `KILL QUERY WHERE query_id LIKE '<caller uuid>:<runId>:%'
//! ASYNC`, built by `ops::KillTarget::own_run`. The principal part is always
//! the authenticated caller's own id, never a request field, and `runId` must
//! be a UUID before it is used, so the pattern holds no wildcard and no quote
//! and cannot reach another person's queries (their ids start with their own
//! id). Stopping someone else's query stays an administrator action in
//! Operations (`/api/ops/workloads/{id}/cancel`, unchanged).
//!
//! The answer is `{ "stopped": n }`, `n` counted from `system.processes`
//! just before the kill (queries that already finished are not counted). A run
//! with nothing running answers `{ "stopped": 0 }`, not an error: Stop is
//! pressed as the last query ends all the time.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::ops::{KillTarget, kill_query_sql};
use crate::state::AppState;
use crate::upstream_error;

const BODY_SHAPE: &str = "body must be {\"runId\": \"<uuid>\"}.";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CancelBody {
    run_id: String,
}

/// `POST /api/query/cancel`.
///
/// # Errors
///
/// 400 when the body is not `{ "runId": "<uuid>" }`; 5xx through
/// `upstream_error` when the engine cannot be asked.
pub async fn cancel(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: CancelBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BODY_SHAPE.to_owned()))?;
    let target = KillTarget::own_run(principal.id.uuid(), &parsed.run_id)
        .ok_or_else(|| ApiError::BadRequest("runId must be a UUID.".to_owned()))?;
    // BI-5 review S2: the run is marked first, so a tile that has not sent its
    // query yet (the next round of a board of more than six) answers "Stopped"
    // instead of starting. `own_run` has already checked the id is a UUID.
    if let Ok(run) = uuid::Uuid::parse_str(parsed.run_id.trim()) {
        crate::query_limits::mark_stopped(principal.id.uuid(), run);
    }
    let ch = &state.clickhouse;
    let counted = ch
        .rows(&target.count_sql(), None)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    let stopped = counted
        .first()
        .and_then(|row| row.get("n"))
        .and_then(|n| {
            n.as_u64()
                .or_else(|| n.as_str().and_then(|t| t.parse().ok()))
        })
        .unwrap_or(0);
    // Sent even when the count is 0: a query can start between the count and
    // here (a tile of the next round), and `KILL` of nothing is harmless
    // (`BI-5` review S2).
    ch.exec(&kill_query_sql(&target), None)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    Ok(ApiJson(json!({ "stopped": stopped })))
}
