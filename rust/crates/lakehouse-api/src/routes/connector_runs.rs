//! Connector operations (`SRC-11`): the run history and run log a person
//! reads, the Resume button, and the reports the orchestrator posts to fill
//! the record.
//!
//! # The two audiences
//!
//! A person with `connector:manage` on a connector of their own tenant
//! (decision D8; the tenant gate is the route layer, as for the other
//! per-connector routes):
//!
//! - `GET /api/connectors/{id}/runs` -- the last 50 runs (D7);
//! - `GET /api/connectors/{id}/runs/{runId}/log` -- one run with its
//!   attempts and log lines; `?format=text` is the same lines as a download
//!   (D5);
//! - `POST /api/connectors/{id}/resume` -- lifts an auto-disable pause (D4).
//!
//! The orchestrator, on its `ingest:read` service identity (the same one
//! `routes::schema_changes` talks to), reports what happened:
//!
//! - `POST /api/connectors/{id}/run-reports` -- a run started;
//! - `POST /api/connectors/{id}/run-reports/{runId}/attempts` -- one attempt
//!   at one table ended;
//! - `POST /api/connectors/{id}/run-reports/{runId}/log` -- a line of the
//!   closed `LogEvent` list;
//! - `POST /api/connectors/{id}/run-reports/{runId}/end` -- the run ended.
//!
//! Those four sit OUTSIDE the tenant gate, for the reason
//! `routes::schema_changes` gives: a service identity belongs to no tenant.
//! `ingest:read` is also held by a user (the seeded Data Engineer), so each
//! handler adds the `schema_changes::require_orchestrator` check: a service
//! identity or an unrestricted administrator only.
//!
//! # Principle 4
//!
//! The report bodies take no free text about a failure. An attempt names its
//! failure with one of the fixed `ErrorClass` words, an unknown word is
//! refused, and a body with any other field is refused
//! (`deny_unknown_fields`); the log lines are rendered by
//! `lakehouse_store::connector_run` from those words, a table name and
//! numbers. So neither the log, the history nor the download can hold a
//! driver's, a source's or the orchestrator's error text, whatever the
//! orchestrator sends.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::audit as store_audit;
use lakehouse_store::connector_run::{
    self, AttemptReport, EndReason, ErrorClass, LogEvent, RUN_HISTORY_LIMIT, RunStatus, RunSummary,
    RunTrigger,
};
use lakehouse_store::connectors::{self, ResumeOutcome};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use super::schema_changes::{check_name, require_orchestrator};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::connectors::{connector_audit_event, parse_body, pool};
use crate::state::AppState;

/// The longest run id accepted; the orchestrator's (`Dagster`) are 36 characters.
const MAX_RUN_ID_LEN: usize = 128;
/// The longest table (source object) name accepted.
const MAX_TABLE_LEN: usize = 256;
/// The most attempts one table can report in a run; the policy allows 4.
const MAX_ATTEMPT: i32 = 50;
/// The longest wait before a next attempt accepted: one day.
const MAX_WAIT_SECONDS: i32 = 86_400;

/// Shown when Resume finds nothing paused.
const NOT_PAUSED: &str = "This connector is not paused.";
/// Shown when Resume finds a pause that repeated failures did not set.
const PAUSED_FOR_ANOTHER_REASON: &str = "This connector is not paused because of repeated \
     failures. If a schema change is waiting, approve it under Schema changes to resume it.";

// ── The person's routes ────────────────────────────────────────────────────

/// `GET /api/connectors/{id}/runs`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsResponse {
    /// The connector's newest runs, newest first, at most 50.
    runs: Vec<RunSummary>,
}

/// `GET /api/connectors/{id}/runs` -- the connector's last 50 runs with
/// status, trigger, rows, duration and attempts (`SRC-11` decision D7).
/// Exact for any number of connectors: the rows are the product's own, keyed
/// by connector, not a search of a shared run list.
///
/// # Errors
///
/// 404 for another tenant's or an unknown connector (the route layer), 503/500
/// as the store reports.
pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<RunsResponse>> {
    Ok(ApiJson(RunsResponse {
        runs: connector_run::list_runs(pool(&state)?, &id, RUN_HISTORY_LIMIT).await?,
    }))
}

