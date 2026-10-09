//! Alerts for failed and finished loads: connector runs (`ingest_job`) and
//! file uploads (`file_ingest_job`) (`SRC-7`).
//!
//! The orchestrator's run sensors post every failed and every successful run
//! to `routes::pipelines::run_failed_event` and `run_finished_event`
//! (`dagster/dispar_orchestrate/pipeline_events.py`), which until `SRC-7`
//! recognised only authored pipelines (F1). Those handlers now dispatch on
//! `jobName` to the functions here; every other job keeps the pipeline path.
//!
//! What each event does, in this order:
//!
//! 1. Find what the run belongs to. A connector run names its connector only
//!    in its run config (`ops.run_ingest.config.connector_id`, F5), read by
//!    id with `DgClient::run_config`. An upload run is found by
//!    `uploads::get_by_run_id`.
//! 2. Confirm the orchestrator's own status for the run (`FAILURE` or
//!    `SUCCESS`), as the pipeline path does: a sensor that posts the wrong
//!    run must not turn into an alert.
//! 3. Dedupe through `pipeline_run_event` BEFORE anything is counted or sent,
//!    keyed `(run_id, kind)` with `pipeline_id` = `connector:<id>` or
//!    `upload:<id>`, so a sensor retry can neither count a failure twice
//!    (the streak that drives the "3 in a row" rule) nor deliver twice. The
//!    cost of that order: if the write that follows fails after the dedupe
//!    row exists, the retry is skipped and the event is lost. Losing one
//!    count is the safe side of the two (a double count could fire "3 in a
//!    row" early).
//! 4. Record the run on the connector (`connectors::record_run_result`) and
//!    deliver through `lakehouse_alerts::evaluate_connector_event`.
//!
//! Retries, stated once (`SRC-7` review SHOULD-FIX 4). Dagster's run status
//! sensor moves past a run whether its function raised or returned, so
//! nothing asks again after the tick. The sensor posts up to three times
//! inside the tick (connection error, timeout or 5xx only; never 4xx,
//! `pipeline_events.py`). So: a 5xx from here may be retried within about
//! ten seconds; one report per run is all there is; and if this API is
//! unreachable for that whole window the alert for that run is not sent and
//! the connector's health catches up at its next run.
//!
//! A cancelled run is neither a failure nor a success and leaves the
//! connector row alone: the orchestrator's `run_failure_sensor` fires on
//! `FAILURE` only and the success sensor on `SUCCESS` only, so a cancelled
//! run is never posted here; if one were posted, the status check below
//! refuses it with 409 before anything is written.
//!
//! Principle 4: what an alert says is built here from ids and names the
//! console already shows (connector or file name, ids, a relative link),
//! never from the orchestrator's error text or a step's output (decision
//! D8). Failures of the orchestrator, the rule store or the database are
//! logged and answered with fixed text.

use lakehouse_alerts::{AlertKind, AlertRule, SilenceSource};
use lakehouse_core::ApiError;
use lakehouse_notify::EmailSender;
use lakehouse_store::PgPool;
use lakehouse_store::connectors::{self, REPEATED_FAILURE_STREAK};
use lakehouse_store::overview::{self, FiredRule};
use lakehouse_store::pipelines;
use lakehouse_store::uploads;
use serde_json::{Value, json};
use time::OffsetDateTime;

use super::pipelines::RunFailedBody;
use crate::routes::alerts::{ApiSilenceSource, smtp_config};
use crate::routes::{connectors as connector_routes, uploads as upload_routes};
use crate::state::AppState;

/// Shown for any orchestrator failure; the cause is logged, not returned.
const ORCHESTRATOR_UNAVAILABLE: &str = "the orchestrator could not be asked about this run";
/// Shown when the alert rules could not be read or delivered to.
const RULES_UNAVAILABLE: &str = "the alert rules could not be read";

/// The `source` label of an `alert_instance` written for a connector event.
const CONNECTOR_SOURCE: &str = "Connector runs";
/// The `source` label of an `alert_instance` written for an upload event.
const UPLOAD_SOURCE: &str = "File uploads";

/// Which half of the run sensors an event came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Outcome {
    Failure,
    Success,
}

