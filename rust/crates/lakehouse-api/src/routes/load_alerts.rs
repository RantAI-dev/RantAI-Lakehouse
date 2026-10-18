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
//! 3. Count the run once (`SRC-17` F2). `connectors::record_run_once` writes
//!    the dedupe row `(run_id, kind)` (`pipeline_id` = `connector:<id>`) and
//!    the connector's health facts in one transaction, and keeps the streak
//!    the run produced, so a sensor retry can neither count a failure twice
//!    (the streak that drives the "3 in a row" rule) nor lose a count to a
//!    failure between the two writes.
//!    (`SRC-11`) The same transaction pauses the connector when this failure
//!    is the one that reaches `CONNECTOR_AUTO_DISABLE_AFTER` in a row, and
//!    the report then also delivers `connector_disabled`; a run the
//!    orchestrator ended as "paused" is not counted at all, and the run
//!    record (`lakehouse_store::connector_run`) is closed from the confirmed
//!    status when no end report arrived.
//! 4. Hand the alerts over once. The rules are read AFTER the run is counted
//!    and BEFORE a second row `(run_id, "<kind>_alerted")` is written; the
//!    delivery follows that row. So a rule-store failure answers 503 with the
//!    run counted and the second row absent: the retry counts nothing again
//!    and delivers, with the run's own streak (that is what makes "3 in a
//!    row" fire once even when the third run's first report met the failure).
//!    A retry after everything went through finds both rows and does nothing.
//!    Before this, the one dedupe row was written first and the rules read
//!    after, and the retry of a failed read was skipped as "already
//!    recorded": the alert was lost, silently. A delivery that was attempted
//!    is never retried by the API (at most once: a webhook that already fired
//!    is not fired again); what happened to it is on the alert instance
//!    (`SRC-17` F1).
//!
//! Retries, stated once (`SRC-7` review SHOULD-FIX 4). Dagster's run status
//! sensor moves past a run whether its function raised or returned, so
//! nothing asks again after the tick. The sensor posts up to three times
//! inside the tick (connection error, timeout or 5xx only; never 4xx,
//! `pipeline_events.py`). So: a 5xx from here may be retried within about
//! ten seconds; one report per run is all there is; and if this API is
//! unreachable for that whole window the alert for that run is not sent and
//! the connector's health catches up at its next run. The same bound holds
//! for a rule store that stays down for the whole window: the run is counted
//! but its alert is not sent (a durable queue is out of scope for `SRC-17`).
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

use lakehouse_alerts::{AlertKind, AlertRule, RuleDelivery, SilenceSource};
use lakehouse_core::ApiError;
use lakehouse_notify::{DeliverResult, EmailSender};
use lakehouse_store::PgPool;
use lakehouse_store::connector_run;
use lakehouse_store::connectors::{self, REPEATED_FAILURE_STREAK};
use lakehouse_store::overview::{self, DeliveryRecord, FiredRule};
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

    /// The `pipeline_run_event.kind` written once the rules were read and the
    /// alerts are being handed over (`SRC-17` F2).
    const fn alerted_kind(self) -> &'static str {
        match self {
            Self::Failure => "failure_alerted",
            Self::Success => "success_alerted",
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
    // `SRC-10` decision 5: a clear empties one table and loads nothing, so it is
    // neither a success nor a failure of a LOAD. Counting it would reset the
    // connector's failure streak (and send a "succeeded" alert) for a run that
    // read nothing from the source. The run stays visible in the run list and as
    // a `cleared` row of `ingest_run`; a clear that fails is seen there and in the
    // orchestrator, not counted against the connector's load health.
    if config
        .pointer("/ops/run_ingest/config/action")
        .and_then(Value::as_str)
        == Some("clear")
    {
        return Ok(Found::Skip("a clear loads nothing"));
    }
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
/// each rule a delivery was attempted for (when `persist_as` names a source),
/// with how that delivery ended (`SRC-17` F1).
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
    let rules = read_rules(state).await?;
    Ok(deliver_with_rules(
        state,
        pool,
        &rules,
        (kind, scope_id),
        title_prefix,
        text,
        persist_as,
    )
    .await)
}