/// The `?format=` of the log route.
#[derive(Debug, Deserialize)]
pub struct LogQuery {
    /// `json` (the default) or `text`.
    format: Option<String>,
}

/// A run id as part of a file name: letters, digits, `-`, `_` and `.` only.
fn file_stem(run_id: &str) -> String {
    run_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `GET /api/connectors/{id}/runs/{runId}/log` -- one run with its attempts
/// and log lines (`SRC-11` decision D5). `?format=text` answers the same
/// lines as a `text/plain` attachment.
///
/// Lines and attempts older than 30 days are gone (the daily job); the run
/// itself stays, so an old run answers with empty `attempts` and `lines`.
///
/// # Errors
///
/// 404 for an unknown run, a run of another connector, or another tenant's or
/// unknown connector (the route layer); 400 for a `format` other than `json`
/// or `text`; 503/500 as the store reports.
pub async fn log(
    State(state): State<AppState>,
    Path((id, run_id)): Path<(String, String)>,
    Query(query): Query<LogQuery>,
) -> ApiResult<Response> {
    let as_text = match query.format.as_deref() {
        None | Some("json") => false,
        Some("text") => true,
        Some(_) => {
            return Err(ApiError::BadRequest("format is json or text".to_owned()).into());
        }
    };
    let log = connector_run::get_run_log(pool(&state)?, &id, &run_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Run {run_id} not found")))?;
    if !as_text {
        return Ok(ApiJson(log).into_response());
    }
    // Bypasses the `ApiJson` choke point on purpose: the body is a text file
    // for download, not a JSON value (the same reason `routes::query`'s
    // download does). Its lines are the ones `log` holds, rendered by the
    // store from fixed words, so there is no source text to leak.
    let mut response = log.to_text().into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    if let Ok(value) = HeaderValue::from_str(&format!(
        "attachment; filename=\"run-{}.log\"",
        file_stem(&run_id)
    )) {
        response
            .headers_mut()
            .insert(header::CONTENT_DISPOSITION, value);
    }
    Ok(response)
}

/// The answer to `POST /api/connectors/{id}/resume`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeResponse {
    /// Always `true`; any other outcome is a 409.
    resumed: bool,
}

/// `POST /api/connectors/{id}/resume` -- lift the pause an auto-disable set
/// and give the connector its full number of tries again (`SRC-11` decision
/// D4). Only that pause: a schema-change pause is lifted by approving the
/// change, and answers 409 here. Written to the audit trail.
///
/// # Errors
///
/// 409 with a fixed text when the connector is not paused or is paused for
/// another reason; 404 for another tenant's or an unknown connector; 503/500
/// as the store reports.
pub async fn resume(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<ResumeResponse>> {
    let pool = pool(&state)?;
    match connectors::resume_connector(pool, &id).await? {
        ResumeOutcome::Resumed => {}
        ResumeOutcome::NotPaused => {
            return Err(ApiError::Conflict(NOT_PAUSED.to_owned()).into());
        }
        ResumeOutcome::PausedForAnotherReason => {
            return Err(ApiError::Conflict(PAUSED_FOR_ANOTHER_REASON.to_owned()).into());
        }
    }
    let event = connector_audit_event(
        &principal,
        "connector.resume",
        &id,
        json!({ "pausedBy": "auto_disable" }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.resume audit event");
    }
    Ok(ApiJson(ResumeResponse { resumed: true }))
}

// ── The orchestrator's reports ─────────────────────────────────────────────

/// A time the orchestrator sends: RFC 3339, or now when it sent none.
fn report_time(field: &str, raw: Option<&str>) -> Result<OffsetDateTime, ApiError> {
    match raw {
        None => Ok(OffsetDateTime::now_utc()),
        Some(raw) => OffsetDateTime::parse(raw, &Rfc3339)
            .map_err(|_| ApiError::BadRequest(format!("{field} is not an RFC 3339 time"))),
    }
}

/// What every report answers: whether this call recorded something
/// (`false` for a repeat of a report already recorded).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedResponse {
    /// `true` when the report was new.
    recorded: bool,
}

/// Fail with 404 when the connector does not exist, so every report names the
/// same missing thing the same way.
async fn require_connector(state: &AppState, id: &str) -> Result<(), ApiError> {
    connectors::get_connector(pool(state)?, id)
        .await?
        .map(|_| ())
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))
}

