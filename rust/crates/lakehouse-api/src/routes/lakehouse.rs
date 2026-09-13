//! `GET /api/lakehouse/*` — the read-only Iceberg warehouse/namespace/table
//! surface (WS2 §4), built on `lakehouse_iceberg::rest` and the shared,
//! lazily-connected client in [`crate::lakehouse_catalog`].
//!
//! Every handler here returns [`ApiResult<ApiJson<Value>>`]. Every JSON
//! body is built by a pure, `rest`-typed function tested directly below
//! (the `catalog.rs` idiom) rather than through a mocked `AppState`.
//!
//! There is deliberately no `supported: false` body anywhere in this
//! module: `Config::lakekeeper_catalog_uri` always has a default, so the
//! catalog is always "configured" from this service's point of view — it
//! may be unreachable (503, [`classify_rest_error`]), but never "not set
//! up".

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_core::ApiError;
use lakehouse_iceberg::rest::{
    self, RestError, SnapshotDetail, TableDetail, TableSummary, WarehouseSummary,
};
use lakehouse_store::PgPool;
use lakehouse_store::maintenance_policy::{self, MaintenancePolicyRow};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::bounded;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::lakehouse_catalog;
use crate::routes::governance::latest_maintenance_run;
use crate::routes::support::str_col;
use crate::state::AppState;

/// Every `{ns}`/`{table}` path segment and `namespace` query value must
/// match this — enforced by [`validate_ident`] before any catalog or
/// `ClickHouse` call.
fn is_valid_ident(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn validate_ident(value: &str, field: &str) -> Result<(), ApiError> {
    if is_valid_ident(value) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "{field} must match ^[a-z0-9_]+$"
        )))
    }
}

/// This deployment exposes exactly one Lakekeeper warehouse
/// (`Config::lakekeeper_warehouse`). A `warehouse` query value naming
/// anything else is a 404, not a 400 — it names a real Lakekeeper concept
/// this service just doesn't happen to serve.
fn validate_warehouse(state: &AppState, warehouse: Option<&str>) -> Result<(), ApiError> {
    match warehouse {
        Some(w) if w != state.config.lakekeeper_warehouse => {
            Err(ApiError::NotFound(format!("warehouse not found: {w}")))
        }
        _ => Ok(()),
    }
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "maintenance policy store unavailable: no Postgres pool is configured".to_owned(),
        )
    })
}

/// Classifies a [`RestError`] into a fixed [`ApiError`], logging the real
/// cause first — `IcebergError`'s own `Display` text never reaches an HTTP
/// response body (AGENTS.md rule 4 / WS2's error-forwarding rule).
///
/// `resource` names what a [`RestError::NotFound`] refers to at this call
/// site — `list_table_idents` returns the SAME `NotFound` for a missing
/// namespace as `load_table_summary`/`load_table_detail` do for a missing
/// table, so the caller (which knows which one it just asked for) supplies
/// the word.
pub(crate) fn classify_rest_error(err: &RestError, resource: &'static str) -> ApiError {
    tracing::warn!(%err, resource, "lakehouse catalog read failed");
    match err {
        RestError::NotFound => ApiError::NotFound(format!("{resource} not found")),
        RestError::Catalog(_) => ApiError::Unavailable("lakehouse catalog unavailable".to_owned()),
    }
}

/// Maps [`lakehouse_catalog::CatalogAccessError`] to the [`ApiError`] a
/// handler returns: a token failure's fixed 503
/// ([`crate::lakekeeper_token::read_token_file`]'s own message, naming
/// `LAKEKEEPER_READ_TOKEN_FILE`) forwarded unchanged, or a catalog failure
/// run through [`classify_rest_error`] with `resource`.
fn classify_catalog_access_error(
    err: lakehouse_catalog::CatalogAccessError,
    resource: &'static str,
) -> ApiError {
    match err {
        lakehouse_catalog::CatalogAccessError::Token(api_err) => api_err,
        lakehouse_catalog::CatalogAccessError::Rest(rest_err) => {
            classify_rest_error(&rest_err, resource)
        }
    }
}

/// Classifies a `ClickHouse` maintenance-history lookup failure. Never `?`
/// on `ChError` directly here: that would forward `ClickHouse`'s own error
/// text as a 422 (`error.rs:145-164`), where this route needs a fixed 503
/// instead (the maintenance-run table not existing yet — no P4 job has run
/// on this deployment — looks the same to a caller as `ClickHouse` being
/// down; both get this fixed message).
fn classify_ch_error(err: &lakehouse_clickhouse::ChError) -> ApiError {
    tracing::warn!(%err, "maintenance-run lookup failed");
    ApiError::Unavailable("maintenance history is unavailable".to_owned())
}

/// RFC 3339 UTC timestamp from a snapshot's `last-updated-ms`, or `None`
/// with no current snapshot or an out-of-range value.
fn last_updated_at(last_updated_ms: Option<i64>) -> Option<String> {
    let ms = last_updated_ms?;
    let odt = time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()?;
    odt.format(&time::format_description::well_known::Rfc3339)
        .ok()
}

fn warehouses_body(warehouses: &[WarehouseSummary]) -> Value {
    json!({
        "warehouses": warehouses
            .iter()
            .map(|w| json!({
                "id": w.name,
                "name": w.name,
                // The Management API is the only endpoint that reports a
                // storage profile, and it is admin-scoped — nothing this
                // service holds can call it (see `lakehouse_iceberg::rest`'s
                // module doc comment).
                "storageProfile": Value::Null,
                // This client always requests vended credentials, but
                // nothing here proves Lakekeeper actually vends them for
                // this warehouse's storage profile — `null`, not a guess.
                "credentialVending": Value::Null,
                "reachable": w.reachable,
            }))
            .collect::<Vec<_>>(),
    })
}

