//! `GET /api/governance/lineage/used-by?table=` (`BI-14`, plan
//! `2026-10-24-bi-14-chart-lineage.md`, decision D5): the stored charts that
//! read a table, directly or through a SQL source or a metric, with their
//! dashboards.
//!
//! # What "reads" means
//!
//! Exactly the start nodes of [`super::charts::reads_of`]: a chart's mart, the
//! tables it joins, its layers' sources, and the metrics its measures name. A
//! chart over a SQL source reads the tables the source's recorded column
//! mappings name (`column_lineage`, `dashboard:read` rows); a chart over a
//! metric reads the metric's mart, or the tables of the metric's SQL source.
//! `via` says which of the three it is (`direct`, `sql_source`, `metric`, in
//! that order of preference).
//!
//! # Who is listed
//!
//! Charts come through the caller's [`Viewer`], so a chart in somebody else's
//! personal space is not named. It is counted: `hiddenCount` is the number of
//! charts that match for the system viewer and not for the caller, with no
//! name, so the answer says "and N more you do not have access to" without
//! telling anything about them. At most [`MAX_CHARTS`] are listed
//! (`truncated` says when more matched).

use std::collections::{BTreeSet, HashMap};

use axum::Extension;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::fields::SourceKind;
use lakehouse_bi::store::{self, StoredChartSpec};
use lakehouse_core::ApiError;
use lakehouse_store::column_lineage;
use serde::Deserialize;
use serde_json::{Value, json};

use super::charts::reads_of;
use super::columns::{graph_key, scope_for};
use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;
use crate::upstream_error;

/// Most charts one answer lists.
pub(crate) const MAX_CHARTS: usize = 200;

const BAD_TABLE: &str = "table must look like serving.<table>, silver.<table> or bronze.<table>.";

/// The query of the route.
#[derive(Debug, Deserialize)]
pub(crate) struct UsedByQuery {
    #[serde(default)]
    table: String,
}

/// A table key (`serving.x`) from what the caller typed: `gold.x` is
/// `serving.x`, case is ignored, and the zone must be one the graph knows.
fn table_key(raw: &str) -> Option<String> {
    let key = graph_key(&raw.trim().to_ascii_lowercase());
    let (zone, name) = key.split_once('.')?;
    let name_ok = !name.is_empty()
        && name.len() <= 128
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    (matches!(zone, "serving" | "silver" | "bronze") && name_ok).then_some(key)
}

/// How `chart` reads `key`, or `None` when it does not.
fn via(
    chart: &StoredChartSpec,
    key: &str,
    metrics: &[lakehouse_bi::metrics::MetricDef],
    readers: &BTreeSet<String>,
) -> Option<&'static str> {
    let reads = reads_of(&chart.def, metrics);
    if reads.marts.iter().any(|m| format!("serving.{m}") == key) {
        return Some("direct");
    }
    if reads.sql_sources.iter().any(|s| readers.contains(s)) {
        return Some("sql_source");
    }
    let through_metric = metrics
        .iter()
        .filter(|m| reads.metrics.contains(&m.id))
        .any(|m| match m.source_kind {
            SourceKind::Mart => {
                format!("serving.{}", m.source_id.trim_start_matches("serving.")) == key
            }
            SourceKind::SqlSource => readers.contains(&m.source_id),
        });
    through_metric.then_some("metric")
}

/// The answer of the route.
///
/// # Errors
///
/// 400 for a table that does not look like one; the errors of tenant
/// resolution; a `ClickHouse` failure through `upstream_error`.
pub(crate) async fn read(
    state: &AppState,
    principal: &Principal,
    headers: &HeaderMap,
    table: &str,
) -> Result<Value, ApiError> {
    let key = table_key(table).ok_or_else(|| ApiError::BadRequest(BAD_TABLE.to_owned()))?;
    let tenant = crate::tenant_scope::resolve(principal, headers)?;
    let shared = crate::routes::catalog::catalog_tenant_refusal(state, principal, headers)
        .await?
        .is_none();
    let ch = &state.clickhouse;
    let failed = |err: &lakehouse_clickhouse::ChError| {
        upstream_error::ch_error(&upstream_error::DATABASE, err)
    };
    let viewer = viewer_of(principal);
    let visible = store::list_stored_charts(ch, &viewer)
        .await
        .map_err(|e| failed(&e))?;
    let everyone = store::list_stored_charts(ch, &Viewer::system())
        .await
        .map_err(|e| failed(&e))?;
    let metrics = lakehouse_bi::metrics::list_metrics(ch)
        .await
        .map_err(|e| failed(&e))?;
    let boards = store::list_boards(ch, &viewer)
        .await
        .map_err(|e| failed(&e))?;
    // The SQL sources whose recorded column mappings read this table.
    let mut readers: BTreeSet<String> = BTreeSet::new();
    if let Some(pool) = state.pg.as_deref() {
        match column_lineage::touching_table(pool, &key, scope_for(principal, tenant, shared)).await
        {
            Ok(rows) => readers.extend(
                rows.into_iter()
                    .filter(|r| {
                        r.producer_kind == "sql_source"
                            && r.from_table.as_deref() == Some(key.as_str())
                    })
                    .map(|r| r.producer_ref),
            ),
            Err(err) => {
                tracing::warn!(%err, "lineage: the table's column mappings could not be read");
            }
        }
    }
    let matching = |charts: &[StoredChartSpec]| -> Vec<(String, String, String, &'static str)> {
        charts
            .iter()
            .filter_map(|c| {
                via(c, &key, &metrics, &readers)
                    .map(|v| (c.spec.id.clone(), c.spec.title.clone(), c.board.clone(), v))
            })
            .collect()
    };
    let mine = matching(&visible);
    let hidden = matching(&everyone).len().saturating_sub(mine.len());
    let titles: HashMap<&str, &str> = boards
        .iter()
        .map(|b| (b.id.as_str(), b.name.as_str()))
        .collect();
    let truncated = mine.len() > MAX_CHARTS;
    let charts: Vec<Value> = mine
        .into_iter()
        .take(MAX_CHARTS)
        .map(|(id, title, board, via)| {
            let dashboard_title = titles
                .get(board.as_str())
                .copied()
                .unwrap_or(board.as_str());
            json!({
                "chartId": id, "chartTitle": title,
                "dashboardId": board, "dashboardTitle": dashboard_title, "via": via,
            })
        })
        .collect();
    let mut body =
        json!({ "supported": true, "table": key, "charts": charts, "hiddenCount": hidden });
    if truncated {
        body["truncated"] = json!(true);
    }
    Ok(body)
}

/// The handler of `GET /api/governance/lineage/used-by` (policy
/// `lineage:read`, owner decision Q6).
///
/// # Errors
///
/// As [`read`].
pub(crate) async fn used_by(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    headers: HeaderMap,
    Query(q): Query<UsedByQuery>,
) -> ApiResult<ApiJson<Value>> {
    Ok(ApiJson(read(&state, &principal, &headers, &q.table).await?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_table_key_is_a_known_zone_and_a_plain_name() {
        assert_eq!(
            table_key(" Serving.Mart_Visits ").as_deref(),
            Some("serving.mart_visits")
        );
        assert_eq!(
            table_key("gold.mart_visits").as_deref(),
            Some("serving.mart_visits")
        );
        assert_eq!(table_key("bronze.raw").as_deref(), Some("bronze.raw"));
        for bad in [
            "",
            "mart_visits",
            "console.bi_chart",
            "serving.",
            "serving.a;b",
            "serving.a b",
        ] {
            assert_eq!(table_key(bad), None, "{bad}");
        }
    }
}
