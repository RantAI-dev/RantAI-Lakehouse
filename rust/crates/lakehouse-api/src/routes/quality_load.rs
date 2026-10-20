//! What a finished load sets going for the tables it loaded (`DATA-14`,
//! finding F4).
//!
//! The orchestrator's success sensor posts every successful run to
//! `routes::pipelines::run_finished_event`. That event now also tells this
//! module which tables the run loaded, and a background task then runs the
//! rules of those whose quality schedule is `on_load`
//! (`routes::quality::run_on_load`). Three kinds of load are recognised:
//!
//! - an authored pipeline: its target table;
//! - a connector run (`ingest_job`): every table its ingest spec loads
//!   (`source_objects[].target`, always in Bronze). A run's success covers
//!   them all; a run that failed triggers nothing, and the tables it had
//!   already committed are left to their interval;
//! - an upload (`file_ingest_job`): the Bronze table it targeted. An upload
//!   run is confirmed `SUCCESS` with the orchestrator first, as a connector
//!   run is, because the upload row may not be settled yet.
//!
//! What this never does: fail the event. Everything here runs in a spawned
//! task after the event's own checks; a lookup that fails is logged and the
//! load is simply not followed up (the table's interval still covers it). A
//! sensor retry is deduplicated on `(run_id, "quality_load")` in
//! `pipeline_run_event`, the table the other run events dedupe on, so a
//! retried report follows the load up once.

use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::{connectors, pipelines, uploads};
use serde_json::Value;

use super::load_alerts::{self, Found, Outcome};
use super::pipelines::RunFailedBody;
use crate::state::AppState;

/// The `pipeline_run_event.kind` that dedupes a load's follow-up.
const DEDUPE_KIND: &str = "quality_load";

/// The tables a load wrote, as lower-cased `zone.table` keys.
type Loaded = (String, Vec<String>);

/// `zone.table` of an authored pipeline's target as a quality table key
/// (`gold` is served as `serving`, as column lineage keys it).
fn pipeline_table(target: &str) -> Option<String> {
    crate::routes::lineage::columns::zone_key(target).map(|key| key.to_lowercase())
}

/// The Bronze tables a connector's ingest spec loads.
// PR #106 review R1 BLOCKER 3: filter by object when run was scoped to a single object
fn connector_tables(source_objects: &Value, object_filter: Option<&str>) -> Vec<String> {
    source_objects
        .as_array()
        .into_iter()
        .flatten()
        .filter(|object| {
            let Some(filter) = object_filter else {
                return true;
            };
            let name = object.get("name").and_then(Value::as_str);
            let target = object.get("target").and_then(Value::as_str);
            name == Some(filter) || target == Some(filter)
        })
        .filter_map(|object| object.get("target").and_then(Value::as_str))
        .filter(|target| !target.trim().is_empty())
        .map(|target| format!("bronze.{}", target.trim().to_lowercase()))
        .collect()
}

/// What this load's owner is called in the dedupe row.
async fn follow_up(state: &AppState, pool: &PgPool, run_id: &str, loaded: Loaded) {
    let (owner, tables) = loaded;
    if tables.is_empty() {
        return;
    }
    match pipelines::record_pipeline_run_event(pool, run_id, &owner, DEDUPE_KIND).await {
        Ok(true) => after_load(state, &tables).await,
        Ok(false) => tracing::debug!(run = run_id, "quality load: sensor retry ignored"),
        Err(err) => tracing::warn!(
            ?err,
            run = run_id,
            "quality load: the load was not recorded"
        ),
    }
}

/// Everything a finished load of `tables` sets going: its row count goes
/// into the table's load history whatever its schedule (`DATA-14`, F5; a
/// table that cannot be counted is not recorded, never recorded as empty),
/// and the rules of those on `on_load` run.
async fn after_load(state: &AppState, tables: &[String]) {
    for table in tables {
        if let Some(rows_after) = crate::routes::quality::count_rows(state, table).await {
            crate::routes::quality_anomaly::note_load(state, table, rows_after).await;
        }
    }
    crate::routes::quality::run_on_load(state, tables).await;
}

/// The tables of the authored pipeline `pipeline_id`'s target.
async fn pipeline_loaded(pool: &PgPool, pipeline_id: &str) -> Option<Loaded> {
    match pipelines::get_pipeline(pool, pipeline_id).await {
        Ok(Some(pipeline)) => Some((
            pipeline_id.to_owned(),
            pipeline_table(&pipeline.target).into_iter().collect(),
        )),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(
                ?err,
                pipeline = pipeline_id,
                "quality load: the pipeline could not be read"
            );
            None
        }
    }
}

