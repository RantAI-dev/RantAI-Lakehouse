//! How far behind the source a change-capture connector is, and the alert for
//! it (`SRC-4` slice 4, decisions S4-b to S4-d). The meaning of lag is in
//! [`crate::capture_lag`]; this module is the routes around it.
//!
//! - `POST /api/connectors/{id}/capture/observations` is the orchestrator's: the
//!   sensor posts each table's backlog every tick, and the applier posts the
//!   source commit time of the last change it wrote. `ingest:read` is the one
//!   permission the orchestrator's service identity holds and a user can hold
//!   it too, so the handler adds `schema_changes::require_orchestrator`, as the
//!   run reports do. The route sits outside the tenant gate (a service identity
//!   belongs to no tenant).
//! - `PUT /api/connectors/{id}/lag-limit` is a person's (`connector:manage`, the
//!   tenant gate is the route layer).
//!
//! # The alert fires once per crossing
//!
//! Each observation post re-evaluates the connector. The `lag_alert_open` flag
//! is flipped with a compare-and-set UPDATE (`capture_lag::flip_lag_alert_open`)
//! and only the call that changed it delivers, so two posts racing over one
//! crossing send one alert. A delivery that cannot be made (the rule store is
//! unreadable) puts the flag back, so the next post tries again instead of
//! losing the alert, unless the undo fails too (then the flag stays as it was
//! flipped and the alert is lost), and a concurrent post that closes an alert
//! whose delivery then fails can leave the order of the two out of step. A
//! connector paused by a person is not evaluated.
//!
//! # Limit
//!
//! If the orchestrator is down nothing posts, so no alert fires; the read shows
//! "not known since" instead (`crate::capture_lag`).

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_alerts::AlertKind;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::audit as store_audit;
use lakehouse_store::capture_lag as store;
use lakehouse_store::connectors;
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::PgPool;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::capture_lag::{
    LagAlert, TableLag, alert_step, back_under_text, crossed_text, tables_lag, worst,
};
use crate::cdc_lifecycle::{self, CaptureState};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::kafka_connect::ConnectState;
use crate::routes::connector_runs::NOT_CHANGE_CAPTURE;
use crate::routes::connectors::{connector_audit_event, parse_body, pool};
use crate::routes::load_alerts::deliver_event;
use crate::routes::schema_changes::require_orchestrator;
use crate::state::AppState;

/// The most tables one post may name.
const MAX_TABLES: usize = 1000;

/// The `AlertItem.source` of a stored lag alert.
const ALERT_SOURCE: &str = "Change capture";

/// The title prefix of a lag alert's message.
const ALERT_TITLE: &str = "Change capture lag";

const BAD_TABLES: &str = "tables must hold between 1 and 1000 entries";
const NOT_SELECTED: &str = "a table is not one the connector selects";
const BAD_ENTRY: &str = "each table needs a backlog (0 or more), a lastSourceTsMs, or both";
const BAD_LIMIT: &str = "minutes must be a whole number from 1 to 1440";

/// One table's report.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TableReport {
    table: String,
    /// Unread messages on the table's topic.
    backlog: Option<i64>,
    /// Source commit time (milliseconds since the Unix epoch) of the last change
    /// the applier wrote to Bronze.
    last_source_ts_ms: Option<i64>,
}

/// The body of `POST .../capture/observations`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ObservationsBody {
    tables: Vec<TableReport>,
}

/// A report that passed validation.
struct Entry {
    table: String,
    backlog: Option<i64>,
    last_source_ts: Option<OffsetDateTime>,
}

/// The answer to `POST .../capture/observations`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationsResponse {
    /// How many table reports were stored.
    recorded: usize,
}

