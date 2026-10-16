//! Quality incidents (`DATA-14`, F2): what is wrong now, by table.
//!
//! A failed run of an authored rule opens an incident
//! ([`open_for_failed_run`]); it stays open until a person resolves it
//! with a reason (`POST /api/governance/quality/incidents/{id}/resolve`,
//! `governance:write`, audited as `quality.incident_resolve`). A passing
//! run does not close one. Deleting a rule resolves its open incidents
//! ([`resolve_for_deleted_rule`]) with the resolution "rule deleted" and
//! the deleting user. The records live in Postgres
//! (`lakehouse_store::quality`, migration `0081`).
//!
//! The legacy rows a quality job recorded in `_silver_meta.quality` have
//! no stable id and are not rules, so they open no incident; only the
//! authored rules and the learned anomalies do.
//!
//! Bookkeeping here never fails the event that caused it: a run whose
//! incident could not be saved is still a saved run, and the next failed
//! run tries again.

use axum::Extension;
use axum::extract::{Path, Query, State};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::governance::QualityRule;
use lakehouse_store::quality::{self, IncidentKind, Resolution, StatusFilter};
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::quality::pool;
use crate::state::AppState;

/// The resolution recorded when the rule an incident belongs to is deleted.
pub(crate) const RULE_DELETED: &str = "rule deleted";

/// Opens (or moves the `last_seen_at` of) the incident for a rule whose
/// run just failed. The detail is what the run measured and the threshold
/// it was held to, so the list says what is wrong without opening the rule.
pub(crate) async fn open_for_failed_run(state: &AppState, rule: &QualityRule, measured: &str) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    let detail = format!("{} — {measured} (threshold: {})", rule.name, rule.threshold);
    match quality::record_failure(
        pg,
        &rule.asset,
        Some(&rule.id),
        IncidentKind::Check,
        &detail,
    )
    .await
    {
        Ok(recorded) => {
            tracing::debug!(rule = %rule.name, opened = recorded.opened, "quality: incident recorded");
        }
        Err(err) => {
            tracing::warn!(?err, rule = %rule.name, "quality: the incident could not be recorded");
        }
    }
}

/// Opens the incident of an anomaly (`DATA-14`, F5) for `asset`, or moves the
/// `last_seen_at` of the one already open. Returns the incident's id when
/// this call opened it, `None` when it was already open or could not be
/// saved (logged). Bookkeeping after a load or a tick, so it never fails.
pub(crate) async fn open_anomaly(
    state: &AppState,
    asset: &str,
    kind: IncidentKind,
    detail: &str,
) -> Option<String> {
    let pg = state.pg.as_deref()?;
    match quality::record_failure(pg, asset, None, kind, detail).await {
        Ok(recorded) => recorded.opened.then_some(recorded.id),
        Err(err) => {
            tracing::warn!(?err, %asset, kind = kind.as_str(), "quality: the anomaly could not be recorded");
            None
        }
    }
}

/// Resolves the open incidents of a rule that was just deleted.
pub(crate) async fn resolve_for_deleted_rule(
    state: &AppState,
    rule: &QualityRule,
    principal: &Principal,
) {
    let Some(pg) = state.pg.as_deref() else {
        return;
    };
    if let Err(err) =
        quality::resolve_for_rule(pg, &rule.id, principal.id.uuid(), RULE_DELETED).await
    {
        tracing::warn!(?err, rule = %rule.name, "quality: incidents of the deleted rule were not resolved");
    }
}

/// The `GET /api/governance/quality/incidents` query.
#[derive(Debug, serde::Deserialize)]
pub struct IncidentsQuery {
    /// `open` (the default), `resolved` or `all`.
    #[serde(default)]
    status: Option<String>,
    /// Narrow to one table.
    #[serde(default)]
    asset: Option<String>,
}

/// `GET /api/governance/quality/incidents?status=&asset=` — incidents,
/// newest first, at most 500. Readable by any login, like the quality
/// rule list it belongs beside.
///
/// # Errors
///
/// `400` for a `status` that is not `open`, `resolved` or `all`;
/// `503`/`500` on a store failure.
pub async fn list_incidents(
    State(state): State<AppState>,
    Query(query): Query<IncidentsQuery>,
) -> ApiResult<ApiJson<Value>> {
    let status = match query.status.as_deref() {
        None | Some("") => StatusFilter::Open,
        Some(other) => StatusFilter::parse(other).ok_or_else(|| {
            ApiError::BadRequest("status must be open, resolved or all.".to_owned())
        })?,
    };
    let asset = query.asset.as_deref().filter(|a| !a.trim().is_empty());
    let incidents = quality::list(pool(&state)?, status, asset).await?;
    Ok(ApiJson(json!({ "incidents": incidents })))
}

/// The `POST …/incidents/{id}/resolve` body.
#[derive(Debug, serde::Deserialize)]
pub struct ResolveBody {
    resolution: String,
}

/// `POST /api/governance/quality/incidents/{id}/resolve` — close an open
/// incident with the reason a person gives. Audited as
/// `quality.incident_resolve` on the incident's table.
///
/// # Errors
///
/// `400` when the body is malformed or the reason is empty or too long;
/// `404` when no incident has that id; `409` when it was resolved before;
/// `503`/`500` on a store failure.
pub async fn resolve_incident(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: ResolveBody = serde_json::from_slice(&body)
        .map_err(|err| ApiError::BadRequest(format!("Invalid request body: {err}")))?;
    let incident = match quality::resolve(pool(&state)?, &id, principal.id.uuid(), &body.resolution)
        .await?
    {
        Resolution::Resolved(incident) => incident,
        Resolution::AlreadyResolved => {
            return Err(ApiError::Conflict("That incident is already resolved.".to_owned()).into());
        }
        Resolution::NotFound => {
            return Err(ApiError::NotFound("Quality incident not found".to_owned()).into());
        }
    };
    crate::routes::governance::audit_table_change(
        &state,
        Some(&principal),
        &incident.asset,
        "quality.incident_resolve",
        json!({
            "incidentId": incident.id,
            "kind": incident.kind,
            "ruleId": incident.rule_id,
            "resolution": incident.resolution,
        }),
    )
    .await;
    Ok(ApiJson(json!(incident)))
}
