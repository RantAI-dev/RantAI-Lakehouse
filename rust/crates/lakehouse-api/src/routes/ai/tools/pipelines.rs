//! Pipeline tools: `trigger_lakehouse_build`, `get_build_status`. Moved
//! out of `ai.rs` unchanged (T0.1 registry refactor).

use serde_json::{Value, json};

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
