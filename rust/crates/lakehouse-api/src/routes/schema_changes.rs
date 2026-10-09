//! Schema changes at the source (`SRC-8`): the orchestrator's observation
//! route and the three routes a person uses (`GET` the list, `POST` the
//! approval; the policy is a field of `PATCH /api/connectors/{id}`).
//!
//! # Who decides
//!
//! The API decides, the orchestrator observes (plan section 2). Before a
//! table loads, `ingest_factory.py` posts the columns it sees to
//! `POST /api/connectors/{id}/schema-observations`. This module compares
//! them with the last accepted columns (`lakehouse_store::schema_diff`,
//! pure), applies the connector's policy, writes the changes, the baseline
//! and the pause in one transaction (`lakehouse_store::schema_change`),
//! raises the alert, and answers `load` (optionally with the columns to
//! load) or `wait`. The orchestrator obeys the answer and fails closed when
//! it cannot get one.
//!
//! # Who may call the observation route
//!
//! Policy: `ingest:read`, the one permission the orchestrator's ingest
//! identity holds (`main::bootstrap_ingest_run_service`, scoped to
//! `ingest:read` ONLY, WS3 plan review X7). That permission is also held by
//! a user (the seeded Data Engineer), so the handler adds the check
//! `routes::pipelines::run_failed_event` makes: only a service identity or
//! an unrestricted administrator. No new permission was invented, and the
//! identity's scope did not grow.
//!
//! This route sits OUTSIDE `require_connector_in_tenants`: a service
//! identity belongs to no tenant (`Principal::tenant_ids` is empty), so the
//! tenant gate would refuse the only caller the route has. Like
//! `GET /api/connectors/ingestible`, it reaches every connector by id. The
//! three user routes sit behind the gate.
//!
//! # Principle 4
//!
//! An alert and a response hold connector, table and column names, counts
//! and a relative link, never source data or upstream error text. Failures
//! of the rule store or the alert delivery are logged and never fail the
//! observation: the orchestrator must still get its decision.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_alerts::AlertKind;
use lakehouse_auth::{Principal, PrincipalId};
use lakehouse_core::ApiError;
use lakehouse_store::audit as store_audit;
use lakehouse_store::schema_change::{
    self, InactiveColumn, ObservationTx, ObservationWrite, ObservedColumn, RecordedChange,
    SchemaChange, SchemaChangePolicy,
};
use lakehouse_store::schema_diff::{Action, Decision, TableShape, evaluate};
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::connectors::{connector_audit_event, parse_body, pool};
use crate::routes::load_alerts::deliver_event;
use crate::state::AppState;

/// Longest table name and column name accepted (`SQL Server` and `Oracle`
/// names are far shorter; a schema-qualified `object` fits).
const MAX_NAME_LEN: usize = 256;
/// Longest type string accepted (`numeric(38,10)[]` is nowhere near it).
const MAX_TYPE_LEN: usize = 256;
/// Most columns one observation may carry. The widest tables of the
/// supported sources hold about a thousand; beyond this is a mistake.
const MAX_COLUMNS: usize = 2000;
/// Longest run id accepted.
const MAX_RUN_ID_LEN: usize = 128;
/// Recent (no longer waiting) changes `GET .../schema-changes` returns.
const RECENT_CHANGES: i64 = 50;
/// The `source` label of the `alert_instance` written for a schema change.
const SCHEMA_SOURCE: &str = "Schema changes";

/// Fixed 500 text for a stored policy this code does not know.
const UNKNOWN_POLICY: &str = "the connector's schema-change policy is not one this version knows";

/// When the orchestrator observes (decision D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Before the table loads: the source can hold the table back.
    BeforeLoad,
    /// After the load (files, REST, `MongoDB`, Kafka, SFTP): the change is
    /// already in the table, so nothing can be held back.
    AfterLoad,
}

/// The `POST .../schema-observations` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservationBody {
    /// The source table (or object).
    object: String,
    /// Its columns as the orchestrator sees them.
    columns: Vec<ObservedColumn>,
    /// Its primary key; empty when it has none.
    primary_key: Vec<String>,
    /// Before or after the load.
    phase: Phase,
    /// The run that observes.
    #[serde(default)]
    run_id: Option<String>,
}

/// The answer to an observation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ObservationResponse {
    /// `"load"` or `"wait"`.
    action: &'static str,
    /// With `load`: the columns to load; `null` means all of them.
    columns: Option<Vec<String>>,
    /// The changes of this observation: those applied now and those that
    /// wait (a still-waiting change is listed again, not as new).
    changes: Vec<SchemaChange>,
}

