//! Column-level lineage (`DATA-13`): the producers that record which source
//! column feeds which output column, and (in the later tasks of the plan)
//! the route that reads it back.
//!
//! The mapping itself is [`crate::sql_lineage::map_select`]; the rows live in
//! `column_lineage` (migration `0079`, `lakehouse_store::column_lineage`).
//! This module decides WHEN a statement becomes a record and with which
//! table keys, never what a statement means.
//!
//! # Best effort, loudly
//!
//! A producer never fails the request it rides on. A pipeline run that
//! succeeded must be reported as succeeded even if its lineage could not be
//! written, so a failure here is logged (fixed text plus the store's own
//! classified error, never an upstream body) and the caller carries on. What
//! this costs is a missing record, which the reader already says as a gap
//! ("no recorded lineage"), not a wrong one. A statement that cannot be
//! mapped at all REPLACES its producer's earlier rows with none, so a
//! changed statement never leaves the old mapping on display.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::AtomicBool;

use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::column_lineage::{self, Direction, MappingRow, NewMapping, Scope};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlparser::dialect::ClickHouseDialect;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::sql_lineage::{self, Lineage};
use crate::state::AppState;

/// Most tables whose columns are fetched for one statement, so a hostile or
/// huge `FROM` list cannot fan out into hundreds of engine queries.
const MAX_TABLES_PER_STATEMENT: usize = 25;

/// The table graph names the Gold zone `serving` (`zone_node`); a pipeline's
/// `gold.x` and a `serving.x` are one table.
pub(crate) fn graph_key(key: &str) -> String {
    match key.split_once('.') {
        Some(("gold", table)) => format!("serving.{table}"),
        _ => key.to_owned(),
    }
}

/// A pipeline's `"zone.table"` end as a table key (`bronze.x`, `silver.x`,
/// `serving.x`), or `None` for a zone the graph does not know.
pub(crate) fn zone_key(zone_table: &str) -> Option<String> {
    let (zone, table) = zone_table.split_once('.')?;
    match zone {
        "bronze" | "silver" => Some(format!("{zone}.{table}")),
        "gold" | "serving" => Some(format!("serving.{table}")),
        _ => None,
    }
}

/// The rows to store for `lineage`, every output landing in `to_table`
/// (`lineage` itself carries only column names).
pub(crate) fn rows_for(lineage: &Lineage, to_table: &str) -> Vec<NewMapping> {
    let mut rows: Vec<NewMapping> = lineage
        .mappings
        .iter()
        .map(|m| NewMapping {
            from_table: Some(graph_key(&m.from_table)),
            from_column: Some(m.from_column.clone()),
            to_table: to_table.to_owned(),
            to_column: m.output.clone(),
            transform: m.transform.as_str().to_owned(),
            partial: false,
            reason: None,
        })
        .collect();
    rows.extend(lineage.partial.iter().map(|p| NewMapping {
        from_table: None,
        from_column: None,
        to_table: to_table.to_owned(),
        to_column: p.output.clone(),
        transform: p.transform.as_str().to_owned(),
        partial: true,
        reason: Some(p.reason.clone()),
    }));
    rows
}

/// Column names already read from the engine during one piece of work, so a
/// pass over many statements asks once per table. `None` is remembered too:
/// a table whose columns could not be read is not asked again.
#[derive(Default)]
pub(crate) struct ColumnCache {
    tables: HashMap<String, Option<Vec<String>>>,
}

impl ColumnCache {
    /// The columns of every table `sql` reads, plus `extra`, keyed the way
    /// the helper keys them. A table whose columns cannot be read is left
    /// out, which makes the helper mark what depends on it partial rather
    /// than guess.
    pub(crate) async fn known_for(
        &mut self,
        state: &AppState,
        sql: &str,
        extra: &[String],
    ) -> HashMap<String, Vec<String>> {
        let mut wanted: BTreeSet<String> =
            crate::sql_rewrite::referenced_tables(sql, &ClickHouseDialect {})
                .unwrap_or_default()
                .into_iter()
                .collect();
        wanted.extend(extra.iter().cloned());
        let engine = crate::policy_engine::PolicyEngineObligations::from_state(state);
        let mut known = HashMap::new();
        for table in wanted.into_iter().take(MAX_TABLES_PER_STATEMENT) {
            if !self.tables.contains_key(&table) {
                let columns = match table.split_once('.') {
                    Some((schema, name)) => engine.table_columns(schema, name).await,
                    None => None,
                };
                self.tables.insert(table.clone(), columns);
            }
            if let Some(Some(columns)) = self.tables.get(&table) {
                known.insert(table, columns.clone());
            }
        }
        known
    }
}

/// The table key a dashboard SQL source is written under.
pub(crate) fn sql_source_key(id: &str) -> String {
    format!("sql_source:{id}")
}

/// Why a probed column has no traced source when the statement's own
/// outputs do not account for it.
const MSG_UNACCOUNTED: &str = "the column's source could not be determined from the SQL";

/// The rows for a SQL source whose output columns are known from a probe:
/// the helper's rows that name a probed column, and one partial row for each
/// probed column the statement did not account for (so every column of the
/// source is either traced or marked as not). A partial output that names no
/// probed column (`*` over unknown columns, an unaliased expression the
/// engine named differently) lends its reason to the unaccounted columns.
pub(crate) fn rows_for_probed(
    lineage: &Lineage,
    probed: &[String],
    to_table: &str,
) -> Vec<NewMapping> {
    let all = rows_for(lineage, to_table);
    let mut dropped_reason: Option<String> = None;
    let mut rows = Vec::new();
    for row in all {
        if probed.contains(&row.to_column) {
            rows.push(row);
        } else if dropped_reason.is_none() {
            dropped_reason.clone_from(&row.reason);
        }
    }
    for name in probed {
        let accounted =
            rows.iter().any(|r| &r.to_column == name) || lineage.constants.contains(name);
        if !accounted {
            rows.push(NewMapping {
                from_table: None,
                from_column: None,
                to_table: to_table.to_owned(),
                to_column: name.clone(),
                transform: "unknown".to_owned(),
                partial: true,
                reason: Some(
                    dropped_reason
                        .clone()
                        .unwrap_or_else(|| MSG_UNACCOUNTED.to_owned()),
                ),
            });
        }
    }
    rows
}