impl Outcome {
    /// The `pipeline_run_event.kind` this outcome dedupes under. `"failure"`
    /// is the kind `run_failed_event` already uses; `"success"` is new and
    /// does not collide with `"slow"` or `"volume_drop"`.
    const fn event_kind(self) -> &'static str {
        match self {
            Self::Failure => "failure",
            Self::Success => "success",
        }
    }

    /// The `Dagster` status the sensor's claim must match.
    const fn dagster_status(self) -> &'static str {
        match self {
            Self::Failure => "FAILURE",
            Self::Success => "SUCCESS",
        }
    }
}

/// Where a run's dedupe, summary and alerts hang: a connector that exists.
struct ConnectorRef {
    id: String,
    name: String,
}

/// What looking for the thing a run belongs to found.
enum Found<T> {
    This(T),
    /// Nothing to do, with the reason the response carries (`{"matched": 0}`
    /// as an unknown job answers today).
    Skip(&'static str),
}

fn unavailable(what: &'static str, err: impl std::fmt::Display) -> ApiError {
    tracing::warn!(%err, what, "a run-event dependency failed");
    ApiError::Unavailable(what.to_owned())
}

fn skipped(reason: &str) -> Value {
    json!({ "matched": 0, "reason": reason })
}

/// The run's end as a time: the orchestrator's `endTime` when it has one,
/// the time of this report otherwise (the sensor posts at the end).
fn run_end(end_time: Option<f64>) -> OffsetDateTime {
    #[allow(
        clippy::cast_possible_truncation,
        reason = "Unix seconds of a run end: far inside i64, and sub-second precision is not kept"
    )]
    end_time
        .and_then(|secs| OffsetDateTime::from_unix_timestamp(secs as i64).ok())
        .unwrap_or_else(OffsetDateTime::now_utc)
}

/// Read the connector a run belongs to from the run's config, and check it
/// exists.
async fn find_connector(
    state: &AppState,
    pool: &PgPool,
    run_id: &str,
) -> Result<Found<ConnectorRef>, ApiError> {
    let config = state
        .dagster
        .run_config(run_id)
        .await
        .map_err(|err| unavailable(ORCHESTRATOR_UNAVAILABLE, err))?;
    let Some(config) = config else {
        return Err(ApiError::NotFound(format!(
            "run {run_id} not found in Dagster"
        )));
    };
    let Some(id) = config
        .pointer("/ops/run_ingest/config/connector_id")
        .and_then(Value::as_str)
    else {
        return Ok(Found::Skip("run config names no connector"));
    };
    match connectors::get_connector(pool, id).await? {
        Some(detail) => Ok(Found::This(ConnectorRef {
            id: detail.connector.id,
            name: detail.connector.name,
        })),
        None => Ok(Found::Skip("run config names an unknown connector")),
    }
}

/// The orchestrator's status for `run_id`, refused with 409 when it is not
/// the one the sensor claims, as the pipeline path does.
async fn confirm_status(
    state: &AppState,
    run_id: &str,
    outcome: Outcome,
) -> Result<lakehouse_dagster::RunStatusInfo, ApiError> {
    let status = state
        .dagster
        .pipeline_run_status(run_id)
        .await
        .map_err(|err| unavailable(ORCHESTRATOR_UNAVAILABLE, err))?;
    let Some(info) = status else {
        return Err(ApiError::NotFound(format!(
            "run {run_id} not found in Dagster"
        )));
    };
    if info.status != outcome.dagster_status() {
        return Err(ApiError::Conflict(format!(
            "run {run_id} is in Dagster status '{}', not {}; refusing to alert",
            info.status,
            outcome.dagster_status()
        )));
    }
    Ok(info)
}

