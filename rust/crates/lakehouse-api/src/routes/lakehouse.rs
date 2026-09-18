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

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::HeaderMap;
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_auth::Principal;
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
use crate::routes::support::{num_or_zero, str_col};
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

/// Borrow the Postgres pool, or fail with a 503. Shared by every Postgres-
/// backed read in this module — the maintenance-policy store (WS2 §4) and,
/// as of WS8 plan Task C4, the tenant lookup [`warehouses`] needs to scope
/// its response — hence the generic message, not a maintenance-policy-
/// specific one that would misdescribe a tenant-lookup failure.
fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable("no Postgres pool is configured".to_owned()))
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

/// Classifies a `ClickHouse` capacity lookup failure — same posture as
/// [`classify_ch_error`], a fixed 503 rather than `ClickHouse`'s own error
/// text (AGENTS.md rule 4).
///
/// `bronze_meta.capacity_snapshot` is written once a day by a single
/// whole-deployment job (`dagster/dispar_orchestrate/
/// capacity_snapshot.py::capacity_snapshot_job`), not lazily per table like
/// `bronze_meta.maintenance_verb_run` (see `is_unknown_table_error`'s doc
/// comment, where absence is the normal steady state for a table that has
/// simply never had a verb run for ONE table). Here, the table not existing
/// yet means the capacity job has never run on this deployment at all —
/// indistinguishable, from a caller's point of view, from `ClickHouse`
/// being down. Both cases take this same fixed 503, never a fabricated
/// `buckets: []` success.
fn classify_capacity_ch_error(err: &lakehouse_clickhouse::ChError) -> ApiError {
    tracing::warn!(%err, "capacity snapshot lookup failed");
    ApiError::Unavailable("capacity metrics are unavailable".to_owned())
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
/// [`maintenance_verb_runs_or_empty`], the only caller, for how that
/// specific error is told apart from every other failure.
async fn latest_maintenance_verb_runs(
    ch: &lakehouse_clickhouse::ChClient,
    table_name: &str,
) -> Result<Vec<Value>, lakehouse_clickhouse::ChError> {
    let rows = ch
        .rows(&maintenance_verb_run_query(table_name), None)
        .await?;
    Ok(rows.iter().map(maintenance_verb_run_row_json).collect())
}

/// True when `body` is `ClickHouse`'s error text for "the table does not
/// exist" — this stack pins `ClickHouse` 26.7.3.19, whose exception text
/// for a missing table ends with the error's code name in parentheses.
/// Observed directly against `ClickHouse` 26.7.3.19 over the HTTP
/// interface (a read-only query against a table that does not exist),
/// e.g. ``"Code: 60. DB::Exception: Unknown table expression identifier \
/// 'lake.bronze_meta.maintenance_verb_run' in scope SELECT 1 FROM \
/// lake.`bronze_meta.maintenance_verb_run`. (UNKNOWN_TABLE) (version \
/// 26.7.3.19 (official build))"``. Matched on the code name
/// (`UNKNOWN_TABLE`) with the numeric code (`60`) checked too, belt and
/// braces. A small named function so it is unit-testable on plain
/// strings, without a live `ClickHouse`.
///
/// This inspects `ClickHouse`'s own error text only to CLASSIFY it into a
/// `bool` — the text itself is never returned or logged verbatim from
/// here, so AGENTS.md rule 4 (upstream error text never reaches a
/// response) still holds; [`classify_ch_error`] is what actually redacts
/// it for every case this function does not recognize.
fn is_unknown_table_error(body: &str) -> bool {
    body.contains("(UNKNOWN_TABLE)") || body.contains("Code: 60.")
}

/// `bronze_meta.maintenance_verb_run` is created lazily by
/// `dagster/dispar_orchestrate/bronze_catalog.py::record_maintenance_verb_run`
/// on the first verb run, so `ClickHouse` reporting it does not exist
/// (`is_unknown_table_error`) truthfully means "nothing has ever been
/// recorded for this table" — the one case where `[]` is honest rather
/// than fabricated.
///
/// Every OTHER `ChError` — including `ClickHouse` being unreachable —
/// takes the same fixed 503 [`classify_ch_error`] gives `lastRun`, so the
/// two fields on this route share one posture. Turning every failure into
/// `[]` (as this function used to) made a genuine outage indistinguishable
/// from "nothing has run" — AGENTS.md rule 2.
///
/// # Errors
/// The fixed 503 [`classify_ch_error`] produces, for any `ChError` other
/// than a `ClickHouse` unknown-table error.
async fn maintenance_verb_runs_or_empty(
    ch: &lakehouse_clickhouse::ChClient,
    table_name: &str,
) -> Result<Vec<Value>, ApiError> {
    match latest_maintenance_verb_runs(ch, table_name).await {
        Ok(rows) => Ok(rows),
        Err(lakehouse_clickhouse::ChError::Server(ref body)) if is_unknown_table_error(body) => {
            tracing::debug!(
                table_name,
                "bronze_meta.maintenance_verb_run does not exist yet, reporting no verb runs"
            );
            Ok(Vec::new())
        }
        Err(err) => Err(classify_ch_error(&err)),
    }
}

/// `GET /api/lakehouse/warehouses` — the caller's tenant's own warehouse,
/// never every warehouse this deployment happens to know about.
///
/// # Tenant scoping (WS8 plan Task C4, Hard Requirement 2)
///
/// `tenant_scope::resolve` runs first, fail closed: a principal belonging
/// to zero tenants (`Ok(None)`) gets an EMPTY list, returned before the
/// Lakekeeper catalog is ever contacted — never "unscoped, show the
/// deployment's warehouse to everyone."
///
/// # Deviation from the plan's Task C4 pseudocode
///
/// The plan's Step 1/Step 2 assume `list_warehouses()` is a real,
/// multi-warehouse Lakekeeper `Management API` call this route filters by
/// id. That is not how this route (or `lakehouse_iceberg::rest::
/// list_warehouses`, see its own module doc comment) actually works: the
/// Management API's warehouse listing is admin-scoped and no long-running
/// service in this stack holds an admin token, so [`rest::list_warehouses`]
/// always reports exactly ONE warehouse — the single one this deployment
/// is configured against (`Config::lakekeeper_warehouse`) — verified
/// reachable, never filtered from a longer list. Scoping this route by
/// `tenant.warehouse_id` therefore means: resolve the tenant, read its
/// `warehouse_id`, and only call Lakekeeper (returning that one
/// configured warehouse) when the tenant's own `warehouse_id` names the
/// SAME warehouse this deployment serves. A tenant with no `warehouse_id`
/// (not provisioned, or a grandfathered `not_applicable` tenant —
/// `0042_tenant_provisioning.sql`) or one whose `warehouse_id` names a
/// DIFFERENT warehouse gets an empty list — never a fallback to the
/// shared/demo warehouse it does not itself own — and, in both cases,
/// without ever calling Lakekeeper (cheap AND fail-closed, matching the
/// no-tenant branch above).
///
/// # Errors
///
/// 404 if `X-Tenant` names a tenant the caller does not belong to
/// (`tenant_scope::resolve`). 503 if no pool is configured or the
/// Lakekeeper catalog cannot be reached; 500 on a database failure reading
/// the tenant row.
pub async fn warehouses(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    let Some(tenant_id) = crate::tenant_scope::resolve(&principal, &headers)? else {
        return Ok(ApiJson(warehouses_body(&[])));
    };
    let tenant =
        lakehouse_store::identity::get_tenant(pool(&state)?, &tenant_id.to_string()).await?;
    let Some(tenant_warehouse_id) = tenant.warehouse_id else {
        // Not provisioned (or `not_applicable`) -- nothing of its own to
        // show. Never falls back to the deployment's shared warehouse.
        return Ok(ApiJson(warehouses_body(&[])));
    };
    if tenant_warehouse_id != state.config.lakekeeper_warehouse {
        // The tenant's own warehouse isn't the one this deployment
        // actually serves -- same "nothing of its own to show" outcome,
        // and no Lakekeeper call for a warehouse that would never be
        // returned anyway.
        return Ok(ApiJson(warehouses_body(&[])));
    }
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
/// is configured ([`lakehouse_store::StoreError::Unavailable`]) or either
/// `ClickHouse` maintenance-history lookup fails — following
/// `routes::governance::maintenance`'s own documented posture: a missing
/// `bronze_meta.maintenance_run` table (the P4 job has never run on this
/// deployment) looks the same as `ClickHouse` being down, and both get the
/// same fixed 503 rather than a fabricated `lastRun: null` success.
/// `lastVerbRuns` shares that posture too, except for the one case where
/// `bronze_meta.maintenance_verb_run` itself does not exist yet — see
/// [`maintenance_verb_runs_or_empty`].
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

    // `lastVerbRuns` — see `maintenance_verb_runs_or_empty`'s doc comment:
    // it shares `last_run`'s fixed 503 for every failure except the table
    // itself not existing yet, which is the honest `[]`.
    let last_verb_runs = if last_run_applies(&ns) {
        maintenance_verb_runs_or_empty(&state.clickhouse, &table).await?
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

/// `bucket_name`/`bytes`/`objects` come back through `toString(...)`, like
/// every other numeric or 64-bit column read in this module, so a large
/// value never round-trips through `serde_json`'s own number parsing.
/// `measured_at` is `ClickHouse`'s own display string, passed straight to
/// `buckets[].measuredAt`; `measured_at_ms` is a `toUnixTimestamp64Milli`
/// epoch, used only internally by [`capacity_body`]'s growth-window
/// comparison, never rendered.
const CAPACITY_SNAPSHOT_QUERY: &str = "SELECT bucket_name, toString(bytes) bytes, \
    toString(objects) objects, \
    toString(toUnixTimestamp64Milli(measured_at)) measured_at_ms, \
    toString(measured_at) measured_at \
    FROM lake.`bronze_meta.capacity_snapshot` \
    WHERE measured_at >= now() - INTERVAL 8 DAY \
    ORDER BY measured_at DESC";

/// Live disk usage, read fresh on every request rather than from the daily
/// snapshot table — `system.parts` reflects merges/compaction that happen
/// far more often than once a day.
const BYTES_ON_DISK_QUERY: &str = "SELECT toString(sum(bytes_on_disk)) bytes_on_disk \
    FROM system.parts WHERE active";

/// A day, in milliseconds — the unit [`capacity_body`]'s growth window is
/// expressed in.
const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// The tolerance around `latest - 7 days` a candidate row's `measured_at`
/// may fall within and still count as "the seven-day-old reading" — see
/// [`capacity_body`]'s doc comment.
const GROWTH_WINDOW_MS: i64 = 12 * 60 * 60 * 1000;

/// `GET /api/lakehouse/capacity`'s body — pure over `rows`, already read
/// from `lake.bronze_meta.capacity_snapshot` for the last 8 days (newest
/// first), and `bytes_on_disk`, `ClickHouse`'s own live `system.parts`
/// total.
///
/// `buckets` reports the newest row seen for each distinct `bucket_name`
/// (this deployment's `capacity_snapshot_job` measures exactly one bucket,
/// `LAKEHOUSE_WAREHOUSE_BUCKET`, but nothing here assumes that).
///
/// `growth7d` is the total bytes across every bucket at the latest
/// timestamp, minus the total at the newest timestamp at or before
/// `latest - 7 days` that falls within a ±12 hour window of it — never the
/// row seven POSITIONS back, which a single missed daily run would
/// silently misalign against the wrong day. When no such reading exists
/// (fewer than ~8 days of history, or a gap wider than the window),
/// `growth7d` is `null` rather than a synthesized number.
fn capacity_body(rows: &[Map<String, Value>], bytes_on_disk: i64) -> Value {
    // The newest row per bucket, for `buckets` — `rows` is already
    // `ORDER BY measured_at DESC`, so the first row seen for a given name
    // is its newest.
    let mut buckets = Vec::new();
    let mut seen_buckets = std::collections::HashSet::new();
    for row in rows {
        let name = str_col(row, "bucket_name");
        if seen_buckets.insert(name.to_owned()) {
            buckets.push(json!({
                "name": name,
                "bytes": num_or_zero(Some(row), "bytes"),
                "objects": num_or_zero(Some(row), "objects"),
                "measuredAt": str_col(row, "measured_at"),
            }));
        }
    }

    // Total bytes across every bucket sharing an exact timestamp — this
    // deployment's single-bucket job writes one row per run, so this is
    // simply that row's total, but a future multi-bucket batch written at
    // the same instant is summed correctly rather than picked arbitrarily.
    let mut totals: Vec<(i64, i64)> = Vec::new();
    for row in rows {
        let ms = num_or_zero(Some(row), "measured_at_ms");
        let bytes = num_or_zero(Some(row), "bytes");
        if let Some(entry) = totals.iter_mut().find(|(t, _)| *t == ms) {
            entry.1 += bytes;
        } else {
            totals.push((ms, bytes));
        }
    }
    totals.sort_by_key(|&(ms, _)| std::cmp::Reverse(ms));

    let growth7d = totals.first().and_then(|&(latest_ms, latest_total)| {
        let target = latest_ms - 7 * DAY_MS;
        totals
            .iter()
            .filter(|&&(ms, _)| (ms - target).abs() <= GROWTH_WINDOW_MS)
            .max_by_key(|&&(ms, _)| ms)
            .map(|&(_, total)| latest_total - total)
    });

    json!({
        "buckets": buckets,
        "clickhouse": { "bytesOnDisk": bytes_on_disk },
        "growth7d": growth7d,
    })
}

/// `GET /api/lakehouse/capacity` — bucket capacity from the daily
/// `capacity_snapshot_job` plus `ClickHouse`'s own live disk usage,
/// replacing the cut `/api/storage*` surface (WS2 §4).
///
/// # Errors
///
/// 503 if either `ClickHouse` query fails — see
/// [`classify_capacity_ch_error`]'s doc comment for why a missing
/// `bronze_meta.capacity_snapshot` table takes the same fixed 503 as any
/// other failure here, never an empty `buckets: []` success.
pub async fn capacity(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let rows = state
        .clickhouse
        .rows(CAPACITY_SNAPSHOT_QUERY, None)
        .await
        .map_err(|err| classify_capacity_ch_error(&err))?;
    let disk_rows = state
        .clickhouse
        .rows(BYTES_ON_DISK_QUERY, None)
        .await
        .map_err(|err| classify_capacity_ch_error(&err))?;
    let bytes_on_disk = num_or_zero(disk_rows.first(), "bytes_on_disk");

    Ok(ApiJson(capacity_body(&rows, bytes_on_disk)))
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

    /// One `lake.bronze_meta.capacity_snapshot` row, in the exact shape
    /// [`CAPACITY_SNAPSHOT_QUERY`] returns: every numeric column already
    /// stringified. `measured_at_ms` is what [`capacity_body`]'s growth
    /// window compares against; `measured_at` is only ever echoed into
    /// `buckets[].measuredAt`, so tests that don't care about its exact
    /// text reuse it as a readable label.
    fn capacity_row(bucket_name: &str, bytes: i64, measured_at_ms: i64) -> Map<String, Value> {
        let mut row = Map::new();
        row.insert("bucket_name".to_owned(), json!(bucket_name));
        row.insert("bytes".to_owned(), json!(bytes.to_string()));
        row.insert("objects".to_owned(), json!("1"));
        row.insert(
            "measured_at_ms".to_owned(),
            json!(measured_at_ms.to_string()),
        );
        row.insert(
            "measured_at".to_owned(),
            json!(format!("day-{measured_at_ms}")),
        );
        row
    }

    #[test]
    fn capacity_body_reports_null_growth_with_a_single_row() {
        let rows = vec![capacity_row("lakehouse-warehouse", 100, 0)];

        let body = capacity_body(&rows, 500);

        assert_eq!(body["growth7d"], Value::Null);
        assert_eq!(body["buckets"][0]["bytes"], json!(100));
        assert_eq!(body["clickhouse"]["bytesOnDisk"], json!(500));
    }

    #[test]
    fn capacity_body_computes_growth7d_from_the_row_exactly_seven_days_back() {
        // Nine consecutive daily rows, newest first (as `ORDER BY
        // measured_at DESC` returns them) — day 0 is "today", day 8 is
        // eight days ago. `bytes` grows by 100 per day, so the row exactly
        // seven days back (day 7) holds 100 and today (day 0) holds 800.
        let rows: Vec<_> = (0..=8)
            .map(|days_ago: i64| {
                capacity_row(
                    "lakehouse-warehouse",
                    100 * (9 - days_ago),
                    -days_ago * DAY_MS,
                )
            })
            .collect();

        let body = capacity_body(&rows, 500);

        assert_eq!(body["growth7d"], json!(700));
    }

    #[test]
    fn capacity_body_is_null_when_the_seven_day_row_is_missing_and_outside_the_window() {
        // Day 7 (exactly seven days back) is missing; the nearest
        // surviving row, day 5, is two full days outside the ±12h window,
        // so no candidate qualifies and growth7d must be null rather than
        // silently comparing against the wrong day.
        let rows = vec![
            capacity_row("lakehouse-warehouse", 800, 0),
            capacity_row("lakehouse-warehouse", 500, -5 * DAY_MS),
            capacity_row("lakehouse-warehouse", 100, -8 * DAY_MS),
        ];

        let body = capacity_body(&rows, 500);

        assert_eq!(body["growth7d"], Value::Null);
    }

    #[test]
    fn capacity_body_accepts_a_run_that_landed_a_few_hours_off_the_seven_day_mark() {
        // The row six days and sixteen hours back — within the ±12 hour
        // window of "seven days back minus twelve hours" is NOT close
        // enough; six days and thirteen hours back IS within twelve hours
        // of the seven-day target's early edge (7d - 13h vs 7d), so it
        // must still be picked rather than yielding null.
        let seven_days_back = -7 * DAY_MS;
        let within_window = seven_days_back + (11 * 60 * 60 * 1000);
        let rows = vec![
            capacity_row("lakehouse-warehouse", 800, 0),
            capacity_row("lakehouse-warehouse", 300, within_window),
        ];

        let body = capacity_body(&rows, 500);

        assert_eq!(body["growth7d"], json!(500));
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
    fn is_unknown_table_error_accepts_a_real_clickhouse_unknown_table_body() {
        // Observed against ClickHouse 26.7.3.19 over the HTTP interface
        // (a read-only query against a table that does not exist), not
        // invented — this stack pins that version (WS2 §4).
        let body = "Code: 60. DB::Exception: Unknown table expression identifier \
                     'lake.bronze_meta.maintenance_verb_run' in scope SELECT 1 FROM \
                     lake.`bronze_meta.maintenance_verb_run`. (UNKNOWN_TABLE) \
                     (version 26.7.3.19 (official build))";
        assert!(is_unknown_table_error(body));
    }

    #[test]
    fn is_unknown_table_error_rejects_an_unrelated_clickhouse_error() {
        let body = "Code: 62. DB::Exception: Syntax error: failed at position 1 (SYNTAX_ERROR)";
        assert!(!is_unknown_table_error(body));
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

        /// `lakehouse_clickhouse::ChClient` posts every query to its
        /// configured URL directly (no fixed path), matching this crate's
        /// own wiremock tests for it.
        fn ch_client(server: &MockServer) -> lakehouse_clickhouse::ChClient {
            lakehouse_clickhouse::ChClient::new(server.uri(), "default".to_owned(), String::new())
        }

        #[tokio::test]
        async fn maintenance_verb_runs_or_empty_reports_no_runs_for_an_unknown_table() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(404).set_body_string(
                    // Observed against ClickHouse 26.7.3.19 over the HTTP
                    // interface (a read-only query against a table that does
                    // not exist), not invented — the same measured body
                    // `is_unknown_table_error_accepts_a_real_clickhouse_unknown_table_body`
                    // asserts against above.
                    "Code: 60. DB::Exception: Unknown table expression identifier \
                     'lake.bronze_meta.maintenance_verb_run' in scope SELECT 1 FROM \
                     lake.`bronze_meta.maintenance_verb_run`. (UNKNOWN_TABLE) \
                     (version 26.7.3.19 (official build))",
                ))
                .mount(&server)
                .await;

            let rows = maintenance_verb_runs_or_empty(&ch_client(&server), "orders")
                .await
                .expect("an unknown-table error is the one truthful empty list");

            assert_eq!(rows, Vec::<Value>::new());
        }

        #[tokio::test]
        async fn maintenance_verb_runs_or_empty_shares_last_runs_fixed_503_for_any_other_failure() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(500)
                        .set_body_string("Code: 210. DB::NetException: Connection refused"),
                )
                .mount(&server)
                .await;

            let err = maintenance_verb_runs_or_empty(&ch_client(&server), "orders")
                .await
                .expect_err("a real ClickHouse failure must not be reported as no verb runs");

            assert_eq!(err.status(), 503);
            assert_eq!(err.to_string(), "maintenance history is unavailable");
        }
    }

    /// `GET /api/lakehouse/warehouses` tenant scoping — WS8 plan Task C4.
    ///
    /// # Deviation from the plan's Task C4 pseudocode
    ///
    /// The plan's own Step 1 assumes a `mock_lakekeeper_warehouse_list`
    /// wiremock helper standing in for a real, multi-warehouse Lakekeeper
    /// `Management API` listing this route filters by id. No such helper
    /// exists anywhere in this codebase, and could not: `[warehouses]`'s
    /// doc comment (this file, above) and `lakehouse_iceberg::rest::
    /// list_warehouses`'s own module doc comment both establish that this
    /// service never lists warehouses from Lakekeeper's Management API at
    /// all (admin-scoped, no long-running service holds that token) — it
    /// always reports exactly the ONE configured warehouse, verified
    /// reachable via a real Iceberg REST catalog handshake
    /// (`IcebergClient::connect`, `GET /v1/config`). The only existing
    /// harness for that handshake in this repository
    /// (`lakehouse-iceberg/tests/g1_lakekeeper.rs`) is `#[ignore]`d and
    /// requires a live `docker compose` stack — building a wiremock stand-
    /// in for the full Iceberg REST protocol is out of scope for this
    /// task's own file list (`routes/lakehouse.rs` only).
    ///
    /// These tests instead prove the actual scoping contract the real code
    /// implements: the store-only branches (no tenant, no `warehouse_id`,
    /// a `warehouse_id` naming a DIFFERENT warehouse) return an empty list
    /// WITHOUT reaching the catalog at all, and the one branch that SHOULD
    /// reach it (`warehouse_id` matches `Config::lakekeeper_warehouse`)
    /// demonstrably does — proven by `LAKEKEEPER_READ_TOKEN_FILE` being
    /// left at its default, unprovisioned path
    /// (`/tokens/lakehouse-api-reader.jwt`, `config.rs`), so a genuine
    /// attempt to reach Lakekeeper fails fast with a 503 the moment it is
    /// attempted, distinguishing "reached the call" from "never tried."
    mod warehouses_route {
        use std::collections::HashMap;

        use lakehouse_auth::{PermissionSet, PrincipalId};
        use lakehouse_store::identity::{self, CreateTenantInput, TenantFilter};
        use uuid::Uuid;

        use super::super::*;
        use crate::config::Config;

        fn database_url_for(pool: &sqlx::PgPool) -> String {
            let options = pool.connect_options();
            format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            )
        }

        fn state_for(pool: &sqlx::PgPool) -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url_for(pool));
            let config = Config::from_map(&env).expect("a valid test Config");
            AppState::new(config)
        }

        fn state_without_pool() -> AppState {
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), "not a postgres url".to_owned());
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        fn principal_with_tenants(tenant_ids: &[Uuid]) -> Principal {
            Principal {
                id: PrincipalId::User(Uuid::from_u128(1)),
                tenant_ids: tenant_ids.to_vec(),
                display_name: "Rina Wijaya".to_owned(),
                permissions: PermissionSet::parse("catalog:read"),
                provider: "session".to_owned(),
                must_change_password: false,
                role_names: Vec::new(),
            }
        }

        fn headers_with_x_tenant(tenant_id: Uuid) -> HeaderMap {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-tenant",
                tenant_id
                    .to_string()
                    .parse()
                    .expect("uuid renders as a valid header value"),
            );
            headers
        }

        async fn warehouses_array(state: &AppState, principal: &Principal) -> Vec<Value> {
            let ApiJson(body) = warehouses(
                State(state.clone()),
                Extension(principal.clone()),
                HeaderMap::new(),
            )
            .await
            .expect("this branch must not error");
            body["warehouses"]
                .as_array()
                .expect("warehouses array")
                .clone()
        }

        async fn seed_tenant(pool: &sqlx::PgPool, slug: &str) -> Uuid {
            let tenant = identity::create_tenant(
                pool,
                &CreateTenantInput {
                    name: "Acme Co".to_owned(),
                    slug: slug.to_owned(),
                    plan: "Standard".to_owned(),
                    residency: "US".to_owned(),
                },
            )
            .await
            .expect("create a test tenant");
            tenant
                .id
                .parse()
                .expect("create_tenant returns a UUID-shaped id")
        }

        /// Hard Requirement 2: a principal belonging to zero tenants gets
        /// an EMPTY list, before the store is ever touched —
        /// `state_without_pool()` proves this: reaching `pool(&state)?`
        /// here would 503, not `Ok` with an empty body.
        #[tokio::test]
        async fn a_tenantless_principal_gets_an_empty_list_before_touching_the_store() {
            let state = state_without_pool();
            let principal = principal_with_tenants(&[]);

            let warehouses = warehouses_array(&state, &principal).await;

            assert!(
                warehouses.is_empty(),
                "Ok(None) from tenant_scope::resolve must render as an empty list"
            );
        }

        /// `X-Tenant` naming a tenant the principal does not belong to is a
        /// 404 (via `tenant_scope::resolve`'s own contract) — never
        /// silently treated as "no tenant."
        #[tokio::test]
        async fn a_foreign_x_tenant_header_is_not_found() {
            let state = state_without_pool();
            let principal = principal_with_tenants(&[Uuid::from_u128(1)]);
            let headers = headers_with_x_tenant(Uuid::from_u128(2)); // not a member

            let err = warehouses(State(state), Extension(principal), headers)
                .await
                .expect_err("a foreign X-Tenant must be refused");

            assert_eq!(err.0.status(), 404);
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_tenant_with_no_warehouse_id_gets_an_empty_list(pool: sqlx::PgPool) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c4-no-warehouse").await;
            // Sanity: freshly created, never provisioned -- warehouse_id
            // really is NULL, not a test-setup accident.
            let stored = identity::list_tenants(&pool, &TenantFilter::default())
                .await
                .expect("list tenants")
                .into_iter()
                .find(|t| t.id == tenant_id.to_string())
                .expect("seeded tenant");
            assert_eq!(stored.warehouse_id, None);
            let principal = principal_with_tenants(&[tenant_id]);

            let warehouses = warehouses_array(&state, &principal).await;

            assert!(
                warehouses.is_empty(),
                "an unprovisioned tenant must never fall back to the shared warehouse"
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_tenant_whose_warehouse_id_names_a_different_warehouse_gets_an_empty_list(
            pool: sqlx::PgPool,
        ) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c4-other-warehouse").await;
            identity::update_tenant_provisioning_status(
                &pool,
                &tenant_id.to_string(),
                "warehouse_ready",
                Some("not-this-deployments-warehouse"),
            )
            .await
            .expect("record a warehouse id");
            let principal = principal_with_tenants(&[tenant_id]);

            let warehouses = warehouses_array(&state, &principal).await;

            assert!(
                warehouses.is_empty(),
                "a tenant's own warehouse that this deployment does not serve must never \
                 fall back to the shared warehouse"
            );
        }

        /// The one branch that SHOULD reach Lakekeeper: proven by a real
        /// attempt failing fast (503, `LAKEKEEPER_READ_TOKEN_FILE` at its
        /// unprovisioned default) rather than short-circuiting to an empty
        /// list the way every other branch above does.
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_tenant_whose_warehouse_id_matches_the_configured_warehouse_reaches_the_catalog(
            pool: sqlx::PgPool,
        ) {
            let state = state_for(&pool);
            let tenant_id = seed_tenant(&pool, "c4-matching-warehouse").await;
            identity::update_tenant_provisioning_status(
                &pool,
                &tenant_id.to_string(),
                "warehouse_ready",
                // `state_for` does not override `LAKEKEEPER_WAREHOUSE`, so
                // `Config::lakekeeper_warehouse` is its default, "default"
                // (`config.rs`).
                Some(&state.config.lakekeeper_warehouse),
            )
            .await
            .expect("record a warehouse id");
            let principal = principal_with_tenants(&[tenant_id]);

            let err = warehouses(State(state), Extension(principal), HeaderMap::new())
                .await
                .expect_err(
                    "a matching warehouse_id must reach the catalog client, which fails fast \
                 (no reader token provisioned in this test) rather than returning early",
                );

            assert_eq!(
                err.0.status(),
                503,
                "must be the catalog-access failure, not a validation/auth error"
            );
        }
    }
}