/// A dashboard SQL source was saved: map its SQL against the output columns
/// the save just probed and replace the source's rows. A statement that
/// cannot be read marks every probed column partial with the reason.
pub(crate) async fn record_sql_source(state: &AppState, source: &lakehouse_bi::sources::SqlSource) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    let probed: Vec<String> = source.columns.iter().map(|c| c.name.clone()).collect();
    let to_table = sql_source_key(&source.id);
    let known = ColumnCache::default()
        .known_for(state, &source.sql, &[])
        .await;
    let rows = match sql_lineage::map_select(&source.sql, &ClickHouseDialect {}, &known) {
        Ok(lineage) => rows_for_probed(&lineage, &probed, &to_table),
        Err(unmappable) => probed
            .iter()
            .map(|name| NewMapping {
                from_table: None,
                from_column: None,
                to_table: to_table.clone(),
                to_column: name.clone(),
                transform: "unknown".to_owned(),
                partial: true,
                reason: Some(unmappable.to_string()),
            })
            .collect(),
    };
    if let Err(err) =
        column_lineage::replace_producer(pool, "sql_source", &source.id, None, &rows).await
    {
        tracing::warn!(%err, source = %source.id, "column lineage: a SQL source's mappings could not be written");
    }
}

/// A dashboard SQL source was deleted: its mappings go with it.
pub(crate) async fn forget_sql_source(state: &AppState, id: &str) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    if let Err(err) = column_lineage::delete_producer(pool, "sql_source", id).await {
        tracing::warn!(%err, source = id, "column lineage: a SQL source's mappings could not be removed");
    }
}

// PR #106 review R1 SHOULD-FIX DATA-13 4: forget a deleted pipeline's column mappings and pending SQL
pub(crate) async fn forget_pipeline(state: &AppState, id: &str) {
    let Some(pool) = state.pg.as_deref() else {
        return;
    };
    if let Err(err) = column_lineage::delete_producer(pool, "pipeline", id).await {
        tracing::warn!(%err, pipeline = id, "column lineage: a pipeline's mappings could not be removed");
    }
    if let Err(err) = column_lineage::clear_pending(pool, id).await {
        tracing::warn!(%err, pipeline = id, "column lineage: a pipeline's pending SQL could not be cleared");
    }
}

// ── Reading ────────────────────────────────────────────────────────────

/// Longest table key or column name accepted in a query.
const MAX_NAME_CHARS: usize = 300;

/// `GET /api/governance/lineage/columns` query. Every field is checked by
/// [`read`] (a missing one is a 400 with a sentence, not axum's own text).
#[derive(Debug, Deserialize)]
pub(crate) struct ColumnQuery {
    #[serde(default)]
    table: String,
    #[serde(default)]
    column: String,
    #[serde(default)]
    direction: String,
}

/// The table key a caller's `table` stands for: lowercased and with the Gold
/// zone spelled `serving`, as the rows are keyed; a `sql_source:<id>` key is
/// kept as written.
fn normalize_table(raw: &str) -> Result<String, ApiError> {
    let table = raw.trim();
    if table.is_empty() || table.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest(
            "table is required, for example serving.mart_x".to_owned(),
        ));
    }
    if table.starts_with("sql_source:") {
        return Ok(table.to_owned());
    }
    Ok(graph_key(&table.to_ascii_lowercase()))
}

fn parse_direction(raw: &str) -> Result<Direction, ApiError> {
    match raw.trim() {
        "" | "up" => Ok(Direction::Up),
        "down" => Ok(Direction::Down),
        _ => Err(ApiError::BadRequest(
            "direction must be up or down".to_owned(),
        )),
    }
}

fn iso(at: OffsetDateTime) -> Value {
    at.format(&Rfc3339).map_or(Value::Null, Value::String)
}

fn mapping_json(row: &MappingRow) -> Value {
    json!({
        "from": { "table": row.from_table, "column": row.from_column },
        "to": { "table": row.to_table, "column": row.to_column },
        "transform": row.transform,
        "partial": row.partial,
        "reason": row.reason,
        "producer": { "kind": row.producer_kind, "ref": row.producer_ref },
        "recordedAt": iso(row.recorded_at),
        "hop": row.hop,
    })
}

/// Every column a chart definition names: the dimension, the measures, the
/// breakdown, a map's latitude and longitude, and the fields of raw tables
/// and pivots (`columns`, `rows`, pivot `values`, the saved sort) and a KPI's
/// comparison date. Columns that are calculated fields are named here too and
/// simply match nothing, because no mapping starts from them.
fn named_columns(input: &lakehouse_bi::store::ChartInput) -> Vec<&str> {
    let tables = &input.tables;
    let mut names: Vec<&str> = vec![input.dimension.as_str()];
    names.extend(input.measures.iter().map(String::as_str));
    names.extend(input.breakdown.as_deref());
    names.extend(input.lat.as_deref());
    names.extend(input.lon.as_deref());
    names.extend(tables.columns.iter().flatten().map(String::as_str));
    names.extend(tables.rows.iter().flatten().map(String::as_str));
    names.extend(tables.values.iter().flatten().map(|v| v.column.as_str()));
    names.extend(tables.sort_column.as_deref());
    names.extend(
        tables
            .compare
            .as_ref()
            .and_then(|c| c.date_column.as_deref()),
    );
    names.retain(|n| !n.is_empty());
    names
}

/// The table key a chart reads: a mart is `serving.<mart>`, a SQL source is
/// `sql_source:<id>`; a chart with neither (text) reads nothing.
fn chart_relation(input: &lakehouse_bi::store::ChartInput) -> Option<String> {
    match input.sql_source.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => Some(sql_source_key(id)),
        None if input.mart.is_empty() => None,
        None => Some(format!("serving.{}", input.mart.to_ascii_lowercase())),
    }
}

/// The saved charts that read a column in `reached` (`(table, column)`
/// pairs, the starting column included), each as the JSON the answer carries.
/// A chart is listed once, however many of its columns are reached.
fn charts_reaching(
    charts: &[lakehouse_bi::store::StoredChartSpec],
    reached: &HashSet<(String, String)>,
    board_titles: &HashMap<String, String>,
) -> Vec<Value> {
    charts
        .iter()
        .filter(|chart| {
            chart_relation(&chart.def).is_some_and(|relation| {
                named_columns(&chart.def)
                    .iter()
                    .any(|name| reached.contains(&(relation.clone(), (*name).to_owned())))
            })
        })
        .map(|chart| {
            json!({
                "chartId": chart.spec.id,
                "chartTitle": chart.spec.title,
                "dashboardId": chart.board,
                "dashboardTitle": board_titles
                    .get(&chart.board)
                    .cloned()
                    .unwrap_or_else(|| chart.board.clone()),
            })
        })
        .collect()
}