/// Send one event to the rules of `kind` and record an `alert_instance` for
/// each rule a delivery was attempted for (when `persist_as` names a source).
/// `scope_id` is the connector id, or the upload id for `upload_failure`; it is
/// also the instance's `affected`.
///
/// `insert_from_fired_rule` dedupes per `rule_id` within 15 minutes. For a
/// rule scoped `"*"`, two different connectors failing within 15 minutes
/// therefore both DELIVER (webhook or email), but only the first appears on
/// the Alerts page; the second is not written. That primitive is not changed
/// here (`SRC-7` plan, task 4).
pub(super) async fn deliver_event(
    state: &AppState,
    pool: &PgPool,
    kind: AlertKind,
    scope_id: &str,
    title_prefix: &str,
    text: &str,
    persist_as: Option<&str>,
) -> Result<usize, ApiError> {
    let http = reqwest::Client::new();
    let email = EmailSender::new(smtp_config(&state.config));
    let silence = ApiSilenceSource { pg: pool };
    let delivered = lakehouse_alerts::evaluate_connector_event(
        &state.clickhouse,
        &http,
        &email,
        kind,
        scope_id,
        title_prefix,
        text,
        Some(&silence as &dyn SilenceSource),
    )
    .await
    .map_err(|err| unavailable(RULES_UNAVAILABLE, err))?;
    if let Some(source) = persist_as {
        persist_instances(pool, &delivered, source, scope_id, text).await;
    }
    Ok(delivered.len())
}

/// Best-effort `alert_instance` rows for the rules that were delivered to: a
/// Postgres failure must not turn a delivered alert into an error response
/// (the same posture as `routes::alerts::persist_fired_results`).
async fn persist_instances(
    pool: &PgPool,
    rules: &[AlertRule],
    source: &str,
    affected: &str,
    detail: &str,
) {
    let now = OffsetDateTime::now_utc();
    for rule in rules {
        let fired = FiredRule {
            rule_id: &rule.id,
            title: &rule.name,
            severity: rule.severity.as_deref(),
            source,
            affected,
            detail,
        };
        if let Err(err) = overview::insert_from_fired_rule(pool, &fired, now).await {
            tracing::warn!(%err, rule_id = %rule.id, "failed to persist a load alert instance (delivery already happened)");
        }
    }
}

/// A connector run reported by a run sensor (`ingest_job`): record it on the
/// connector and deliver `connector_failure`, and `connector_repeated_failure`
/// when the failure is the third in a row, or `connector_success`.
///
/// # Errors
///
/// 404 for a run the orchestrator does not know, 409 when its status is not
/// the one the sensor claims, 503 with fixed text when the orchestrator or
/// the alert rules cannot be reached, and the store's classified error on a
/// database failure.
pub(super) async fn connector_run_event(
    state: &AppState,
    pool: &PgPool,
    req: &RunFailedBody,
    outcome: Outcome,
) -> Result<Value, ApiError> {
    let connector = match find_connector(state, pool, &req.run_id).await? {
        Found::This(connector) => connector,
        Found::Skip(reason) => return Ok(skipped(reason)),
    };
    let info = confirm_status(state, &req.run_id, outcome).await?;
    let first_seen = pipelines::record_pipeline_run_event(
        pool,
        &req.run_id,
        &format!("connector:{}", connector.id),
        outcome.event_kind(),
    )
    .await?;
    if !first_seen {
        return Ok(skipped("run already recorded; sensor retry ignored"));
    }
    let health = connectors::record_run_result(
        pool,
        &connector.id,
        outcome == Outcome::Success,
        run_end(info.end_time),
    )
    .await?;
    let link = format!("/connectors/{}", connector.id);
    let who = format!("Connector {} ({})", connector.name, connector.id);
    let mut matched = 0;
    match outcome {
        Outcome::Failure => {
            let text = format!("{who} run {} failed. View: {link}", req.run_id);
            matched += deliver_event(
                state,
                pool,
                AlertKind::ConnectorFailure,
                &connector.id,
                "Connector run failed",
                &text,
                Some(CONNECTOR_SOURCE),
            )
            .await?;
            if health.failure_streak == REPEATED_FAILURE_STREAK {
                let text = format!(
                    "{who} has failed {} runs in a row; the latest is run {}. View: {link}",
                    health.failure_streak, req.run_id
                );
                matched += deliver_event(
                    state,
                    pool,
                    AlertKind::ConnectorRepeatedFailure,
                    &connector.id,
                    "Connector failing repeatedly",
                    &text,
                    Some(CONNECTOR_SOURCE),
                )
                .await?;
            }
        }
        Outcome::Success => {
            let text = format!("{who} run {} succeeded. View: {link}", req.run_id);
            matched += deliver_event(
                state,
                pool,
                AlertKind::ConnectorSuccess,
                &connector.id,
                "Connector run succeeded",
                &text,
                None,
            )
            .await?;
        }
    }
    Ok(json!({ "matched": matched }))
}