fn namespaces_body(names: &[String], table_counts: &HashMap<String, usize>) -> Value {
    json!({
        "namespaces": names
            .iter()
            .map(|name| json!({
                "name": name,
                "tableCount": table_counts.get(name),
            }))
            .collect::<Vec<_>>(),
    })
}

/// `currentSnapshotId` and (in [`snapshot_json`]) `id`/`parentId` are all
/// Iceberg snapshot ids: 64-bit (`i64`), normally random, and routinely
/// larger than JavaScript's `Number.MAX_SAFE_INTEGER` (2^53−1). Emitted as
/// a JSON number they would be silently rounded by the browser's
/// `JSON.parse`, corrupting the id everywhere it is used — a rendered
/// value, a React `key`, and eventually a `FOR VERSION AS OF <id>` SQL
/// literal. Emitting them as JSON strings instead means they round-trip
/// exactly.
fn table_row(namespace: &str, name: &str, summary: Option<&TableSummary>) -> Value {
    match summary {
        Some(s) => json!({
            "namespace": s.namespace,
            "name": s.name,
            "formatVersion": s.format_version,
            "currentSnapshotId": s.current_snapshot_id.map(|id| id.to_string()),
            "lastUpdatedAt": last_updated_at(s.last_updated_ms),
            "fileCount": s.stats.file_count,
            "recordCount": s.stats.record_count,
            "totalBytes": s.stats.total_bytes,
        }),
        // The per-table load did not finish within the bounded budget (or
        // failed) — still listed, since we know it exists, but every field
        // besides identity is honestly `null` rather than a stale guess.
        None => json!({
            "namespace": namespace,
            "name": name,
            "formatVersion": Value::Null,
            "currentSnapshotId": Value::Null,
            "lastUpdatedAt": Value::Null,
            "fileCount": Value::Null,
            "recordCount": Value::Null,
            "totalBytes": Value::Null,
        }),
    }
}

fn tables_body(
    namespace: &str,
    idents: &[TableIdent],
    summaries: &HashMap<String, TableSummary>,
) -> Value {
    let tables: Vec<Value> = idents
        .iter()
        .map(|ident| table_row(namespace, ident.name(), summaries.get(ident.name())))
        .collect();
    json!({ "tables": tables })
}

/// See [`table_row`]'s doc comment: `id` and `parentId` are also 64-bit
/// Iceberg snapshot ids and are emitted as strings for the same reason.
fn snapshot_json(s: &SnapshotDetail) -> Value {
    json!({
        "id": s.id.to_string(),
        "parentId": s.parent_id.map(|id| id.to_string()),
        "timestampMs": s.timestamp_ms,
        "operation": s.operation,
        "summary": {
            "addedRecords": s.added_records,
            "deletedRecords": s.deleted_records,
            "totalRecords": s.total_records,
            "totalDataFiles": s.total_data_files,
        },
    })
}

fn table_detail_body(detail: &TableDetail) -> Value {
    json!({
        "schema": detail.schema.iter().map(|f| json!({
            "id": f.id,
            "name": f.name,
            "type": f.r#type,
            "required": f.required,
        })).collect::<Vec<_>>(),
        "partitionSpec": detail.partition_fields.iter().map(|f| json!({
            "sourceId": f.source_id,
            "transform": f.transform,
            "name": f.name,
        })).collect::<Vec<_>>(),
        "properties": detail.properties,
        "snapshots": detail.snapshots.iter().map(snapshot_json).collect::<Vec<_>>(),
        "stats": {
            "fileCount": detail.stats.file_count,
            // Needs a manifest read this workstream does not do — not
            // guessed from the snapshot summary.
            "smallFileCount": Value::Null,
            "smallFileThresholdBytes": Value::Null,
            "recordCount": detail.stats.record_count,
            "totalBytes": detail.stats.total_bytes,
            "snapshotCount": detail.snapshot_count,
            "metadataLogCount": detail.metadata_log_count,
        },
    })
}

fn maintenance_body(
    namespace: &str,
    table_name: &str,
    policy: Option<&MaintenancePolicyRow>,
    last_run: Option<Value>,
    last_verb_runs: &[Value],
) -> Value {
    let last_run = last_run.unwrap_or(Value::Null);
    match policy {
        Some(p) => json!({
            "namespace": namespace,
            "tableName": table_name,
            "configured": true,
            "snapshotsToKeep": p.snapshots_to_keep,
            "orphanAgeHours": p.orphan_age_hours,
            "compactSmallFiles": p.compact_small_files,
            "schedule": p.schedule,
            "lastRun": last_run,
            "lastVerbRuns": last_verb_runs,
        }),
        None => json!({
            "namespace": namespace,
            "tableName": table_name,
            "configured": false,
            "snapshotsToKeep": Value::Null,
            "orphanAgeHours": Value::Null,
            // Not configured: the maintenance job's real default behavior
            // is to skip compaction, not "unknown".
            "compactSmallFiles": false,
            "schedule": Value::Null,
            "lastRun": last_run,
            "lastVerbRuns": last_verb_runs,
        }),
    }
}

/// Builds the `SELECT` behind [`latest_maintenance_verb_runs`]: every row
/// `dagster/dispar_orchestrate/bronze_catalog.py::record_maintenance_verb_run`
/// wrote for `table_name`'s NEWEST recorded run (`run_at = (SELECT
/// max(run_at) ...)`), narrowed to that one table via a bound, escaped
/// literal — never `format!` on an identifier here, `table_name` is a
/// value, not a column/table name. `bronze_meta.maintenance_verb_run` is a
/// brand-new table (WS2 §4): most tables, and every deployment before this
/// feature existed, will never have a row in it — see
/// [`maintenance_verb_runs_or_empty`] for how that absence is handled.
fn maintenance_verb_run_query(table_name: &str) -> String {
    let lit = lakehouse_core::ident::SqlLiteral::from(table_name);
    format!(
        "SELECT verb, engine, outcome, detail, run_at \
         FROM lake.`bronze_meta.maintenance_verb_run` \
         WHERE table_name = {lit} AND run_at = \
         (SELECT max(run_at) FROM lake.`bronze_meta.maintenance_verb_run` WHERE table_name = {lit}) \
         ORDER BY verb"
    )
}