/// `GET .../schema-changes`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaChangesResponse {
    /// What waits for a decision, oldest first.
    pending: Vec<SchemaChange>,
    /// The newest changes that no longer wait.
    recent: Vec<SchemaChange>,
    /// Columns marked inactive.
    inactive_columns: Vec<InactiveColumn>,
}

/// The `POST .../schema-changes/approve` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ApproveBody {
    /// The table whose waiting changes are approved.
    object: String,
}

/// The answer to an approval.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveResponse {
    /// The table.
    object: String,
    /// The changes now approved.
    approved: Vec<SchemaChange>,
    /// Whether the connector's pause was lifted.
    pause_lifted: bool,
}

/// A name that is not blank, short and free of control characters, so it
/// can sit in an alert text and a log line as it is.
fn check_name(what: &str, value: &str, max: usize) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::BadRequest(format!("{what} must not be blank")));
    }
    if value.chars().count() > max {
        return Err(ApiError::BadRequest(format!(
            "{what} is longer than {max} characters"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(ApiError::BadRequest(format!(
            "{what} must not contain control characters"
        )));
    }
    Ok(())
}

/// Bound and sanity-check an observation. Pure: unit-tested below.
///
/// # Errors
///
/// 400 for a blank or oversize name, no columns, too many columns, or a
/// repeated column name.
fn validate(body: &ObservationBody) -> Result<(), ApiError> {
    check_name("object", &body.object, MAX_NAME_LEN)?;
    if body.columns.is_empty() {
        return Err(ApiError::BadRequest(
            "columns must not be empty: a table with no columns is not an observation".to_owned(),
        ));
    }
    if body.columns.len() > MAX_COLUMNS || body.primary_key.len() > MAX_COLUMNS {
        return Err(ApiError::BadRequest(format!(
            "an observation carries at most {MAX_COLUMNS} columns"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for column in &body.columns {
        check_name("column name", &column.name, MAX_NAME_LEN)?;
        check_name("typeName", &column.type_name, MAX_TYPE_LEN)?;
        if !seen.insert(column.name.as_str()) {
            return Err(ApiError::BadRequest(
                "column names must be unique within a table".to_owned(),
            ));
        }
    }
    for key in &body.primary_key {
        check_name("primaryKey column", key, MAX_NAME_LEN)?;
    }
    if let Some(run_id) = &body.run_id {
        check_name("runId", run_id, MAX_RUN_ID_LEN)?;
    }
    Ok(())
}

/// What the observation compares, given the phase.
///
/// After the load (decision D4) a column missing from the batch cannot be
/// told from one that is empty in it, so the previous columns the batch
/// lacks are carried over: they are neither reported as removed nor dropped
/// from the baseline. The key is not observed for these sources and also
/// carries over.
fn shape_for_phase(
    phase: Phase,
    previous: Option<&TableShape>,
    columns: Vec<ObservedColumn>,
    primary_key: Vec<String>,
) -> TableShape {
    let (Phase::AfterLoad, Some(previous)) = (phase, previous) else {
        return TableShape {
            columns,
            primary_key,
        };
    };
    let mut merged = columns;
    for old in &previous.columns {
        if !merged.iter().any(|c| c.name == old.name) {
            merged.push(old.clone());
        }
    }
    TableShape {
        columns: merged,
        primary_key: previous.primary_key.clone(),
    }
}

/// `POST /api/connectors/{id}/schema-observations` -- see the module doc.
///
/// # Errors
///
/// 403 for a caller that is neither the orchestrator's service identity nor
/// an unrestricted administrator, 400 for an invalid body (a source of
/// unbounded size or a table with no columns is refused), 404 for an
/// unknown connector, 503/500 as the store reports.
pub async fn observe(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<ObservationResponse>> {
    // `ingest:read` is also a user's permission; only the orchestrator (or
    // an unrestricted administrator) reports what a source looks like.
    if !matches!(principal.id, PrincipalId::Service(_))
        && !crate::routes::catalog::is_unrestricted(&principal)
    {
        return Err(ApiError::PermissionDenied(
            "only the orchestrator's service identity reports source schemas".to_owned(),
        )
        .into());
    }
    let req: ObservationBody = parse_body(&body)?;
    validate(&req)?;
    let pool = pool(&state)?;
    let detail = lakehouse_store::connectors::get_connector(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    let Some(policy) = SchemaChangePolicy::parse(&detail.connector.schema_change_policy) else {
        // Fail closed: an unknown policy is not guessed at.
        tracing::error!(connector_id = %id, "connector has an unknown schema-change policy");
        return Err(ApiError::Internal(UNKNOWN_POLICY.to_owned()).into());
    };
    let can_hold_back = req.phase == Phase::BeforeLoad;

    let mut tx = ObservationTx::begin(pool, &id, &req.object).await?;
    let accepted = tx.accepted().await?;
    let previous = accepted.as_ref().map(TableShape::from);
    let observed = shape_for_phase(req.phase, previous.as_ref(), req.columns, req.primary_key);
    let decision = evaluate(policy, can_hold_back, previous.as_ref(), &observed);
    // Only changes that took effect (and a first observation) move the
    // baseline; a waiting change must keep being compared against what the
    // table was last told to hold.
    let accept_observed = previous.is_none()
        || (!decision.changes.is_empty()
            && decision
                .changes
                .iter()
                .all(|c| c.status == schema_change::ChangeStatus::Applied));
    let rows: Vec<_> = decision
        .changes
        .iter()
        .map(lakehouse_store::schema_diff::ClassifiedChange::to_new_change)
        .collect();
    let recorded = tx
        .record(&ObservationWrite {
            observed_columns: &observed.columns,
            observed_primary_key: &observed.primary_key,
            changes: &rows,
            accept_observed,
            pause_connector: decision.pause_connector,
            run_id: req.run_id.as_deref(),
            now: OffsetDateTime::now_utc(),
        })
        .await?;
    tx.commit().await?;

    announce(
        &state,
        pool,
        &id,
        &detail.connector.name,
        &req.object,
        &recorded,
    )
    .await;
    Ok(ApiJson(response(&decision, recorded)))
}

fn response(decision: &Decision, recorded: Vec<RecordedChange>) -> ObservationResponse {
    ObservationResponse {
        action: match decision.action {
            Action::Load => "load",
            Action::Wait => "wait",
        },
        columns: decision.columns.clone(),
        changes: recorded.into_iter().map(|r| r.change).collect(),
    }
}

/// Raise `connector_schema_change` once for the changes this observation
/// created. A still-waiting change seen again is not new and raises nothing
/// (identity: `lakehouse_store::schema_change`). Best effort: the
/// orchestrator needs its decision whatever the rule store or the delivery
/// does, so a failure is logged and swallowed.
async fn announce(
    state: &AppState,
    pool: &lakehouse_store::PgPool,
    connector_id: &str,
    connector_name: &str,
    object: &str,
    recorded: &[RecordedChange],
) {
    let new: Vec<&RecordedChange> = recorded.iter().filter(|r| r.is_new).collect();
    if new.is_empty() {
        return;
    }
    let waiting = new.iter().filter(|r| r.change.status == "pending").count();
    let text = format!(
        "Connector {connector_name} ({connector_id}): table {object} changed at the source, \
         {} change(s), {waiting} waiting for a decision. View: /connectors/{connector_id}",
        new.len(),
    );
    if let Err(err) = deliver_event(
        state,
        pool,
        AlertKind::ConnectorSchemaChange,
        connector_id,
        "Source schema changed",
        &text,
        Some(SCHEMA_SOURCE),
    )
    .await
    {
        tracing::warn!(?err, connector_id, "schema-change alert was not delivered");
    }
}

/// `GET /api/connectors/{id}/schema-changes` -- what waits, the most recent
/// changes (50) and the inactive columns of the connector.
///
/// # Errors
///
/// 404 for another tenant's or an unknown connector (the route layer), 503/500
/// as the store reports.
pub async fn list(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<SchemaChangesResponse>> {
    let pool = pool(&state)?;
    Ok(ApiJson(SchemaChangesResponse {
        pending: schema_change::list_pending(pool, &id).await?,
        recent: schema_change::list_recent(pool, &id, RECENT_CHANGES).await?,
        inactive_columns: schema_change::list_inactive_columns(pool, &id).await?,
    }))
}

/// `POST /api/connectors/{id}/schema-changes/approve` -- approve every
/// waiting change of one table (decision D5: approve only, no reject).
///
/// # Errors
///
/// 404 when nothing waits for that table or the connector is another
/// tenant's or unknown, 400 for a bad body, 503/500 as the store reports.
pub async fn approve(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<ApproveResponse>> {
    let req: ApproveBody = parse_body(&body)?;
    check_name("object", &req.object, MAX_NAME_LEN)?;
    let pool = pool(&state)?;
    let decided_by = principal.id.uuid().to_string();
    let approval = schema_change::approve_object(
        pool,
        &id,
        &req.object,
        Some(&decided_by),
        OffsetDateTime::now_utc(),
    )
    .await?
    .ok_or_else(|| ApiError::NotFound("No schema change is waiting for that table".to_owned()))?;
    // Counts and the table name only: never a column value, never source data.
    let event = connector_audit_event(
        &principal,
        "connector.schema_change.approve",
        &id,
        json!({
            "object": req.object,
            "changes": approval.approved.len(),
            "pauseLifted": approval.pause_lifted,
        }),
        "executed",
    );
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, connector_id = %id, "failed to record connector.schema_change.approve audit event");
    }
    Ok(ApiJson(ApproveResponse {
        object: req.object,
        approved: approval.approved,
        pause_lifted: approval.pause_lifted,
    }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn col(name: &str, type_name: &str) -> ObservedColumn {
        ObservedColumn {
            name: name.to_owned(),
            type_name: type_name.to_owned(),
            nullable: true,
        }
    }

    fn body(columns: Vec<ObservedColumn>) -> ObservationBody {
        ObservationBody {
            object: "orders".to_owned(),
            columns,
            primary_key: vec!["id".to_owned()],
            phase: Phase::BeforeLoad,
            run_id: None,
        }
    }

    #[test]
    fn a_plain_observation_is_valid() {
        assert!(validate(&body(vec![col("id", "integer")])).is_ok());
    }

    #[test]
    fn an_observation_with_no_columns_is_refused() {
        assert!(validate(&body(vec![])).is_err());
    }

    #[test]
    fn an_observation_with_too_many_columns_is_refused() {
        let columns = (0..=MAX_COLUMNS)
            .map(|i| col(&format!("c{i}"), "text"))
            .collect();
        assert!(validate(&body(columns)).is_err());
    }

    #[test]
    fn blank_long_and_control_character_names_are_refused() {
        let long = "x".repeat(MAX_NAME_LEN + 1);
        for name in ["", "  ", long.as_str(), "a\nb"] {
            assert!(
                validate(&body(vec![col(name, "text")])).is_err(),
                "{name:?}"
            );
        }
        let mut object = body(vec![col("id", "integer")]);
        object.object = "orders\u{7}".to_owned();
        assert!(validate(&object).is_err());
    }

    #[test]
    fn a_repeated_column_name_is_refused_but_a_case_difference_is_two_columns() {
        assert!(validate(&body(vec![col("id", "integer"), col("id", "text")])).is_err());
        assert!(validate(&body(vec![col("id", "integer"), col("Id", "text")])).is_ok());
    }

    #[test]
    fn the_body_refuses_unknown_fields_and_unknown_phases() {
        let ok = json!({"object": "o", "columns": [], "primaryKey": [], "phase": "before_load"});
        assert!(serde_json::from_value::<ObservationBody>(ok).is_ok());
        for bad in [
            json!({"object": "o", "columns": [], "primaryKey": [], "phase": "before_load", "x": 1}),
            json!({"object": "o", "columns": [], "primaryKey": [], "phase": "during"}),
            json!({"object": "o", "columns": [], "phase": "before_load"}),
            json!({"object": "o", "columns": [{"name": "a", "typeName": "t", "nullable": true, "x": 1}],
                   "primaryKey": [], "phase": "before_load"}),
        ] {
            assert!(serde_json::from_value::<ObservationBody>(bad).is_err());
        }
    }

    /// Decision D4: after the load, a column the batch lacks is not removed.
    #[test]
    fn after_the_load_a_missing_column_is_carried_over_not_removed() {
        let previous = TableShape {
            columns: vec![col("id", "integer"), col("note", "text")],
            primary_key: vec!["id".to_owned()],
        };
        let observed = shape_for_phase(
            Phase::AfterLoad,
            Some(&previous),
            vec![col("id", "integer")],
            Vec::new(),
        );
        assert_eq!(observed.columns.len(), 2);
        assert_eq!(observed.primary_key, vec!["id".to_owned()]);
        assert!(lakehouse_store::schema_diff::diff(Some(&previous), &observed).is_empty());
        let before = shape_for_phase(
            Phase::BeforeLoad,
            Some(&previous),
            vec![col("id", "integer")],
            vec!["id".to_owned()],
        );
        assert_eq!(before.columns.len(), 1);
    }

    // ---- routes, against Postgres and mocked ClickHouse / webhook ----------

    use std::collections::HashMap;

    use axum::body::{Body, to_bytes};
    use axum::http::{Request, StatusCode};
    use lakehouse_auth::PermissionSet;
    use lakehouse_store::connectors::{
        CreateConnectorInput, CredentialKind, CredentialSource, CredentialSpec,
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use uuid::Uuid;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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

    /// A user holding `ingest:read` (the Data Engineer's grant) and
    /// `connector:manage`, a member of `tenants`.
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
        dagster: MockServer,
        webhook: MockServer,
        tenant: Uuid,
    }

    /// Postgres plus mocked `Dagster`, `ClickHouse` and webhook servers.
    /// `rules` are `(id, kind, connector)` rows the rule store answers with;
    /// each rule's webhook is `/<id>` on the webhook mock (the approach of
    /// `load_alerts`' tests).
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
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&ch)
            .await;
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
            dagster,
            webhook,
            tenant,
        }
    }

    /// A new connector of the harness tenant on `policy`.
    async fn connector(h: &Harness, name: &str, policy: &str) -> String {
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
        let id = lakehouse_store::connectors::create_connector(pool, &input)
            .await
            .expect("a connector")
            .0
            .id;
        sqlx::query("UPDATE connector SET tenant_id = $2, schema_change_policy = $3 WHERE id = $1")
            .bind(&id)
            .bind(h.tenant)
            .bind(policy)
            .execute(pool)
            .await
            .expect("tenant and policy");
        id
    }

    /// One request through the real `/api/connectors` sub-router (tenant
    /// gate included, `auth_gate` not: `tests/route_auth.rs` covers the
    /// policy table) as `principal`.
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

    fn observation(object: &str, columns: &[(&str, &str)], key: &[&str], phase: &str) -> Value {
        json!({
            "object": object,
            "columns": columns.iter()
                .map(|(n, t)| json!({"name": n, "typeName": t, "nullable": true}))
                .collect::<Vec<_>>(),
            "primaryKey": key,
            "phase": phase,
            "runId": "run-1",
        })
    }

    /// Observe `orders` as the orchestrator and return the answer.
    async fn observe_as_service(
        h: &Harness,
        id: &str,
        columns: &[(&str, &str)],
        phase: &str,
    ) -> Value {
        let (status, answer) = call(
            h,
            &service(),
            "POST",
            &format!("/api/connectors/{id}/schema-observations"),
            Some(observation("orders", columns, &["id"], phase)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{answer}");
        answer
    }

    const BASE: [(&str, &str); 2] = [("id", "integer"), ("note", "text")];
    const WITH_QTY: [(&str, &str); 3] = [("id", "integer"), ("note", "text"), ("qty", "integer")];
    const WITHOUT_NOTE: [(&str, &str); 1] = [("id", "integer")];

    async fn posts(h: &Harness, rule: &str) -> Vec<String> {
        h.webhook
            .received_requests()
            .await
            .expect("request log")
            .iter()
            .filter(|r| r.url.path() == format!("/{rule}"))
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    async fn baseline_columns(pool: &sqlx::PgPool, id: &str) -> usize {
        let (n,): (i32,) = sqlx::query_as(
            "SELECT jsonb_array_length(columns)::int FROM connector_source_schema \
             WHERE connector_id = $1 AND object_name = 'orders'",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        usize::try_from(n).unwrap()
    }

    async fn paused(pool: &sqlx::PgPool, id: &str) -> bool {
        sqlx::query_scalar("SELECT paused_at IS NOT NULL FROM connector WHERE id = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    /// Spec "Per-connector policy": an added column under each of the four
    /// policies.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_added_column_is_handled_by_each_policy(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        for (policy, action, columns, status, baseline, is_paused) in [
            ("apply_non_breaking", "load", None, "applied", 3, false),
            ("apply_all", "load", None, "applied", 3, false),
            (
                "ask_first",
                "load",
                Some(json!(["id", "note"])),
                "pending",
                2,
                false,
            ),
            ("pause", "wait", None, "pending", 2, true),
        ] {
            let id = connector(&h, &format!("added under {policy}"), policy).await;
            let first = observe_as_service(&h, &id, &BASE, "before_load").await;
            assert_eq!(first["action"], "load");
            assert_eq!(
                first["changes"],
                json!([]),
                "a first observation is the baseline"
            );

            let answer = observe_as_service(&h, &id, &WITH_QTY, "before_load").await;
            assert_eq!(answer["action"], action, "{policy}");
            assert_eq!(
                answer["columns"],
                columns.unwrap_or(Value::Null),
                "{policy}"
            );
            assert_eq!(answer["changes"][0]["status"], status, "{policy}");
            assert_eq!(answer["changes"][0]["kind"], "column_added");
            assert_eq!(answer["changes"][0]["columnName"], "qty");
            assert_eq!(baseline_columns(&pool, &id).await, baseline, "{policy}");
            assert_eq!(paused(&pool, &id).await, is_paused, "{policy}");
        }
    }

    /// Spec "Breaking changes": a removed column waits under every policy
    /// and the baseline does not move.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_removed_column_waits_under_every_policy_and_the_baseline_stays(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        for policy in ["apply_non_breaking", "apply_all", "ask_first", "pause"] {
            let id = connector(&h, &format!("removed under {policy}"), policy).await;
            observe_as_service(&h, &id, &BASE, "before_load").await;
            let answer = observe_as_service(&h, &id, &WITHOUT_NOTE, "before_load").await;
            assert_eq!(answer["action"], "wait", "{policy}");
            assert_eq!(answer["columns"], Value::Null);
            assert_eq!(answer["changes"][0]["kind"], "column_removed");
            assert_eq!(answer["changes"][0]["status"], "pending");
            assert_eq!(answer["changes"][0]["breaking"], true);
            assert_eq!(baseline_columns(&pool, &id).await, 2, "{policy}");
            assert_eq!(paused(&pool, &id).await, policy == "pause", "{policy}");
        }
    }

    /// Decision D5: approving resumes the table, keeps the column's history
    /// marked inactive, and a column that returns is active again.
    #[sqlx::test(migrations = "../../migrations")]
    async fn approving_resumes_the_table_and_marks_the_column_inactive(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "approve", "apply_non_breaking").await;
        let manager = user(&[h.tenant]);
        observe_as_service(&h, &id, &BASE, "before_load").await;
        observe_as_service(&h, &id, &WITHOUT_NOTE, "before_load").await;

        let (status, listed) = call(
            &h,
            &manager,
            "GET",
            &format!("/api/connectors/{id}/schema-changes"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed["pending"].as_array().unwrap().len(), 1);
        assert_eq!(listed["recent"], json!([]));

        let (status, approved) = call(
            &h,
            &manager,
            "POST",
            &format!("/api/connectors/{id}/schema-changes/approve"),
            Some(json!({ "object": "orders" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{approved}");
        assert_eq!(approved["approved"][0]["status"], "approved");
        assert_eq!(
            approved["approved"][0]["decidedBy"],
            manager.id.uuid().to_string()
        );

        // The next run loads, and the removal is not detected again.
        let answer = observe_as_service(&h, &id, &WITHOUT_NOTE, "before_load").await;
        assert_eq!(answer["action"], "load");
        assert_eq!(answer["changes"], json!([]));
        let (_, listed) = call(
            &h,
            &manager,
            "GET",
            &format!("/api/connectors/{id}/schema-changes"),
            None,
        )
        .await;
        assert_eq!(listed["pending"], json!([]));
        assert_eq!(listed["recent"].as_array().unwrap().len(), 1);
        assert_eq!(listed["inactiveColumns"][0]["columnName"], "note");

        // Nothing waits any more: a second approval is a 404.
        let (status, _) = call(
            &h,
            &manager,
            "POST",
            &format!("/api/connectors/{id}/schema-changes/approve"),
            Some(json!({ "object": "orders" })),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // The column comes back: applied, and no longer inactive.
        let answer = observe_as_service(&h, &id, &BASE, "before_load").await;
        assert_eq!(answer["changes"][0]["kind"], "column_added");
        let (_, listed) = call(
            &h,
            &manager,
            "GET",
            &format!("/api/connectors/{id}/schema-changes"),
            None,
        )
        .await;
        assert_eq!(listed["inactiveColumns"], json!([]));
    }

    /// Spec "pause": the connector stops, `ingest/run` answers 409 without
    /// asking Dagster, and the due list leaves it out; approval lifts it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_pause_policy_pauses_the_connector_until_approval(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "paused", "pause").await;
        sqlx::query(
            "UPDATE connector SET adapter = 'sql', ingest_mode = 'batch', \
             schedule_cron = '0 2 * * *' WHERE id = $1",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
        let manager = user(&[h.tenant]);
        let due = "/api/connectors/ingestible?dueAfter=2026-09-30T01:59:00Z&dueUntil=2026-09-30T02:00:00Z";
        let listed = |body: &Value| {
            body.as_array()
                .unwrap()
                .iter()
                .any(|c| c["id"] == id.as_str())
        };
        let (_, before) = call(&h, &service(), "GET", due, None).await;
        assert!(listed(&before), "due before the pause: {before}");

        observe_as_service(&h, &id, &BASE, "before_load").await;
        let answer = observe_as_service(&h, &id, &WITH_QTY, "before_load").await;
        assert_eq!(answer["action"], "wait");
        assert!(paused(&pool, &id).await);

        let (status, refusal) = call(
            &h,
            &manager,
            "POST",
            &format!("/api/connectors/{id}/ingest/run"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(refusal.to_string().contains("paused"), "{refusal}");
        assert!(
            h.dagster.received_requests().await.expect("log").is_empty(),
            "Dagster must not be asked about a paused connector"
        );
        let (_, after) = call(&h, &service(), "GET", due, None).await;
        assert!(!listed(&after), "a paused connector is not due: {after}");
        let (_, all) = call(&h, &service(), "GET", "/api/connectors/ingestible", None).await;
        let entry = all
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == id.as_str())
            .unwrap();
        assert_eq!(
            entry["paused"], true,
            "the unfiltered list still carries it"
        );

        let (status, approved) = call(
            &h,
            &manager,
            "POST",
            &format!("/api/connectors/{id}/schema-changes/approve"),
            Some(json!({ "object": "orders" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(approved["pauseLifted"], true);
        assert!(!paused(&pool, &id).await);
        let (_, resumed) = call(&h, &service(), "GET", due, None).await;
        assert!(listed(&resumed));
    }

    /// Changing the policy away from `pause` does not lift a pause; only
    /// approval does. A policy outside the four is a 400.
    #[sqlx::test(migrations = "../../migrations")]
    async fn changing_the_policy_does_not_lift_a_pause(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "policy patch", "pause").await;
        let manager = user(&[h.tenant]);
        observe_as_service(&h, &id, &BASE, "before_load").await;
        observe_as_service(&h, &id, &WITH_QTY, "before_load").await;
        assert!(paused(&pool, &id).await);

        let (status, patched) = call(
            &h,
            &manager,
            "PATCH",
            &format!("/api/connectors/{id}"),
            Some(json!({ "schemaChangePolicy": "ask_first" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{patched}");
        assert_eq!(patched["schemaChangePolicy"], "ask_first");
        assert!(patched["pausedAt"].is_string(), "{patched}");
        assert!(paused(&pool, &id).await, "only an approval lifts the pause");

        let (status, _) = call(
            &h,
            &manager,
            "PATCH",
            &format!("/api/connectors/{id}"),
            Some(json!({ "schemaChangePolicy": "whenever" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    /// Only the orchestrator's service identity (or an unrestricted
    /// administrator) reports a source schema; a user with `ingest:read`
    /// gets 403 and nothing is written.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_user_calling_the_observation_route_is_refused(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "user observes", "apply_non_breaking").await;
        let (status, _) = call(
            &h,
            &user(&[h.tenant]),
            "POST",
            &format!("/api/connectors/{id}/schema-observations"),
            Some(observation("orders", &BASE, &["id"], "before_load")),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM connector_source_schema")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }

    /// Another tenant's user gets, on the three user routes, the answer an
    /// unknown connector gets.
    #[sqlx::test(migrations = "../../migrations")]
    async fn another_tenants_user_gets_the_answer_an_unknown_connector_gets(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "tenant gate", "apply_non_breaking").await;
        let outsider = user(&[Uuid::new_v4()]);
        let approve_body = json!({ "object": "orders" });
        let cases = [
            ("GET", "schema-changes", None),
            ("POST", "schema-changes/approve", Some(approve_body.clone())),
            ("PATCH", "", Some(json!({ "schemaChangePolicy": "pause" }))),
        ];
        for (verb, tail, body) in cases {
            let path = |who: &str| {
                format!("/api/connectors/{who}/{tail}")
                    .trim_end_matches('/')
                    .to_owned()
            };
            let (status, real) = call(&h, &outsider, verb, &path(&id), body.clone()).await;
            let (unknown_status, unknown) =
                call(&h, &outsider, verb, &path("conn-does-not-exist"), body).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{verb} {tail}");
            assert_eq!(status, unknown_status);
            assert_eq!(
                real.to_string().replace(&id, "X"),
                unknown.to_string().replace("conn-does-not-exist", "X"),
                "{verb} {tail}"
            );
        }
        let policy: String =
            sqlx::query_scalar("SELECT schema_change_policy FROM connector WHERE id = $1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(policy, "apply_non_breaking", "the outsider changed nothing");
    }

    /// The same observation twice writes nothing new and alerts once, for a
    /// change that took effect and for one that waits.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_repeated_observation_writes_nothing_new_and_alerts_once(pool: sqlx::PgPool) {
        let h0 = harness(&pool, &[]).await;
        let id = connector(&h0, "idempotent", "apply_non_breaking").await;
        let h = harness(&pool, &[("al-s", "connector_schema_change", &id)]).await;

        observe_as_service(&h, &id, &BASE, "before_load").await;
        assert!(
            posts(&h, "al-s").await.is_empty(),
            "a baseline raises no alert"
        );

        // Applied: the second time the baseline already holds the column.
        observe_as_service(&h, &id, &WITH_QTY, "before_load").await;
        let again = observe_as_service(&h, &id, &WITH_QTY, "before_load").await;
        assert_eq!(again["changes"], json!([]));
        assert_eq!(posts(&h, "al-s").await.len(), 1);

        // Waiting: the same removal seen on the next run is the same row,
        // listed again and not alerted again.
        let first = observe_as_service(&h, &id, &WITHOUT_NOTE_QTY, "before_load").await;
        let second = observe_as_service(&h, &id, &WITHOUT_NOTE_QTY, "before_load").await;
        assert_eq!(first["changes"][0]["id"], second["changes"][0]["id"]);
        assert_eq!(second["action"], "wait");
        let (rows,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM connector_schema_change WHERE connector_id = $1 AND status = 'pending'",
        )
        .bind(&id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(rows, 1);
        let alerts = posts(&h, "al-s").await;
        assert_eq!(alerts.len(), 2, "one for the addition, one for the removal");
        for needle in ["idempotent", id.as_str(), "orders", "/connectors/"] {
            assert!(
                alerts[1].contains(needle),
                "{needle} missing from {}",
                alerts[1]
            );
        }
    }

    const WITHOUT_NOTE_QTY: [(&str, &str); 2] = [("id", "integer"), ("qty", "integer")];

    /// Decision D4: after the load nothing is held back. A column missing
    /// from the batch is not a removal; an added column is applied; `pause`
    /// still pauses the connector for later runs.
    #[sqlx::test(migrations = "../../migrations")]
    async fn after_the_load_changes_are_applied_and_a_missing_column_is_not_removed(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "after load", "pause").await;
        observe_as_service(&h, &id, &BASE, "after_load").await;

        let missing = observe_as_service(&h, &id, &WITHOUT_NOTE, "after_load").await;
        assert_eq!(missing["action"], "load");
        assert_eq!(missing["changes"], json!([]));
        assert_eq!(baseline_columns(&pool, &id).await, 2);
        assert!(!paused(&pool, &id).await);

        let added = observe_as_service(&h, &id, &WITH_QTY, "after_load").await;
        assert_eq!(
            added["action"], "load",
            "the change is already in the table"
        );
        assert_eq!(added["changes"][0]["status"], "applied");
        assert_eq!(baseline_columns(&pool, &id).await, 3);
        assert!(paused(&pool, &id).await, "later runs stop");
    }

    /// An unknown connector is a 404 and a malformed or oversize body a 400,
    /// before anything is stored.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_observation_route_refuses_unknown_connectors_and_bad_bodies(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "bad bodies", "apply_non_breaking").await;
        let path = format!("/api/connectors/{id}/schema-observations");
        let (status, _) = call(
            &h,
            &service(),
            "POST",
            "/api/connectors/conn-nope/schema-observations",
            Some(observation("orders", &BASE, &["id"], "before_load")),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let many: Vec<(String, &str)> = (0..=MAX_COLUMNS)
            .map(|i| (format!("c{i}"), "text"))
            .collect();
        let many_refs: Vec<(&str, &str)> = many.iter().map(|(n, t)| (n.as_str(), *t)).collect();
        for bad in [
            observation("orders", &many_refs, &[], "before_load"),
            observation("", &BASE, &["id"], "before_load"),
            observation("orders", &[], &[], "before_load"),
            json!({"object": "orders", "columns": [], "primaryKey": [], "phase": "before_load", "extra": 1}),
        ] {
            let (status, _) = call(&h, &service(), "POST", &path, Some(bad)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        let (rows,): (i64,) = sqlx::query_as("SELECT count(*) FROM connector_source_schema")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0);
    }
}