/// A failed `file_ingest_job` run: settle the upload as failed (nobody has to
/// open it), then deliver `upload_failure`.
///
/// The upload is settled through the same path a reader uses
/// (`routes::uploads::Settler`), so its failure reason is one of the API's
/// fixed texts. The dedupe row is written only after the upload is `failed`.
/// If the orchestrator or `ClickHouse` could not be asked, the upload stays
/// `ingesting`, nothing is recorded, and the answer is a 503 with fixed text
/// (`SRC-7` review SHOULD-FIX 4): the sensor retries a 5xx a few times inside
/// its tick (`pipeline_events.py`, `_post_event`). That is the whole retry:
/// Dagster's run sensor does not ask again after the tick, so an upload
/// still unsettled when the attempts end is not alerted here, and stays
/// `ingesting` until somebody opens it.
///
/// # Errors
///
/// As [`connector_run_event`]; 503 when the upload could not be settled as
/// failed yet.
pub(super) async fn upload_run_failed(
    state: &AppState,
    pool: &PgPool,
    req: &RunFailedBody,
) -> Result<Value, ApiError> {
    let Some(row) = uploads::get_by_run_id(pool, &req.run_id).await? else {
        return Ok(skipped("no upload owns this run"));
    };
    confirm_status(state, &req.run_id, Outcome::Failure).await?;
    let settled = upload_routes::settle_loading_upload(state, pool, row).await;
    if settled.status != "failed" {
        // A 5xx, not `{"matched": 0}` with 200: the sensor retries a 5xx
        // inside its tick and takes a 200 as final.
        return Err(ApiError::Unavailable(
            "the upload could not be settled as failed yet".to_owned(),
        ));
    }
    let first_seen = pipelines::record_pipeline_run_event(
        pool,
        &req.run_id,
        &format!("upload:{}", settled.id),
        Outcome::Failure.event_kind(),
    )
    .await?;
    if !first_seen {
        return Ok(skipped("run already alerted; sensor retry ignored"));
    }
    let text = format!(
        "File {} (upload {}) failed to load in run {}. View: /connectors/upload?id={}",
        settled.original_filename, settled.id, req.run_id, settled.id
    );
    let matched = deliver_event(
        state,
        pool,
        AlertKind::UploadFailure,
        &settled.id,
        "Upload failed",
        &text,
        Some(UPLOAD_SOURCE),
    )
    .await?;
    Ok(json!({ "matched": matched }))
}

/// Whether `job_name` is the one job every connector's ingest runs as.
pub(super) fn is_connector_job(job_name: &str) -> bool {
    job_name == connector_routes::INGEST_JOB
}