/// The charts and dashboards that depend on the column, for a `down` answer.
/// `Err(())` when they could not be read; the caller says so rather than
/// answering an empty list.
///
/// The charts and dashboards that read a reached column, as the caller may
/// see them: the personal space (`BI-18` part C) is closed to everybody but
/// its owner and an administrator, so the read takes the caller's
/// [`lakehouse_bi::access::Viewer`] and never the whole list.
async fn dependent_charts(
    state: &AppState,
    reached: &HashSet<(String, String)>,
    viewer: &lakehouse_bi::access::Viewer,
) -> Result<Vec<Value>, ()> {
    let ch = &state.clickhouse;
    let charts = lakehouse_bi::store::list_stored_charts(ch, viewer)
        .await
        .map_err(|err| tracing::warn!(?err, "column lineage: dashboard charts unavailable"))?;
    let titles: HashMap<String, String> = lakehouse_bi::store::list_boards(ch, viewer)
        .await
        .map_err(|err| tracing::warn!(?err, "column lineage: dashboard names unavailable"))?
        .into_iter()
        .map(|b| (b.id, b.name))
        .collect();
    Ok(charts_reaching(&charts, reached, &titles))
}

/// The table key a graph node id stands for: `table:silver.x` and
/// `table:serving.x` are warehouse tables, `bronze:x` is `bronze.x`; every
/// other node (a dataset, a connector, an Iceberg copy) has no columns here.
pub(super) fn node_table_key(node_id: &str) -> Option<String> {
    if let Some(table) = node_id.strip_prefix("table:") {
        return Some(graph_key(&table.to_ascii_lowercase()));
    }
    node_id
        .strip_prefix("bronze:")
        .map(|t| format!("bronze.{}", t.to_ascii_lowercase()))
}

/// The rows `principal` may read: the one rule the graph's `columnMappings`,
/// the chart route and the used-by list all apply (`BI-14`).
pub(super) fn scope_for(principal: &Principal, tenant: Option<uuid::Uuid>, shared: bool) -> Scope {
    Scope {
        tenant,
        all_tenants: tenant.is_none() && crate::routes::catalog::is_unrestricted(principal),
        shared,
        dashboards: principal.has("dashboard:read"),
    }
}

/// The recorded mappings that touch any of the focus nodes, as the table
/// graph's `columnMappings`. A store failure is logged and answers an empty
/// list: the graph itself is still an honest answer, and the dedicated
/// columns route reports the failure properly.
pub(crate) async fn mappings_touching(
    state: &AppState,
    principal: &Principal,
    tenant: Option<uuid::Uuid>,
    shared: bool,
    focus_nodes: &[String],
) -> Value {
    let Some(pool) = state.pg.as_deref() else {
        return json!([]);
    };
    let scope = scope_for(principal, tenant, shared);
    let keys: BTreeSet<String> = focus_nodes
        .iter()
        .filter_map(|n| node_table_key(n))
        .collect();
    let mut out: Vec<Value> = Vec::new();
    for key in &keys {
        match column_lineage::touching_table(pool, key, scope).await {
            Ok(rows) => out.extend(rows.iter().map(mapping_json)),
            Err(err) => {
                tracing::warn!(%err, "column lineage: the focus table's mappings could not be read");
            }
        }
    }
    Value::Array(out)
}

/// What the answer to a column query carries. See the module doc and the
/// plan: the rows reached by following mappings up to ten hops, the
/// retention line, and (for `down`) the dependent charts when the caller may
/// list dashboards.
///
/// # Errors
///
/// [`ApiError::BadRequest`] for a missing or malformed parameter,
/// [`ApiError::NotFound`] when `X-Tenant` names a tenant the caller does not
/// belong to, [`ApiError::Unavailable`] without a Postgres pool or when the
/// shared-catalog rule cannot be evaluated, and the classified store error
/// otherwise.
pub(crate) async fn read(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
    query: &ColumnQuery,
) -> Result<Value, ApiError> {
    let table = normalize_table(&query.table)?;
    let column = query.column.trim();
    if column.is_empty() || column.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest("column is required".to_owned()));
    }
    let direction = parse_direction(&query.direction)?;
    let pool = state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "column lineage store unavailable: no Postgres pool is configured".to_owned(),
        )
    })?;

    // The same tenant rules as the table-level route (`build`): shared
    // records (builds, the catalog) only where the shared catalog may be
    // read; a tenant's own records for that tenant; every tenant's only for
    // an unrestricted caller who belongs to none.
    let tenant = crate::tenant_scope::resolve(principal, headers)?;
    let refusal = crate::routes::catalog::catalog_tenant_refusal(state, principal, headers).await?;
    let unrestricted = crate::routes::catalog::is_unrestricted(principal);
    let scope = Scope {
        tenant,
        all_tenants: tenant.is_none() && unrestricted,
        shared: refusal.is_none(),
        dashboards: principal.has("dashboard:read"),
    };
    let mut coverage: Vec<String> = Vec::new();
    if let Some(reason) = refusal {
        coverage.push(format!(
            "lineage of the shared catalog's tables is not shown: {reason}"
        ));
    }
    if tenant.is_none() && !unrestricted {
        coverage.push("lineage of pipelines is not shown: you belong to no tenant".to_owned());
    }

    let rows = column_lineage::walk(pool, &table, column, direction, scope).await?;
    let oldest = column_lineage::oldest_recorded_at(pool, scope).await?;
    let mut body = json!({
        "supported": true,
        "table": table,
        "column": column,
        "direction": if direction == Direction::Up { "up" } else { "down" },
        "mappings": rows.iter().map(mapping_json).collect::<Vec<_>>(),
        "retention": {
            "days": column_lineage::RETENTION_DAYS,
            "oldestRecordedAt": oldest.map_or(Value::Null, iso),
        },
        "coverage": coverage,
    });
    if direction == Direction::Down {
        if principal.has("dashboard:read") {
            let mut reached: HashSet<(String, String)> = rows
                .iter()
                .map(|r| (r.to_table.clone(), r.to_column.clone()))
                .collect();
            // A chart may read the starting column itself.
            reached.insert((table, column.to_owned()));
            // BI-19 part B1: through the caller's rules; if they cannot be
            // read the charts are reported unavailable, never unfiltered.
            match crate::routes::dashboard_access::viewer_for(state, principal).await {
                Ok(viewer) => match dependent_charts(state, &reached, &viewer).await {
                    Ok(charts) => body["charts"] = Value::Array(charts),
                    Err(()) => body["chartsUnavailable"] = Value::Bool(true),
                },
                Err(_unreadable) => body["chartsUnavailable"] = Value::Bool(true),
            }
        } else {
            body["chartsHidden"] = Value::Bool(true);
        }
    }
    Ok(body)
}

