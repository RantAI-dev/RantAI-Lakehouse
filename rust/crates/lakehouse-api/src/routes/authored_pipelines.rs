//! Console-authored pipelines (`pl-*`) as real orchestrator jobs.
//!
//! # How an authored pipeline runs
//!
//! `dagster/dispar_orchestrate/authored_factory.py` builds one job
//! (`authored__<id>`) and, when the authored schedule is a cron, one
//! schedule (`authored__<id>_schedule`) per pipeline in status `ready` or
//! `paused`. It reads them from [`runnable`] when the code location is
//! imported. So the orchestrator only learns about a change when the code
//! location reloads: every write here that changes what should exist (mark
//! ready, edit, delete, pause, resume) ends by asking it to reload
//! ([`reload_orchestrator`]). The code location runs `dagster code-server
//! start`, which re-imports its module on reload; the plain `dagster api
//! grpc` server it used before would not have.
//!
//! # Who may read every tenant's definitions
//!
//! [`runnable`] lists authored pipelines across all tenants, as
//! `GET /api/connectors/ingestible` does for connectors, because the
//! orchestrator runs every tenant's work. The policy floor is
//! `pipeline:read`, which people hold too, so the handler additionally
//! refuses anyone who is neither a service identity nor a Platform Admin
//! (`*:*`, who already reads across tenants: `catalog::is_unrestricted`,
//! the same rule the shared catalog applies). Anyone else reading it would
//! see other tenants' pipelines.
//!
//! # What reaches a response
//!
//! A reload failure is the orchestrator's own text (often a Python
//! traceback). It is logged and never returned; the response says only
//! whether the orchestrator picked the change up (AGENTS.md principle 4).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_store::pipelines::{self, UpdatePipelineInput};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiRejection, ApiResult};
use crate::json::ApiJson;
use crate::routes::pipelines::record_pipeline_audit;
use crate::state::AppState;

/// Ticks read per request: enough to see a day of a 15-minute schedule.
const MAX_TICKS: u32 = 50;

/// The Dagster job name `authored_factory.py` gives pipeline `id`:
/// `authored__` plus the id with every character outside `[A-Za-z0-9_]`
/// replaced by `_` (Python's `_dagster_safe_name`; ids are ASCII slugs, so
/// `str.isalnum` and `char::is_ascii_alphanumeric` agree on them).
#[must_use]
pub fn job_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("authored__{safe}")
}

/// The schedule name `authored_factory.py` gives pipeline `id`'s job.
#[must_use]
pub fn schedule_name(id: &str) -> String {
    format!("{}_schedule", job_name(id))
}

/// Ask the orchestrator to rebuild its authored jobs. Best effort: the
/// write that called this has already succeeded, and a failed reload is
/// reported as `false`, never as the write failing. The reason is logged,
/// not returned.
pub async fn reload_orchestrator(state: &AppState) -> bool {
    match state.dagster.reload_location().await {
        Ok(outcome) => match outcome.error {
            None => true,
            Some(err) => {
                tracing::warn!(%err, "authored pipelines: orchestrator reload was refused");
                false
            }
        },
        Err(err) => {
            tracing::warn!(%err, "authored pipelines: orchestrator unreachable for reload");
            false
        }
    }
}

/// Whether the orchestrator has a job for pipeline `id` right now.
pub async fn job_is_loaded(state: &AppState, id: &str) -> bool {
    let name = job_name(id);
    state
        .dagster
        .list_jobs()
        .await
        .is_ok_and(|jobs| jobs.contains(&name))
}

/// `GET /api/pipelines/runnable` — every `ready`/`paused` authored
/// pipeline with its definition, for the orchestrator's job factory.
///
/// # Errors
///
/// 403 for anyone but a service identity or a Platform Admin (see the
/// module doc); 503 with
/// no database; 500 on a database failure.
pub async fn runnable(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    if !matches!(principal.id, PrincipalId::Service(_))
        && !crate::routes::catalog::is_unrestricted(&principal)
    {
        return Err(ApiError::PermissionDenied(
            "only the orchestrator's service identity reads every tenant's pipelines".to_owned(),
        )
        .into());
    }
    let pool = crate::routes::pipelines::pool(&state)?;
    let list = pipelines::list_runnable_pipelines(pool).await?;
    Ok(ApiJson(json!({ "pipelines": list })))
}