/// Whether `job_name` is the upload loader's job.
pub(super) fn is_upload_job(job_name: &str) -> bool {
    job_name == upload_routes::FILE_INGEST_JOB
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::Extension;
    use axum::body::Bytes;
    use axum::extract::State;
    use axum::http::StatusCode;
    use lakehouse_auth::{PermissionSet, Principal, PrincipalId};
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;
    use crate::routes::pipelines::{run_failed_event, run_finished_event};

    /// A seeded connector (`0002`-era seed, present in every `#[sqlx::test]`
    /// database).
    const CONNECTOR: &str = "conn-pg-lakehouse";

    fn service() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "dagster-orchestrator".to_owned(),
            permissions: PermissionSet::parse("pipeline:write"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    fn user() -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            display_name: "Rina Wijaya".to_owned(),
            provider: "session".to_owned(),
            ..service()
        }
    }

    fn body(run_id: &str, job: &str) -> Bytes {
        Bytes::from(serde_json::to_vec(&json!({ "runId": run_id, "jobName": job })).unwrap())
    }

    struct Harness {
        state: AppState,
        dagster: MockServer,
        webhook: MockServer,
    }

    /// Postgres plus mocked `Dagster`, `ClickHouse` and webhook servers.
    /// `rules` are `(id, kind, connector)` rows the rule store answers with;
    /// each rule's webhook is `/<id>` on the webhook mock.
    async fn harness(pool: &sqlx::PgPool, rules: &[(&str, &str, &str)]) -> Harness {
        let dagster = MockServer::start().await;
        let ch = MockServer::start().await;
        let webhook = MockServer::start().await;
        let options = pool.connect_options();
        let database_url = format!(
            "postgres://{}:postgres@{}:{}/{}",
            options.get_username(),
            options.get_host(),
            options.get_port(),
            options.get_database().expect("a named test database"),
        );
        let mut env = HashMap::new();
        env.insert("DATABASE_URL".to_owned(), database_url);
        env.insert(
            "DAGSTER_URL".to_owned(),
            format!("{}/graphql", dagster.uri()),
        );
        env.insert("CH_URL".to_owned(), ch.uri());
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        let data: Vec<Value> = rules
            .iter()
            .map(|(id, kind, connector)| {
                json!({
                    "id": id, "name": format!("Rule {id}"), "type": kind,
                    "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0",
                    "board": "", "channel": "webhook",
                    "target": format!("{}/{id}", webhook.uri()),
                    "enabled": "1", "created_at": "", "severity": "high",
                    "pipeline": "", "connector": connector,
                })
            })
            .collect();
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "rows": data.len(), "data": data })),
            )
            .mount(&ch)
            .await;
        // An upload's recorded result (`bronze_meta.ingest_run`): none.
        Mock::given(method("POST"))
            .and(body_string_contains("ingest_run"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "rows": 0, "data": [] })),
            )
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ch)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&webhook)
            .await;
        Harness {
            state,
            dagster,
            webhook,
        }
    }

    impl Harness {
        /// The orchestrator knows `run_id`: it ran `connector` and ended in
        /// `status`.
        async fn knows_run(&self, run_id: &str, connector: Option<&str>, status: &str) {
            let config = connector.map_or_else(
                || json!({}),
                |c| json!({ "ops": { "run_ingest": { "config": { "connector_id": c } } } }),
            );
            Mock::given(method("POST"))
                .and(body_string_contains("runOrError"))
                .and(body_string_contains(run_id))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "runOrError": { "__typename": "Run", "runConfig": config } }
                })))
                .mount(&self.dagster)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("pipelineRunOrError"))
                .and(body_string_contains(run_id))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "data": { "pipelineRunOrError": {
                        "__typename": "Run", "status": status,
                        "startTime": 1000.0, "endTime": 2000.0, "stepStats": []
                    } }
                })))
                .mount(&self.dagster)
                .await;
        }

        async fn posts_to(&self, rule_id: &str) -> Vec<String> {
            self.webhook
                .received_requests()
                .await
                .expect("request log")
                .iter()
                .filter(|r| r.url.path() == format!("/{rule_id}"))
                .map(|r| String::from_utf8_lossy(&r.body).into_owned())
                .collect()
        }

        async fn failed(&self, run_id: &str) -> Value {
            run_failed_event(
                State(self.state.clone()),
                Extension(service()),
                body(run_id, "ingest_job"),
            )
            .await
            .expect("accepted")
            .0
        }
    }

    async fn streak(pool: &sqlx::PgPool) -> (String, i32) {
        sqlx::query_as("SELECT health, failure_streak FROM connector WHERE id = $1")
            .bind(CONNECTOR)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// A failed connector run delivers `connector_failure`, stamps the
    /// connector row and writes one `alert_instance`; the message names the
    /// connector and run and links to the console, and holds nothing from
    /// the orchestrator.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_failed_connector_run_delivers_stamps_the_row_and_persists_an_instance(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[("al-f", "connector_failure", CONNECTOR)]).await;
        h.knows_run("run-1", Some(CONNECTOR), "FAILURE").await;

        let resp = h.failed("run-1").await;
        assert_eq!(resp["matched"], 1);
        let posts = h.posts_to("al-f").await;
        assert_eq!(posts.len(), 1);
        for needle in ["run-1", CONNECTOR, "/connectors/conn-pg-lakehouse"] {
            assert!(
                posts[0].contains(needle),
                "{needle} missing from {}",
                posts[0]
            );
        }
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        let (last_failure,): (Option<time::OffsetDateTime>,) =
            sqlx::query_as("SELECT last_run_failure_at FROM connector WHERE id = $1")
                .bind(CONNECTOR)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(last_failure.is_some());
        let (instances,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM alert_instance WHERE rule_id = 'al-f'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(instances, 1);
    }

    /// A sensor retry neither delivers twice nor counts the failure twice.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_sensor_retry_neither_delivers_nor_counts_a_second_time(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", "*")]).await;
        h.knows_run("run-1", Some(CONNECTOR), "FAILURE").await;

        assert_eq!(h.failed("run-1").await["matched"], 1);
        assert_eq!(h.failed("run-1").await["matched"], 0);
        assert_eq!(h.posts_to("al-f").await.len(), 1);
        assert_eq!(streak(&pool).await.1, 1);
        let (rows,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM pipeline_run_event \
             WHERE pipeline_id = 'connector:conn-pg-lakehouse' AND kind = 'failure'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 1);
    }

    /// The third failure in a row delivers the repeated-failure rule once;
    /// the fourth does not. The plain failure rule fires every time.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_third_failure_delivers_the_repeated_rule_once_and_the_fourth_does_not(
        pool: sqlx::PgPool,
    ) {
        let h = harness(
            &pool,
            &[
                ("al-f", "connector_failure", CONNECTOR),
                ("al-r", "connector_repeated_failure", CONNECTOR),
            ],
        )
        .await;
        for run in ["run-1", "run-2", "run-3", "run-4"] {
            h.knows_run(run, Some(CONNECTOR), "FAILURE").await;
            h.failed(run).await;
        }
        assert_eq!(h.posts_to("al-f").await.len(), 4);
        let repeated = h.posts_to("al-r").await;
        assert_eq!(repeated.len(), 1);
        assert!(repeated[0].contains("run-3"), "{}", repeated[0]);
        assert_eq!(streak(&pool).await, ("unhealthy".to_owned(), 4));
    }

    /// A success clears the streak, and delivers `connector_success` only
    /// to a rule that names this connector; with no such rule nothing is
    /// sent (decision D4).
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_success_resets_the_streak_and_alerts_only_a_rule_for_that_connector(
        pool: sqlx::PgPool,
    ) {
        let h = harness(
            &pool,
            &[
                ("al-s", "connector_success", CONNECTOR),
                ("al-other", "connector_success", "conn-other"),
            ],
        )
        .await;
        h.knows_run("run-1", Some(CONNECTOR), "FAILURE").await;
        h.failed("run-1").await;
        h.knows_run("run-2", Some(CONNECTOR), "SUCCESS").await;
        let resp = run_finished_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-2", "ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 1);
        assert_eq!(streak(&pool).await, ("healthy".to_owned(), 0));
        assert_eq!(h.posts_to("al-s").await.len(), 1);
        assert!(h.posts_to("al-other").await.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_success_with_no_success_rule_sends_nothing(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", "*")]).await;
        h.knows_run("run-1", Some(CONNECTOR), "SUCCESS").await;
        let resp = run_finished_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-1", "ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 0);
        assert!(h.webhook.received_requests().await.unwrap().is_empty());
        assert_eq!(streak(&pool).await.0, "healthy");
    }

    /// A caller that is neither the orchestrator nor an administrator is
    /// refused before anything is read or written.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_user_who_is_not_the_orchestrator_is_refused(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", "*")]).await;
        let err = run_failed_event(
            State(h.state.clone()),
            Extension(user()),
            body("run-1", "ingest_job"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0.status(), StatusCode::FORBIDDEN);
        assert_eq!(streak(&pool).await.1, 0);
    }

    /// A run that is not `FAILURE` (a cancelled run, a misdirected sensor)
    /// is refused with 409 and leaves the connector row alone.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_run_that_is_not_failed_is_refused_and_leaves_the_row_alone(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", "*")]).await;
        h.knows_run("run-1", Some(CONNECTOR), "CANCELED").await;
        let before = streak(&pool).await;
        let err = run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-1", "ingest_job"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0.status(), StatusCode::CONFLICT);
        assert_eq!(streak(&pool).await, before);
        assert!(h.webhook.received_requests().await.unwrap().is_empty());
    }

    /// A run whose config names no connector, or one that does not exist,
    /// answers `{"matched": 0}` with a reason, like an unknown job.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_run_with_no_known_connector_answers_matched_zero_with_a_reason(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", "*")]).await;
        h.knows_run("run-none", None, "FAILURE").await;
        h.knows_run("run-ghost", Some("conn-ghost"), "FAILURE")
            .await;
        for run in ["run-none", "run-ghost"] {
            let resp = h.failed(run).await;
            assert_eq!(resp["matched"], 0);
            assert!(resp["reason"].is_string());
        }
        assert!(h.webhook.received_requests().await.unwrap().is_empty());
    }

    /// Upstream error text never reaches the response (principle 4): the
    /// orchestrator answers the config read with a body naming a secret, and
    /// the 503 carries only the fixed sentence.
    #[sqlx::test(migrations = "../../migrations")]
    async fn orchestrator_error_text_never_reaches_the_response(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", "*")]).await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("not json: password=hunter2-upstream"),
            )
            .mount(&h.dagster)
            .await;
        let err = run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-1", "ingest_job"),
        )
        .await
        .unwrap_err();
        assert_eq!(err.0.status(), StatusCode::SERVICE_UNAVAILABLE);
        let text = format!("{:?}", err.0);
        assert!(!text.contains("hunter2"), "{text}");
        assert!(text.contains(ORCHESTRATOR_UNAVAILABLE), "{text}");
    }

    // ── uploads (SRC-7 task 5) ──────────────────────────────────────────

    const SEED_TENANT: &str = "11111111-1111-4111-8111-000000000001";

    /// An upload whose load was started under `run_id`, as the ingest route
    /// leaves it: `ingesting`, never read since.
    async fn loading_upload(pool: &sqlx::PgPool, id: &str, run_id: &str) {
        use lakehouse_store::uploads::{LoadMode, NewUpload, insert, mark_ingesting};
        insert(
            pool,
            &NewUpload {
                id,
                original_filename: "stock.csv",
                storage_key: &format!("uploads/{id}.csv"),
                content_type: "text/csv",
                size_bytes: 10,
                sha256: "",
                uploaded_by: "Test User",
                tenant_id: SEED_TENANT.parse().unwrap(),
            },
        )
        .await
        .unwrap();
        mark_ingesting(
            pool,
            id,
            &json!({ "encoding": "utf-8", "delimiter": ",", "headerRow": 0 }),
            "stock_raw",
            LoadMode::Replace,
            Some(run_id),
        )
        .await
        .unwrap()
        .unwrap();
    }

    async fn upload_status(pool: &sqlx::PgPool, id: &str) -> (String, Option<String>) {
        sqlx::query_as("SELECT status, error FROM file_upload WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// An upload still `ingesting` whose run failed becomes `failed` with
    /// the API's own fixed reason and one alert goes out, with nobody having
    /// read it. The message names the file and the upload and links to the
    /// upload page; a sensor retry delivers nothing more.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_failed_upload_run_settles_the_upload_and_delivers_one_alert(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-u", "upload_failure", "*")]).await;
        loading_upload(&pool, "up-1", "run-up").await;
        h.knows_run("run-up", None, "FAILURE").await;

        let resp = run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-up", "file_ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 1);
        let (status, error) = upload_status(&pool, "up-1").await;
        assert_eq!(status, "failed");
        assert_eq!(
            error.as_deref(),
            Some("The load stopped before it recorded a result.")
        );
        let posts = h.posts_to("al-u").await;
        assert_eq!(posts.len(), 1);
        for needle in ["stock.csv", "up-1", "run-up", "/connectors/upload?id=up-1"] {
            assert!(
                posts[0].contains(needle),
                "{needle} missing from {}",
                posts[0]
            );
        }
        assert!(
            !posts[0].contains("recorded a result"),
            "no reason text in the alert"
        );
        let (instances,): (i64,) =
            sqlx::query_as("SELECT count(*) FROM alert_instance WHERE rule_id = 'al-u'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(instances, 1);

        let retry = run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-up", "file_ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(retry["matched"], 0);
        assert_eq!(h.posts_to("al-u").await.len(), 1);
    }

    /// A run no upload owns is `{"matched": 0}` with a reason, not an error.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_failed_run_of_no_upload_answers_matched_zero(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-u", "upload_failure", "*")]).await;
        let resp = run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-nobody", "file_ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 0);
        assert!(resp["reason"].is_string());
        assert!(h.webhook.received_requests().await.unwrap().is_empty());
    }
}