/// Remember the SQL the API approved for `pipeline_id`'s run, to be mapped
/// when the run is reported successful. Logged, never returned: the
/// orchestrator's enforce call must not fail because lineage could not be
/// noted.
pub(crate) async fn note_pipeline_sql(pool: &PgPool, pipeline_id: &str, sql: &str) {
    if let Err(err) = column_lineage::put_pending(pool, pipeline_id, sql).await {
        tracing::warn!(%err, pipeline = pipeline_id, "column lineage: the approved SQL could not be kept");
    }
}

/// A pipeline run was reported successful: map the SQL approved for it,
/// replace the pipeline's rows with the result and forget the pending SQL.
///
/// Nothing pending (a run that never reached the enforce call, or an event
/// delivered twice) is a no-op. A failed run never reaches this function: the
/// success event is refused first (`routes::pipelines::run_finished_event`).
// PR #106 review R1 SHOULD-FIX DATA-13 1, SHOULD-FIX DATA-13 2
fn unmappable_rows(
    target: &str,
    reason: &str,
    known: &HashMap<String, Vec<String>>,
) -> Vec<NewMapping> {
    if let Some(probed) = known.get(target).filter(|p| !p.is_empty()) {
        probed
            .iter()
            .map(|col| NewMapping {
                from_table: None,
                from_column: None,
                to_table: target.to_owned(),
                to_column: col.clone(),
                transform: "unknown".to_owned(),
                partial: true,
                reason: Some(reason.to_owned()),
            })
            .collect()
    } else {
        vec![NewMapping {
            from_table: None,
            from_column: None,
            to_table: target.to_owned(),
            to_column: "*".to_owned(),
            transform: "unknown".to_owned(),
            partial: true,
            reason: Some(reason.to_owned()),
        }]
    }
}

// PR #106 review R2 SHOULD-FIX 5: expand every '*' row, not just the first
fn expand_star_rows(
    rows: Vec<NewMapping>,
    probed: Option<&Vec<String>>,
    target: &str,
) -> Vec<NewMapping> {
    let (star_rows, mut non_star_rows): (Vec<_>, Vec<_>) =
        rows.into_iter().partition(|r| r.to_column == "*");
    if star_rows.is_empty() {
        return non_star_rows;
    }
    if let Some(columns) = probed.filter(|c| !c.is_empty()) {
        for star in star_rows {
            for col in columns {
                if !non_star_rows.iter().any(|r| {
                    r.to_column == *col
                        && r.from_table == star.from_table
                        && r.from_column == star.from_column
                        && r.reason == star.reason
                }) {
                    non_star_rows.push(NewMapping {
                        from_table: star.from_table.clone(),
                        from_column: star.from_column.clone(),
                        to_table: target.to_owned(),
                        to_column: col.clone(),
                        transform: star.transform.clone(),
                        partial: star.partial,
                        reason: star.reason.clone(),
                    });
                }
            }
        }
    } else {
        non_star_rows.extend(star_rows);
    }
    non_star_rows
}

// PR #106 review R1 SHOULD-FIX DATA-13 1, SHOULD-FIX DATA-13 2
fn map_statement_or_unmappable(
    sql: &str,
    target: &str,
    known: &HashMap<String, Vec<String>>,
) -> Vec<NewMapping> {
    if sql.len() > MAX_BUILD_QUERY_CHARS {
        unmappable_rows(target, "the statement exceeds 20,000 characters", known)
    } else {
        match sql_lineage::map_select(sql, &ClickHouseDialect {}, known) {
            Ok(lineage) => {
                let rows = rows_for(&lineage, target);
                expand_star_rows(rows, known.get(target), target)
            }
            Err(unmappable) => unmappable_rows(target, &unmappable.to_string(), known),
        }
    }
}

pub(crate) async fn record_pipeline_run(state: &AppState, pool: &PgPool, pipeline_id: &str) {
    let sql = match column_lineage::get_pending(pool, pipeline_id).await {
        Ok(Some(sql)) => sql,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(%err, pipeline = pipeline_id, "column lineage: the pending SQL could not be read");
            return;
        }
    };
    let pipeline = match lakehouse_store::pipelines::get_pipeline(pool, pipeline_id).await {
        Ok(Some(pipeline)) => pipeline,
        Ok(None) => return,
        Err(err) => {
            tracing::warn!(%err, pipeline = pipeline_id, "column lineage: the pipeline could not be read");
            return;
        }
    };
    let Some(target) = zone_key(&pipeline.target) else {
        tracing::warn!(
            pipeline = pipeline_id,
            "column lineage: the pipeline's target zone is not one the graph knows"
        );
        return;
    };
    let mut cache = ColumnCache::default();
    let known = cache
        .known_for(state, &sql, std::slice::from_ref(&target))
        .await;
    // PR #106 review R1 SHOULD-FIX DATA-13 2: statement over 20,000 chars or unmappable leaves partial rows with reason
    let rows = map_statement_or_unmappable(&sql, &target, &known);
    if let Err(err) =
        column_lineage::replace_producer(pool, "pipeline", pipeline_id, pipeline.tenant_id, &rows)
            .await
    {
        tracing::warn!(%err, pipeline = pipeline_id, "column lineage: the pipeline's mappings could not be written");
        return;
    }
    if let Err(err) = column_lineage::clear_pending(pool, pipeline_id).await {
        tracing::warn!(%err, pipeline = pipeline_id, "column lineage: the pending SQL could not be cleared");
    }
}

/// Statements read from the query log per pass. A deployment with more waits
/// for the next tick; the cursor moves only past what was handled.
const BUILD_BATCH: usize = 200;
/// A statement longer than this is not read (the log keeps the whole text).
const MAX_BUILD_QUERY_CHARS: usize = 20_000;
/// How far back the first pass looks: `ClickHouse`'s own query-log horizon,
/// the same one the table-level build edges use (`builds::BUILDS_SQL`).
const FIRST_PASS_DAYS: i64 = 180;

static BUILDS_RUNNING: AtomicBool = AtomicBool::new(false);

/// The builds written to `silver`/`serving` after `after_ms` (Unix
/// milliseconds), oldest first. `after_ms` is an `i64` this module computed,
/// never text from a request. Like `builds::BUILDS_SQL` it keeps statements
/// that read at least one other table (`length(tables) > 1`) and only the
/// initiating query of a distributed one.
fn builds_sql(after_ms: i64) -> String {
    format!(
        "SELECT toString(toUnixTimestamp64Milli(event_time_microseconds)) AS ts, query \
         FROM system.query_log \
         WHERE type = 'QueryFinish' AND is_initial_query = 1 \
           AND query_kind IN ('Create', 'Insert') AND length(tables) > 1 \
           AND hasAny(databases, ['silver', 'serving']) \
           AND event_date >= toDate(fromUnixTimestamp64Milli({after_ms})) \
           AND event_time_microseconds > fromUnixTimestamp64Milli({after_ms}) \
         ORDER BY event_time_microseconds LIMIT {BUILD_BATCH}"
    )
}