/// The `StoreError::NotFound` of a report on an unknown run, said as such.
fn run_not_found(run_id: &str) -> impl Fn(lakehouse_store::StoreError) -> ApiError + '_ {
    move |err| match err {
        lakehouse_store::StoreError::NotFound => {
            ApiError::NotFound(format!("Run {run_id} not found for this connector"))
        }
        other => ApiError::from(other),
    }
}

/// The `POST .../run-reports` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StartBody {
    /// The orchestrator's run id.
    run_id: String,
    /// What started the run.
    trigger: RunTrigger,
    /// When it started, RFC 3339; the time of this report when absent.
    #[serde(default)]
    started_at: Option<String>,
}

/// `POST /api/connectors/{id}/run-reports` -- a run started (`SRC-11`
/// decision D7). Repeating the report answers `recorded: false`.
///
/// # Errors
///
/// 403 for a caller that is neither the orchestrator's service identity nor
/// an unrestricted administrator; 400 for an invalid body; 404 for an unknown
/// connector; 409 when the run id already belongs to another connector;
/// 503/500 as the store reports.
pub async fn report_start(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<RecordedResponse>> {
    require_orchestrator(&principal)?;
    let req: StartBody = parse_body(&body)?;
    check_name("runId", &req.run_id, MAX_RUN_ID_LEN)?;
    let started_at = report_time("startedAt", req.started_at.as_deref())?;
    require_connector(&state, &id).await?;
    let recorded =
        connector_run::start_run(pool(&state)?, &id, &req.run_id, req.trigger, started_at).await?;
    Ok(ApiJson(RecordedResponse { recorded }))
}

/// The `POST .../run-reports/{runId}/attempts` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptBody {
    /// The source object's name.
    table: String,
    /// 1 for the first attempt.
    attempt: i32,
    /// When the attempt started, RFC 3339.
    started_at: String,
    /// When it ended, RFC 3339.
    ended_at: String,
    /// Rows the attempt loaded; 0 when absent.
    #[serde(default)]
    rows_loaded: i64,
    /// Absent for a success; one of the fixed failure words otherwise.
    #[serde(default)]
    error_class: Option<ErrorClass>,
    /// Seconds before the next attempt, only for a failure that is retried.
    #[serde(default)]
    wait_seconds: Option<i32>,
}

