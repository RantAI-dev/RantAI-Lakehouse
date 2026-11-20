//! Orchestrator-only recording of a re-upload's downstream chain outcomes
//! (`BLD-14`, `DEC-22`, D8; migration `0153_pipeline_chain_step.sql`).
//!
//! The Dagster chain sensor posts one outcome per (re-upload, pipeline) here
//! so the pipeline page can say why a pipeline did or did not run. The route
//! is under `/runnable/` and admits only the orchestrator's service identity
//! or a Platform Admin, exactly like [`super::authored_pipelines::runnable`]:
//! `pipeline:read` in `POLICY_TABLE` is a floor a person also holds, not the
//! gate. Every field is validated and bound; an upstream or database error's
//! own text is never returned (principles 3 and 4).

use axum::Extension;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_core::ident::Ident;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;

/// `rootRunId` is a Dagster run id: 1 to 64 letters, digits or hyphens.
const ROOT_RUN_ID_INVALID: &str = "rootRunId must be 1 to 64 letters, digits or hyphens";
/// `sourceTable` is a bronze location with a safe table name.
const SOURCE_TABLE_INVALID: &str = "sourceTable must be bronze.<table> with a valid table name";
/// `outcome` is the closed set the table's CHECK accepts.
const OUTCOME_INVALID: &str = "outcome must be started, not_run or superseded";
/// `reason` is the closed set the table's CHECK accepts.
const REASON_INVALID: &str = "reason must be upstream_failed, upstream_cancelled or wait_expired";
/// A started step names one well-formed run; nothing else does.
/// BLD-14 review R5: the field's shape and length are validated, not just
/// its presence, so a malformed id cannot enter a stored record.
const RUN_ID_INVALID: &str =
    "runId must be 1 to 64 letters, digits or hyphens and only for outcome started";
/// A `not_run` step names its cause; nothing else does.
const CAUSE_INVALID: &str = "causePipelineId is required only for outcome not_run";
/// The cause must be in the same tenant as the pipeline whose page shows it.
const CAUSE_SCOPE_INVALID: &str = "causePipelineId must name a pipeline in the same tenant";

/// The body the chain sensor posts, mirroring the store's `ChainStep`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainStepBody {
    /// The `file_ingest_job` run that loaded the table.
    pub root_run_id: String,
    /// The `bronze.<table>` the re-upload loaded.
    pub source_table: String,
    /// `started`, `not_run` or `superseded`.
    pub outcome: String,
    /// Why a `not_run` pipeline did not run.
    pub reason: Option<String>,
    /// The started run's id.
    pub run_id: Option<String>,
    /// The pipeline that stopped the chain.
    pub cause_pipeline_id: Option<String>,
}

impl ChainStepBody {
    /// Check the field shapes and the outcome/reason/run/cause pairing
    /// before anything reaches the store: the sentence names the field that
    /// was wrong, and no upstream text is echoed.
    fn validate(&self) -> Result<(), ApiError> {
        let root = &self.root_run_id;
        if root.is_empty()
            || root.len() > 64
            || !root.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        {
            return Err(ApiError::Unprocessable(ROOT_RUN_ID_INVALID.to_owned()));
        }
        let Some(table) = self.source_table.strip_prefix("bronze.") else {
            return Err(ApiError::Unprocessable(SOURCE_TABLE_INVALID.to_owned()));
        };
        if Ident::new(table).is_err() {
            return Err(ApiError::Unprocessable(SOURCE_TABLE_INVALID.to_owned()));
        }
        let not_run = self.outcome == "not_run";
        let started = self.outcome == "started";
        if !not_run && !started && self.outcome != "superseded" {
            return Err(ApiError::Unprocessable(OUTCOME_INVALID.to_owned()));
        }
        let reason_ok = self
            .reason
            .as_deref()
            .is_none_or(|r| matches!(r, "upstream_failed" | "upstream_cancelled" | "wait_expired"));
        if !reason_ok {
            return Err(ApiError::Unprocessable(REASON_INVALID.to_owned()));
        }
        if started != self.run_id.as_deref().is_some_and(|s| !s.is_empty()) {
            return Err(ApiError::Unprocessable(RUN_ID_INVALID.to_owned()));
        }
        // BLD-14 review R5: validate the run id's shape and length, as the
        // root run id is validated above. A fixed sentence names the field.
        if let Some(run_id) = &self.run_id
            && (run_id.len() > 64
                || !run_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-'))
        {
            return Err(ApiError::Unprocessable(RUN_ID_INVALID.to_owned()));
        }
        if not_run
            != self
                .cause_pipeline_id
                .as_deref()
                .is_some_and(|s| !s.is_empty())
        {
            return Err(ApiError::Unprocessable(CAUSE_INVALID.to_owned()));
        }
        if not_run != self.reason.is_some() {
            return Err(ApiError::Unprocessable(REASON_INVALID.to_owned()));
        }
        Ok(())
    }
}

/// `POST /api/pipelines/runnable/{id}/chain-step` — one chain outcome from
/// the orchestrator. The first write for a (root, pipeline) pair wins; a
/// retried sensor post answers `{"recorded": false}` and changes nothing.
///
/// # Errors
///
/// 403 for anyone but the orchestrator's service identity or a Platform
/// Admin; 404 for an unknown authored id or one outside the caller's tenant;
/// 422 for a field that fails [`ChainStepBody::validate`] or a cause in
/// another tenant; 503 with no database; 500 on a store failure.
pub async fn record_chain_step(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<ChainStepBody>,
) -> ApiResult<ApiJson<Value>> {
    if !matches!(principal.id, PrincipalId::Service(_))
        && !crate::routes::catalog::is_unrestricted(&principal)
    {
        return Err(ApiError::PermissionDenied(
            "only the orchestrator's service identity records a pipeline's chain step".to_owned(),
        )
        .into());
    }
    body.validate()?;
    let pool = crate::routes::pipelines::pool(&state)?;
    if !matches!(principal.id, PrincipalId::Service(_)) {
        super::authored_pipelines::in_scope(Some(pool), &principal, &headers, &id).await?;
    }
    if !id.starts_with("pl-") {
        return Err(ApiError::NotFound(format!("Pipeline {id} not found")).into());
    }
    let Some(pipeline) = lakehouse_store::pipelines::get_pipeline(pool, &id).await? else {
        return Err(ApiError::NotFound(format!("Pipeline {id} not found")).into());
    };
    // A cause outside the tenant would let a reader learn another tenant's
    // pipeline name through `causePipelineName` (section 3).
    if let Some(cause_id) = &body.cause_pipeline_id {
        let cause = lakehouse_store::pipelines::get_pipeline(pool, cause_id).await?;
        if cause
            .as_ref()
            .is_none_or(|c| c.tenant_id != pipeline.tenant_id)
        {
            return Err(ApiError::Unprocessable(CAUSE_SCOPE_INVALID.to_owned()).into());
        }
    }
    let step = lakehouse_store::pipeline_chain::ChainStep {
        root_run_id: body.root_run_id,
        pipeline_id: id,
        source_table: body.source_table,
        outcome: body.outcome,
        reason: body.reason,
        run_id: body.run_id,
        cause_pipeline_id: body.cause_pipeline_id,
    };
    let recorded = lakehouse_store::pipeline_chain::record_chain_step(pool, &step).await?;
    Ok(ApiJson(json!({ "recorded": recorded })))
}