/// `PUT /api/pipelines/{id}` body: an authored pipeline's editable fields.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateBody {
    kind: String,
    source_zone: String,
    source_table: String,
    #[serde(default)]
    incremental_column: Option<String>,
    #[serde(default)]
    transforms: Vec<String>,
    #[serde(default)]
    fbic_enabled: bool,
    target_zone: String,
    target_table: String,
    schedule: String,
    #[serde(default)]
    owner: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

fn authored_only(id: &str) -> Result<(), ApiRejection> {
    if id.starts_with("pl-") {
        Ok(())
    } else {
        // A Dagster job is defined in code; it is edited there, not here.
        Err(ApiError::NotFound(format!("Pipeline {id} not found")).into())
    }
}

/// The body every authored write returns: the pipeline, plus whether the
/// orchestrator picked the change up.
fn with_orchestrator(pipeline: &pipelines::Pipeline, reloaded: bool) -> Value {
    let mut body = serde_json::to_value(pipeline).unwrap_or_else(|_| json!({}));
    if let Value::Object(obj) = &mut body {
        obj.insert("orchestratorReloaded".to_owned(), json!(reloaded));
    }
    body
}

/// `PUT /api/pipelines/{id}` — replace an authored pipeline's definition.
/// Transforms go through the same grammar as `POST /api/pipelines`.
///
/// # Errors
///
/// 404 for an unknown or non-authored id; 400 for a malformed body or a
/// transform outside the grammar; 503/500 from the store.
pub async fn update(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    authored_only(&id)?;
    let body: UpdateBody = crate::routes::pipelines::parse_body(&body)?;
    for (index, transform) in body.transforms.iter().enumerate() {
        crate::transform_grammar::parse_transform(transform).map_err(|err| {
            ApiError::BadRequest(format!("invalid transform at transforms[{index}]: {err}"))
        })?;
    }
    let input = UpdatePipelineInput {
        kind: body.kind,
        source_zone: body.source_zone,
        source_table: body.source_table,
        incremental_column: body.incremental_column.filter(|c| !c.trim().is_empty()),
        transforms: body.transforms,
        fbic_enabled: body.fbic_enabled,
        target_zone: body.target_zone,
        target_table: body.target_table,
        schedule: body.schedule,
        owner: body.owner.filter(|o| !o.trim().is_empty()),
        description: body.description.filter(|d| !d.trim().is_empty()),
    };
    let pool = crate::routes::pipelines::pool(&state)?;
    let updated = pipelines::update_pipeline(pool, &id, &input)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Pipeline {id} not found")))?;
    record_pipeline_audit(&state, &principal, "pipeline.update", &id).await;
    // A draft has no job; only a runnable pipeline's job has to be rebuilt.
    let reloaded = if updated.status == "draft" {
        false
    } else {
        reload_orchestrator(&state).await
    };
    Ok(ApiJson(with_orchestrator(&updated, reloaded)))
}

/// `DELETE /api/pipelines/{id}` — delete an authored pipeline. Its past
/// runs stay in the orchestrator's history; its job and schedule go away
/// on the reload that follows.
///
/// # Errors
///
/// 404 for an unknown or non-authored id; 503/500 from the store.
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> Response {
    if let Err(rejection) = authored_only(&id) {
        return rejection.into_response();
    }
    let pool = match crate::routes::pipelines::pool(&state) {
        Ok(pool) => pool,
        Err(err) => return ApiRejection(err).into_response(),
    };
    match pipelines::delete_pipeline(pool, &id).await {
        Ok(true) => {
            record_pipeline_audit(&state, &principal, "pipeline.delete", &id).await;
            let reloaded = reload_orchestrator(&state).await;
            (
                StatusCode::OK,
                ApiJson(json!({ "id": id, "deleted": true, "orchestratorReloaded": reloaded })),
            )
                .into_response()
        }
        Ok(false) => {
            ApiRejection(ApiError::NotFound(format!("Pipeline {id} not found"))).into_response()
        }
        Err(err) => ApiRejection(err.into()).into_response(),
    }
}