fn validate(req: &ObservationsBody, selected: &[String]) -> Result<Vec<Entry>, ApiError> {
    if req.tables.is_empty() || req.tables.len() > MAX_TABLES {
        return Err(ApiError::BadRequest(BAD_TABLES.to_owned()));
    }
    req.tables
        .iter()
        .map(|report| {
            if !selected.contains(&report.table) {
                return Err(ApiError::BadRequest(NOT_SELECTED.to_owned()));
            }
            let last_source_ts = report
                .last_source_ts_ms
                .map(|ms| OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000))
                .transpose()
                .map_err(|_| ApiError::BadRequest(BAD_ENTRY.to_owned()))?;
            let bad_backlog = report.backlog.is_some_and(|b| b < 0);
            if bad_backlog || (report.backlog.is_none() && last_source_ts.is_none()) {
                return Err(ApiError::BadRequest(BAD_ENTRY.to_owned()));
            }
            Ok(Entry {
                table: report.table.clone(),
                backlog: report.backlog,
                last_source_ts,
            })
        })
        .collect()
}

/// The tables connector `id` selects, from its stored ingest spec.
pub(super) async fn selected_tables(pool: &PgPool, id: &str) -> Result<Vec<String>, ApiError> {
    let spec = connectors::get_ingest_spec(pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    Ok(cdc_lifecycle::table_names(&spec.source_objects))
}

/// Whether `Kafka Connect` says connector `id` runs. Anything else (stopped,
/// failed, starting, unreachable, not configured) is "not running": a backlog of
/// 0 is not proof of being up to date then (`crate::capture_lag`).
pub(super) async fn is_running(state: &AppState, id: &str) -> Result<bool, ApiError> {
    Ok(matches!(
        cdc_lifecycle::capture_state(state, id).await?,
        CaptureState::State(ConnectState::Running)
    ))
}

/// `POST /api/connectors/{id}/capture/observations` -- see the module doc.
///
/// # Errors
///
/// 403 for a caller that is neither the orchestrator's service identity nor an
/// unrestricted administrator, 400 for a body that is not valid (no table, too
/// many, a table the connector does not select, a negative backlog, an entry
/// with nothing in it), 404 for an unknown connector, 422 for a connector that
/// does not capture changes, 503/500 as the store reports. A lag alert that
/// cannot be evaluated or delivered is logged and does not fail the post.
pub async fn observe(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<ObservationsResponse>> {
    require_orchestrator(&principal)?;
    let req: ObservationsBody = parse_body(&body)?;
    let pool = pool(&state)?;
    if !cdc_lifecycle::is_change_capture(&state, &id).await? {
        return Err(ApiError::Unprocessable(NOT_CHANGE_CAPTURE.to_owned()).into());
    }
    let selected = selected_tables(pool, &id).await?;
    let entries = validate(&req, &selected)?;
    let running = is_running(&state, &id).await?;
    let now = OffsetDateTime::now_utc();
    for entry in &entries {
        if let Some(backlog) = entry.backlog {
            // Up to date needs both: nothing waiting AND the capture running.
            store::record_backlog(
                pool,
                &id,
                &entry.table,
                backlog,
                running && backlog == 0,
                now,
            )
            .await?;
        }
        if let Some(changed) = entry.last_source_ts {
            // Slice 4 review SHOULD-FIX 2: a source time in the future would stick
            // (`last_source_ts` only moves forward) and hide every crossing until the
            // wall clock caught up, so it is stored as now and the cap is logged.
            let changed = if changed > now {
                tracing::warn!(connector_id = %id, table = %entry.table, "a source change time in the future was capped to now");
                now
            } else {
                changed
            };
            store::record_last_change(pool, &id, &entry.table, changed).await?;
        }
    }
    // The observations are stored; the alert is a consequence, never a reason to
    // refuse them.
    if let Err(err) = evaluate(&state, pool, &id, &selected, running, now).await {
        tracing::warn!(?err, connector_id = %id, "the lag of a connector could not be evaluated");
    }
    Ok(ApiJson(ObservationsResponse {
        recorded: entries.len(),
    }))
}

/// Compare the connector's lag with its limit and alert on a change of state.
async fn evaluate(
    state: &AppState,
    pool: &PgPool,
    id: &str,
    selected: &[String],
    running: bool,
    now: OffsetDateTime,
) -> Result<(), ApiError> {
    let detail = connectors::get_connector(pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    if detail
        .connector
        .paused_reason
        .as_deref()
        .is_some_and(connectors::is_person_pause_reason)
    {
        return Ok(());
    }
    let stored = store::list_tables(pool, id).await?;
    let lags = tables_lag(selected, &stored, running, false, now);
    let limit = store::get_lag_limit(pool, id).await?;
    let name = &detail.connector.name;
    match alert_step(limit.open, limit.minutes, &lags) {
        LagAlert::Nothing => Ok(()),
        LagAlert::Crossed { table, seconds } => {
            let text = crossed_text(name, id, &table, seconds, limit.minutes);
            announce(state, pool, id, false, &text).await
        }
        LagAlert::BackUnder => {
            let text = back_under_text(name, id, limit.minutes);
            announce(state, pool, id, true, &text).await
        }
    }
}

/// Flip the open flag away from `was_open` and, for the call that flipped it,
/// deliver `text`. A delivery that fails puts the flag back.
async fn announce(
    state: &AppState,
    pool: &PgPool,
    id: &str,
    was_open: bool,
    text: &str,
) -> Result<(), ApiError> {
    if !store::flip_lag_alert_open(pool, id, was_open).await? {
        // Another post won this crossing and sends its alert.
        return Ok(());
    }
    if let Err(err) = deliver_event(
        state,
        pool,
        AlertKind::ConnectorLag,
        id,
        ALERT_TITLE,
        text,
        Some(ALERT_SOURCE),
    )
    .await
    {
        tracing::warn!(?err, connector_id = %id, "the lag alert was not delivered; the next post tries again");
        if let Err(undo) = store::flip_lag_alert_open(pool, id, !was_open).await {
            tracing::error!(%undo, connector_id = %id, "could not put the lag alert flag back");
        }
    }
    Ok(())
}

/// The body of `PUT /api/connectors/{id}/lag-limit`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LagLimitBody {
    minutes: i64,
}

/// The answer to `PUT /api/connectors/{id}/lag-limit`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LagLimitResponse {
    /// The limit now in force, in minutes.
    minutes: i32,
}

/// `PUT /api/connectors/{id}/lag-limit` -- set the lag a change-capture
/// connector may have before it alerts (`SRC-4` decision S4-d). Written to the
/// audit trail.
///
/// # Errors
///
/// 400 for a limit that is not a whole number from 1 to 1440, 404 for another
/// tenant's or an unknown connector, 422 for a connector that does not capture
/// changes, 503/500 as the store reports.
pub async fn set_limit(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<LagLimitResponse>> {
    let req: LagLimitBody = parse_body(&body)?;
    let minutes = i32::try_from(req.minutes)
        .ok()
        .filter(|m| (store::MIN_LAG_ALERT_MINUTES..=store::MAX_LAG_ALERT_MINUTES).contains(m))
        .ok_or_else(|| ApiError::BadRequest(BAD_LIMIT.to_owned()))?;
    let pool = pool(&state)?;
    if !cdc_lifecycle::is_change_capture(&state, &id).await? {
        return Err(ApiError::Unprocessable(NOT_CHANGE_CAPTURE.to_owned()).into());
    }
    store::set_lag_limit(pool, &id, minutes).await?;
    let event = connector_audit_event(
        &principal,
        "connector.lag_limit",
        &id,
        json!({ "minutes": minutes }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.lag_limit audit event");
    }
    Ok(ApiJson(LagLimitResponse { minutes }))
}

/// One table's lag as the console reads it.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TableLagJson {
    /// The source object's name.
    table: String,
    /// `up_to_date`, `behind`, `not_known_yet`, `not_known_since` or `paused`.
    status: &'static str,
    /// Whole seconds behind; present only for `behind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    seconds: Option<i64>,
    /// RFC 3339 time of the newest report; present only for `not_known_since`.
    #[serde(skip_serializing_if = "Option::is_none")]
    since: Option<String>,
}

/// The connector's worst table.
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorstJson {
    table: String,
    seconds: i64,
}

/// Lag as `GET /api/connectors/{id}/change-capture` carries it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LagReadout {
    /// One entry per selected table.
    tables: Vec<TableLagJson>,
    /// The table furthest behind; absent when none is behind by a number.
    #[serde(skip_serializing_if = "Option::is_none")]
    worst: Option<WorstJson>,
    /// The limit, in minutes.
    limit_minutes: i32,
    /// Whether a lag alert is open (fired, not yet answered by "back under").
    alert_open: bool,
}

fn table_json(table: &str, lag: TableLag) -> TableLagJson {
    let (status, seconds, since) = match lag {
        TableLag::Paused => ("paused", None, None),
        TableLag::NotKnownYet => ("not_known_yet", None, None),
        TableLag::NotKnownSince { since } => ("not_known_since", None, since.format(&Rfc3339).ok()),
        TableLag::UpToDate => ("up_to_date", None, None),
        TableLag::Behind { seconds } => ("behind", Some(seconds), None),
    };
    TableLagJson {
        table: table.to_owned(),
        status,
        seconds,
        since,
    }
}

/// Read connector `id`'s lag at `now` for the change-capture read.
///
/// # Errors
///
/// 404 for an unknown connector; 503/500 as the store reports.
pub(super) async fn read_lag(
    state: &AppState,
    id: &str,
    running: bool,
    now: OffsetDateTime,
) -> Result<LagReadout, ApiError> {
    let pool = pool(state)?;
    let detail = connectors::get_connector(pool, id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    let paused = detail
        .connector
        .paused_reason
        .as_deref()
        .is_some_and(connectors::is_person_pause_reason);
    let selected = selected_tables(pool, id).await?;
    let stored = store::list_tables(pool, id).await?;
    let lags = tables_lag(&selected, &stored, running, paused, now);
    let limit = store::get_lag_limit(pool, id).await?;
    Ok(LagReadout {
        worst: worst(&lags).map(|(table, seconds)| WorstJson {
            table: table.to_owned(),
            seconds,
        }),
        tables: lags.iter().map(|(t, l)| table_json(t, *l)).collect(),
        limit_minutes: limit.minutes,
        alert_open: limit.open,
    })
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
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::config::Config;

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

    /// A user holding `ingest:read` and `connector:manage`, a member of `tenants`.
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
        webhook: MockServer,
        connect: MockServer,
        /// Kept alive on purpose: a dropped `MockServer` clears its mocks.
        _clickhouse: MockServer,
        tenant: Uuid,
    }

    /// Postgres plus mocked `Kafka Connect` (which says the connector runs),
    /// `ClickHouse` and webhook servers. With `rules_up` the rule store answers
    /// with one `connector_lag` rule for every connector (its webhook is
    /// `/al-lag`); without it the rule store answers 500.
    async fn harness(pool: &sqlx::PgPool, rules_up: bool) -> Harness {
        let ch = MockServer::start().await;
        let webhook = MockServer::start().await;
        let connect = MockServer::start().await;
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
        env.insert("CH_URL".to_owned(), ch.uri());
        env.insert("KAFKA_CONNECT_URL".to_owned(), connect.uri());
        let mut state = AppState::new(Config::from_map(&env).expect("a valid test Config"));
        // The webhook mock listens on loopback, which the real address policy
        // refuses (`SEC-10`); the policy has its own tests.
        state.webhook_sender_override = Some(lakehouse_notify::MappedResolver::new().into_sender());
        if rules_up {
            let rule = json!({
                "id": "al-lag", "name": "Rule al-lag", "type": "connector_lag",
                "mart": "", "measure": "", "agg": "sum", "op": ">", "threshold": "0",
                "board": "", "channel": "webhook",
                "target": format!("{}/al-lag", webhook.uri()),
                "enabled": "1", "created_at": "", "severity": "high",
                "pipeline": "", "connector": "*",
            });
            Mock::given(method("POST"))
                .and(body_string_contains("FROM console.alert_rule"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({ "meta": [], "rows": 1, "data": [rule] })),
                )
                .mount(&ch)
                .await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200))
                .mount(&ch)
                .await;
        } else {
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&ch)
                .await;
        }
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&webhook)
            .await;
        let tenant: Uuid = sqlx::query_scalar("SELECT id FROM tenant LIMIT 1")
            .fetch_one(pool)
            .await
            .expect("a seeded tenant");
        Harness {
            state,
            webhook,
            connect,
            _clickhouse: ch,
            tenant,
        }
    }

    /// A change-capture connector of the harness tenant that selects two tables;
    /// `Kafka Connect` answers `RUNNING` for it.
    async fn cdc_connector(h: &Harness, name: &str) -> String {
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
        sqlx::query(
            "UPDATE connector SET tenant_id = $2, adapter = 'cdc', ingest_mode = 'cdc', \
             source_objects = $3 WHERE id = $1",
        )
        .bind(&id)
        .bind(h.tenant)
        .bind(json!([{ "name": "public.orders" }, { "name": "public.items" }]))
        .execute(pool)
        .await
        .expect("a change-capture spec");
        Mock::given(method("GET"))
            .and(path(format!("/connectors/lakehouse-{id}/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "connector": {"state": "RUNNING"},
                "tasks": [{"state": "RUNNING"}],
            })))
            .mount(&h.connect)
            .await;
        id
    }

    async fn call(
        h: &Harness,
        principal: &Principal,
        verb: &str,
        uri: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
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
        let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn post(h: &Harness, id: &str, body: Value) -> (StatusCode, Value) {
        call(
            h,
            &service(),
            "POST",
            &format!("/api/connectors/{id}/capture/observations"),
            Some(body),
        )
        .await
    }

    fn now_ms() -> i64 {
        i64::try_from(OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000).unwrap()
    }

    async fn deliveries(h: &Harness) -> Vec<String> {
        h.webhook
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    async fn open_flag(pool: &sqlx::PgPool, id: &str) -> bool {
        store::get_lag_limit(pool, id).await.unwrap().open
    }

    /// `SRC-4` S4-d: crossing the limit sends one alert however many posts see
    /// it; coming back under sends one more; and the fixed words hold only names
    /// and numbers.
    #[sqlx::test(migrations = "../../migrations")]
    async fn each_crossing_of_the_limit_alerts_once_and_each_return_under_alerts_once(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, true).await;
        let id = cdc_connector(&h, "lag one").await;
        let me = user(&[h.tenant]);
        let (status, answer) = call(
            &h,
            &me,
            "PUT",
            &format!("/api/connectors/{id}/lag-limit"),
            Some(json!({ "minutes": 1 })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["minutes"], 1);

        // Orders has changes waiting and the last applied one is ten minutes old;
        // items is up to date.
        let behind = json!({ "tables": [
            { "table": "public.orders", "backlog": 5, "lastSourceTsMs": now_ms() - 600_000 },
            { "table": "public.items", "backlog": 0 },
        ]});
        for _ in 0..3 {
            let (status, answer) = post(&h, &id, behind.clone()).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
            assert_eq!(answer["recorded"], 2);
        }
        let sent = deliveries(&h).await;
        assert_eq!(sent.len(), 1, "one alert for one crossing: {sent:?}");
        assert!(sent[0].contains("public.orders"), "{}", sent[0]);
        assert!(sent[0].contains("over the limit of 1 min"), "{}", sent[0]);
        assert!(open_flag(&pool, &id).await);

        let caught_up = json!({ "tables": [
            { "table": "public.orders", "backlog": 0 },
            { "table": "public.items", "backlog": 0 },
        ]});
        for _ in 0..2 {
            let (status, _) = post(&h, &id, caught_up.clone()).await;
            assert_eq!(status, StatusCode::OK);
        }
        let sent = deliveries(&h).await;
        assert_eq!(sent.len(), 2, "one more for the return: {sent:?}");
        assert!(
            sent[1].contains("back under the limit of 1 min"),
            "{}",
            sent[1]
        );
        assert!(!open_flag(&pool, &id).await);
    }

    /// A delivery that cannot be made (the rule store is down) puts the flag back,
    /// so the next post tries again instead of losing the alert.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_alert_that_cannot_be_delivered_is_tried_again_by_the_next_post(pool: sqlx::PgPool) {
        let h = harness(&pool, false).await;
        let id = cdc_connector(&h, "lag two").await;
        sqlx::query("UPDATE connector SET lag_alert_minutes = 1 WHERE id = $1")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        let behind = json!({ "tables": [
            { "table": "public.orders", "backlog": 5, "lastSourceTsMs": now_ms() - 600_000 },
        ]});
        let (status, _) = post(&h, &id, behind).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the observation is stored regardless"
        );
        assert!(!open_flag(&pool, &id).await, "the flag was put back");
        let stored = store::list_tables(&pool, &id).await.unwrap();
        assert_eq!(stored[0].backlog, Some(5));
    }

    /// Slice 4 review SHOULD-FIX 2: a source time in the future is stored as now.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_source_time_in_the_future_is_capped_to_now(pool: sqlx::PgPool) {
        let h = harness(&pool, true).await;
        let id = cdc_connector(&h, "lag cap").await;
        let (status, _) = post(
            &h,
            &id,
            json!({ "tables": [{ "table": "public.orders", "lastSourceTsMs": now_ms() + 3_600_000 }] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let stored = store::list_tables(&pool, &id).await.unwrap();
        let changed = stored[0].last_source_ts.unwrap();
        assert!(changed <= OffsetDateTime::now_utc(), "{changed}");
    }

    /// A connector paused by a person is not evaluated.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_connector_paused_by_a_person_raises_no_lag_alert(pool: sqlx::PgPool) {
        let h = harness(&pool, true).await;
        let id = cdc_connector(&h, "lag three").await;
        connectors::pause_connector(&pool, &id).await.unwrap();
        sqlx::query("UPDATE connector SET lag_alert_minutes = 1 WHERE id = $1")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        let behind = json!({ "tables": [
            { "table": "public.orders", "backlog": 5, "lastSourceTsMs": now_ms() - 600_000 },
        ]});
        let (status, _) = post(&h, &id, behind).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(deliveries(&h).await.len(), 0);
        assert!(!open_flag(&pool, &id).await);
    }

    /// Only the orchestrator posts; a body is refused for a table the connector
    /// does not select, a negative backlog, an empty entry, an unknown field and a
    /// connector that does not capture changes. Nothing is stored for a refusal.
    #[sqlx::test(migrations = "../../migrations")]
    async fn observations_are_the_orchestrators_and_a_bad_body_stores_nothing(pool: sqlx::PgPool) {
        let h = harness(&pool, true).await;
        let id = cdc_connector(&h, "lag four").await;
        let me = user(&[h.tenant]);
        let uri = format!("/api/connectors/{id}/capture/observations");
        let (status, _) = call(
            &h,
            &me,
            "POST",
            &uri,
            Some(json!({ "tables": [{ "table": "public.orders", "backlog": 0 }] })),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a user is not the orchestrator"
        );

        for body in [
            json!({ "tables": [] }),
            json!({ "tables": [{ "table": "public.other", "backlog": 0 }] }),
            json!({ "tables": [{ "table": "public.orders", "backlog": -1 }] }),
            json!({ "tables": [{ "table": "public.orders" }] }),
            json!({ "tables": [{ "table": "public.orders", "backlog": 0, "message": "x" }] }),
        ] {
            let (status, answer) = post(&h, &id, body.clone()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body} -> {answer}");
        }
        assert_eq!(store::list_tables(&pool, &id).await.unwrap().len(), 0);

        let plain = {
            let id = cdc_connector(&h, "lag five").await;
            sqlx::query("UPDATE connector SET adapter = 'sql' WHERE id = $1")
                .bind(&id)
                .execute(&pool)
                .await
                .unwrap();
            id
        };
        let (status, _) = post(
            &h,
            &plain,
            json!({ "tables": [{ "table": "public.orders", "backlog": 0 }] }),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = post(
            &h,
            "no-such-connector",
            json!({ "tables": [{ "table": "public.orders", "backlog": 0 }] }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The lag limit takes a whole number from 1 to 1440, is audited, is a
    /// change-capture connector's only, and another tenant's user sees none.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_lag_limit_is_validated_audited_and_tenant_gated(pool: sqlx::PgPool) {
        let h = harness(&pool, true).await;
        let id = cdc_connector(&h, "lag six").await;
        let me = user(&[h.tenant]);
        let uri = format!("/api/connectors/{id}/lag-limit");
        for bad in [
            json!({ "minutes": 0 }),
            json!({ "minutes": 1441 }),
            json!({ "minutes": 1.5 }),
            json!({ "minutes": "5" }),
            json!({}),
        ] {
            let (status, _) = call(&h, &me, "PUT", &uri, Some(bad.clone())).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}");
        }
        for ok in [1, 1440] {
            let (status, answer) = call(&h, &me, "PUT", &uri, Some(json!({ "minutes": ok }))).await;
            assert_eq!(status, StatusCode::OK, "{answer}");
        }
        assert_eq!(
            store::get_lag_limit(&pool, &id).await.unwrap().minutes,
            1440
        );
        let (audited,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM audit_event WHERE action = 'connector.lag_limit' AND resource_id = $1",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audited, 2);
        let stranger = user(&[Uuid::from_u128(99)]);
        let (status, _) = call(&h, &stranger, "PUT", &uri, Some(json!({ "minutes": 5 }))).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The change-capture read gives each table's lag in fixed words: a table
    /// nobody reported is "not known yet", an empty topic with the capture
    /// running is "up to date", changes waiting is "behind" with the seconds, the
    /// worst table is named, and a person's pause shows no number.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_change_capture_read_words_each_tables_lag_and_never_invents_one(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, true).await;
        let id = cdc_connector(&h, "lag seven").await;
        let me = user(&[h.tenant]);
        let uri = format!("/api/connectors/{id}/change-capture");
        let (_, answer) = call(&h, &me, "GET", &uri, None).await;
        assert_eq!(answer["state"], "running", "{answer}");
        assert_eq!(answer["lag"]["limitMinutes"], 15);
        assert_eq!(answer["lag"]["alertOpen"], false);
        let statuses: Vec<&str> = answer["lag"]["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["status"].as_str().unwrap())
            .collect();
        assert_eq!(statuses, ["not_known_yet", "not_known_yet"]);
        assert!(answer["lag"].get("worst").is_none());

        post(
            &h,
            &id,
            json!({ "tables": [
                { "table": "public.orders", "backlog": 9, "lastSourceTsMs": now_ms() - 90_000 },
                { "table": "public.items", "backlog": 0 },
            ]}),
        )
        .await;
        let (_, answer) = call(&h, &me, "GET", &uri, None).await;
        let tables = answer["lag"]["tables"].as_array().unwrap();
        let by_name = |name: &str| tables.iter().find(|t| t["table"] == name).unwrap().clone();
        assert_eq!(by_name("public.items")["status"], "up_to_date");
        let orders = by_name("public.orders");
        assert_eq!(orders["status"], "behind");
        let seconds = orders["seconds"].as_i64().unwrap();
        assert!((90..=100).contains(&seconds), "{seconds}");
        assert_eq!(answer["lag"]["worst"]["table"], "public.orders");

        // Seen long ago: not known since, with the time, not a stale number.
        sqlx::query(
            "UPDATE connector_capture_table SET observed_at = now() - interval '10 minutes' \
             WHERE connector_id = $1",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
        let (_, answer) = call(&h, &me, "GET", &uri, None).await;
        let first = &answer["lag"]["tables"][0];
        assert_eq!(first["status"], "not_known_since", "{answer}");
        assert!(first["since"].is_string());
        assert!(first.get("seconds").is_none());

        connectors::pause_connector(&pool, &id).await.unwrap();
        let (_, answer) = call(&h, &me, "GET", &uri, None).await;
        assert_eq!(answer["lag"]["tables"][0]["status"], "paused");
        assert!(answer["lag"]["tables"][0].get("seconds").is_none());
    }
}