/// Maps one `bronze_meta.maintenance_verb_run` row to the JSON shape
/// [`maintenance`]'s `lastVerbRuns` array returns per entry.
fn maintenance_verb_run_row_json(row: &Map<String, Value>) -> Value {
    json!({
        "verb": str_col(row, "verb"),
        "engine": str_col(row, "engine"),
        "outcome": str_col(row, "outcome"),
        "detail": str_col(row, "detail"),
        "runAt": str_col(row, "run_at"),
    })
}

/// # Errors
/// Returns [`lakehouse_clickhouse::ChError`] if the query fails, including
/// `bronze_meta.maintenance_verb_run` not existing yet — see
/// [`maintenance_verb_runs_or_empty`], the only caller, for why that is
/// NOT surfaced as a 503 the way [`latest_maintenance_run`]'s missing
/// table is.
async fn latest_maintenance_verb_runs(
    ch: &lakehouse_clickhouse::ChClient,
    table_name: &str,
) -> Result<Vec<Value>, lakehouse_clickhouse::ChError> {
    let rows = ch
        .rows(&maintenance_verb_run_query(table_name), None)
        .await?;
    Ok(rows.iter().map(maintenance_verb_run_row_json).collect())
}

/// Unlike `bronze_meta.maintenance_run` (whose absence means "the P4
/// maintenance job has never run on this deployment" — a real outage
/// signal `classify_ch_error` turns into a fixed 503), a missing
/// `bronze_meta.maintenance_verb_run` is the NORMAL steady state for any
/// table that has never had a policy run a Trino verb, and for every
/// deployment that predates this feature. Turning that into the same 503
/// would make a brand-new, additive field regress an endpoint that was
/// already serving successfully — so a lookup failure here degrades to an
/// honest empty list (logged, never silently swallowed) instead of
/// failing the whole `GET`.
async fn maintenance_verb_runs_or_empty(
    ch: &lakehouse_clickhouse::ChClient,
    table_name: &str,
) -> Vec<Value> {
    match latest_maintenance_verb_runs(ch, table_name).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                %err,
                table_name,
                "maintenance-verb-run lookup failed, reporting no verb runs"
            );
            Vec::new()
        }
    }
}

/// `GET /api/lakehouse/warehouses`.
///
/// # Errors
/// 503 if the catalog cannot be reached.
pub async fn warehouses(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let client = lakehouse_catalog::client(&state)
        .await
        .map_err(|err| classify_catalog_access_error(err, "warehouse"))?;
    // Infallible — an unreachable warehouse is reported as
    // `reachable: false`, never as an error (see `rest::list_warehouses`).
    let warehouses = rest::list_warehouses(&client, &state.config.lakekeeper_warehouse).await;
    Ok(ApiJson(warehouses_body(&warehouses)))
}

#[derive(Debug, Deserialize)]
pub struct NamespacesQuery {
    #[serde(default)]
    warehouse: Option<String>,
}

/// `GET /api/lakehouse/namespaces?warehouse=`.
///
/// # Errors
/// 404 if `warehouse` names anything other than the configured warehouse,
/// 503 if the catalog cannot be reached, or if the reader token is
/// unavailable.
pub async fn namespaces(
    State(state): State<AppState>,
    Query(query): Query<NamespacesQuery>,
) -> ApiResult<ApiJson<Value>> {
    validate_warehouse(&state, query.warehouse.as_deref())?;

    let idents = lakehouse_catalog::call(&state, |client| async move {
        rest::list_namespaces(&client).await
    })
    .await
    .map_err(|err| classify_catalog_access_error(err, "namespace"))?;
    let names: Vec<String> = idents.iter().map(NamespaceIdent::to_url_string).collect();

    // One client, fetched ONCE, shared by every fan-out item below: each
    // item calls `rest::list_table_idents` directly on it, never through
    // `lakehouse_catalog::call`, so one namespace's catalog error can never
    // evict the shared cache out from under the other in-flight lookups
    // (a failed or unfinished item is simply a `null` `tableCount` — see
    // `namespaces_body`).
    let client = lakehouse_catalog::client(&state)
        .await
        .map_err(|err| classify_catalog_access_error(err, "namespace"))?;
    let table_counts = bounded::run_with_budget(names.clone(), Duration::from_secs(2), 8, {
        move |name: String| {
            let client = client.clone();
            async move {
                let ns = NamespaceIdent::from_strs([name.as_str()]).ok()?;
                rest::list_table_idents(&client, &ns)
                    .await
                    .ok()
                    .map(|idents| idents.len())
            }
        }
    })
    .await;

    Ok(ApiJson(namespaces_body(&names, &table_counts)))
}

#[derive(Debug, Deserialize)]
pub struct TablesQuery {
    #[serde(default)]
    warehouse: Option<String>,
    #[serde(default)]
    namespace: Option<String>,
}