/// Read the alert rules once. A failure is logged and answered with fixed
/// text (principle 4).
async fn read_rules(state: &AppState) -> Result<Vec<AlertRule>, ApiError> {
    lakehouse_alerts::load_rules(&state.clickhouse)
        .await
        .map_err(|err| unavailable(RULES_UNAVAILABLE, err))
}

/// [`deliver_event`] over rules already read. It cannot fail on the rule
/// store, which is the point of reading them first (`SRC-17` F2). Returns how
/// many deliveries were attempted.
async fn deliver_with_rules(
    state: &AppState,
    pool: &PgPool,
    rules: &[AlertRule],
    (kind, scope_id): (AlertKind, &str),
    title_prefix: &str,
    text: &str,
    persist_as: Option<&str>,
) -> usize {
    // SEC-10 (merged with `SRC-17`): connector-event webhooks go through the
    // same address-checking sender as every other alert delivery, never a
    // bare `reqwest` client.
    let http = state.webhook_sender();
    let email = EmailSender::new(smtp_config(&state.config));
    let silence = ApiSilenceSource { pg: pool };
    let attempted = lakehouse_alerts::deliver_connector_event_to(
        rules,
        &http,
        &email,
        kind,
        scope_id,
        title_prefix,
        text,
        Some(&silence as &dyn SilenceSource),
    )
    .await;
    if let Some(source) = persist_as {
        persist_instances(pool, &attempted, source, scope_id, text).await;
    }
    attempted.len()
}

/// How a delivery ended, in the form the alert instance stores it
/// (`SRC-17` F1). `error` is always one of the fixed sentences of
/// `lakehouse_notify::DeliveryFailure` (never upstream text), so it is
/// stored and shown as it is. A failed result with no reason records
/// nothing rather than an invented one.
pub(super) fn delivery_record(result: &DeliverResult) -> Option<DeliveryRecord<'_>> {
    if result.ok {
        Some(DeliveryRecord::Delivered)
    } else {
        result.error.as_deref().map(DeliveryRecord::NotDelivered)
    }
}

