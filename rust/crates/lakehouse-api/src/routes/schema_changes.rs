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
use lakehouse_store::connectors as store_connectors;
use lakehouse_store::ingest_spec::default_bronze_target;
use lakehouse_store::schema_change::{
    self, ApproveOutcome, InactiveColumn, ObservationTx, ObservationWrite, ObservedColumn,
    RecordedChange, SchemaChange, SchemaChangePolicy, TableCandidate, TableRefusal,
};
use lakehouse_store::schema_diff::{Action, Decision, TableShape, evaluate};
use lakehouse_store::uploads;
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

/// Most table names one `POST .../schema-observations/tables` may carry.
const MAX_TABLES: usize = 2000;

/// Fixed 409 text: the connector's policy is not `apply_all` (decision D2),
/// so the orchestrator should not have asked.
const NOT_APPLY_ALL: &str = "This connector's schema-change policy is not \"Apply all changes\", \
     so new tables are not added on their own.";

/// Fixed 409 text: only batch SQL sources the orchestrator can list the tables
/// of (`PostgreSQL`, `MySQL`/`MariaDB`, SQL Server) take new tables on their own.
const TABLES_NOT_SUPPORTED: &str =
    "New tables are added on their own only for PostgreSQL, MySQL, MariaDB and SQL Server sources.";

/// Fixed 409 text: a type change the existing column cannot hold is waiting
/// (`SRC-8` decision 10, review BLOCKER 2). No source text goes in.
const CANNOT_BE_LOADED: &str = "This type change cannot be loaded into the existing column. \
     Change the column back at the source, or remove the table from the connector \
     and add it again under a new target.";

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

/// The `POST .../schema-observations/tables` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NewTablesBody {
    /// `<schema>.<table>` of tables that appeared in a schema the connector
    /// already loads from and that it does not select yet.
    tables: Vec<String>,
    /// The run that found them.
    #[serde(default)]
    run_id: Option<String>,
}

/// A table that was not added, and why (a fixed text).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotAdded {
    /// `<schema>.<table>`.
    table: String,
    /// One of the fixed texts of `TableRefusal::reason`.
    reason: &'static str,
}

/// The answer to `POST .../schema-observations/tables`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewTablesResponse {
    /// Tables now selected by the connector (load mode `replace`).
    added: Vec<String>,
    /// Tables left out, each with the reason.
    not_added: Vec<NotAdded>,
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
        // `SRC-8` task 11: same rules and bounds as the column's own name.
        if let Some(loaded_name) = &column.loaded_name {
            check_name("loadedName", loaded_name, MAX_NAME_LEN)?;
        }
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