fn unix_ms(at: OffsetDateTime) -> i64 {
    i64::try_from(at.unix_timestamp_nanos() / 1_000_000).unwrap_or(0)
}

fn from_unix_ms(ms: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000).ok()
}

/// The warehouse table a build writes, as a table key; `None` for anything
/// that is not a Silver or Gold table, and for a pipeline's staging table
/// (`__staging_<target>`, swapped into place by `EXCHANGE TABLES`).
fn build_target(target: &str) -> Option<String> {
    let key = graph_key(target);
    let (zone, table) = key.split_once('.')?;
    ((zone == "silver" || zone == "serving") && !table.starts_with("__staging_")).then_some(key)
}

/// Map the builds the query log recorded since the last pass and write one
/// producer per target table (`build`, ref = the table, shared: the log is
/// one per deployment). Returns how many targets were written.
///
/// * A pipeline's own target is skipped: the pipeline's success event writes
///   a better record, and its statements in the log are the rewritten ones.
/// * Of several statements for one target in a batch, the newest wins, as in
///   the table-level edge.
/// * A statement that cannot be mapped clears its target's rows (the table
///   was last written by something that cannot be traced).
/// * The cursor moves only to the newest time handled, so a pass that failed
///   reading the log changes nothing and the next tick retries.
fn is_expr_zero(expr: &sqlparser::ast::Expr) -> bool {
    match expr {
        sqlparser::ast::Expr::Value(v) => match &v.value {
            sqlparser::ast::Value::Number(n, _) => n == "0",
            _ => false,
        },
        sqlparser::ast::Expr::Nested(inner) => is_expr_zero(inner),
        _ => false,
    }
}

struct LimitZeroVisitor(bool);

impl sqlparser::ast::Visitor for LimitZeroVisitor {
    type Break = ();

    fn pre_visit_query(&mut self, query: &sqlparser::ast::Query) -> std::ops::ControlFlow<()> {
        if let Some(clause) = &query.limit_clause {
            let is_zero = match clause {
                sqlparser::ast::LimitClause::LimitOffset { limit, .. } => {
                    limit.as_ref().is_some_and(is_expr_zero)
                }
                sqlparser::ast::LimitClause::OffsetCommaLimit { limit, .. } => is_expr_zero(limit),
            };
            if is_zero {
                self.0 = true;
                return std::ops::ControlFlow::Break(());
            }
        }
        std::ops::ControlFlow::Continue(())
    }
}

// PR #106 review R2 SHOULD-FIX 4: match the AST's LIMIT clause, not raw tokens
fn is_limit_zero(sql: &str) -> bool {
    let Ok(statements) = sqlparser::parser::Parser::parse_sql(&ClickHouseDialect {}, sql) else {
        return false;
    };
    let mut visitor = LimitZeroVisitor(false);
    let _ = sqlparser::ast::Visit::visit(&statements, &mut visitor);
    visitor.0
}

pub(crate) async fn run_builds_pass(state: &AppState, pool: &PgPool) -> usize {
    let cursor = match column_lineage::builds_cursor(pool).await {
        Ok(cursor) => cursor,
        Err(err) => {
            tracing::warn!(%err, "column lineage: the build cursor could not be read");
            return 0;
        }
    };
    let after =
        cursor.unwrap_or_else(|| OffsetDateTime::now_utc() - time::Duration::days(FIRST_PASS_DAYS));
    let rows = match state
        .clickhouse
        .rows(&builds_sql(unix_ms(after)), None)
        .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(?err, "column lineage: the query log could not be read");
            return 0;
        }
    };
    // PR #106 review R1 SHOULD-FIX DATA-13 3: fail pass when list_pipelines fails instead of unwrap_or_default
    let pipeline_targets: HashSet<String> = match lakehouse_store::pipelines::list_pipelines(
        pool,
        &lakehouse_store::pipelines::PipelineFilter {
            tenant_id: None,
            all_tenants: true,
        },
    )
    .await
    {
        Ok(list) => list
            .into_iter()
            .filter_map(|p| zone_key(&p.target))
            .collect(),
        Err(err) => {
            tracing::warn!(%err, "column lineage: could not list pipelines for build targets");
            return 0;
        }
    };

    let mut cache = ColumnCache::default();
    let mut latest: HashMap<String, Vec<NewMapping>> = HashMap::new();
    let mut newest_ms: Option<i64> = None;
    for row in &rows {
        let ts = super::text(row, "ts").parse::<i64>().ok();
        newest_ms = newest_ms.max(ts);
        let query = super::text(row, "query");
        // The target is known from the statement's own text; the columns of
        // the tables it reads come second, and a positional INSERT needs the
        // target's own.
        let Some(target) = sql_lineage::statement_target(&query, &ClickHouseDialect {})
            .as_deref()
            .and_then(build_target)
        else {
            continue;
        };
        if pipeline_targets.contains(&target) {
            continue;
        }
        // PR #106 review R1 SHOULD-FIX DATA-13 3: skip Dagster's CREATE ... LIMIT 0 statements
        if is_limit_zero(&query) {
            continue;
        }
        let known = cache
            .known_for(state, &query, std::slice::from_ref(&target))
            .await;
        // PR #106 review R1 SHOULD-FIX DATA-13 2: statement over 20,000 chars or unmappable leaves partial rows with reason
        let rows = map_statement_or_unmappable(&query, &target, &known);
        latest.insert(target, rows);
    }
    let mut written = 0;
    for (target, rows) in &latest {
        match column_lineage::replace_producer(pool, "build", target, None, rows).await {
            Ok(()) => written += 1,
            Err(err) => {
                tracing::warn!(%err, table = %target, "column lineage: a build's mappings could not be written");
            }
        }
    }
    if let Some(at) = newest_ms.and_then(from_unix_ms)
        && let Err(err) = column_lineage::set_builds_cursor(pool, at).await
    {
        tracing::warn!(%err, "column lineage: the build cursor could not be moved");
    }
    written
}