/// `GET /api/pipelines/{id}/schedule-ticks` — the schedule's recent
/// evaluations, newest first. A Dagster job's schedule is its first one
/// (the same rule `schedule_label` follows); an authored pipeline's is
/// `authored__<id>_schedule`. No schedule reads as an empty list with
/// `schedule: null`; an unreachable orchestrator as `unavailable`.
pub async fn schedule_ticks(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let schedule = if id.starts_with("pl-") {
        Some(schedule_name(&id))
    } else {
        match state.dagster.list_jobs_with_schedules().await {
            Ok(jobs) => jobs
                .iter()
                .find(|j| j.name == id)
                .and_then(|j| j.schedules.first())
                .map(|s| s.name.clone()),
            Err(err) => {
                tracing::warn!(%err, "schedule ticks: orchestrator unreachable");
                return unavailable();
            }
        }
    };
    let Some(schedule) = schedule else {
        return (
            StatusCode::OK,
            ApiJson(json!({ "schedule": Value::Null, "ticks": [], "unavailable": Value::Null })),
        )
            .into_response();
    };
    match state.dagster.schedule_ticks(&schedule, MAX_TICKS).await {
        Ok(ticks) => (
            StatusCode::OK,
            ApiJson(json!({ "schedule": schedule, "ticks": ticks, "unavailable": Value::Null })),
        )
            .into_response(),
        Err(err) => {
            tracing::warn!(%err, "schedule ticks: orchestrator unreachable");
            unavailable()
        }
    }
}

fn unavailable() -> Response {
    (
        StatusCode::OK,
        ApiJson(json!({
            "schedule": Value::Null,
            "ticks": [],
            "unavailable": "the orchestrator could not be reached",
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use lakehouse_auth::PermissionSet;
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    fn principal(id: PrincipalId) -> Principal {
        Principal {
            id,
            tenant_ids: Vec::new(),
            display_name: "fixture".to_owned(),
            permissions: PermissionSet::parse("pipeline:read pipeline:write"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn state_without_pool() -> AppState {
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
        AppState::new(Config::from_map(&env).expect("a valid test Config"))
    }

    #[test]
    fn job_and_schedule_names_match_the_python_factory() {
        // `authored_factory._dagster_safe_name("pl-ui-check-flow-mumidzub")`.
        assert_eq!(
            job_name("pl-ui-check-flow-mumidzub"),
            "authored__pl_ui_check_flow_mumidzub"
        );
        assert_eq!(schedule_name("pl-a-1"), "authored__pl_a_1_schedule");
    }

    /// A person holding `pipeline:read` must not read every tenant's
    /// definitions; the refusal comes before the store is touched.
    #[tokio::test]
    async fn runnable_refuses_a_person_even_with_pipeline_read() {
        let err = runnable(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
        )
        .await
        .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::FORBIDDEN);
    }

    /// A service identity passes the check and reaches the store (here
    /// absent, so 503), proving the gate is on the principal kind.
    #[tokio::test]
    async fn runnable_lets_a_service_identity_through_to_the_store() {
        let err = runnable(
            State(state_without_pool()),
            Extension(principal(PrincipalId::Service(Uuid::from_u128(2)))),
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.into_response().status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    /// A Dagster job is defined in code: editing or deleting it here is a
    /// 404, not a silent no-op.
    #[tokio::test]
    async fn a_dagster_job_cannot_be_edited_or_deleted_here() {
        let body = Bytes::from_static(br#"{"kind":"batch","sourceZone":"a","sourceTable":"b","targetZone":"c","targetTable":"d","schedule":"manual"}"#);
        let err = update(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
            Path("gold_export_job".to_owned()),
            body,
        )
        .await
        .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

        let response = delete(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
            Path("gold_export_job".to_owned()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn an_edit_with_a_transform_outside_the_grammar_is_refused() {
        let body = Bytes::from_static(br#"{"kind":"batch","sourceZone":"a","sourceTable":"b","targetZone":"c","targetTable":"d","schedule":"manual","transforms":["filter(1=1; DROP TABLE x)"]}"#);
        let err = update(
            State(state_without_pool()),
            Extension(principal(PrincipalId::User(Uuid::from_u128(1)))),
            Path("pl-x-1".to_owned()),
            body,
        )
        .await
        .unwrap_err();
        assert_eq!(err.into_response().status(), StatusCode::BAD_REQUEST);
    }
}