/// `POST /api/connectors/{id}/run-reports/{runId}/attempts` -- one attempt at
/// one table ended (`SRC-11` decisions D1, D2, D5). The first report of a
/// `(run, table, attempt)` is stored and writes its log lines; a repeat
/// answers `recorded: false`.
///
/// # Errors
///
/// 403 as [`report_start`]; 400 for an invalid body (an unknown field, an
/// unknown `errorClass`, a time that is not RFC 3339, an attempt outside
/// 1..=50, a wait outside a day, an end before the start, a wait on a success);
/// 404 for an unknown connector or a run that is not this connector's;
/// 503/500 as the store reports.
pub async fn report_attempt(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((id, run_id)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<ApiJson<RecordedResponse>> {
    require_orchestrator(&principal)?;
    let req: AttemptBody = parse_body(&body)?;
    check_name("table", &req.table, MAX_TABLE_LEN)?;
    if !(1..=MAX_ATTEMPT).contains(&req.attempt) {
        return Err(ApiError::BadRequest(format!("attempt is 1 to {MAX_ATTEMPT}")).into());
    }
    if req.rows_loaded < 0 {
        return Err(ApiError::BadRequest("rowsLoaded must not be negative".to_owned()).into());
    }
    if req
        .wait_seconds
        .is_some_and(|w| !(0..=MAX_WAIT_SECONDS).contains(&w))
    {
        return Err(ApiError::BadRequest("waitSeconds is 0 to 86400".to_owned()).into());
    }
    if req.error_class.is_none() && req.wait_seconds.is_some() {
        return Err(ApiError::BadRequest("a success has no waitSeconds".to_owned()).into());
    }
    let started_at = report_time("startedAt", Some(req.started_at.as_str()))?;
    let ended_at = report_time("endedAt", Some(req.ended_at.as_str()))?;
    if ended_at < started_at {
        return Err(ApiError::BadRequest("endedAt is before startedAt".to_owned()).into());
    }
    require_connector(&state, &id).await?;
    let recorded = connector_run::record_attempt(
        pool(&state)?,
        &id,
        &run_id,
        &AttemptReport {
            table: &req.table,
            attempt: req.attempt,
            started_at,
            ended_at,
            rows: req.rows_loaded,
            error_class: req.error_class,
            wait_seconds: req.wait_seconds,
        },
    )
    .await
    .map_err(run_not_found(&run_id))?;
    Ok(ApiJson(RecordedResponse { recorded }))
}

/// The `POST .../run-reports/{runId}/log` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LogBody {
    /// One of the closed list of events.
    event: LogEvent,
    /// The table, for the events about one.
    #[serde(default)]
    table: Option<String>,
    /// A count, for `columns_left_out`.
    #[serde(default)]
    count: Option<i32>,
    /// When it happened, RFC 3339; the time of this report when absent.
    #[serde(default)]
    at: Option<String>,
}

/// `POST /api/connectors/{id}/run-reports/{runId}/log` -- add one line of the
/// closed `LogEvent` list to the run's log (`SRC-11` decision D5). The text of
/// the line is the store's, from the event, the table and the count.
///
/// # Errors
///
/// 403 as [`report_start`]; 400 for an invalid body or an event missing the
/// table or count it needs; 404 for an unknown connector or a run that is not
/// this connector's; 503/500 as the store reports.
pub async fn report_log(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((id, run_id)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<ApiJson<RecordedResponse>> {
    require_orchestrator(&principal)?;
    let req: LogBody = parse_body(&body)?;
    if let Some(table) = &req.table {
        check_name("table", table, MAX_TABLE_LEN)?;
    }
    if req.count.is_some_and(|n| n < 0) {
        return Err(ApiError::BadRequest("count must not be negative".to_owned()).into());
    }
    let at = report_time("at", req.at.as_deref())?;
    require_connector(&state, &id).await?;
    connector_run::add_log_line(
        pool(&state)?,
        &id,
        &run_id,
        req.event,
        req.table.as_deref(),
        req.count,
        at,
    )
    .await
    .map_err(|err| match err {
        lakehouse_store::StoreError::Validation(message) => ApiError::BadRequest(message),
        other => run_not_found(&run_id)(other),
    })?;
    Ok(ApiJson(RecordedResponse { recorded: true }))
}

/// The `POST .../run-reports/{runId}/end` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EndBody {
    /// `succeeded`, `failed` or `paused`.
    status: RunStatus,
    /// Why, one of the fixed words.
    reason: EndReason,
    /// When it ended, RFC 3339; the time of this report when absent.
    #[serde(default)]
    ended_at: Option<String>,
}

/// `POST /api/connectors/{id}/run-reports/{runId}/end` -- the run ended
/// (`SRC-11` decisions D5, D7, D13: `paused` is a run that loaded nothing
/// because the connector is paused). The first end wins; a repeat, or an end
/// after the run sensor already closed the run, answers `recorded: false`.
///
/// # Errors
///
/// 403 as [`report_start`]; 400 for an invalid body or `status: running`;
/// 404 for an unknown connector or a run that is not this connector's;
/// 503/500 as the store reports.
pub async fn report_end(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path((id, run_id)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<ApiJson<RecordedResponse>> {
    require_orchestrator(&principal)?;
    let req: EndBody = parse_body(&body)?;
    if req.status == RunStatus::Running {
        return Err(ApiError::BadRequest("a run cannot end as running".to_owned()).into());
    }
    let ended_at = report_time("endedAt", req.ended_at.as_deref())?;
    require_connector(&state, &id).await?;
    let recorded = connector_run::end_run(
        pool(&state)?,
        &id,
        &run_id,
        req.status,
        req.reason,
        ended_at,
    )
    .await
    .map_err(run_not_found(&run_id))?;
    Ok(ApiJson(RecordedResponse { recorded }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use std::collections::HashMap;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use lakehouse_auth::{PermissionSet, PrincipalId};
    use lakehouse_store::connectors::{
        CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec,
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;
    use crate::config::Config;

    /// A source's own error text, as the orchestrator might be tempted to
    /// forward. None of the routes may store or return it.
    const SOURCE_TEXT: &str = "password authentication failed for user \"app_reader\"";

    fn service() -> Principal {
        Principal {
            id: PrincipalId::Service(Uuid::from_u128(2)),
            tenant_ids: Vec::new(),
            display_name: "ingest-run-service".to_owned(),
            permissions: PermissionSet::parse("ingest:read"),
            provider: "service".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    /// A user holding `ingest:read` and `connector:manage`, a member of
    /// `tenants`.
    fn user(tenants: &[Uuid]) -> Principal {
        Principal {
            id: PrincipalId::User(Uuid::from_u128(1)),
            tenant_ids: tenants.to_vec(),
            display_name: "Rina Wijaya".to_owned(),
            permissions: PermissionSet::parse("ingest:read, connector:manage"),
            provider: "session".to_owned(),
            must_change_password: false,
            role_names: Vec::new(),
        }
    }

    struct Harness {
        state: AppState,
        tenant: Uuid,
    }

    /// Postgres only: these routes touch neither the orchestrator nor the
    /// rule store.
    async fn harness(pool: &sqlx::PgPool) -> Harness {
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
        let state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenant LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("a seeded tenant");
        Harness { state, tenant }
    }

    async fn connector(h: &Harness, name: &str) -> String {
        let pool = h.state.pg.as_deref().expect("pool");
        let input = CreateConnectorInput {
            name: name.to_owned(),
            kind: "PostgreSQL".to_owned(),
            direction: "source".to_owned(),
            host: "db.example".to_owned(),
            credential: CredentialSpec {
                source: CredentialSource::Env,
                primary: CredentialKind::Password,
                secondary: None,
            },
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: "in-region".to_owned(),
            capabilities: vec![],
            owner: None,
        };
        let id = connectors::create_connector(pool, &input)
            .await
            .expect("a connector")
            .0
            .id;
        sqlx::query("UPDATE connector SET tenant_id = $2 WHERE id = $1")
            .bind(&id)
            .bind(h.tenant)
            .execute(pool)
            .await
            .expect("tenant");
        id
    }

    /// One request through the real `/api/connectors` sub-router (tenant gate
    /// included, `auth_gate` not: `tests/route_auth.rs` covers the policy
    /// table) as `principal`; the body is returned as text.
    async fn send(
        h: &Harness,
        principal: &Principal,
        verb: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let app = crate::routes::connectors_router(&h.state)
            .layer(Extension(principal.clone()))
            .with_state(h.state.clone());
        let mut request = Request::builder().method(verb).uri(uri);
        let payload = match body {
            Some(value) => {
                request = request.header("content-type", "application/json");
                Body::from(serde_json::to_vec(&value).unwrap())
            }
            None => Body::empty(),
        };
        let response = app
            .oneshot(request.body(payload).unwrap())
            .await
            .expect("a response");
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            headers,
            String::from_utf8_lossy(&bytes).into_owned(),
        )
    }

    async fn call(
        h: &Harness,
        principal: &Principal,
        verb: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (status, _, text) = send(h, principal, verb, uri, body).await;
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    const T0: &str = "2026-10-14T10:00:00Z";
    const T1: &str = "2026-10-14T10:00:05Z";
    const T2: &str = "2026-10-14T10:01:10Z";
    const T3: &str = "2026-10-14T10:01:14Z";

    async fn report(h: &Harness, id: &str, path: &str, body: Value) -> (StatusCode, Value) {
        call(
            h,
            &service(),
            "POST",
            &format!("/api/connectors/{id}/run-reports{path}"),
            Some(body),
        )
        .await
    }

    /// `SRC-11` D5/D7: the orchestrator reports a run with a failed first
    /// attempt and a second that succeeds; the person sees the run in the
    /// history and its log (JSON and text), attempt by attempt, with the
    /// fixed words only.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_reported_run_is_listed_and_its_log_shows_each_attempt(pool: sqlx::PgPool) {
        let h = harness(&pool).await;
        let id = connector(&h, "runs one").await;
        let me = user(&[h.tenant]);

        let (status, answer) = report(
            &h,
            &id,
            "",
            json!({ "runId": "run-1", "trigger": "schedule", "startedAt": T0 }),
        )
        .await;
        assert_eq!(
            (status, &answer["recorded"]),
            (StatusCode::OK, &json!(true))
        );
        let (_, again) = report(&h, &id, "", json!({ "runId": "run-1", "trigger": "user" })).await;
        assert_eq!(again["recorded"], false, "a repeated start changes nothing");

        let (status, answer) = report(
            &h,
            &id,
            "/run-1/attempts",
            json!({
                "table": "public.orders", "attempt": 1, "startedAt": T0, "endedAt": T1,
                "errorClass": "unreachable", "waitSeconds": 60
            }),
        )
        .await;
        assert_eq!(
            (status, &answer["recorded"]),
            (StatusCode::OK, &json!(true))
        );
        let (status, _) = report(
            &h,
            &id,
            "/run-1/attempts",
            json!({
                "table": "public.orders", "attempt": 2, "startedAt": T2, "endedAt": T3,
                "rowsLoaded": 120
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = report(
            &h,
            &id,
            "/run-1/log",
            json!({ "event": "columns_left_out", "table": "public.orders", "count": 2 }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = report(
            &h,
            &id,
            "/run-1/end",
            json!({ "status": "succeeded", "reason": "completed", "endedAt": T3 }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, listed) =
            call(&h, &me, "GET", &format!("/api/connectors/{id}/runs"), None).await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let run = &listed["runs"][0];
        assert_eq!(listed["runs"].as_array().unwrap().len(), 1);
        assert_eq!(run["runId"], "run-1");
        assert_eq!(run["trigger"], "schedule");
        assert_eq!(run["status"], "succeeded");
        assert_eq!(run["rowsLoaded"], 120);
        assert_eq!(run["attempts"], 2);
        assert_eq!(run["durationSeconds"], 74);
        assert_eq!(run["endReason"], "completed");

        let uri = format!("/api/connectors/{id}/runs/run-1/log");
        let (status, log) = call(&h, &me, "GET", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{log}");
        assert_eq!(log["attempts"].as_array().unwrap().len(), 2);
        assert_eq!(log["attempts"][0]["errorClass"], "unreachable");
        assert_eq!(log["attempts"][0]["waitSeconds"], 60);
        assert_eq!(log["attempts"][1]["rowsLoaded"], 120);
        let events: Vec<&str> = log["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["event"].as_str().unwrap())
            .collect();
        assert_eq!(
            events,
            [
                "run_started",
                "attempt",
                "retry_wait",
                "attempt",
                "columns_left_out",
                "run_ended"
            ]
        );

        let (status, headers, text) =
            send(&h, &me, "GET", &format!("{uri}?format=text"), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            headers["content-type"].to_str().unwrap(),
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            headers["content-disposition"].to_str().unwrap(),
            "attachment; filename=\"run-run-1.log\""
        );
        // The download is the screen's lines, one per line, after a heading.
        assert_eq!(text.lines().count(), 1 + 6);
        for line in log["lines"].as_array().unwrap() {
            assert!(text.contains(line["text"].as_str().unwrap()));
        }
        assert!(text.contains("attempt 1: failed, the source could not be reached"));
        assert!(text.contains("2 columns left out"));

        let (status, _) = call(&h, &me, "GET", &format!("{uri}?format=xml"), None).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// `SRC-11` principle 4: a report cannot carry a source's error text. An
    /// unknown field and an unknown class are refused and nothing is stored;
    /// the log and the download hold none of it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_report_cannot_put_a_sources_error_text_in_the_log(pool: sqlx::PgPool) {
        let h = harness(&pool).await;
        let id = connector(&h, "runs two").await;
        let me = user(&[h.tenant]);
        report(&h, &id, "", json!({ "runId": "run-1", "trigger": "user" })).await;

        for body in [
            json!({
                "table": "t", "attempt": 1, "startedAt": T0, "endedAt": T1,
                "errorClass": "unreachable", "message": SOURCE_TEXT
            }),
            json!({
                "table": "t", "attempt": 1, "startedAt": T0, "endedAt": T1,
                "errorClass": SOURCE_TEXT
            }),
        ] {
            let (status, answer) = report(&h, &id, "/run-1/attempts", body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        }
        let (status, _) = report(
            &h,
            &id,
            "/run-1/log",
            json!({ "event": "note", "table": SOURCE_TEXT }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = report(
            &h,
            &id,
            "/run-1/attempts",
            json!({
                "table": "t", "attempt": 1, "startedAt": T0, "endedAt": T1,
                "errorClass": "sign_in_refused"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let uri = format!("/api/connectors/{id}/runs/run-1/log");
        let (_, json_log) = call(&h, &me, "GET", &uri, None).await;
        let (_, _, text) = send(&h, &me, "GET", &format!("{uri}?format=text"), None).await;
        assert!(!json_log.to_string().contains("password authentication"));
        assert!(!text.contains("password authentication"));
        assert!(text.contains("the source refused the sign-in"));
        assert_eq!(json_log["attempts"].as_array().unwrap().len(), 1);
    }

    /// Only the orchestrator's service identity (or an unrestricted
    /// administrator) files reports: a user holding `ingest:read` is
    /// refused, and so is a report for a connector that does not exist or a
    /// run of another connector.
    #[sqlx::test(migrations = "../../migrations")]
    async fn only_the_orchestrator_files_reports_and_a_run_belongs_to_one_connector(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool).await;
        let one = connector(&h, "runs three").await;
        let two = connector(&h, "runs four").await;
        let body = json!({ "runId": "run-1", "trigger": "schedule" });

        let (status, _) = call(
            &h,
            &user(&[h.tenant]),
            "POST",
            &format!("/api/connectors/{one}/run-reports"),
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = report(&h, "conn-nope", "", body.clone()).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        for bad in [
            json!({ "runId": "", "trigger": "schedule" }),
            json!({ "runId": "run-1", "trigger": "whenever" }),
            json!({ "runId": "run-1", "trigger": "schedule", "startedAt": "yesterday" }),
            json!({ "runId": "run-1", "trigger": "schedule", "extra": 1 }),
        ] {
            let (status, answer) = report(&h, &one, "", bad).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        }

        assert_eq!(report(&h, &one, "", body.clone()).await.0, StatusCode::OK);
        // The same run id for another connector is a conflict; a report on the
        // run through the other connector is a 404.
        assert_eq!(report(&h, &two, "", body).await.0, StatusCode::CONFLICT);
        let (status, _) = report(
            &h,
            &two,
            "/run-1/end",
            json!({ "status": "failed", "reason": "table_failed" }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = report(
            &h,
            &one,
            "/run-1/end",
            json!({ "status": "running", "reason": "other" }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = report(
            &h,
            &one,
            "/run-1/attempts",
            json!({
                "table": "t", "attempt": 0, "startedAt": T0, "endedAt": T1
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = report(
            &h,
            &one,
            "/run-1/attempts",
            json!({
                "table": "t", "attempt": 1, "startedAt": T1, "endedAt": T0
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "an end before the start");
    }

    /// D8: history, log and resume answer another tenant's user as an
    /// unknown connector (404), and a user of the tenant sees a run that does
    /// not exist as 404 too.
    #[sqlx::test(migrations = "../../migrations")]
    async fn another_tenants_user_gets_the_answer_an_unknown_connector_gets(pool: sqlx::PgPool) {
        let h = harness(&pool).await;
        let id = connector(&h, "runs five").await;
        report(&h, &id, "", json!({ "runId": "run-1", "trigger": "user" })).await;
        let stranger = user(&[Uuid::from_u128(99)]);
        let me = user(&[h.tenant]);

        for (verb, path) in [
            ("GET", format!("/api/connectors/{id}/runs")),
            ("GET", format!("/api/connectors/{id}/runs/run-1/log")),
            ("POST", format!("/api/connectors/{id}/resume")),
        ] {
            let (status, _) = call(&h, &stranger, verb, &path, None).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{verb} {path}");
        }
        let (status, _) = call(
            &h,
            &me,
            "GET",
            &format!("/api/connectors/{id}/runs/run-nope/log"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    async fn pause(pool: &sqlx::PgPool, id: &str, reason: &str) {
        sqlx::query(
            "UPDATE connector SET paused_reason = $2, paused_at = now(), failure_streak = 30 \
             WHERE id = $1",
        )
        .bind(id)
        .bind(reason)
        .execute(pool)
        .await
        .unwrap();
    }

    /// `SRC-11` D4: Resume lifts the auto-disable pause, sets the count to 0
    /// and is audited; with nothing paused, or a schema-change pause, it
    /// answers 409 with a fixed text and changes nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn resume_lifts_only_the_auto_disable_pause_and_is_audited(pool: sqlx::PgPool) {
        let h = harness(&pool).await;
        let id = connector(&h, "runs six").await;
        let me = user(&[h.tenant]);
        let uri = format!("/api/connectors/{id}/resume");

        let (status, answer) = call(&h, &me, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], NOT_PAUSED);

        pause(
            &pool,
            &id,
            lakehouse_store::schema_change::SCHEMA_CHANGE_PAUSE_REASON,
        )
        .await;
        let (status, answer) = call(&h, &me, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], PAUSED_FOR_ANOTHER_REASON);
        assert!(answer["error"].as_str().unwrap().contains("Schema changes"));

        pause(&pool, &id, "Paused after 30 failed runs in a row").await;
        let (status, answer) = call(&h, &me, "POST", &uri, None).await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["resumed"], true);
        let (paused, streak): (bool, i32) = sqlx::query_as(
            "SELECT paused_at IS NOT NULL, failure_streak FROM connector WHERE id = $1",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!((paused, streak), (false, 0));
        let (audited,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM audit_event WHERE action = 'connector.resume' AND resource_id = $1",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audited, 1);
        let (status, _) = call(&h, &me, "POST", &uri, None).await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a second resume finds nothing paused"
        );
    }

    /// The history answers the newest 50 whatever other connectors ran.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_history_route_answers_fifty_runs_of_this_connector_only(pool: sqlx::PgPool) {
        let h = harness(&pool).await;
        let id = connector(&h, "runs seven").await;
        let other = connector(&h, "runs eight").await;
        let base = OffsetDateTime::now_utc() - time::Duration::hours(3);
        let store = h.state.pg.as_deref().unwrap();
        for n in 0..55 {
            connector_run::start_run(
                store,
                &id,
                &format!("run-{n:02}"),
                RunTrigger::Schedule,
                base + time::Duration::minutes(n),
            )
            .await
            .unwrap();
        }
        connector_run::start_run(store, &other, "other-1", RunTrigger::User, base)
            .await
            .unwrap();
        let (status, listed) = call(
            &h,
            &user(&[h.tenant]),
            "GET",
            &format!("/api/connectors/{id}/runs"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let runs = listed["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 50);
        assert_eq!(runs[0]["runId"], "run-54");
        assert!(
            runs.iter()
                .all(|r| r["runId"].as_str().unwrap().starts_with("run-"))
        );
    }

    #[test]
    fn a_run_id_becomes_a_safe_file_name() {
        assert_eq!(file_stem("a1b2-c3_d.4"), "a1b2-c3_d.4");
        assert_eq!(file_stem("../../etc\"x y"), ".._.._etc_x_y");
    }
}