/// Starts [`run_builds_pass`] in the background and returns at once (the
/// alerts tick must not wait on the query log). `false`, starting nothing,
/// while another pass runs or when there is no Postgres pool.
pub(crate) fn spawn_builds_pass(state: &AppState) -> bool {
    let Some(running) = crate::routes::schema_versions::PassGuard::claim(&BUILDS_RUNNING) else {
        return false;
    };
    let state = state.clone();
    tokio::spawn(async move {
        let _running = running;
        let Some(pool) = state.pg.as_deref() else {
            return;
        };
        let written = run_builds_pass(&state, pool).await;
        tracing::info!(written, "column lineage: build pass finished");
        // DATA-13 retention (F8): rows older than a year go on the same
        // tick, so the answer's "kept since" line is true.
        match column_lineage::purge_expired(pool).await {
            Ok(0) => {}
            Ok(removed) => tracing::info!(removed, "column lineage: expired rows removed"),
            Err(err) => tracing::warn!(%err, "column lineage: expired rows could not be removed"),
        }
    });
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;
    use crate::sql_lineage::{Mapping, PartialOutput, Transform};

    #[test]
    fn the_gold_zone_is_the_serving_database() {
        assert_eq!(graph_key("gold.mart"), "serving.mart");
        assert_eq!(graph_key("serving.mart"), "serving.mart");
        assert_eq!(graph_key("bronze.orders"), "bronze.orders");
        assert_eq!(zone_key("gold.mart").as_deref(), Some("serving.mart"));
        assert_eq!(zone_key("silver.x").as_deref(), Some("silver.x"));
        assert_eq!(zone_key("bronze.x").as_deref(), Some("bronze.x"));
        assert_eq!(zone_key("lake.x"), None);
        assert_eq!(zone_key("nodot"), None);
    }

    #[test]
    fn mappings_and_partials_become_complete_and_partial_rows() {
        let lineage = Lineage {
            target: None,
            mappings: vec![Mapping {
                output: "amt".to_owned(),
                from_table: "gold.orders".to_owned(),
                from_column: "Amount".to_owned(),
                transform: Transform::Cast,
            }],
            partial: vec![PartialOutput {
                output: "n".to_owned(),
                reason: "built with UNION".to_owned(),
                transform: Transform::Copy,
            }],
            constants: vec!["one".to_owned()],
        };
        let rows = rows_for(&lineage, "silver.out");
        assert_eq!(rows.len(), 2, "a constant is not a row");
        assert_eq!(rows[0].from_table.as_deref(), Some("serving.orders"));
        assert_eq!(rows[0].from_column.as_deref(), Some("Amount"));
        assert_eq!(rows[0].to_table, "silver.out");
        assert_eq!(rows[0].transform, "cast");
        assert!(!rows[0].partial);
        assert_eq!(rows[1].from_table, None);
        assert_eq!(rows[1].from_column, None);
        assert!(rows[1].partial);
        assert_eq!(rows[1].reason.as_deref(), Some("built with UNION"));
    }

    #[test]
    fn every_probed_column_of_a_sql_source_is_traced_or_marked_partial() {
        let lineage = Lineage {
            target: None,
            mappings: vec![Mapping {
                output: "place".to_owned(),
                from_table: "serving.mart_x".to_owned(),
                from_column: "place".to_owned(),
                transform: Transform::Copy,
            }],
            partial: vec![PartialOutput {
                output: "*".to_owned(),
                reason: "the columns of serving.other are not known".to_owned(),
                transform: Transform::Copy,
            }],
            constants: vec!["one".to_owned()],
        };
        let probed = ["place", "visitors", "one"].map(str::to_owned);
        let rows = rows_for_probed(&lineage, &probed, "sql_source:s1");
        assert_eq!(
            rows.len(),
            2,
            "place is traced, visitors is partial, one is a constant"
        );
        assert_eq!(rows[0].to_column, "place");
        assert!(!rows[0].partial);
        assert_eq!(rows[1].to_column, "visitors");
        assert!(rows[1].partial);
        assert_eq!(
            rows[1].reason.as_deref(),
            Some("the columns of serving.other are not known"),
            "the star's reason explains the column it stood for"
        );
        assert!(rows.iter().all(|r| r.to_table == "sql_source:s1"));
    }

    #[test]
    fn a_probed_column_nothing_accounts_for_says_so() {
        let rows = rows_for_probed(&Lineage::default(), &["a".to_owned()], "sql_source:s1");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].reason.as_deref(), Some(MSG_UNACCOUNTED));
    }

    mod builds_pass {
        use std::collections::HashMap;

        use lakehouse_store::column_lineage::{Direction, Scope, touching_table, walk};
        use serde_json::json;
        use wiremock::matchers::{body_string_contains, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;
        use crate::config::Config;

        fn state_for(pool: &sqlx::PgPool, ch_url: &str) -> AppState {
            let options = pool.connect_options();
            let database_url = format!(
                "postgres://{}:postgres@{}:{}/{}",
                options.get_username(),
                options.get_host(),
                options.get_port(),
                options
                    .get_database()
                    .expect("#[sqlx::test] always targets a named database"),
            );
            let mut env = HashMap::new();
            env.insert("DATABASE_URL".to_owned(), database_url);
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        fn everything() -> Scope {
            Scope {
                tenant: None,
                all_tenants: true,
                shared: true,
                dashboards: true,
            }
        }

        /// `system.columns` answers the same two columns for every table:
        /// enough for a positional INSERT and for `*`.
        async fn mount_columns(ch: &MockServer) {
            Mock::given(method("POST"))
                .and(body_string_contains("system.columns"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "name", "type": "String"},
                        {"name": "default_kind", "type": "String"},
                        {"name": "default_expression", "type": "String"}
                    ],
                    "data": [
                        {"name": "id", "default_kind": "", "default_expression": ""},
                        {"name": "name", "default_kind": "", "default_expression": ""},
                    ],
                    "rows": 2,
                })))
                .mount(ch)
                .await;
        }

        async fn mount_log(ch: &MockServer, rows: &[(&str, &str)]) {
            let data: Vec<_> = rows
                .iter()
                .map(|(ts, query)| json!({ "ts": ts, "query": query }))
                .collect();
            Mock::given(method("POST"))
                .and(body_string_contains("system.query_log"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [{"name": "ts", "type": "String"}, {"name": "query", "type": "String"}],
                    "data": data,
                    "rows": rows.len(),
                })))
                .mount(ch)
                .await;
        }

        async fn seed_pipeline_into_silver_orders_clean(pool: &sqlx::PgPool) {
            lakehouse_store::pipelines::create_pipeline(
                pool,
                &lakehouse_store::pipelines::CreatePipelineInput {
                    name: "builds-probe".to_owned(),
                    kind: "batch".to_owned(),
                    source_zone: "silver".to_owned(),
                    source_table: "users".to_owned(),
                    target_zone: "silver".to_owned(),
                    target_table: "orders_clean".to_owned(),
                    schedule: "manual".to_owned(),
                    description: None,
                    incremental_column: None,
                    transforms: Vec::new(),
                    fbic_enabled: false,
                    max_retries: None,
                    owner: None,
                    tenant_id: None,
                    depends_on: Vec::new(),
                    connector_id: None,
                    id: None,
                    run_as: None,
                    target_lag: None,
                },
                None,
            )
            .await
            .expect("seed the pipeline");
        }

        // PR #106 review R1 SHOULD-FIX DATA-13 1
        #[sqlx::test(migrations = "../../migrations")]
        async fn a_build_with_star_over_unknown_columns_stores_partial_for_probed_columns(
            pool: sqlx::PgPool,
        ) {
            let ch = MockServer::start().await;
            // mount_columns answers columns for serving.mart, but unmounted_table is not mounted
            Mock::given(method("POST"))
                .and(body_string_contains("mart"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [
                        {"name": "name", "type": "String"},
                        {"name": "default_kind", "type": "String"},
                        {"name": "default_expression", "type": "String"}
                    ],
                    "data": [
                        {"name": "id", "default_kind": "", "default_expression": ""},
                        {"name": "name", "default_kind": "", "default_expression": ""},
                    ],
                    "rows": 2,
                })))
                .mount(&ch)
                .await;
            Mock::given(method("POST"))
                .and(body_string_contains("unmounted_table"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "meta": [], "data": [], "rows": 0
                })))
                .mount(&ch)
                .await;
            mount_log(
                &ch,
                &[(
                    "1760000000000",
                    "CREATE TABLE serving.mart AS SELECT * FROM silver.unmounted_table",
                )],
            )
            .await;
            let state = state_for(&pool, &ch.uri());
            let written = run_builds_pass(&state, &pool).await;
            assert_eq!(written, 1);
            let mart = touching_table(&pool, "serving.mart", everything())
                .await
                .expect("read");
            assert!(
                !mart.iter().any(|r| r.to_column == "*"),
                "to_column = '*' must not be stored"
            );
            let cols: Vec<String> = mart.iter().map(|r| r.to_column.clone()).collect();
            assert_eq!(cols, vec!["id", "name"]);
            assert!(mart.iter().all(|r| r.partial));
        }

        // PR #106 review R1 SHOULD-FIX DATA-13 2
        #[sqlx::test(migrations = "../../migrations")]
        async fn an_unmappable_or_overlong_build_stores_partial_rows_with_reason(
            pool: sqlx::PgPool,
        ) {
            let ch = MockServer::start().await;
            mount_columns(&ch).await;
            let long_query = format!(
                "INSERT INTO serving.mart SELECT id, name FROM silver.users WHERE '{}' = 'x'",
                "a".repeat(20_001)
            );
            mount_log(
                &ch,
                &[
                    (
                        "1760000000000",
                        "INSERT INTO serving.mart SELECT 1 FROM (SELECT count() FROM silver.users UNION ALL SELECT 2)",
                    ),
                    ("1760000001000", &long_query),
                ],
            )
            .await;
            let state = state_for(&pool, &ch.uri());
            let written = run_builds_pass(&state, &pool).await;
            assert_eq!(written, 1);
            let mart = touching_table(&pool, "serving.mart", everything())
                .await
                .expect("read");
            assert_eq!(mart.len(), 2);
            assert!(mart.iter().all(|r| r.partial));
            assert!(
                mart.iter()
                    .all(|r| r.reason.as_deref() == Some("the statement exceeds 20,000 characters"))
            );
        }

        // PR #106 review R2 SHOULD-FIX 5
        #[test]
        fn expand_star_rows_handles_every_star_row() {
            let star1 = NewMapping {
                from_table: Some("silver.t1".to_owned()),
                from_column: None,
                to_table: "silver.target".to_owned(),
                to_column: "*".to_owned(),
                transform: "unknown".to_owned(),
                partial: true,
                reason: Some("unknown t1".to_owned()),
            };
            let star2 = NewMapping {
                from_table: Some("silver.t2".to_owned()),
                from_column: None,
                to_table: "silver.target".to_owned(),
                to_column: "*".to_owned(),
                transform: "unknown".to_owned(),
                partial: true,
                reason: Some("unknown t2".to_owned()),
            };

            let probed = vec!["id".to_owned(), "name".to_owned()];
            let expanded = expand_star_rows(
                vec![star1.clone(), star2.clone()],
                Some(&probed),
                "silver.target",
            );
            assert!(!expanded.iter().any(|r| r.to_column == "*"));
            assert_eq!(expanded.len(), 4);

            let unprobed = expand_star_rows(vec![star1, star2], None, "silver.target");
            assert_eq!(unprobed.len(), 2);
            assert_eq!(unprobed.iter().filter(|r| r.to_column == "*").count(), 2);
        }

        // PR #106 review R2 SHOULD-FIX 4
        #[test]
        fn is_limit_zero_matches_ast_limit_and_not_tokens() {
            assert!(is_limit_zero("SELECT * FROM silver.users LIMIT 0"));
            assert!(is_limit_zero(
                "CREATE TABLE silver.dagster ENGINE = MergeTree ORDER BY tuple() AS SELECT * FROM silver.users LIMIT 0"
            ));
            assert!(!is_limit_zero(
                "CREATE TABLE silver.limit_0 AS SELECT id FROM silver.users"
            ));
            assert!(!is_limit_zero("SELECT 'LIMIT 0' AS msg FROM silver.users"));
            assert!(!is_limit_zero("SELECT * FROM silver.users LIMIT 10"));
        }

        // PR #106 review R1 SHOULD-FIX DATA-13 3
        #[sqlx::test(migrations = "../../migrations")]
        async fn dagster_limit_zero_is_skipped_even_when_not_a_pipeline_target(pool: sqlx::PgPool) {
            let ch = MockServer::start().await;
            mount_columns(&ch).await;
            mount_log(
                &ch,
                &[(
                    "1760000000000",
                    "CREATE TABLE IF NOT EXISTS silver.dagster_table ENGINE = MergeTree                      ORDER BY tuple() AS SELECT id FROM silver.users LIMIT 0",
                )],
            )
            .await;
            let state = state_for(&pool, &ch.uri());
            let written = run_builds_pass(&state, &pool).await;
            assert_eq!(
                written, 0,
                "LIMIT 0 must be skipped and never recorded as a build"
            );
            assert!(
                touching_table(&pool, "silver.dagster_table", everything())
                    .await
                    .expect("read")
                    .is_empty()
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn the_newest_build_of_a_target_wins_and_pipeline_and_staging_targets_are_skipped(
            pool: sqlx::PgPool,
        ) {
            let ch = MockServer::start().await;
            mount_columns(&ch).await;
            mount_log(
                &ch,
                &[
                    (
                        "1760000000000",
                        "INSERT INTO serving.mart SELECT id, name FROM silver.users",
                    ),
                    (
                        "1760000001000",
                        "CREATE TABLE IF NOT EXISTS silver.orders_clean ENGINE = MergeTree \
                         ORDER BY tuple() AS SELECT id FROM silver.users LIMIT 0",
                    ),
                    (
                        "1760000002000",
                        "INSERT INTO silver.__staging_x SELECT id FROM silver.users",
                    ),
                    (
                        "1760000003000",
                        "INSERT INTO serving.mart SELECT id, upper(name) AS name FROM silver.users",
                    ),
                    ("1760000004000", "OPTIMIZE TABLE serving.mart"),
                ],
            )
            .await;
            seed_pipeline_into_silver_orders_clean(&pool).await;
            let state = state_for(&pool, &ch.uri());

            let written = run_builds_pass(&state, &pool).await;
            assert_eq!(written, 1, "only serving.mart is a build target here");

            let mart = touching_table(&pool, "serving.mart", everything())
                .await
                .expect("read");
            let pairs: Vec<(String, String, String)> = mart
                .iter()
                .map(|r| {
                    (
                        r.from_column.clone().unwrap_or_default(),
                        r.to_column.clone(),
                        r.transform.clone(),
                    )
                })
                .collect();
            assert_eq!(
                pairs,
                vec![
                    ("id".to_owned(), "id".to_owned(), "copy".to_owned()),
                    (
                        "name".to_owned(),
                        "name".to_owned(),
                        "expression".to_owned()
                    ),
                ]
            );
            assert!(
                mart.iter()
                    .all(|r| r.producer_kind == "build" && r.producer_ref == "serving.mart")
            );
            for skipped in ["silver.orders_clean", "silver.__staging_x"] {
                assert!(
                    touching_table(&pool, skipped, everything())
                        .await
                        .expect("read")
                        .is_empty(),
                    "{skipped} must not be recorded by the build pass"
                );
            }
            let cursor = column_lineage::builds_cursor(&pool).await.expect("cursor");
            assert_eq!(cursor.map(unix_ms), Some(1_760_000_004_000));

            // The mapping also walks from the Bronze/Silver side.
            let down = walk(&pool, "silver.users", "name", Direction::Down, everything())
                .await
                .expect("walk");
            assert_eq!(down.len(), 1);
            assert_eq!(down[0].to_table, "serving.mart");
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn a_pass_that_cannot_read_the_log_writes_nothing_and_keeps_the_cursor(
            pool: sqlx::PgPool,
        ) {
            let ch = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(500))
                .mount(&ch)
                .await;
            let state = state_for(&pool, &ch.uri());
            assert_eq!(run_builds_pass(&state, &pool).await, 0);
            assert_eq!(
                column_lineage::builds_cursor(&pool).await.expect("cursor"),
                None
            );
        }

        #[sqlx::test(migrations = "../../migrations")]
        async fn the_next_pass_asks_only_for_statements_after_the_cursor(pool: sqlx::PgPool) {
            let ch = MockServer::start().await;
            mount_log(&ch, &[]).await;
            let state = state_for(&pool, &ch.uri());
            column_lineage::set_builds_cursor(
                &pool,
                from_unix_ms(1_760_000_004_000).expect("time"),
            )
            .await
            .expect("cursor");

            run_builds_pass(&state, &pool).await;

            let requests = ch.received_requests().await.expect("recorded");
            let body = String::from_utf8_lossy(&requests[0].body).into_owned();
            assert!(
                body.contains("fromUnixTimestamp64Milli(1760000004000)"),
                "{body}"
            );
        }

        #[test]
        fn a_build_target_is_a_warehouse_table_that_is_not_a_staging_copy() {
            assert_eq!(
                build_target("serving.mart").as_deref(),
                Some("serving.mart")
            );
            assert_eq!(build_target("gold.mart").as_deref(), Some("serving.mart"));
            assert_eq!(build_target("silver.x").as_deref(), Some("silver.x"));
            assert_eq!(build_target("silver.__staging_x"), None);
            assert_eq!(build_target("console.alerts"), None);
            assert_eq!(build_target("nodot"), None);
        }
    }

    /// The charts a column depends on, as the caller may see them: another
    /// user's private dashboard is not one of them (`BI-18` part C, part 2).
    mod dependent_charts {
        use std::collections::HashMap;

        use serde_json::json;
        use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

        use super::super::*;
        use crate::config::Config;

        fn state_for(ch_url: &str) -> AppState {
            let mut env = HashMap::new();
            env.insert("CH_URL".to_owned(), ch_url.to_owned());
            // No Postgres: the read below does not need one.
            env.insert(
                "DATABASE_URL".to_owned(),
                "not a connection string".to_owned(),
            );
            AppState::new(Config::from_map(&env).expect("a valid test Config"))
        }

        /// A chart on somebody else's private dashboard. It comes back only
        /// if the read did not restrict it — the statement without the
        /// viewer's clause — which is how the personal space stays out of the
        /// answer.
        struct PrivateChart;

        impl Respond for PrivateChart {
            fn respond(&self, request: &Request) -> ResponseTemplate {
                let sql = String::from_utf8_lossy(&request.body);
                let data = if sql.contains("FROM console.bi_chart")
                    && !sql.contains("owner_id != ''")
                {
                    vec![json!({
                        "id": "c_private", "board": "b_private", "created_by": "ui",
                        "created_at": "2026-01-01 00:00:00",
                        "spec_json": json!({
                            "spec": {
                                "id": "c_private", "title": "Private chart", "kind": "bar",
                                "mart": "mart_x", "sql": "SELECT 1", "x": "kab", "y": "visitors",
                            },
                            "def": {
                                "title": "Private chart", "mart": "mart_x", "kind": "bar",
                                "dimension": "kab", "measures": ["visitors"],
                            },
                            "hasYear": false,
                        })
                        .to_string(),
                    })]
                } else {
                    Vec::new()
                };
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": data, "rows": data.len() }))
            }
        }

        #[tokio::test]
        async fn a_chart_on_another_users_private_dashboard_is_not_listed() {
            let server = MockServer::start().await;
            Mock::given(wiremock::matchers::method("POST"))
                .respond_with(PrivateChart)
                .mount(&server)
                .await;
            let reached: HashSet<(String, String)> =
                HashSet::from([("serving.mart_x".to_owned(), "kab".to_owned())]);
            let answer = dependent_charts(
                &state_for(&server.uri()),
                &reached,
                &lakehouse_bi::access::Viewer::user("u_other", false),
            )
            .await
            .expect("the charts could be read");
            assert!(
                answer.is_empty(),
                "a chart on another user's private dashboard is not listed: {answer:?}"
            );
        }
    }
}
