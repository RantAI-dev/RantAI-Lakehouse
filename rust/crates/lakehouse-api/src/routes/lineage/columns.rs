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

use std::collections::{BTreeSet, HashMap};

use lakehouse_store::PgPool;
use lakehouse_store::column_lineage::{self, NewMapping};
use sqlparser::dialect::ClickHouseDialect;

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

/// The columns of every table `sql` reads, plus `extra`, keyed the way the
/// helper keys them. A table whose columns cannot be read is left out, which
/// makes the helper mark what depends on it partial rather than guess.
pub(crate) async fn known_columns(
    state: &AppState,
    sql: &str,
    extra: &[String],
) -> HashMap<String, Vec<String>> {
    let mut tables: BTreeSet<String> =
        crate::sql_rewrite::referenced_tables(sql, &ClickHouseDialect {})
            .unwrap_or_default()
            .into_iter()
            .collect();
    tables.extend(extra.iter().cloned());
    let engine = crate::policy_engine::PolicyEngineObligations::from_state(state);
    let mut known = HashMap::new();
    for table in tables.into_iter().take(MAX_TABLES_PER_STATEMENT) {
        let Some((schema, name)) = table.split_once('.') else {
            continue;
        };
        if let Some(columns) = engine.table_columns(schema, name).await {
            known.insert(table, columns);
        }
    }
    known
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
    let known = known_columns(state, &sql, &[]).await;
    let rows = match sql_lineage::map_select(&sql, &ClickHouseDialect {}, &known) {
        Ok(lineage) => rows_for(&lineage, &target),
        Err(unmappable) => {
            // The statement cannot be read, so its outputs cannot even be
            // named: write no rows, and drop the earlier ones so a stale
            // mapping is not shown for a statement that no longer produces it.
            tracing::info!(pipeline = pipeline_id, reason = %unmappable, "column lineage: the pipeline's SQL cannot be mapped");
            Vec::new()
        }
    };
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

#[cfg(test)]
mod tests {
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
}