/// The tables the connector behind run `run_id` loads. `None` when the run
/// belongs to no connector (a clear, an unknown connector).
// PR #106 review R2 SHOULD-FIX 3: surface Dagster run inspection failures fail-closed
async fn connector_loaded(
    state: &AppState,
    pool: &PgPool,
    run_id: &str,
) -> Result<Option<Loaded>, ApiError> {
    let connector = match load_alerts::find_connector(state, pool, run_id).await? {
        Found::This(connector) => connector,
        Found::Skip(_) => return Ok(None),
    };
    // PR #106 review R1 BLOCKER 3: skip clear and filter scoped object
    // PR #106 review R3 BLOCKER 10: classify before surfacing; return fixed message with reference id
    let config = state
        .dagster
        .run_config(run_id)
        .await
        .map_err(|err| load_alerts::unavailable(load_alerts::ORCHESTRATOR_UNAVAILABLE, err))?;
    let Some(config) = config else {
        return Err(ApiError::NotFound(format!(
            "run {run_id} not found in Dagster"
        )));
    };
    if config
        .pointer("/ops/run_ingest/config/action")
        .and_then(Value::as_str)
        == Some("clear")
    {
        return Ok(None);
    }
    let object_filter = config
        .pointer("/ops/run_ingest/config/object")
        .and_then(Value::as_str);

    match connectors::get_ingest_spec(pool, &connector.id).await? {
        Some(spec) => Ok(Some((
            format!("connector:{}", connector.id),
            connector_tables(&spec.source_objects, object_filter),
        ))),
        None => Ok(None),
    }
}

/// The Bronze table the upload behind run `run_id` targeted, once the
/// orchestrator confirms the run succeeded.
async fn upload_loaded(state: &AppState, pool: &PgPool, run_id: &str) -> Option<Loaded> {
    let upload = match uploads::get_by_run_id(pool, run_id).await {
        Ok(Some(upload)) => upload,
        Ok(None) => return None,
        Err(err) => {
            tracing::warn!(
                ?err,
                run = run_id,
                "quality load: the upload could not be read"
            );
            return None;
        }
    };
    let table = upload.bronze_table.as_deref()?.trim().to_lowercase();
    if table.is_empty() {
        return None;
    }
    match load_alerts::confirm_status(state, run_id, Outcome::Success).await {
        Ok(_) => Some((
            format!("upload:{}", upload.id),
            vec![format!("bronze.{table}")],
        )),
        // The same fixed-text classification the alert path logs.
        Err(ApiError::Conflict(_) | ApiError::NotFound(_)) => None,
        Err(err) => {
            tracing::warn!(%err, run = run_id, "quality load: the upload run could not be confirmed");
            None
        }
    }
}

/// Follows up a confirmed-successful authored pipeline run, in the
/// background.
pub(super) fn spawn_pipeline(state: &AppState, run_id: &str, pipeline_id: &str) {
    let (state, run_id, pipeline_id) = (state.clone(), run_id.to_owned(), pipeline_id.to_owned());
    tokio::spawn(async move {
        let Some(pool) = state.pg.as_deref() else {
            return;
        };
        if let Some(loaded) = pipeline_loaded(pool, &pipeline_id).await {
            follow_up(&state, pool, &run_id, loaded).await;
        }
    });
}

/// A connector run reported as successful: the `SRC-7` handling
/// (`load_alerts::connector_run_event`, whose answer is returned unchanged),
/// and then, once that has confirmed the run succeeded, the follow-up of the
/// tables its ingest spec loads, in the background.
///
/// # Errors
///
/// As [`load_alerts::connector_run_event`]; the follow-up cannot add one.
pub(super) async fn connector_success(
    state: &AppState,
    pool: &PgPool,
    req: &RunFailedBody,
) -> Result<Value, ApiError> {
    let handled = load_alerts::connector_run_event(state, pool, req, Outcome::Success).await?;
    // PR #106 review R1 BLOCKER 3: follow up only when the event counted a load (not skipped)
    if handled.get("reason").is_none() {
        // PR #106 review R2 SHOULD-FIX 3: surface Dagster run inspection failures fail-closed
        let loaded = connector_loaded(state, pool, &req.run_id).await?;
        if let Some(loaded) = loaded {
            spawn_follow_up(state, &req.run_id, loaded);
        }
    }
    Ok(handled)
}

fn spawn_follow_up(state: &AppState, run_id: &str, loaded: Loaded) {
    let (state, run_id) = (state.clone(), run_id.to_owned());
    tokio::spawn(async move {
        let Some(pool) = state.pg.as_deref() else {
            return;
        };
        follow_up(&state, pool, &run_id, loaded).await;
    });
}

/// Follows up the run of `req` when it is an upload's, in the background; any
/// other job is left alone.
pub(super) fn spawn_upload(state: &AppState, req: &RunFailedBody) {
    if !load_alerts::is_upload_job(&req.job_name) {
        return;
    }
    let (state, run_id) = (state.clone(), req.run_id.clone());
    tokio::spawn(async move {
        let Some(pool) = state.pg.as_deref() else {
            return;
        };
        if let Some(loaded) = upload_loaded(&state, pool, &run_id).await {
            follow_up(&state, pool, &run_id, loaded).await;
        }
    });
}