/// `GET /api/lakehouse/tables?warehouse=&namespace=`.
///
/// # Errors
/// 400 if `namespace` is missing or not `[a-z0-9_]+`-shaped, 404 if
/// `warehouse` names anything other than the configured warehouse or if
/// `namespace` does not exist, 503 if the catalog cannot be reached.
pub async fn tables(
    State(state): State<AppState>,
    Query(query): Query<TablesQuery>,
) -> ApiResult<ApiJson<Value>> {
    validate_warehouse(&state, query.warehouse.as_deref())?;
    let namespace = query
        .namespace
        .ok_or_else(|| ApiError::BadRequest("namespace is required".to_owned()))?;
    validate_ident(&namespace, "namespace")?;
    let ns = NamespaceIdent::from_strs([namespace.as_str()])
        .map_err(|_| ApiError::BadRequest("namespace is invalid".to_owned()))?;

    let idents = lakehouse_catalog::call(&state, {
        let ns = ns.clone();
        move |client| {
            let ns = ns.clone();
            async move { rest::list_table_idents(&client, &ns).await }
        }
    })
    .await
    .map_err(|err| classify_catalog_access_error(err, "namespace"))?;

    let names: Vec<String> = idents.iter().map(|ident| ident.name().to_owned()).collect();
    // One client, fetched ONCE, shared by every fan-out item below — see
    // `namespaces`' matching comment for why a per-item call must not go
    // through `lakehouse_catalog::call`.
    let client = lakehouse_catalog::client(&state)
        .await
        .map_err(|err| classify_catalog_access_error(err, "table"))?;
    let summaries = bounded::run_with_budget(names, Duration::from_secs(2), 8, {
        let ns = ns.clone();
        move |name: String| {
            let client = client.clone();
            let ident = TableIdent::new(ns.clone(), name);
            async move { rest::load_table_summary(&client, &ident).await.ok() }
        }
    })
    .await;

    Ok(ApiJson(tables_body(&namespace, &idents, &summaries)))
}

/// `GET /api/lakehouse/tables/{ns}/{table}`.
///
/// # Errors
/// 400 if `ns`/`table` are not `[a-z0-9_]+`-shaped, 404 if the table does
/// not exist, 503 if the catalog cannot be reached.
pub async fn table_detail(
    State(state): State<AppState>,
    Path((ns, table)): Path<(String, String)>,
) -> ApiResult<ApiJson<Value>> {
    validate_ident(&ns, "ns")?;
    validate_ident(&table, "table")?;
    let namespace = NamespaceIdent::from_strs([ns.as_str()])
        .map_err(|_| ApiError::BadRequest("ns is invalid".to_owned()))?;
    let ident = TableIdent::new(namespace, table);

    let detail = lakehouse_catalog::call(&state, {
        let ident = ident.clone();
        move |client| {
            let ident = ident.clone();
            async move { rest::load_table_detail(&client, &ident).await }
        }
    })
    .await
    .map_err(|err| classify_catalog_access_error(err, "table"))?;

    Ok(ApiJson(table_detail_body(&detail)))
}

/// `true` only for `bronze`: the P4 maintenance job (and its
/// `bronze_meta.maintenance_run` history table) only ever compacts
/// `bronze` tables, and that table has no namespace column — a
/// `table_name`-only filter would attribute `bronze/orders`' run to
/// `silver/orders` if both existed. [`maintenance`] uses this to decide
/// whether to query `ClickHouse` at all for a given namespace, rather than
/// returning a row that merely happens to share the table's name.
fn last_run_applies(ns: &str) -> bool {
    ns == "bronze"
}

/// `GET /api/lakehouse/tables/{ns}/{table}/maintenance`.
///
/// # Errors
/// 400 if `ns`/`table` are not `[a-z0-9_]+`-shaped, 503 if no Postgres pool
/// is configured ([`lakehouse_store::StoreError::Unavailable`]) or the
/// `ClickHouse` maintenance-run lookup fails — following
/// `routes::governance::maintenance`'s own documented posture: a missing
/// `bronze_meta.maintenance_run` table (the P4 job has never run on this
/// deployment) looks the same as `ClickHouse` being down, and both get the
/// same fixed 503 rather than a fabricated `lastRun: null` success.
pub async fn maintenance(
    State(state): State<AppState>,
    Path((ns, table)): Path<(String, String)>,
) -> ApiResult<ApiJson<Value>> {
    validate_ident(&ns, "ns")?;
    validate_ident(&table, "table")?;

    let policy = maintenance_policy::get_policy(pool(&state)?, &ns, &table).await?;

    // `bronze_meta.maintenance_run` has no namespace column and the job
    // only maintains `bronze` — see `last_run_applies`. For any other
    // namespace, `lastRun` is honestly `null` without querying
    // `ClickHouse` at all, rather than risking another namespace's
    // same-named table's run.
    let last_run = if last_run_applies(&ns) {
        latest_maintenance_run(&state.clickhouse, Some(&table))
            .await
            .map_err(|err| classify_ch_error(&err))?
            .into_iter()
            .next()
    } else {
        None
    };

    // `lastVerbRuns` — see `maintenance_verb_runs_or_empty`'s doc comment
    // for why a lookup failure here degrades to an empty list instead of
    // the 503 `last_run`'s own failure gives.
    let last_verb_runs = if last_run_applies(&ns) {
        maintenance_verb_runs_or_empty(&state.clickhouse, &table).await
    } else {
        Vec::new()
    };

    Ok(ApiJson(maintenance_body(
        &ns,
        &table,
        policy.as_ref(),
        last_run,
        &last_verb_runs,
    )))
}

/// `POST /api/lakehouse/tables/{ns}/{table}/maintenance`'s body. CamelCase
/// on the wire, matching [`maintenance_body`]'s GET shape.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MaintenancePolicyBody {
    snapshots_to_keep: Option<i32>,
    orphan_age_hours: Option<i32>,
    compact_small_files: bool,
    schedule: Option<String>,
}

/// Parses `body` as [`MaintenancePolicyBody`], or a 400 with a fixed
/// English message — mirrors `routes::connectors::parse_body`'s
/// `Bytes`-in, `ApiError::BadRequest`-out shape; the message itself is
/// fixed rather than embedding `serde_json`'s own text (WS1 T17c: new
/// response strings are English, and a fixed message is simpler to assert
/// on than whatever wording a parser version happens to produce).
fn parse_maintenance_policy_body(body: &Bytes) -> Result<MaintenancePolicyBody, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::BadRequest("body must be JSON".to_owned()))
}