/// `ingest:read` is also a user's permission; only the orchestrator (or an
/// unrestricted administrator) reports what a source looks like.
fn require_orchestrator(principal: &Principal) -> Result<(), ApiError> {
    if matches!(principal.id, PrincipalId::Service(_))
        || crate::routes::catalog::is_unrestricted(principal)
    {
        return Ok(());
    }
    Err(ApiError::PermissionDenied(
        "only the orchestrator's service identity reports source schemas".to_owned(),
    ))
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
    require_orchestrator(&principal)?;
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

/// Bound and sanity-check a list of new tables, dropping repeats and keeping
/// the order. Pure: unit-tested below.
///
/// # Errors
///
/// 400 for an empty list, more than 2,000 names, a blank, oversize or
/// control-character name, or a bad run id.
fn validate_tables(body: &NewTablesBody) -> Result<Vec<&str>, ApiError> {
    if body.tables.is_empty() {
        return Err(ApiError::BadRequest(
            "tables must not be empty: there is nothing to add".to_owned(),
        ));
    }
    if body.tables.len() > MAX_TABLES {
        return Err(ApiError::BadRequest(format!(
            "at most {MAX_TABLES} tables can be added in one request"
        )));
    }
    let mut seen = std::collections::HashSet::new();
    let mut names = Vec::with_capacity(body.tables.len());
    for table in &body.tables {
        check_name("table", table, MAX_NAME_LEN)?;
        if seen.insert(table.as_str()) {
            names.push(table.as_str());
        }
    }
    if let Some(run_id) = &body.run_id {
        check_name("runId", run_id, MAX_RUN_ID_LEN)?;
    }
    Ok(names)
}

/// Whether the dial's driver is one whose tables the orchestrator lists
/// (`adapters/sql.py::list_tables`): `PostgreSQL`, `MySQL` (which also carries
/// `MariaDB`: there is no `mariadb` driver value, `SqlDriver`) and SQL Server.
fn lists_tables(adapter: Option<&str>, dial: &serde_json::Value) -> bool {
    adapter == Some("sql")
        && matches!(
            dial.get("driver").and_then(serde_json::Value::as_str),
            Some("postgres" | "mysql" | "mssql")
        )
}

/// `POST /api/connectors/{id}/schema-observations/tables` -- add the tables
/// that appeared in a schema the connector already loads from (`SRC-8`
/// decision D2). Only for a connector on the `apply_all` policy; the
/// orchestrator asks once per run.
///
/// Each new table gets the Bronze target the console's table picker would
/// give it (`default_bronze_target`) and load mode `replace`, and is appended
/// to the connector's tables through the store's ingest-spec write, so every
/// check a person's save goes through (and `SEC-14`'s re-point rule, which
/// sees an unchanged target) applies. A table whose target another connector
/// or another table uses, or which uploads reserved, is NOT added: it is
/// listed as a `table_added` change that waits, with a fixed reason, and
/// answered under `notAdded`. A table the connector already selects is left
/// out of both lists, so a second identical request adds nothing. One alert
/// for the batch.
///
/// # Errors
///
/// 403 for a caller that is neither the orchestrator's service identity nor an
/// unrestricted administrator; 400 for an invalid body; 404 for an unknown
/// connector; 409 with a fixed text when the connector's policy is not
/// `apply_all` or its source is not one the orchestrator lists tables of;
/// 503/500 as the store reports.
pub async fn new_tables(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<NewTablesResponse>> {
    require_orchestrator(&principal)?;
    let req: NewTablesBody = parse_body(&body)?;
    let names = validate_tables(&req)?;
    let pool = pool(&state)?;
    let detail = store_connectors::get_connector(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    let Some(policy) = SchemaChangePolicy::parse(&detail.connector.schema_change_policy) else {
        tracing::error!(connector_id = %id, "connector has an unknown schema-change policy");
        return Err(ApiError::Internal(UNKNOWN_POLICY.to_owned()).into());
    };
    if policy != SchemaChangePolicy::ApplyAll {
        return Err(ApiError::Conflict(NOT_APPLY_ALL.to_owned()).into());
    }
    let spec = store_connectors::get_ingest_spec(pool, &id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("Connector {id} not found")))?;
    if !lists_tables(spec.adapter.as_deref(), &spec.dial) {
        return Err(ApiError::Conflict(TABLES_NOT_SUPPORTED.to_owned()).into());
    }
    let selected: std::collections::HashSet<&str> = spec
        .source_objects
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|object| object.get("name").and_then(serde_json::Value::as_str))
        .collect();

    let mut candidates = Vec::with_capacity(names.len());
    for name in names.into_iter().filter(|name| !selected.contains(name)) {
        let target = default_bronze_target(&detail.connector.name, name);
        // The same two questions `ingest_spec_put` asks of a target
        // (`refuse_uploaded_targets` and the connector-to-connector rule),
        // answered per table here instead of failing the whole request.
        let refusal = if uploads::table_claimed(pool, &target).await? {
            Some(TableRefusal::ReservedForUploads)
        } else if store_connectors::any_connector_targets(pool, &target).await? {
            Some(TableRefusal::TargetTaken)
        } else {
            None
        };
        candidates.push(TableCandidate {
            name: name.to_owned(),
            target,
            refusal,
        });
    }
    let outcome = schema_change::record_table_additions(
        pool,
        &id,
        &candidates,
        req.run_id.as_deref(),
        OffsetDateTime::now_utc(),
    )
    .await?;
    announce_tables(&state, pool, &id, &detail.connector.name, &outcome).await;
    Ok(ApiJson(NewTablesResponse {
        added: outcome.added,
        not_added: outcome
            .not_added
            .into_iter()
            .map(|(table, refusal)| NotAdded {
                table,
                reason: refusal.reason(),
            })
            .collect(),
    }))
}

/// One `connector_schema_change` alert for a batch of tables, when any of its
/// changes is new (a refusal seen again raises nothing). Names and counts
/// only; best effort like [`announce`].
async fn announce_tables(
    state: &AppState,
    pool: &lakehouse_store::PgPool,
    connector_id: &str,
    connector_name: &str,
    outcome: &schema_change::TableAdditions,
) {
    let new = outcome.changes.iter().filter(|r| r.is_new).count();
    if new == 0 {
        return;
    }
    let text = format!(
        "Connector {connector_name} ({connector_id}): {new} new table(s) appeared at the source, \
         {} added, {} not added. View: /connectors/{connector_id}",
        outcome.added.len(),
        outcome.not_added.len(),
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
        tracing::warn!(?err, connector_id, "new-table alert was not delivered");
    }
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
/// tenant's or unknown, 409 with a fixed text when what waits includes a type
/// change the column cannot hold (decision 10), 400 for a bad body, 503/500 as
/// the store reports.
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
    // A pending `table_added` row (a table that could not be added) is
    // dismissed through this same route with the table's name: approving it
    // means "seen" and adds nothing (`SRC-8 review SHOULD-FIX 3`). The wire
    // shape is the same for both.
    let approval = match schema_change::approve_object(
        pool,
        &id,
        &req.object,
        Some(&decided_by),
        OffsetDateTime::now_utc(),
    )
    .await?
    {
        ApproveOutcome::Approved(approval) => approval,
        ApproveOutcome::NothingWaits => {
            return Err(ApiError::NotFound(
                "No schema change is waiting for that table".to_owned(),
            )
            .into());
        }
        // Decision 10: nothing was approved, not even the table's other
        // pending changes (approval accepts the observed shape as a whole).
        ApproveOutcome::CannotBeLoaded => {
            return Err(ApiError::Conflict(CANNOT_BE_LOADED.to_owned()).into());
        }
    };
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
            loaded_name: None,
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

    /// `SRC-8` task 11: `loadedName` is optional, takes the name rules and
    /// bounds of a column name, and an unknown field is still refused.
    #[test]
    fn a_loaded_name_is_optional_and_follows_the_column_name_rules() {
        let mut named = col("OrderDate", "date");
        named.loaded_name = Some("order_date".to_owned());
        assert!(validate(&body(vec![named])).is_ok());
        for bad in ["", "  ", "x".repeat(MAX_NAME_LEN + 1).as_str(), "a\nb"] {
            let mut column = col("OrderDate", "date");
            column.loaded_name = Some(bad.to_owned());
            assert!(validate(&body(vec![column])).is_err(), "{bad:?}");
        }
        let wire = json!({"name": "a", "typeName": "t", "nullable": true, "loadedName": "a"});
        assert!(serde_json::from_value::<ObservedColumn>(wire).is_ok());
        let unknown = json!({"name": "a", "typeName": "t", "nullable": true, "loaded": "a"});
        assert!(serde_json::from_value::<ObservedColumn>(unknown).is_err());
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

    /// `SRC-8` task 11: the loaded name sent with a before-load column is kept
    /// with the baseline and written to the inactive column when the removal
    /// is approved; a column sent without one stores `null` and marks nothing.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_approved_removal_keeps_the_loaded_name_the_orchestrator_sent(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "loaded name", "apply_non_breaking").await;
        let observe = |columns: Value| {
            let h = &h;
            let id = &id;
            async move {
                let (status, answer) = call(
                    h,
                    &service(),
                    "POST",
                    &format!("/api/connectors/{id}/schema-observations"),
                    Some(json!({
                        "object": "orders", "columns": columns,
                        "primaryKey": ["id"], "phase": "before_load",
                    })),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{answer}");
            }
        };
        let id_col = json!({"name": "id", "typeName": "integer", "nullable": false});
        observe(json!([
            id_col,
            {"name": "OrderDate", "typeName": "date", "nullable": true, "loadedName": "order_date"},
            {"name": "legacy", "typeName": "text", "nullable": true},
        ]))
        .await;
        observe(json!([id_col])).await;

        let (status, approved) = approve_orders(&h, &id, "orders").await;
        assert_eq!(status, StatusCode::OK, "{approved}");

        let inactive = listed(&h, &id).await["inactiveColumns"].clone();
        assert_eq!(inactive.as_array().unwrap().len(), 2);
        let by_name = |name: &str| {
            inactive
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["columnName"] == name)
                .unwrap()
                .clone()
        };
        assert_eq!(by_name("OrderDate")["loadedName"], "order_date");
        assert_eq!(by_name("legacy")["loadedName"], Value::Null);
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

    // --- SRC-8 task 8: new tables under "apply all" (decision D2) -----------

    /// Give connector `id` an ingest spec: a `PostgreSQL` dial and one table.
    async fn with_spec(h: &Harness, id: &str, adapter: &str, driver: &str) {
        let pool = h.state.pg.as_deref().expect("pool");
        lakehouse_store::connectors::set_ingest_spec(
            pool,
            id,
            &lakehouse_store::connectors::IngestSpecInput {
                adapter: adapter.to_owned(),
                ingest_mode: "batch".to_owned(),
                dial: json!({
                    "driver": driver, "host": "db.internal", "port": 5432,
                    "database": "shop", "user": "reader",
                }),
                source_objects: json!([{"name": "public.orders", "target": "taken_by_orders"}]),
                schedule_cron: None,
            },
        )
        .await
        .expect("an ingest spec");
    }

    async fn post_tables(
        h: &Harness,
        who: &Principal,
        id: &str,
        tables: Value,
    ) -> (StatusCode, Value) {
        call(
            h,
            who,
            "POST",
            &format!("/api/connectors/{id}/schema-observations/tables"),
            Some(json!({ "tables": tables, "runId": "run-t" })),
        )
        .await
    }

    async fn selected(pool: &sqlx::PgPool, id: &str) -> Vec<(String, String)> {
        lakehouse_store::connectors::get_ingest_spec(pool, id)
            .await
            .unwrap()
            .unwrap()
            .source_objects
            .as_array()
            .unwrap()
            .iter()
            .map(|o| {
                (
                    o["name"].as_str().unwrap().to_owned(),
                    o["target"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// Spec "Per-connector policy" (D2): a table that appeared joins the
    /// connector with the console picker's target and is listed as applied.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_new_table_is_added_with_the_pickers_target_and_listed_as_applied(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "Shop DB", "apply_all").await;
        with_spec(&h, &id, "sql", "postgres").await;

        let (status, answer) = post_tables(
            &h,
            &service(),
            &id,
            json!(["public.orders", "public.Order Details", "public.customers"]),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{answer}");
        // `public.orders` is already selected: in neither list.
        assert_eq!(
            answer,
            json!({
                "added": ["public.Order Details", "public.customers"],
                "notAdded": [],
            })
        );
        assert_eq!(
            selected(&pool, &id).await,
            [
                ("public.orders".to_owned(), "taken_by_orders".to_owned()),
                (
                    "public.Order Details".to_owned(),
                    "shop_db_order_details".to_owned()
                ),
                (
                    "public.customers".to_owned(),
                    "shop_db_customers".to_owned()
                ),
            ]
        );
        let (status, listed) = call(
            &h,
            &user(&[h.tenant]),
            "GET",
            &format!("/api/connectors/{id}/schema-changes"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let recent = listed["recent"].as_array().unwrap();
        assert_eq!(recent.len(), 2);
        for change in recent {
            assert_eq!(change["kind"], "table_added");
            assert_eq!(change["status"], "applied");
            assert_eq!(change["runId"], "run-t");
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_taken_target_is_listed_as_not_added_with_the_reason(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "Shop DB", "apply_all").await;
        with_spec(&h, &id, "sql", "postgres").await;
        // Another connector already lands in `shop_db_customers`.
        let other = connector(&h, "Other", "apply_all").await;
        with_spec(&h, &other, "sql", "postgres").await;
        sqlx::query("UPDATE connector SET source_objects = $2 WHERE id = $1")
            .bind(&other)
            .bind(json!([{"name": "s.c", "target": "shop_db_customers"}]))
            .execute(&pool)
            .await
            .unwrap();

        let (status, answer) = post_tables(
            &h,
            &service(),
            &id,
            json!(["public.customers", "public.invoices"]),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["added"], json!(["public.invoices"]));
        assert_eq!(answer["notAdded"][0]["table"], "public.customers");
        assert_eq!(
            answer["notAdded"][0]["reason"],
            TableRefusal::TargetTaken.reason()
        );
        assert_eq!(
            selected(&pool, &id).await.len(),
            2,
            "orders and invoices only"
        );
        let pending: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT object_name, status, after_value FROM connector_schema_change \
             WHERE connector_id = $1 AND status = 'pending'",
        )
        .bind(&id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            pending,
            [(
                "public.customers".to_owned(),
                "pending".to_owned(),
                Some(TableRefusal::TargetTaken.reason().to_owned())
            )]
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_target_reserved_for_uploads_is_not_added(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "Shop DB", "apply_all").await;
        with_spec(&h, &id, "sql", "postgres").await;
        sqlx::query("INSERT INTO upload_table_claim (bronze_table, upload_id) VALUES ('shop_db_files', 'up-1')")
            .execute(&pool)
            .await
            .unwrap();

        let (status, answer) = post_tables(&h, &service(), &id, json!(["public.files"])).await;

        assert_eq!(status, StatusCode::OK, "{answer}");
        assert_eq!(answer["added"], json!([]));
        assert_eq!(
            answer["notAdded"][0]["reason"],
            TableRefusal::ReservedForUploads.reason()
        );
        assert_eq!(selected(&pool, &id).await.len(), 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_second_identical_post_adds_nothing_and_alerts_once(pool: sqlx::PgPool) {
        let h0 = harness(&pool, &[]).await;
        let id = connector(&h0, "Shop DB", "apply_all").await;
        let h = harness(&pool, &[("al-t", "connector_schema_change", &id)]).await;
        with_spec(&h, &id, "sql", "postgres").await;
        let tables = json!(["public.customers", "public.invoices"]);

        let (_, first) = post_tables(&h, &service(), &id, tables.clone()).await;
        let stored = selected(&pool, &id).await;
        let (status, second) = post_tables(&h, &service(), &id, tables).await;

        assert_eq!(first["added"].as_array().unwrap().len(), 2);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(second, json!({"added": [], "notAdded": []}));
        assert_eq!(selected(&pool, &id).await, stored);
        let alerts = posts(&h, "al-t").await;
        assert_eq!(
            alerts.len(),
            1,
            "one alert for the batch, none for the repeat"
        );
        for needle in [
            "Shop DB",
            id.as_str(),
            "2 new table(s)",
            "2 added",
            "/connectors/",
        ] {
            assert!(
                alerts[0].contains(needle),
                "{needle} missing from {}",
                alerts[0]
            );
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_policy_other_than_apply_all_gets_409_and_nothing_is_added(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        for policy in ["apply_non_breaking", "ask_first", "pause"] {
            let id = connector(&h, &format!("p {policy}"), policy).await;
            with_spec(&h, &id, "sql", "postgres").await;
            let (status, answer) =
                post_tables(&h, &service(), &id, json!(["public.customers"])).await;
            assert_eq!(status, StatusCode::CONFLICT, "{policy}");
            assert_eq!(answer["error"], NOT_APPLY_ALL, "{policy}");
            assert_eq!(selected(&pool, &id).await.len(), 1);
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_source_whose_tables_the_orchestrator_does_not_list_gets_409(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        // Oracle is not listed; a connector with no ingest spec has no tables.
        let oracle = connector(&h, "ora", "apply_all").await;
        with_spec(&h, &oracle, "sql", "oracle").await;
        let none = connector(&h, "none", "apply_all").await;
        for id in [&oracle, &none] {
            let (status, answer) = post_tables(&h, &service(), id, json!(["a.b"])).await;
            assert!(
                status == StatusCode::CONFLICT || status == StatusCode::NOT_FOUND,
                "{status} {answer}"
            );
        }
        let (status, answer) = post_tables(&h, &service(), &oracle, json!(["a.b"])).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(answer["error"], TABLES_NOT_SUPPORTED);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn only_the_orchestrator_may_add_tables_and_a_bad_body_is_a_400(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "Shop DB", "apply_all").await;
        with_spec(&h, &id, "sql", "postgres").await;

        // A user holding `ingest:read` and `connector:manage` is refused.
        let (status, _) =
            post_tables(&h, &user(&[h.tenant]), &id, json!(["public.customers"])).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let many: Vec<String> = (0..=MAX_TABLES).map(|i| format!("s.t{i}")).collect();
        for bad in [json!([]), json!(many), json!([""]), json!(["a\nb"])] {
            let (status, _) = post_tables(&h, &service(), &id, bad).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
        let (status, _) = call(
            &h,
            &service(),
            "POST",
            &format!("/api/connectors/{id}/schema-observations/tables"),
            Some(json!({"tables": ["a.b"], "extra": 1})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = post_tables(&h, &service(), "conn-nope", json!(["a.b"])).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(selected(&pool, &id).await.len(), 1);
    }

    const QTY_INTEGER: [(&str, &str); 2] = [("id", "integer"), ("qty", "integer")];
    const QTY_TEXT: [(&str, &str); 2] = [("id", "integer"), ("qty", "text")];
    const QTY_TEXT_NOTE: [(&str, &str); 3] = [("id", "integer"), ("qty", "text"), ("note", "text")];

    async fn listed(h: &Harness, id: &str) -> Value {
        let (status, listed) = call(
            h,
            &user(&[h.tenant]),
            "GET",
            &format!("/api/connectors/{id}/schema-changes"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        listed
    }

    async fn approve_orders(h: &Harness, id: &str, object: &str) -> (StatusCode, Value) {
        call(
            h,
            &user(&[h.tenant]),
            "POST",
            &format!("/api/connectors/{id}/schema-changes/approve"),
            Some(json!({ "object": object })),
        )
        .await
    }

    /// `SRC-8 review BLOCKER 2` (decision 10): a SQL type change the column
    /// cannot hold waits under every policy, is shown as not approvable in
    /// the observation answer and in the list, and approval is a 409 with the
    /// fixed text that approves nothing, not even the table's other changes.
    #[sqlx::test(migrations = "../../migrations")]
    async fn approving_a_type_change_the_column_cannot_hold_is_a_409_that_approves_nothing(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[]).await;
        for policy in ["apply_non_breaking", "apply_all", "ask_first", "pause"] {
            let id = connector(&h, &format!("unloadable {policy}"), policy).await;
            observe_as_service(&h, &id, &QTY_INTEGER, "before_load").await;
            let answer = observe_as_service(&h, &id, &QTY_TEXT_NOTE, "before_load").await;
            assert_eq!(answer["action"], "wait", "{policy}");
            let changes = answer["changes"].as_array().unwrap();
            assert_eq!(changes.len(), 2, "{policy}");
            assert!(
                changes.iter().all(|c| c["canApprove"] == false),
                "{policy}: {answer}"
            );

            let list = listed(&h, &id).await;
            assert!(
                list["pending"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|c| c["canApprove"] == false),
                "{policy}: {list}"
            );

            let (status, refused) = approve_orders(&h, &id, "orders").await;
            assert_eq!(status, StatusCode::CONFLICT, "{policy}");
            assert_eq!(refused["error"], CANNOT_BE_LOADED, "{policy}");
            let list = listed(&h, &id).await;
            assert_eq!(list["pending"].as_array().unwrap().len(), 2, "{policy}");
            assert_eq!(list["recent"], json!([]), "{policy}");
        }
    }

    /// The way out the 409 names: the source is put back, the next
    /// observation shows no change, and the table loads with nothing pending.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_table_waiting_on_an_unloadable_type_change_loads_when_the_source_is_put_back(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "put back", "pause").await;
        observe_as_service(&h, &id, &QTY_INTEGER, "before_load").await;
        let waiting = observe_as_service(&h, &id, &QTY_TEXT, "before_load").await;
        assert_eq!(waiting["action"], "wait");

        let back = observe_as_service(&h, &id, &QTY_INTEGER, "before_load").await;

        assert_eq!(back["action"], "load");
        assert_eq!(back["changes"], json!([]));
        let list = listed(&h, &id).await;
        assert_eq!(list["pending"], json!([]));
        let (status, _) = approve_orders(&h, &id, "orders").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "nothing waits any more");
    }

    /// `SRC-8 review BLOCKER 3a`: under policy `pause`, a type change the
    /// column cannot hold makes the table wait but does NOT pause the
    /// connector, so a later run observes the column put back and the table
    /// loads, with nothing pending.
    #[sqlx::test(migrations = "../../migrations")]
    async fn under_pause_an_unloadable_type_change_waits_without_pausing_and_clears_when_put_back(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "stuck", "pause").await;
        observe_as_service(&h, &id, &QTY_INTEGER, "before_load").await;

        let waiting = observe_as_service(&h, &id, &QTY_TEXT, "before_load").await;

        assert_eq!(waiting["action"], "wait");
        assert!(!paused(&pool, &id).await, "the connector keeps running");

        let back = observe_as_service(&h, &id, &QTY_INTEGER, "before_load").await;

        assert_eq!(back["action"], "load");
        assert_eq!(listed(&h, &id).await["pending"], json!([]));
    }

    /// After the load (files, REST, `MongoDB`, Kafka, SFTP) such a change is
    /// already in the table as a second column (`SRC-8-RESULT`), so it is
    /// recorded as applied and never waits or blocks.
    #[sqlx::test(migrations = "../../migrations")]
    async fn after_the_load_an_unloadable_type_change_is_applied_and_never_waits(
        pool: sqlx::PgPool,
    ) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "after type", "apply_non_breaking").await;
        observe_as_service(&h, &id, &QTY_INTEGER, "after_load").await;

        let answer = observe_as_service(&h, &id, &QTY_TEXT, "after_load").await;

        assert_eq!(answer["action"], "load");
        assert_eq!(answer["changes"][0]["kind"], "type_changed");
        assert_eq!(answer["changes"][0]["status"], "applied");
        assert_eq!(answer["changes"][0]["canApprove"], false);
        assert_eq!(listed(&h, &id).await["pending"], json!([]));
    }

    /// A removal alone is approvable, and the flag says so.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_waiting_removal_is_marked_approvable(pool: sqlx::PgPool) {
        let h = harness(&pool, &[]).await;
        let id = connector(&h, "approvable", "apply_non_breaking").await;
        observe_as_service(&h, &id, &BASE, "before_load").await;
        let answer = observe_as_service(&h, &id, &WITHOUT_NOTE, "before_load").await;
        assert_eq!(answer["changes"][0]["canApprove"], true);
        assert_eq!(listed(&h, &id).await["pending"][0]["canApprove"], true);
        let (_, approved) = approve_orders(&h, &id, "orders").await;
        assert_eq!(approved["approved"][0]["canApprove"], false);
    }

    /// `SRC-8 review SHOULD-FIX 3`: a table "apply all" could not add is
    /// dismissed through the approve route with its name, adds nothing, and
    /// the same refusal then raises no new row and no new alert.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_refused_table_is_dismissed_through_approve_and_its_alert_does_not_repeat(
        pool: sqlx::PgPool,
    ) {
        let h0 = harness(&pool, &[]).await;
        let id = connector(&h0, "Shop DB", "apply_all").await;
        let h = harness(&pool, &[("al-d", "connector_schema_change", &id)]).await;
        with_spec(&h, &id, "sql", "postgres").await;
        let other = connector(&h, "Other", "apply_all").await;
        with_spec(&h, &other, "sql", "postgres").await;
        sqlx::query("UPDATE connector SET source_objects = $2 WHERE id = $1")
            .bind(&other)
            .bind(json!([{"name": "s.c", "target": "shop_db_customers"}]))
            .execute(&pool)
            .await
            .unwrap();
        let tables = json!(["public.customers"]);
        post_tables(&h, &service(), &id, tables.clone()).await;
        assert_eq!(posts(&h, "al-d").await.len(), 1);
        let list = listed(&h, &id).await;
        assert_eq!(list["pending"][0]["kind"], "table_added");
        assert_eq!(list["pending"][0]["canApprove"], true);

        let (status, dismissed) = approve_orders(&h, &id, "public.customers").await;

        assert_eq!(status, StatusCode::OK, "{dismissed}");
        assert_eq!(dismissed["approved"][0]["status"], "approved");
        assert_eq!(dismissed["pauseLifted"], false);
        assert_eq!(selected(&pool, &id).await.len(), 1, "nothing was added");
        let (status, again) = post_tables(&h, &service(), &id, tables).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(again["notAdded"][0]["table"], "public.customers");
        assert_eq!(posts(&h, "al-d").await.len(), 1, "no second alert");
        assert_eq!(listed(&h, &id).await["pending"], json!([]));
    }

    #[test]
    fn the_table_list_drops_repeats_and_keeps_the_order() {
        let body = NewTablesBody {
            tables: vec!["b.t".to_owned(), "a.t".to_owned(), "b.t".to_owned()],
            run_id: None,
        };
        assert_eq!(validate_tables(&body).unwrap(), ["b.t", "a.t"]);
    }

    #[test]
    fn the_orchestrator_lists_tables_of_postgres_mysql_and_sql_server_sql_connectors_only() {
        for driver in ["postgres", "mysql", "mssql"] {
            assert!(
                lists_tables(Some("sql"), &json!({"driver": driver})),
                "{driver}"
            );
        }
        assert!(!lists_tables(Some("sql"), &json!({"driver": "oracle"})));
        assert!(!lists_tables(Some("cdc"), &json!({"driver": "postgres"})));
        assert!(!lists_tables(None, &json!({})));
    }
}
