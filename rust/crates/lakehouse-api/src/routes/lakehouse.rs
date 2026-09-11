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

use axum::extract::{Path, Query, State};
use iceberg::{NamespaceIdent, TableIdent};
use lakehouse_core::ApiError;
use lakehouse_iceberg::rest::{
    self, RestError, SnapshotDetail, TableDetail, TableSummary, WarehouseSummary,
};
use lakehouse_store::PgPool;
use lakehouse_store::maintenance_policy::{self, MaintenancePolicyRow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::bounded;
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::lakehouse_catalog;
use crate::routes::governance::latest_maintenance_run;
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
pub(crate) fn classify_rest_error(err: &RestError) -> ApiError {
    tracing::warn!(%err, "lakehouse catalog read failed");
    match err {
        RestError::NotFound => ApiError::NotFound("table not found".to_owned()),
        RestError::Catalog(_) => ApiError::Unavailable("lakehouse catalog unavailable".to_owned()),
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

fn table_row(namespace: &str, name: &str, summary: Option<&TableSummary>) -> Value {
    match summary {
        Some(s) => json!({
            "namespace": s.namespace,
            "name": s.name,
            "formatVersion": s.format_version,
            "currentSnapshotId": s.current_snapshot_id,
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

fn snapshot_json(s: &SnapshotDetail) -> Value {
    json!({
        "id": s.id,
        "parentId": s.parent_id,
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
        }),
    }
}

/// `GET /api/lakehouse/warehouses`.
///
/// # Errors
/// 503 if the catalog cannot be reached.
pub async fn warehouses(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let client = lakehouse_catalog::client(&state)
        .await
        .map_err(|err| classify_rest_error(&err))?;
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
/// 400 if `warehouse` is present but not `[a-z0-9_]+`-shaped isn't
/// required here (warehouse identity is opaque), 404 if it names a
/// warehouse other than the one configured, 503 if the catalog cannot be
/// reached.
pub async fn namespaces(
    State(state): State<AppState>,
    Query(query): Query<NamespacesQuery>,
) -> ApiResult<ApiJson<Value>> {
    validate_warehouse(&state, query.warehouse.as_deref())?;

    let idents = lakehouse_catalog::call(&state, |client| async move {
        rest::list_namespaces(&client).await
    })
    .await
    .map_err(|err| classify_rest_error(&err))?;
    let names: Vec<String> = idents.iter().map(NamespaceIdent::to_url_string).collect();

    let table_counts = bounded::run_with_budget(names.clone(), Duration::from_secs(2), 8, {
        let state = state.clone();
        move |name: String| {
            let state = state.clone();
            async move {
                let ns = NamespaceIdent::from_strs([name.as_str()]).ok()?;
                lakehouse_catalog::call(&state, |client| {
                    let ns = ns.clone();
                    async move { rest::list_table_idents(&client, &ns).await }
                })
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
    .map_err(|err| classify_rest_error(&err))?;

    let names: Vec<String> = idents.iter().map(|ident| ident.name().to_owned()).collect();
    let summaries = bounded::run_with_budget(names, Duration::from_secs(2), 8, {
        let state = state.clone();
        let ns = ns.clone();
        move |name: String| {
            let state = state.clone();
            let ident = TableIdent::new(ns.clone(), name);
            async move {
                lakehouse_catalog::call(&state, |client| {
                    let ident = ident.clone();
                    async move { rest::load_table_summary(&client, &ident).await }
                })
                .await
                .ok()
            }
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
    .map_err(|err| classify_rest_error(&err))?;

    Ok(ApiJson(table_detail_body(&detail)))
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

    let last_run = latest_maintenance_run(&state.clickhouse, Some(&table))
        .await
        .map_err(|err| classify_ch_error(&err))?
        .into_iter()
        .next();

    Ok(ApiJson(maintenance_body(
        &ns,
        &table,
        policy.as_ref(),
        last_run,
    )))
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
        assert_eq!(rows[0]["currentSnapshotId"], json!(42));
        assert_eq!(rows[0]["fileCount"], json!(3));
        assert_eq!(rows[0]["lastUpdatedAt"], json!("2023-11-14T22:13:20Z"));
        assert_eq!(rows[1]["namespace"], json!("bronze"));
        assert_eq!(rows[1]["name"], json!("unloaded"));
        assert_eq!(rows[1]["currentSnapshotId"], Value::Null);
        assert_eq!(rows[1]["fileCount"], Value::Null);
        assert_eq!(rows[1]["lastUpdatedAt"], Value::Null);
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

    #[test]
    fn maintenance_body_reports_unconfigured_with_the_jobs_real_default() {
        let body = maintenance_body("bronze", "orders", None, None);

        assert_eq!(body["configured"], json!(false));
        assert_eq!(body["snapshotsToKeep"], Value::Null);
        assert_eq!(body["compactSmallFiles"], json!(false));
        assert_eq!(body["lastRun"], Value::Null);
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

        let body = maintenance_body("bronze", "orders", Some(&policy), Some(last_run.clone()));

        assert_eq!(body["configured"], json!(true));
        assert_eq!(body["snapshotsToKeep"], json!(10));
        assert_eq!(body["compactSmallFiles"], json!(true));
        assert_eq!(body["schedule"], json!("daily"));
        assert_eq!(body["lastRun"], last_run);
    }

    #[test]
    fn classify_rest_error_maps_not_found_to_404_with_no_upstream_text() {
        let err = classify_rest_error(&RestError::NotFound);
        assert_eq!(err.status(), 404);
        assert_eq!(err.to_string(), "table not found");
    }

    #[test]
    fn classify_rest_error_maps_catalog_failure_to_503_with_fixed_text() {
        let err = classify_rest_error(&RestError::Catalog(
            lakehouse_iceberg::IcebergError::Catalog(
                "connection refused to internal-host:1234".to_owned(),
            ),
        ));
        assert_eq!(err.status(), 503);
        assert_eq!(err.to_string(), "lakehouse catalog unavailable");
        assert!(!err.to_string().contains("internal-host"));
    }
}