/// Bounds shared with `table_maintenance_policy`'s CHECK constraints
/// (migration `0030`), checked here first so a bad request gets a 400
/// naming the field rather than the generic `StoreError::Database` a CHECK
/// violation would otherwise surface as.
fn validate_maintenance_policy_body(body: &MaintenancePolicyBody) -> Result<(), ApiError> {
    if body.snapshots_to_keep.is_some_and(|n| n < 1) {
        return Err(ApiError::BadRequest(
            "snapshotsToKeep must be at least 1: the current snapshot is always kept".to_owned(),
        ));
    }
    if body.orphan_age_hours.is_some_and(|n| n < 1) {
        return Err(ApiError::BadRequest(
            "orphanAgeHours must be at least 1: an age of 0 could delete files an in-flight \
             write still needs"
                .to_owned(),
        ));
    }
    if let Some(schedule) = &body.schedule
        && schedule != "daily"
        && schedule != "weekly"
    {
        return Err(ApiError::BadRequest(
            "schedule must be \"daily\", \"weekly\", or null".to_owned(),
        ));
    }
    Ok(())
}

/// `POST /api/lakehouse/tables/{ns}/{table}/maintenance` — create or
/// replace this table's maintenance policy.
///
/// Deliberately does not check the table exists in the catalog: the
/// policy store (`table_maintenance_policy`) is independent of the
/// catalog, and `dagster/dispar_orchestrate/maintenance.py`'s job only
/// ever acts on tables it discovers there, so a policy for a table that
/// does not exist (yet, or ever) is simply inert. Requiring a catalog
/// round trip here would make writing a policy depend on Lakekeeper being
/// up, for no benefit — the maintenance GET route's own `configured: true`
/// already shows a caller their policy was saved.
///
/// # Errors
///
/// 400 if `ns`/`table` are not `[a-z0-9_]+`-shaped, the body is not JSON,
/// or a bound (`snapshotsToKeep`/`orphanAgeHours` below 1, an unrecognized
/// `schedule`) is out of range; 401/403 from the auth gate
/// (`governance:write`); 503 if no Postgres pool is configured.
pub async fn set_maintenance_policy(
    State(state): State<AppState>,
    Path((ns, table)): Path<(String, String)>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    validate_ident(&ns, "ns")?;
    validate_ident(&table, "table")?;
    let parsed = parse_maintenance_policy_body(&body)?;
    validate_maintenance_policy_body(&parsed)?;

    let input = maintenance_policy::MaintenancePolicyInput {
        namespace: ns.clone(),
        table_name: table.clone(),
        snapshots_to_keep: parsed.snapshots_to_keep,
        orphan_age_hours: parsed.orphan_age_hours,
        compact_small_files: parsed.compact_small_files,
        schedule: parsed.schedule.clone(),
    };
    maintenance_policy::upsert_policy(pool(&state)?, &input).await?;

    // The GET maintenance shape, minus `lastRun`: a form that just saved a
    // policy shows what was stored without a `ClickHouse` round trip.
    Ok(ApiJson(json!({
        "namespace": ns,
        "tableName": table,
        "configured": true,
        "snapshotsToKeep": parsed.snapshots_to_keep,
        "orphanAgeHours": parsed.orphan_age_hours,
        "compactSmallFiles": parsed.compact_small_files,
        "schedule": parsed.schedule,
    })))
}

