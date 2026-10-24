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

/// Why [`stop_own_queries`] did not stop anything.
#[derive(Debug)]
pub(crate) enum StopFailure {
    /// The `runId` is not a UUID, so no pattern was built.
    NotAUuid,
    /// The engine could not be asked; the text stays with the caller's log.
    Engine(lakehouse_clickhouse::ChError),
}

/// Stop `principal`'s own running queries on the engine: the ones of run
/// `run_id`, or every one they have running when `run_id` is `None`. The one
/// implementation behind `POST /api/query/cancel` and the assistant's
/// `stop_query` (`BI-5` re-review R2: the tool had a second copy that did not
/// mark the run stopped and skipped the kill when the count was 0).
///
/// With a run, the run is marked stopped first so a tile that has not sent its
/// query yet answers "Stopped" (`BI-5` review S2). The kill is sent even when
/// the count is 0: a query can start between the count and the kill, and
/// `KILL` of nothing is harmless. Returns the number counted just before the
/// kill.
///
/// # Errors
///
/// [`StopFailure::NotAUuid`] for a `run_id` that is not a UUID,
/// [`StopFailure::Engine`] when the engine fails.
pub(crate) async fn stop_own_queries(
    ch: &lakehouse_clickhouse::ChClient,
    principal: uuid::Uuid,
    run_id: Option<&str>,
) -> Result<u64, StopFailure> {
    let target = match run_id {
        Some(run_id) => KillTarget::own_run(principal, run_id).ok_or(StopFailure::NotAUuid)?,
        None => KillTarget::own_all(principal),
    };
    // `own_run` has already checked the id is a UUID.
    if let Some(run) = run_id.and_then(|id| uuid::Uuid::parse_str(id.trim()).ok()) {
        crate::query_limits::mark_stopped(principal, run);
    }
    let counted = ch
        .rows(&target.count_sql(), None)
        .await
        .map_err(StopFailure::Engine)?;
    let stopped = counted
        .first()
        .and_then(|row| row.get("n"))
        .and_then(|n| {
            n.as_u64()
                .or_else(|| n.as_str().and_then(|t| t.parse().ok()))
        })
        .unwrap_or(0);
    ch.exec(&kill_query_sql(&target), None)
        .await
        .map_err(StopFailure::Engine)?;
    Ok(stopped)
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
    let stopped = stop_own_queries(&state.clickhouse, principal.id.uuid(), Some(&parsed.run_id))
        .await
        .map_err(|failure| match failure {
            StopFailure::NotAUuid => ApiError::BadRequest("runId must be a UUID.".to_owned()),
            StopFailure::Engine(err) => upstream_error::ch_error(&upstream_error::DATABASE, &err),
        })?;
    Ok(ApiJson(json!({ "stopped": stopped })))
}
