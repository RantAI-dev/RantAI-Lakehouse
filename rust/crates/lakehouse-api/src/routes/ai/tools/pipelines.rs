//! Pipeline tools: `trigger_lakehouse_build`, `get_build_status`. Moved
//! out of `ai.rs` unchanged (T0.1 registry refactor). Also the Tier 1
//! pipeline-operations tools (T1.3 of the copilot-operations-handover
//! plan): `list_pipelines`, `list_pipeline_runs`, `trigger_pipeline`,
//! `retry_pipeline_run`, `pause_pipeline`, `resume_pipeline`,
//! `cancel_pipeline_run`.
//!
//! Every T1.3 function calls the REAL `routes::pipelines::*` handler (via
//! [`super::response_to_value`]) rather than re-implementing the
//! authored-vs-`Dagster`-job branching those handlers already do — see
//! [`super::response_to_value`]'s doc comment for why this is a stronger
//! form of reuse than calling `DgClient` a second, independent way here.

use serde_json::{Map, Value, json};

use axum::extract::{Path, State};

use super::{arg_str, response_to_value};
use crate::state::AppState;

pub(super) async fn trigger_build(dagster: &lakehouse_dagster::DgClient) -> Value {
    match dagster.launch_run("refresh_lakehouse").await {
        Ok(outcome) => {
            if let Some(error) = outcome.error {
                return json!({ "error": error });
            }
            json!({
                "launched": true,
                "runId": outcome.run_id,
                "note": "Pipeline Bronze→Silver→Gold dijalankan. Cek status dengan get_build_status.",
            })
        }
        Err(err) => json!({ "error": err.to_string() }),
    }
}

pub(super) async fn get_build_status(dagster: &lakehouse_dagster::DgClient) -> Value {
    let jobs = dagster.list_jobs().await;
    let runs = dagster.list_runs(10).await;
    match (jobs, runs) {
        (Ok(jobs), Ok(runs)) => {
            let recent: Vec<Value> = runs
                .iter()
                .map(|r| {
                    json!({
                        "job": r.job_name,
                        "status": lakehouse_dagster::map_run_status(&r.status),
                        "startedAt": r.start_time.map(lakehouse_dagster::iso_from_unix_seconds),
                    })
                })
                .collect();
            json!({ "jobs": jobs, "recentRuns": recent })
        }
        (Err(err), _) | (_, Err(err)) => json!({ "error": err.to_string() }),
    }
}

// ── T1.3 pipeline-operations tools ──────────────────────────────────────

pub(super) async fn list_pipelines(state: &AppState) -> Value {
    response_to_value(crate::routes::pipelines::list(State(state.clone())).await).await
}

pub(super) async fn list_pipeline_runs(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    response_to_value(crate::routes::pipelines::runs(State(state.clone()), Path(id)).await).await
}

pub(super) async fn trigger_pipeline(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    response_to_value(crate::routes::pipelines::trigger(State(state.clone()), Path(id)).await).await
}

pub(super) async fn retry_pipeline_run(state: &AppState, args: &Map<String, Value>) -> Value {
    let run_id = arg_str(args, "runId");
    if run_id.is_empty() {
        return json!({ "error": "runId wajib diisi" });
    }
    response_to_value(crate::routes::pipelines::retry_run(State(state.clone()), Path(run_id)).await)
        .await
}

pub(super) async fn pause_pipeline(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    response_to_value(crate::routes::pipelines::pause(State(state.clone()), Path(id)).await).await
}

pub(super) async fn resume_pipeline(state: &AppState, args: &Map<String, Value>) -> Value {
    let id = arg_str(args, "id");
    if id.is_empty() {
        return json!({ "error": "id wajib diisi" });
    }
    response_to_value(crate::routes::pipelines::resume(State(state.clone()), Path(id)).await).await
}

pub(super) async fn cancel_pipeline_run(state: &AppState, args: &Map<String, Value>) -> Value {
    let run_id = arg_str(args, "runId");
    if run_id.is_empty() {
        return json!({ "error": "runId wajib diisi" });
    }
    response_to_value(
        crate::routes::pipelines::cancel_run(State(state.clone()), Path(run_id)).await,
    )
    .await
}

#[cfg(test)]
mod t1_3_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use super::*;
    use crate::config::Config;

    fn state() -> AppState {
        AppState::new(Config::from_map(&HashMap::new()).unwrap())
    }

    #[tokio::test]
    async fn every_id_or_run_id_tool_requires_its_argument() {
        let s = state();
        assert_eq!(
            list_pipeline_runs(&s, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        assert_eq!(
            trigger_pipeline(&s, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        assert_eq!(
            pause_pipeline(&s, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        assert_eq!(
            resume_pipeline(&s, &Map::new()).await,
            json!({ "error": "id wajib diisi" })
        );
        assert_eq!(
            retry_pipeline_run(&s, &Map::new()).await,
            json!({ "error": "runId wajib diisi" })
        );
        assert_eq!(
            cancel_pipeline_run(&s, &Map::new()).await,
            json!({ "error": "runId wajib diisi" })
        );
    }
}