/// `GET /api/lakehouse/maintenance-policies` — every configured table
/// maintenance policy.
///
/// Read by `dagster/dispar_orchestrate/maintenance.py` using the
/// scope-less `lakehouse-maintenance-policy-reader` service identity
/// (`main.rs`'s `bootstrap_lakehouse_maintenance_service`). Mounted as
/// `Policy::RequiresAuth`, not a specific permission: the list carries no
/// secret, only per-table retention/compaction settings, and the
/// maintenance service identity holds no scopes to check against.
///
/// # Errors
///
/// 503 if no Postgres pool is configured.
pub async fn list_maintenance_policies(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let rows = maintenance_policy::list_all_policies(pool(&state)?).await?;
    Ok(ApiJson(json!({
        "policies": rows.iter().map(|r| json!({
            "namespace": r.namespace,
            "tableName": r.table_name,
            "snapshotsToKeep": r.snapshots_to_keep,
            "orphanAgeHours": r.orphan_age_hours,
            "compactSmallFiles": r.compact_small_files,
            "schedule": r.schedule,
        })).collect::<Vec<_>>()
    })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_iceberg::rest::{FieldDetail, PartitionFieldDetail, TableStats};

    use super::*;

    #[test]
    fn is_valid_ident_accepts_lowercase_digits_and_underscore() {
        assert!(is_valid_ident("bronze_orders_1"));
        assert!(!is_valid_ident("Bronze"));
        assert!(!is_valid_ident("has space"));
        assert!(!is_valid_ident(""));
        assert!(!is_valid_ident("has-dash"));
    }

    #[test]
    fn warehouses_body_lists_the_configured_warehouse_with_honest_nulls() {
        let warehouses = vec![WarehouseSummary {
            name: "default".to_owned(),
            reachable: true,
        }];

        let body = warehouses_body(&warehouses);

        assert_eq!(body["warehouses"][0]["id"], json!("default"));
        assert_eq!(body["warehouses"][0]["name"], json!("default"));
        assert_eq!(body["warehouses"][0]["reachable"], json!(true));
        assert_eq!(body["warehouses"][0]["storageProfile"], Value::Null);
        assert_eq!(body["warehouses"][0]["credentialVending"], Value::Null);
    }

    #[test]
    fn namespaces_body_nulls_a_table_count_that_never_finished() {
        let names = vec!["bronze".to_owned(), "silver".to_owned()];
        let mut counts = HashMap::new();
        counts.insert("bronze".to_owned(), 3_usize);

        let body = namespaces_body(&names, &counts);

        assert_eq!(body["namespaces"][0]["name"], json!("bronze"));
        assert_eq!(body["namespaces"][0]["tableCount"], json!(3));
        assert_eq!(body["namespaces"][1]["name"], json!("silver"));
        assert_eq!(body["namespaces"][1]["tableCount"], Value::Null);
    }

    fn ident(ns: &str, name: &str) -> TableIdent {
        TableIdent::new(
            NamespaceIdent::from_strs([ns]).expect("namespace"),
            name.to_owned(),
        )
    }

    #[test]
    fn tables_body_lists_a_table_that_never_loaded_with_null_stats() {
        let idents = vec![ident("bronze", "orders"), ident("bronze", "unloaded")];
        let mut summaries = HashMap::new();
        summaries.insert(
            "orders".to_owned(),
            TableSummary {
                namespace: "bronze".to_owned(),
                name: "orders".to_owned(),
                format_version: 2,
                current_snapshot_id: Some(42),
                last_updated_ms: Some(1_700_000_000_000),
                stats: TableStats {
                    file_count: Some(3),
                    record_count: Some(150),
                    total_bytes: Some(2048),
                },
            },
        );

        let body = tables_body("bronze", &idents, &summaries);

        let rows = body["tables"].as_array().expect("tables array");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["name"], json!("orders"));
        assert_eq!(rows[0]["currentSnapshotId"], json!("42"));
        assert_eq!(rows[0]["fileCount"], json!(3));
        assert_eq!(rows[0]["lastUpdatedAt"], json!("2023-11-14T22:13:20Z"));
        assert_eq!(rows[1]["namespace"], json!("bronze"));
        assert_eq!(rows[1]["name"], json!("unloaded"));
        assert_eq!(rows[1]["currentSnapshotId"], Value::Null);
        assert_eq!(rows[1]["fileCount"], Value::Null);
        assert_eq!(rows[1]["lastUpdatedAt"], Value::Null);
    }

    /// Iceberg snapshot ids are `i64`, normally random and far above
    /// `Number.MAX_SAFE_INTEGER` (2^53−1 = `9_007_199_254_740_991`). Any id
    /// past that point must survive as a JSON *string* — a JSON number
    /// would be rounded by the browser's `JSON.parse`.
    #[test]
    fn table_row_emits_current_snapshot_id_as_a_string_above_javascript_safe_integer() {
        let idents = vec![ident("bronze", "orders")];
        let mut summaries = HashMap::new();
        summaries.insert(
            "orders".to_owned(),
            TableSummary {
                namespace: "bronze".to_owned(),
                name: "orders".to_owned(),
                format_version: 2,
                current_snapshot_id: Some(9_007_199_254_740_993_i64),
                last_updated_ms: None,
                stats: TableStats::default(),
            },
        );

        let body = tables_body("bronze", &idents, &summaries);

        assert_eq!(
            body["tables"][0]["currentSnapshotId"],
            json!("9007199254740993")
        );
    }

    #[test]
    fn last_updated_at_is_none_with_no_snapshot() {
        assert_eq!(last_updated_at(None), None);
    }

    #[test]
    fn table_detail_body_nulls_small_file_fields_and_maps_snapshots() {
        let detail = TableDetail {
            schema: vec![FieldDetail {
                id: 1,
                name: "id".to_owned(),
                r#type: "long".to_owned(),
                required: true,
            }],
            partition_fields: vec![PartitionFieldDetail {
                source_id: 2,
                transform: "day".to_owned(),
                name: "created_at_day".to_owned(),
            }],
            properties: HashMap::new(),
            snapshots: vec![SnapshotDetail {
                id: 1,
                parent_id: None,
                timestamp_ms: 1_700_000_000_000,
                operation: "append".to_owned(),
                added_records: Some(100),
                deleted_records: None,
                total_records: Some(100),
                total_data_files: Some(1),
                stats: TableStats::default(),
            }],
            snapshot_count: 1,
            metadata_log_count: 1,
            stats: TableStats {
                file_count: Some(1),
                record_count: Some(100),
                total_bytes: Some(1024),
            },
        };

        let body = table_detail_body(&detail);

        assert_eq!(body["schema"][0]["name"], json!("id"));
        assert_eq!(body["schema"][0]["required"], json!(true));
        assert_eq!(body["partitionSpec"][0]["sourceId"], json!(2));
        assert_eq!(body["snapshots"][0]["id"], json!("1"));
        assert_eq!(body["snapshots"][0]["parentId"], Value::Null);
        assert_eq!(body["snapshots"][0]["summary"]["addedRecords"], json!(100));
        assert_eq!(
            body["snapshots"][0]["summary"]["deletedRecords"],
            Value::Null
        );
        assert_eq!(body["stats"]["fileCount"], json!(1));
        assert_eq!(body["stats"]["smallFileCount"], Value::Null);
        assert_eq!(body["stats"]["smallFileThresholdBytes"], Value::Null);
        assert_eq!(body["stats"]["snapshotCount"], json!(1));
        assert_eq!(body["stats"]["metadataLogCount"], json!(1));
    }

    /// `id` and `parentId` must both survive as strings past
    /// `Number.MAX_SAFE_INTEGER`, and a `None` `parentId` must still be
    /// `null` rather than the string `"null"`.
    #[test]
    fn snapshot_json_emits_id_and_parent_id_as_strings_above_javascript_safe_integer() {
        let snapshot = SnapshotDetail {
            id: 9_007_199_254_740_993_i64,
            parent_id: Some(9_007_199_254_740_993_i64),
            timestamp_ms: 1_700_000_000_000,
            operation: "append".to_owned(),
            added_records: None,
            deleted_records: None,
            total_records: None,
            total_data_files: None,
            stats: TableStats::default(),
        };

        let body = snapshot_json(&snapshot);

        assert_eq!(body["id"], json!("9007199254740993"));
        assert_eq!(body["parentId"], json!("9007199254740993"));
    }

    #[test]
    fn snapshot_json_nulls_parent_id_for_the_first_snapshot() {
        let snapshot = SnapshotDetail {
            id: 1,
            parent_id: None,
            timestamp_ms: 1_700_000_000_000,
            operation: "append".to_owned(),
            added_records: None,
            deleted_records: None,
            total_records: None,
            total_data_files: None,
            stats: TableStats::default(),
        };

        let body = snapshot_json(&snapshot);

        assert_eq!(body["id"], json!("1"));
        assert_eq!(body["parentId"], Value::Null);
    }

    #[test]
    fn maintenance_body_reports_unconfigured_with_the_jobs_real_default() {
        let body = maintenance_body("bronze", "orders", None, None, &[]);

        assert_eq!(body["configured"], json!(false));
        assert_eq!(body["snapshotsToKeep"], Value::Null);
        assert_eq!(body["compactSmallFiles"], json!(false));
        assert_eq!(body["lastRun"], Value::Null);
        assert_eq!(body["lastVerbRuns"], json!([]));
    }

    #[test]
    fn maintenance_body_reports_a_configured_policy_and_its_last_run() {
        let policy = MaintenancePolicyRow {
            namespace: "bronze".to_owned(),
            table_name: "orders".to_owned(),
            snapshots_to_keep: Some(10),
            orphan_age_hours: Some(48),
            compact_small_files: true,
            schedule: Some("daily".to_owned()),
        };
        let last_run = json!({ "tableName": "orders", "runAt": "2026-01-01T00:00:00Z" });
        let last_verb_runs = vec![json!({
            "verb": "optimize",
            "engine": "trino",
            "outcome": "applied",
            "detail": "",
            "runAt": "2026-01-01T00:00:00Z",
        })];

        let body = maintenance_body(
            "bronze",
            "orders",
            Some(&policy),
            Some(last_run.clone()),
            &last_verb_runs,
        );

        assert_eq!(body["configured"], json!(true));
        assert_eq!(body["snapshotsToKeep"], json!(10));
        assert_eq!(body["compactSmallFiles"], json!(true));
        assert_eq!(body["schedule"], json!("daily"));
        assert_eq!(body["lastRun"], last_run);
        assert_eq!(body["lastVerbRuns"], json!(last_verb_runs));
    }

    #[test]
    fn maintenance_verb_run_query_binds_the_table_name_as_a_literal_not_an_identifier() {
        let sql = maintenance_verb_run_query("orders'; DROP TABLE x; --");

        assert!(sql.contains("WHERE table_name = 'orders''; DROP TABLE x; --'"));
        assert!(sql.contains("FROM lake.`bronze_meta.maintenance_verb_run`"));
        assert!(sql.contains("ORDER BY verb"));
    }

    #[test]
    fn maintenance_verb_run_row_json_uses_camel_case_field_names() {
        let mut row = Map::new();
        row.insert("verb".to_owned(), json!("expire_snapshots"));
        row.insert("engine".to_owned(), json!("trino"));
        row.insert("outcome".to_owned(), json!("refused"));
        row.insert("detail".to_owned(), json!("min-retention floor"));
        row.insert("run_at".to_owned(), json!("2026-01-01T00:00:00Z"));

        let body = maintenance_verb_run_row_json(&row);

        assert_eq!(
            body,
            json!({
                "verb": "expire_snapshots",
                "engine": "trino",
                "outcome": "refused",
                "detail": "min-retention floor",
                "runAt": "2026-01-01T00:00:00Z",
            })
        );
    }

    #[test]
    fn classify_rest_error_maps_not_found_to_404_naming_the_table() {
        let err = classify_rest_error(&RestError::NotFound, "table");
        assert_eq!(err.status(), 404);
        assert_eq!(err.to_string(), "table not found");
    }

    #[test]
    fn classify_rest_error_maps_not_found_to_404_naming_the_namespace() {
        // `list_table_idents`'s `NotFound` means the NAMESPACE is missing,
        // not a table — `tables` must pass the right word rather than the
        // fixed "table not found" every `NotFound` used to get.
        let err = classify_rest_error(&RestError::NotFound, "namespace");
        assert_eq!(err.status(), 404);
        assert_eq!(err.to_string(), "namespace not found");
    }

    #[test]
    fn classify_rest_error_maps_catalog_failure_to_503_with_fixed_text() {
        let err = classify_rest_error(
            &RestError::Catalog(lakehouse_iceberg::IcebergError::Catalog(
                "connection refused to internal-host:1234".to_owned(),
            )),
            "table",
        );
        assert_eq!(err.status(), 503);
        assert_eq!(err.to_string(), "lakehouse catalog unavailable");
        assert!(!err.to_string().contains("internal-host"));
    }

    #[test]
    fn last_run_applies_only_to_bronze() {
        assert!(last_run_applies("bronze"));
        assert!(!last_run_applies("silver"));
        assert!(!last_run_applies("gold"));
    }

    fn valid_maintenance_policy_body() -> MaintenancePolicyBody {
        MaintenancePolicyBody {
            snapshots_to_keep: Some(10),
            orphan_age_hours: Some(48),
            compact_small_files: true,
            schedule: Some("daily".to_owned()),
        }
    }

    #[test]
    fn validate_maintenance_policy_body_accepts_the_minimum_bounds() {
        let mut body = valid_maintenance_policy_body();
        body.snapshots_to_keep = Some(1);
        assert!(validate_maintenance_policy_body(&body).is_ok());
    }

    #[test]
    fn validate_maintenance_policy_body_rejects_zero_snapshots_to_keep() {
        let mut body = valid_maintenance_policy_body();
        body.snapshots_to_keep = Some(0);
        let err = validate_maintenance_policy_body(&body).expect_err("0 keeps nothing new");
        assert_eq!(err.status(), 400);
        assert!(err.to_string().contains("snapshotsToKeep"));
    }

    #[test]
    fn validate_maintenance_policy_body_rejects_a_negative_snapshots_to_keep() {
        let mut body = valid_maintenance_policy_body();
        body.snapshots_to_keep = Some(-1);
        assert!(validate_maintenance_policy_body(&body).is_err());
    }

    #[test]
    fn validate_maintenance_policy_body_rejects_zero_orphan_age_hours() {
        let mut body = valid_maintenance_policy_body();
        body.orphan_age_hours = Some(0);
        let err = validate_maintenance_policy_body(&body)
            .expect_err("an orphan age of 0 could delete in-flight writes");
        assert_eq!(err.status(), 400);
        assert!(err.to_string().contains("orphanAgeHours"));
    }

    #[test]
    fn validate_maintenance_policy_body_rejects_an_unknown_schedule() {
        let mut body = valid_maintenance_policy_body();
        body.schedule = Some("hourly".to_owned());
        let err = validate_maintenance_policy_body(&body).expect_err("hourly is not a schedule");
        assert_eq!(err.status(), 400);
        assert!(err.to_string().contains("schedule"));
    }

    #[test]
    fn validate_maintenance_policy_body_accepts_daily_weekly_and_null_schedule() {
        for schedule in [Some("daily".to_owned()), Some("weekly".to_owned()), None] {
            let mut body = valid_maintenance_policy_body();
            body.schedule = schedule;
            assert!(validate_maintenance_policy_body(&body).is_ok());
        }
    }

    #[test]
    fn validate_maintenance_policy_body_accepts_an_all_null_policy() {
        let body = MaintenancePolicyBody {
            snapshots_to_keep: None,
            orphan_age_hours: None,
            compact_small_files: false,
            schedule: None,
        };
        assert!(validate_maintenance_policy_body(&body).is_ok());
    }

    #[test]
    fn parse_maintenance_policy_body_rejects_malformed_json() {
        let err = parse_maintenance_policy_body(&Bytes::from_static(b"not json at all"))
            .expect_err("malformed body must be rejected");
        assert_eq!(err.status(), 400);
        assert_eq!(err.to_string(), "body must be JSON");
    }

    /// Exercises `namespaces` end to end against a real (local, throwaway)
    /// HTTP server standing in for Lakekeeper — no live catalog, matching
    /// `lakehouse_iceberg::rest`'s own wiremock test style — to prove the
    /// fan-out fix at the level the bug actually lived at: `routes::
    /// lakehouse`'s per-item calls, not `lakehouse_catalog`'s retry
    /// mechanism in isolation.
    mod wiremock_tests {
        use std::collections::HashMap;
        use std::time::{SystemTime, UNIX_EPOCH};

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::*;
        use crate::config::Config;

        fn temp_token_file() -> std::path::PathBuf {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock before epoch")
                .as_nanos();
            std::env::temp_dir().join(format!("lakehouse-catalog-test-token-{nanos}"))
        }

        async fn state_pointed_at(server: &MockServer) -> AppState {
            let token_path = temp_token_file();
            tokio::fs::write(&token_path, "test-reader-token")
                .await
                .expect("write a throwaway token file");

            let mut env = HashMap::new();
            env.insert("LAKEKEEPER_CATALOG_URI".to_owned(), server.uri());
            env.insert("LAKEKEEPER_WAREHOUSE".to_owned(), "default".to_owned());
            env.insert(
                "LAKEKEEPER_READ_TOKEN_FILE".to_owned(),
                token_path.to_string_lossy().into_owned(),
            );
            AppState::new(Config::from_map(&env).expect("a valid test config"))
        }

        #[tokio::test]
        async fn one_namespaces_catalog_error_never_evicts_the_shared_client() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/v1/config"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_string(r#"{"defaults":{},"overrides":{}}"#),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_string(r#"{"namespaces":[["bronze"],["broken"]]}"#),
                )
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/v1/namespaces/bronze/tables"))
                .respond_with(ResponseTemplate::new(200).set_body_string(
                    r#"{"identifiers":[{"namespace":["bronze"],"name":"orders"}]}"#,
                ))
                .mount(&server)
                .await;
            // "broken" always 500s: a `RestError::Catalog`, not `NotFound`.
            // Under the old per-item `lakehouse_catalog::call` pattern this
            // would evict the shared client (see `bounded::run_with_budget`
            // §4 follow-up finding 1) and force a reconnect; the fix must
            // leave `/v1/config` hit exactly once no matter how many items
            // fail this way.
            Mock::given(method("GET"))
                .and(path("/v1/namespaces/broken/tables"))
                .respond_with(ResponseTemplate::new(500).set_body_string(
                    r#"{"error":{"message":"boom","type":"InternalServerError","code":500}}"#,
                ))
                .mount(&server)
                .await;

            let state = state_pointed_at(&server).await;

            let body = namespaces(State(state), Query(NamespacesQuery { warehouse: None }))
                .await
                .expect("namespaces succeeds even though one item fails")
                .0;

            assert_eq!(body["namespaces"][0]["name"], json!("bronze"));
            assert_eq!(body["namespaces"][0]["tableCount"], json!(1));
            assert_eq!(body["namespaces"][1]["name"], json!("broken"));
            assert_eq!(body["namespaces"][1]["tableCount"], Value::Null);

            let requests = server
                .received_requests()
                .await
                .expect("wiremock records requests by default");
            let config_hits = requests
                .iter()
                .filter(|r| r.url.path() == "/v1/config")
                .count();
            let broken_hits = requests
                .iter()
                .filter(|r| r.url.path() == "/v1/namespaces/broken/tables")
                .count();
            assert_eq!(
                config_hits, 1,
                "one item's catalog error must not force a reconnect"
            );
            assert_eq!(
                broken_hits, 1,
                "a fan-out item must not be retried by lakehouse_catalog::call"
            );
        }
    }
}