/// Best-effort `alert_instance` rows for the rules a delivery was attempted
/// for, delivered or not: a Postgres failure must not turn a delivered alert
/// into an error response (the same posture as
/// `routes::alerts::persist_fired_results`). An alert whose delivery failed
/// is still listed, with the reason (`SRC-17` F1: the event happened).
async fn persist_instances(
    pool: &PgPool,
    attempted: &[RuleDelivery],
    source: &str,
    affected: &str,
    detail: &str,
) {
    let now = OffsetDateTime::now_utc();
    for RuleDelivery { rule, result } in attempted {
        let fired = FiredRule {
            rule_id: &rule.id,
            title: &rule.name,
            severity: rule.severity.as_deref(),
            source,
            affected,
            detail,
        };
        if let Err(err) = overview::insert_from_fired_rule_with_delivery(
            pool,
            &fired,
            delivery_record(result),
            now,
        )
        .await
        {
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
/// the alert rules cannot be reached (the run is then counted, if it was not
/// yet, and the retry delivers, `SRC-17` F2), and the store's classified error on a
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
    // `SRC-11` D13: a run that ended as "paused" loaded nothing (the connector
    // was paused when it started). It is neither a success nor a failure of a
    // load, and counting it would reset the failure streak of a connector that
    // was just paused because of it.
    if connector_run::run_status(pool, &connector.id, &req.run_id)
        .await?
        .as_deref()
        == Some(connector_run::RunStatus::Paused.as_str())
    {
        return Ok(skipped("a paused run loads nothing"));
    }
    let info = confirm_status(state, &req.run_id, outcome).await?;
    // `SRC-11` D7: close the run record from the orchestrator's own final
    // status when its end report never came (a killed run, an orchestrator
    // that could not reach this API). Idempotent, and a no-op for a run with
    // no record or one that already ended.
    connector_run::settle_run(
        pool,
        &connector.id,
        &req.run_id,
        outcome == Outcome::Success,
        run_end(info.end_time),
    )
    .await?;
    // Count the run once, with its streak kept for a retry (`SRC-17` F2).
    let recorded = connectors::record_run_once(
        pool,
        &req.run_id,
        &connector.id,
        outcome.event_kind(),
        outcome == Outcome::Success,
        run_end(info.end_time),
        // `SRC-11` D3: the count that pauses the connector.
        state.config.connector_auto_disable_after,
    )
    .await?;
    // `SRC-17` F2 (delivery review 2026-10-12): read the rules BEFORE the
    // second marker, so a rule-store failure leaves the run counted and the
    // alerts un-handed-over, and the retry delivers (module doc, item 4).
    let rules = read_rules(state).await?;
    let handed_over = pipelines::record_pipeline_run_event(
        pool,
        &req.run_id,
        &format!("connector:{}", connector.id),
        outcome.alerted_kind(),
    )
    .await?;
    if !handed_over {
        return Ok(skipped("run already recorded; sensor retry ignored"));
    }
    let link = format!("/connectors/{}", connector.id);
    let who = format!("Connector {} ({})", connector.name, connector.id);
    let mut matched = 0;
    match outcome {
        Outcome::Failure => {
            matched +=
                deliver_failure_alerts(state, pool, &rules, &connector, &recorded, &req.run_id)
                    .await;
        }
        Outcome::Success => {
            let text = format!("{who} run {} succeeded. View: {link}", req.run_id);
            matched += deliver_with_rules(
                state,
                pool,
                &rules,
                (AlertKind::ConnectorSuccess, &connector.id),
                "Connector run succeeded",
                &text,
                None,
            )
            .await;
        }
    }
    Ok(json!({ "matched": matched }))
}

/// The alerts of a failed connector run: `connector_failure`, then
/// `connector_repeated_failure` on the run that makes the streak three, then
/// `connector_disabled` on the run that paused the connector. Split out of
/// [`connector_run_event`] (`SRC-11` review 1 BLOCKER 1, `too_many_lines`);
/// returns the number of matched rules.
async fn deliver_failure_alerts(
    state: &AppState,
    pool: &PgPool,
    rules: &[AlertRule],
    connector: &ConnectorRef,
    recorded: &connectors::RunRecord,
    run_id: &str,
) -> usize {
    let link = format!("/connectors/{}", connector.id);
    let who = format!("Connector {} ({})", connector.name, connector.id);
    let mut matched = 0;
    let text = format!("{who} run {run_id} failed. View: {link}");
    matched += deliver_with_rules(
        state,
        pool,
        rules,
        (AlertKind::ConnectorFailure, &connector.id),
        "Connector run failed",
        &text,
        Some(CONNECTOR_SOURCE),
    )
    .await;
    // The run's own streak (kept on its dedupe row), not the
    // connector's current one: a retry of the third run's report
    // still fires "3 in a row", once, and a later run's streak
    // never fires it for this one.
    if recorded.failure_streak == Some(REPEATED_FAILURE_STREAK) {
        let text = format!(
            "{who} has failed {REPEATED_FAILURE_STREAK} runs in a row; the latest is run {run_id}. View: {link}"
        );
        matched += deliver_with_rules(
            state,
            pool,
            rules,
            (AlertKind::ConnectorRepeatedFailure, &connector.id),
            "Connector failing repeatedly",
            &text,
            Some(CONNECTOR_SOURCE),
        )
        .await;
    }
    // `SRC-11` D3: the run whose failure paused the connector raises
    // `connector_disabled`, once. `auto_disabled` is the run's own
    // fact (kept for its retries, like the streak above), so a
    // rule-store failure on this report still delivers on the retry,
    // and no other run delivers it.
    if recorded.auto_disabled {
        let text = format!(
            "{who} was paused after {} failed runs in a row; the latest is run {}. \
             Resume it from its page once the cause is fixed. View: {link}",
            state.config.connector_auto_disable_after, run_id
        );
        matched += deliver_with_rules(
            state,
            pool,
            rules,
            (AlertKind::ConnectorDisabled, &connector.id),
            "Connector paused",
            &text,
            Some(CONNECTOR_SOURCE),
        )
        .await;
    }
    matched
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
    // `SRC-17` F2: read the rules before the dedupe row, as the connector
    // path does. A rule-store failure answers 503 with nothing recorded, so
    // the retry delivers; the dedupe row below is written only once the
    // rules are in hand.
    let rules = read_rules(state).await?;
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
    let matched = deliver_with_rules(
        state,
        pool,
        &rules,
        (AlertKind::UploadFailure, &settled.id),
        "Upload failed",
        &text,
        Some(UPLOAD_SOURCE),
    )
    .await;
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
        /// Kept alive on purpose (PR #101 CI, `a_failed_connector_run_delivers_stamps_the_row_and_persists_an_instance`
        /// and `a_success_run_of_a_connector_with_a_waiting_schema_change_is_a_success_and_never_a_failure`):
        /// dropping a `MockServer` clears its mocks and hands the port back
        /// to wiremock's pool, so a `ClickHouse` mock dropped at the end of
        /// `harness` answered 404 (the rules "could not be read") or, once
        /// another test took the port, with that test's rules.
        clickhouse: MockServer,
    }

    /// Postgres plus mocked `Dagster`, `ClickHouse` and webhook servers.
    /// `rules` are `(id, kind, connector)` rows the rule store answers with;
    /// each rule's webhook is `/<id>` on the webhook mock.
    async fn harness(pool: &sqlx::PgPool, rules: &[(&str, &str, &str)]) -> Harness {
        harness_with(pool, rules, &[]).await
    }

    /// [`harness`] with extra environment settings (`SRC-11`: a low
    /// `CONNECTOR_AUTO_DISABLE_AFTER`).
    async fn harness_with(
        pool: &sqlx::PgPool,
        rules: &[(&str, &str, &str)],
        extra_env: &[(&str, &str)],
    ) -> Harness {
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
        for (key, value) in extra_env {
            env.insert((*key).to_owned(), (*value).to_owned());
        }
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        // The webhook mock listens on loopback, which the real address
        // policy refuses (`SEC-10`); the policy has its own tests.
        state.webhook_sender_override = Some(lakehouse_notify::MappedResolver::new().into_sender());
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
            clickhouse: ch,
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
            self.knows_run_config(run_id, config, status).await;
        }

        /// As [`Self::knows_run`], with the whole run config given (a run of
        /// one table carries `object` and `action` beside the connector id).
        async fn knows_run_config(&self, run_id: &str, config: Value, status: &str) {
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

    /// `SRC-17` F1 (D-a): a webhook that answers 500 leaves an alert that is
    /// listed (the failure happened) but reads "not delivered", with the
    /// API's fixed reason and nothing of what the server said. A webhook that
    /// answers 200 reads delivered. The response's `matched` still counts the
    /// attempt.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_webhook_that_answers_an_error_leaves_an_alert_that_says_it_was_not_delivered(
        pool: sqlx::PgPool,
    ) {
        let h = harness(
            &pool,
            &[
                ("al-bad", "connector_failure", CONNECTOR),
                ("al-good", "connector_failure", CONNECTOR),
            ],
        )
        .await;
        // Higher priority than the harness's catch-all 200.
        Mock::given(method("POST"))
            .and(wiremock::matchers::path("/al-bad"))
            .respond_with(ResponseTemplate::new(500).set_body_string("upstream-secret-detail"))
            .with_priority(1)
            .mount(&h.webhook)
            .await;
        h.knows_run("run-1", Some(CONNECTOR), "FAILURE").await;

        let resp = h.failed("run-1").await;
        assert_eq!(resp["matched"], 2);

        let listed = lakehouse_store::overview::list_alerts(&pool).await.unwrap();
        let bad = listed
            .iter()
            .find(|a| a.rule_id.as_deref() == Some("al-bad"))
            .expect("a failed delivery is still listed");
        let good = listed
            .iter()
            .find(|a| a.rule_id.as_deref() == Some("al-good"))
            .expect("the delivered alert is listed");
        assert_eq!(bad.delivered, Some(false));
        assert_eq!(
            bad.delivery_reason.as_deref(),
            Some("the webhook server answered with an error")
        );
        assert_eq!(good.delivered, Some(true));
        assert_eq!(good.delivery_reason, None);
        // The list route serializes the item as is: camelCase names, and no
        // trace of the server's status or body.
        let json = serde_json::to_value(bad).unwrap();
        assert_eq!(json["delivered"], false);
        assert!(!json.to_string().contains("upstream-secret-detail"));
        // Not the whole JSON: the instance id is a UUID and may hold "500".
        assert!(!json["deliveryReason"].as_str().unwrap().contains("500"));
    }

    /// The rule store answers 500 for its next read only (the retry reads
    /// it fine), as the alert rules' `ClickHouse` would during a blip.
    async fn rule_store_fails_once(h: &Harness) {
        Mock::given(method("POST"))
            .and(body_string_contains("FROM console.alert_rule"))
            .respond_with(ResponseTemplate::new(500).set_body_string("ch-secret-detail"))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&h.clickhouse)
            .await;
    }

    async fn failed_status(h: &Harness, run_id: &str) -> u16 {
        run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body(run_id, "ingest_job"),
        )
        .await
        .unwrap_err()
        .0
        .status()
    }

    async fn run_event_rows(pool: &sqlx::PgPool, run_id: &str) -> Vec<(String, i64)> {
        let mut rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT kind, count(*) FROM pipeline_run_event WHERE run_id = $1 GROUP BY kind",
        )
        .bind(run_id)
        .fetch_all(pool)
        .await
        .unwrap();
        rows.sort();
        rows
    }

    /// `SRC-17` F2, property 1: a rule-store failure on a report answers 503
    /// with the run counted exactly once and nothing delivered, and the
    /// orchestrator's retry delivers once and counts nothing again. (Before,
    /// the dedupe row existed already and the retry was skipped: the alert
    /// was lost.) The failure does not leak the store's text.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rule_store_failure_followed_by_a_retry_delivers_once_and_counts_once(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[("al-f", "connector_failure", CONNECTOR)]).await;
        h.knows_run("run-1", Some(CONNECTOR), "FAILURE").await;
        rule_store_fails_once(&h).await;

        let status = failed_status(&h, "run-1").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(h.posts_to("al-f").await, [] as [std::string::String; 0]);
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        // Counted, not yet handed over.
        assert_eq!(
            run_event_rows(&pool, "run-1").await,
            [("failure".to_owned(), 1)]
        );

        let retry = h.failed("run-1").await;
        assert_eq!(retry["matched"], 1);
        assert_eq!(h.posts_to("al-f").await.len(), 1);
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        assert_eq!(
            run_event_rows(&pool, "run-1").await,
            [("failure".to_owned(), 1), ("failure_alerted".to_owned(), 1)]
        );
    }

    /// `SRC-17` F2, property 2: a retry after a report that went all the way
    /// through delivers nothing and counts nothing, however often it comes.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_retry_after_a_full_success_delivers_nothing_and_counts_nothing(pool: sqlx::PgPool) {
        let h = harness(&pool, &[("al-f", "connector_failure", CONNECTOR)]).await;
        h.knows_run("run-1", Some(CONNECTOR), "FAILURE").await;

        assert_eq!(h.failed("run-1").await["matched"], 1);
        for _ in 0..2 {
            let retry = h.failed("run-1").await;
            assert_eq!(retry["matched"], 0);
        }
        assert_eq!(h.posts_to("al-f").await.len(), 1);
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        assert_eq!(
            run_event_rows(&pool, "run-1").await,
            [("failure".to_owned(), 1), ("failure_alerted".to_owned(), 1)]
        );
    }

    /// `SRC-17` F2: the "3 in a row" rule fires once, on the third run, even
    /// when that run's first report met a rule-store failure. The retry
    /// decides by the third run's own streak, kept on its dedupe row, and the
    /// fourth run (streak 4) does not fire it again.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_repeated_rule_fires_once_when_the_third_runs_first_report_met_a_rule_store_failure(
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
        }
        h.failed("run-1").await;
        h.failed("run-2").await;

        rule_store_fails_once(&h).await;
        assert_eq!(
            failed_status(&h, "run-3").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(streak(&pool).await, ("unhealthy".to_owned(), 3));
        assert_eq!(h.posts_to("al-r").await, [] as [std::string::String; 0]);

        let retry = h.failed("run-3").await;
        assert_eq!(retry["matched"], 2, "the failure and the repeated rule");
        assert_eq!(h.failed("run-3").await["matched"], 0);
        h.failed("run-4").await;

        let repeated = h.posts_to("al-r").await;
        assert_eq!(repeated.len(), 1);
        assert!(repeated[0].contains("run-3"), "{}", repeated[0]);
        assert_eq!(h.posts_to("al-f").await.len(), 4);
        assert_eq!(streak(&pool).await, ("unhealthy".to_owned(), 4));
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
        assert_eq!(h.posts_to("al-other").await, [] as [std::string::String; 0]);
    }

    /// `SRC-10` decision 5: a clear loads nothing, so its run is neither a
    /// success nor a failure of a load. Whatever the orchestrator says about it,
    /// the connector's health and failure streak are left alone, no alert is
    /// sent, and no run event is written; a refresh, which does load, counts as
    /// any run does.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_clear_run_leaves_the_connectors_run_facts_alone_and_a_refresh_counts_as_a_run(
        pool: sqlx::PgPool,
    ) {
        let h = harness(
            &pool,
            &[
                ("al-f", "connector_failure", CONNECTOR),
                ("al-s", "connector_success", CONNECTOR),
            ],
        )
        .await;
        h.knows_run("run-fail", Some(CONNECTOR), "FAILURE").await;
        h.failed("run-fail").await;
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        let config = |action: &str| {
            json!({ "ops": { "run_ingest": { "config": {
                "connector_id": CONNECTOR, "object": "public.orders", "action": action
            } } } })
        };

        // A SUCCESS clear does not reset the streak and sends no success alert.
        h.knows_run_config("run-clear", config("clear"), "SUCCESS")
            .await;
        let resp = run_finished_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-clear", "ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 0);
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        assert_eq!(h.posts_to("al-s").await, [] as [std::string::String; 0]);

        // A FAILED clear is not counted against the load either.
        h.knows_run_config("run-clear-fail", config("clear"), "FAILURE")
            .await;
        h.failed("run-clear-fail").await;
        assert_eq!(streak(&pool).await, ("degraded".to_owned(), 1));
        assert_eq!(
            h.posts_to("al-f").await.len(),
            1,
            "only the first, real failure"
        );
        let (events,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM pipeline_run_event WHERE run_id IN ('run-clear', 'run-clear-fail')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(events, 0);

        // A refresh loads: an ordinary run for health and alerts.
        h.knows_run_config("run-refresh", config("refresh_remove"), "SUCCESS")
            .await;
        let resp = run_finished_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-refresh", "ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 1);
        assert_eq!(streak(&pool).await, ("healthy".to_owned(), 0));
        assert_eq!(h.posts_to("al-s").await.len(), 1);
    }

    /// `SRC-8` decision D7: a table that waits for a schema-change decision
    /// is not a failure. The orchestrator records such a table as `waiting`
    /// and the mapped step and the run both SUCCEED
    /// (`test_ingest_factory.py`), so what reaches this route is a SUCCESS
    /// event like any other. Only a FAILURE-status run stamps a failure: this
    /// pins that the pending change changes nothing here (no
    /// `connector_failure`, no repeated-failure, no failure streak, a
    /// `success` event row and no `failure` one), and that the change keeps
    /// waiting for a person.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_success_run_of_a_connector_with_a_waiting_schema_change_is_a_success_and_never_a_failure(
        pool: sqlx::PgPool,
    ) {
        let h = harness(
            &pool,
            &[
                ("al-f", "connector_failure", CONNECTOR),
                ("al-r", "connector_repeated_failure", CONNECTOR),
                ("al-s", "connector_success", CONNECTOR),
            ],
        )
        .await;
        sqlx::query(
            "INSERT INTO connector_schema_change \
               (id, connector_id, object_name, kind, column_name, before_value, breaking, status, detected_at) \
             VALUES ('chg-waiting', $1, 'public.orders', 'column_removed', 'note', 'text', true, 'pending', now())",
        )
        .bind(CONNECTOR)
        .execute(&pool)
        .await
        .unwrap();
        h.knows_run("run-1", Some(CONNECTOR), "SUCCESS").await;

        let resp = run_finished_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-1", "ingest_job"),
        )
        .await
        .expect("accepted")
        .0;

        assert_eq!(resp["matched"], 1, "only the success rule matched");
        assert_eq!(h.posts_to("al-s").await.len(), 1);
        assert_eq!(h.posts_to("al-f").await, [] as [std::string::String; 0]);
        assert_eq!(h.posts_to("al-r").await, [] as [std::string::String; 0]);
        assert_eq!(streak(&pool).await, ("healthy".to_owned(), 0));
        let kinds: Vec<(String, i64)> = sqlx::query_as(
            "SELECT kind, count(*) FROM pipeline_run_event \
             WHERE pipeline_id = 'connector:conn-pg-lakehouse' GROUP BY kind",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        // SRC-17 F2: a handed-over report leaves two rows, the count and the
        // marker written once the rules were read (`success_alerted`).
        // Before, the single `success` row was all there was. Still no
        // `failure` row.
        let mut kinds = kinds;
        kinds.sort();
        assert_eq!(
            kinds,
            [("success".to_owned(), 1), ("success_alerted".to_owned(), 1)]
        );
        let (still_waiting,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM connector_schema_change WHERE id = 'chg-waiting' AND status = 'pending'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(still_waiting, 1);
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

    /// `SRC-17` F2 for uploads: a rule-store failure answers 503 with the
    /// upload settled as failed and no dedupe row, so the retry delivers
    /// once; a further retry delivers nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rule_store_failure_on_a_failed_upload_run_is_retried_into_one_alert(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[("al-u", "upload_failure", "*")]).await;
        loading_upload(&pool, "up-1", "run-up").await;
        h.knows_run("run-up", None, "FAILURE").await;
        rule_store_fails_once(&h).await;

        let first = run_failed_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-up", "file_ingest_job"),
        )
        .await
        .unwrap_err();
        assert_eq!(first.0.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(upload_status(&pool, "up-1").await.0, "failed");
        assert_eq!(h.posts_to("al-u").await, [] as [std::string::String; 0]);
        assert_eq!(
            run_event_rows(&pool, "run-up").await,
            [] as [(std::string::String, i64); 0]
        );

        for (expected_matched, expected_posts) in [(1, 1), (0, 1)] {
            let resp = run_failed_event(
                State(h.state.clone()),
                Extension(service()),
                body("run-up", "file_ingest_job"),
            )
            .await
            .expect("accepted")
            .0;
            assert_eq!(resp["matched"], expected_matched);
            assert_eq!(h.posts_to("al-u").await.len(), expected_posts);
        }
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

    async fn paused_reason(pool: &sqlx::PgPool) -> Option<String> {
        sqlx::query_scalar("SELECT paused_reason FROM connector WHERE id = $1")
            .bind(CONNECTOR)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// `SRC-11` D3: the failed run that reaches the threshold pauses the
    /// connector and delivers `connector_disabled` once; its retry and the
    /// next failure deliver nothing more. The text names the connector, the
    /// run and the number, and links to the console.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_run_that_reaches_the_threshold_pauses_the_connector_and_raises_connector_disabled_once(
        pool: sqlx::PgPool,
    ) {
        let h = harness_with(
            &pool,
            &[
                ("al-d", "connector_disabled", CONNECTOR),
                ("al-f", "connector_failure", CONNECTOR),
            ],
            &[("CONNECTOR_AUTO_DISABLE_AFTER", "3")],
        )
        .await;
        for run in ["run-1", "run-2", "run-3", "run-4"] {
            h.knows_run(run, Some(CONNECTOR), "FAILURE").await;
        }
        h.failed("run-1").await;
        h.failed("run-2").await;
        assert_eq!(paused_reason(&pool).await, None);
        assert_eq!(h.posts_to("al-d").await, [] as [std::string::String; 0]);

        let third = h.failed("run-3").await;
        assert_eq!(
            third["matched"], 2,
            "the failure rule and connector_disabled"
        );
        assert_eq!(
            paused_reason(&pool).await.as_deref(),
            Some("Paused after 3 failed runs in a row")
        );
        let posts = h.posts_to("al-d").await;
        assert_eq!(posts.len(), 1);
        for needle in [
            "run-3",
            CONNECTOR,
            "3 failed runs",
            "/connectors/conn-pg-lakehouse",
        ] {
            assert!(
                posts[0].contains(needle),
                "{needle} missing from {}",
                posts[0]
            );
        }

        assert_eq!(h.failed("run-3").await["matched"], 0, "a sensor retry");
        h.failed("run-4").await;
        assert_eq!(h.posts_to("al-d").await.len(), 1);
        assert_eq!(h.posts_to("al-f").await.len(), 4);
    }

    /// `SRC-11` D3 x `SRC-17` F2: when the pausing run's first report meets a
    /// rule-store failure, the connector is already paused and counted, and
    /// the retry delivers `connector_disabled` once.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rule_store_failure_on_the_pausing_report_still_delivers_connector_disabled_on_the_retry(
        pool: sqlx::PgPool,
    ) {
        let h = harness_with(
            &pool,
            &[("al-d", "connector_disabled", CONNECTOR)],
            &[("CONNECTOR_AUTO_DISABLE_AFTER", "2")],
        )
        .await;
        for run in ["run-1", "run-2"] {
            h.knows_run(run, Some(CONNECTOR), "FAILURE").await;
        }
        h.failed("run-1").await;
        rule_store_fails_once(&h).await;
        assert_eq!(
            failed_status(&h, "run-2").await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(paused_reason(&pool).await.is_some());
        assert_eq!(h.posts_to("al-d").await, [] as [std::string::String; 0]);

        assert_eq!(h.failed("run-2").await["matched"], 1);
        assert_eq!(h.failed("run-2").await["matched"], 0);
        assert_eq!(h.posts_to("al-d").await.len(), 1);
    }

    /// `SRC-11` D13: a run the orchestrator ended as "paused" is not counted
    /// as a success (it would reset the streak of the connector it just
    /// found paused) and delivers nothing. A run still `running` in the
    /// record is closed from the confirmed status.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_paused_run_is_not_counted_and_an_unclosed_run_is_settled(pool: sqlx::PgPool) {
        use lakehouse_store::connector_run::{
            EndReason, RunStatus, RunTrigger, end_run, run_status, start_run,
        };
        let h = harness(&pool, &[("al-s", "connector_success", CONNECTOR)]).await;
        for run in ["run-1", "run-2"] {
            h.knows_run(run, Some(CONNECTOR), "FAILURE").await;
            h.failed(run).await;
        }
        assert_eq!(streak(&pool).await.1, 2);

        let now = OffsetDateTime::now_utc();
        start_run(&pool, CONNECTOR, "run-p", RunTrigger::Other, None, now)
            .await
            .unwrap();
        end_run(
            &pool,
            CONNECTOR,
            "run-p",
            RunStatus::Paused,
            EndReason::ConnectorPaused,
            now,
        )
        .await
        .unwrap();
        h.knows_run("run-p", Some(CONNECTOR), "SUCCESS").await;
        let resp = run_finished_event(
            State(h.state.clone()),
            Extension(service()),
            body("run-p", "ingest_job"),
        )
        .await
        .expect("accepted")
        .0;
        assert_eq!(resp["matched"], 0);
        assert_eq!(streak(&pool).await.1, 2, "the streak was not reset");
        assert_eq!(h.posts_to("al-s").await, [] as [std::string::String; 0]);
        assert_eq!(
            run_status(&pool, CONNECTOR, "run-p")
                .await
                .unwrap()
                .as_deref(),
            Some("paused")
        );

        // A run the orchestrator never closed.
        start_run(&pool, CONNECTOR, "run-u", RunTrigger::Schedule, None, now)
            .await
            .unwrap();
        h.knows_run("run-u", Some(CONNECTOR), "FAILURE").await;
        h.failed("run-u").await;
        assert_eq!(
            run_status(&pool, CONNECTOR, "run-u")
                .await
                .unwrap()
                .as_deref(),
            Some("failed")
        );
    }
}
