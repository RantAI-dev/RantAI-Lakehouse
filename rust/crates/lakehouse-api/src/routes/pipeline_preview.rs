//! `POST /api/pipelines/preview` (BLD-1 part 1 / D16, decisions 5, 13 and
//! 17): the first rows of ONE input or step of a pipeline definition that may
//! not be saved yet.
//!
//! # Why the API and not the console
//!
//! The preview runs the same three steps the save path (D9) and the run route
//! (D8) run, in the same order, because it must answer what a run would:
//!
//! 1. [`crate::pipeline_compile::compile`] checks D2 to D6 on the part of the
//!    definition up to `upTo` and turns it into one statement. Nothing is
//!    stored, so the editor can preview a step it has not saved.
//! 2. The tenant gate and the policy rewrite, **as the caller**
//!    ([`crate::routes::query::GateActor::Caller`]) — decision 13: a preview
//!    shows what the person looking may see, never what any pipeline's author
//!    may see. This is the same gate and the same
//!    [`crate::policy_engine::rewrite_sql_for_roles`] Query Studio uses.
//! 3. The rewritten statement, wrapped as `SELECT * FROM (<rewritten>) AS step
//!    LIMIT 101` so a preview returns [`PREVIEW_ROWS`] rows and says whether
//!    more existed. Nothing that names a table reaches `ClickHouse` before
//!    steps 1 and 2 succeed, not even the column probe.
//!
//! # What is refused, and as what
//!
//! - A shape, limit, name or step refusal ([`crate::pipeline_compile`]): 400,
//!   the message built by
//!   [`crate::routes::authored_pipelines::definition_message`] so it starts
//!   with `inputs[i]`, `steps[i]` or `upTo` (D7).
//! - The gate's or the rewrite's refusal: 422 with the fixed sentence this
//!   code base wrote, the same status the save path gives it. An outage keeps
//!   its own status — an unreachable database is not a bad request.
//! - A `ClickHouse` failure: reported under a fixed sentence and a reference
//!   id, through [`crate::upstream_error`].
//!
//! No input table or column name comes from anywhere but the caller's own
//! request; `ClickHouse`'s text never reaches the response.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::pipelines::{PipelineInput, PipelineStep};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::authored_pipelines::{
    catalog_db, definition_message, gate_refusal_as_unprocessable,
};
use crate::routes::query::{GateActor, StatementGate};
use crate::routes::support::placeholders_for;
use crate::state::AppState;
use crate::upstream_error;

/// Rows one preview returns (decision 5, `BLD-1-AC3`).
const PREVIEW_ROWS: usize = 100;

/// One row more than the cap is asked of the engine, so `truncated` tells a
/// cut from an exact fit — the same reason Query Studio asks for one more than
/// it keeps.
const PREVIEW_ROW_LIMIT: u32 = 101;

/// `POST /api/pipelines/preview` body (D16).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviewBody {
    /// D1's inputs, exactly as a save would carry them.
    #[serde(default)]
    inputs: Vec<PipelineInput>,
    /// D1's steps, exactly as a save would carry them.
    #[serde(default)]
    steps: Vec<PipelineStep>,
    /// The input or step to preview: the definition is compiled up to here.
    #[serde(default)]
    up_to: String,
}

/// `POST /api/pipelines/preview` — compile `upTo`, run it as the caller, and
/// answer its columns, the first [`PREVIEW_ROWS`] rows, and whether more
/// existed.
///
/// # Errors
///
/// 400 for a body that does not parse, a missing `upTo`, or a definition
/// refusal naming `inputs[i]`/`steps[i]`/`upTo`; 422 when the gate or the
/// rewrite refuses; a `ClickHouse` failure as `POST /api/query/run` reports it;
/// an outage of the deployment's own databases keeps its status.
pub async fn preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let body: PreviewBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("body JSON is invalid".to_owned()))?;
    let up_to = body.up_to.trim();
    if up_to.is_empty() {
        return Err(ApiError::BadRequest("upTo is required.".to_owned()).into());
    }
    // D16: D2 to D6 on the part up to `upTo`, and nothing beyond it.
    let compiled = crate::pipeline_compile::compile(
        &body.inputs,
        &body.steps,
        Some(up_to),
        catalog_db(&state),
    )
    .map_err(|err| ApiError::BadRequest(definition_message(&err)))?;
    let rewritten = rewrite_as_caller(&state, &principal, &headers, &compiled).await?;
    let columns = crate::routes::support::describe_columns(&state.clickhouse, &rewritten)
        .await
        .map_err(|err| crate::upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    if columns.is_empty() {
        return Err(ApiError::Unprocessable("The preview returns no columns.".to_owned()).into());
    }
    let sql = preview_sql(&rewritten);
    let result = state
        .clickhouse
        .query(&sql, None)
        .await
        .map_err(|err| crate::upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    Ok(ApiJson(answer(&columns, &result.data)))
}

/// The tenant gate and the policy rewrite for `sql`, as the CALLER (D16,
/// decision 13) — the same `StatementGate` and
/// [`crate::policy_engine::rewrite_sql_for_roles`] Query Studio runs.
///
/// A refusal is a 422 with the fixed sentence the gate or the rewrite wrote:
/// the preview's contract has one status for "an access rule refuses this",
/// as the save path does (D9). An outage keeps its own status
/// ([`gate_refusal_as_unprocessable`]).
///
/// # Errors
///
/// 422 for the gate's and the rewrite's refusals; the outage's own status.
async fn rewrite_as_caller(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
    sql: &str,
) -> Result<String, ApiError> {
    StatementGate::clickhouse(state, GateActor::Caller { principal, headers })
        .check(sql)
        .await
        .map_err(gate_refusal_as_unprocessable)?;
    let obligations = crate::policy_engine::PolicyEngineObligations::from_state(state);
    crate::policy_engine::rewrite_sql_for_roles(
        sql,
        &sqlparser::dialect::ClickHouseDialect {},
        crate::sql_rewrite::Engine::ClickHouse,
        &principal.role_names,
        &placeholders_for(principal),
        &obligations,
    )
    .await
    .map_err(|err| ApiError::Unprocessable(crate::policy_engine::enforcement_error_message(&err)))
}

/// D16's wrapper: `SELECT * FROM (<rewritten>) AS step LIMIT 101`, one row
/// past [`PREVIEW_ROWS`] so [`answer`] can say whether anything was cut.
fn preview_sql(rewritten: &str) -> String {
    format!("SELECT * FROM (\n{rewritten}\n) AS step LIMIT {PREVIEW_ROW_LIMIT}")
}

/// The response body: the probed columns, the rows as arrays in column order
/// (D16's wire shape, not the objects `ClickHouse` answers with), and whether
/// the row cap cut them.
fn answer(
    columns: &[lakehouse_bi::sources::SourceColumn],
    data: &[serde_json::Map<String, Value>],
) -> Value {
    let mut rows: Vec<Vec<Value>> = data
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| row.get(&column.name).cloned().unwrap_or(Value::Null))
                .collect()
        })
        .collect();
    let truncated = rows.len() > PREVIEW_ROWS;
    rows.truncate(PREVIEW_ROWS);
    json!({ "columns": columns, "rows": rows, "truncated": truncated })
}
