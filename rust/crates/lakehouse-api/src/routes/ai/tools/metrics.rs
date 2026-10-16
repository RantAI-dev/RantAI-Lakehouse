//! Metric tools (`BI-2`, with `AI-7`): `list_metrics`, `describe_metric`,
//! `query_metric`, `create_metric`, `update_metric` and `delete_metric`.
//!
//! # Posture
//!
//! The writes call the console route's own handlers (`routes::metrics`), so
//! the assistant cannot reach anything the console cannot: the same checker,
//! the same name rules, the same refusal while a chart uses a metric. The
//! permission each tool needs is on its registry entry and enforced by the
//! gate before this module runs (`metric:write` for the three writes).
//!
//! `query_metric` reads a number **as the person who asked**. It builds the
//! very chart a dashboard tile would be (`lakehouse_bi::metrics::value_chart`)
//! and runs its statement with that person's roles, placeholders and policy
//! obligations (`routes::metrics::value_query`), so a source the person cannot
//! read is refused by the role rewrite, a masked column is read masked and a
//! row filter applies. With no signed-in person (a call outside a session)
//! there are no roles to read as, and the tool refuses (fail closed). The
//! database's text never reaches the model: a failure is the tile's fixed
//! sentence and a reference.

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use lakehouse_auth::Principal;
use lakehouse_bi::fields::{self, SourceKind};
use lakehouse_bi::filters::FilterDef;
use lakehouse_bi::metrics::{self, ValueQuery};
use serde_json::{Map, Value, json};

use super::api_result_to_value;
use crate::routes::metrics as routes;
use crate::state::AppState;

fn route_args(args: &Map<String, Value>) -> Map<String, Value> {
    let mut out = args.clone();
    if let Some(source) = out.remove("sqlSource") {
        out.insert("source".to_owned(), source);
    }
    out
}

fn body(args: &Map<String, Value>) -> Bytes {
    Bytes::from(serde_json::to_vec(&Value::Object(route_args(args))).unwrap_or_default())
}

/// Whether `name` is one of the metric tools (one guard arm in `run_tool`).
pub(super) fn handles(name: &str) -> bool {
    matches!(
        name,
        "list_metrics"
            | "describe_metric"
            | "query_metric"
            | "create_metric"
            | "update_metric"
            | "delete_metric"
    )
}

/// The route answers `{ ok, metric }`; the audit trail and the model read the
/// id at the top.
fn with_id(mut result: Value) -> Value {
    if let Some(id) = result.pointer("/metric/id").cloned() {
        result["id"] = id;
    }
    result
}

/// A metric by `name` (or `id`) among the saved ones.
async fn find(state: &AppState, args: &Map<String, Value>) -> Result<metrics::MetricDef, Value> {
    let by = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let (name, id) = (by("name").or_else(|| by("metric")), by("id"));
    if name.is_none() && id.is_none() {
        return Err(json!({ "error": "give the metric's name" }));
    }
    let all = metrics::list_metrics(&state.clickhouse)
        .await
        .map_err(|failure| {
            let _logged =
                crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &failure);
            json!({ "error": "The metrics could not be read." })
        })?;
    all.into_iter()
        .find(|m| name.as_deref() == Some(m.name.as_str()) || id.as_deref() == Some(m.id.as_str()))
        .ok_or_else(|| json!({ "error": "no metric has that name; call list_metrics" }))
}

async fn list_metrics(state: &AppState, args: &Map<String, Value>) -> Value {
    let q = match serde_json::from_value(Value::Object(route_args(args))) {
        Ok(q) => q,
        Err(_unparsed) => return json!({ "error": "give mart or sqlSource, or nothing for all" }),
    };
    let listed = api_result_to_value(routes::list(State(state.clone()), Query(q)).await).await;
    // Names, labels, descriptions and sources: what the model needs to choose
    // one. The formula is for describe_metric.
    let rows: Vec<Value> = listed["metrics"]
        .as_array()
        .map(|all| {
            all.iter()
                .map(|m| {
                    json!({
                        "name": m["name"], "label": m["label"], "description": m["description"],
                        "sourceKind": m["sourceKind"], "sourceId": m["sourceId"],
                        "sourceTitle": m["sourceTitle"], "timeColumn": m["timeColumn"],
                        "grain": m["grain"],
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if listed.get("error").is_some() {
        return listed;
    }
    json!({ "metrics": rows })
}

async fn describe_metric(state: &AppState, args: &Map<String, Value>) -> Value {
    let found = match find(state, args).await {
        Ok(m) => m,
        Err(e) => return e,
    };
    let usage =
        api_result_to_value(routes::usage(State(state.clone()), Path(found.id.clone())).await)
            .await;
    json!({ "metric": found, "usage": usage })
}

async fn query_metric(
    state: &AppState,
    principal: Option<&Principal>,
    args: &Map<String, Value>,
) -> Value {
    let Some(principal) = principal else {
        return json!({ "error": "query_metric reads as the signed-in person, and there is none." });
    };
    let found = match find(state, args).await {
        Ok(m) => m,
        Err(e) => return e,
    };
    let text = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let filters: Vec<FilterDef> = match args.get("filters") {
        None | Some(Value::Null) => Vec::new(),
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(f) => f,
            Err(_unparsed) => {
                return json!({ "error": "filters are {column, op, values…} or {segment}" });
            }
        },
    };
    let source = match found.source_kind {
        SourceKind::Mart => {
            crate::routes::calc_fields::resolve(&state.clickhouse, Some(&found.source_id), None)
                .await
        }
        SourceKind::SqlSource => {
            crate::routes::calc_fields::resolve(&state.clickhouse, None, Some(&found.source_id))
                .await
        }
    };
    let source = match source {
        Ok(s) => s,
        Err(_unresolved) => return json!({ "error": "this metric's source is not available." }),
    };
    let catalog =
        match fields::list_catalog_fields_for(&state.clickhouse, source.kind, &source.id).await {
            Ok(found) => found,
            Err(failure) => {
                let _logged =
                    crate::upstream_error::report_ch(&crate::upstream_error::DATABASE, &failure);
                return json!({ "error": "The metric could not be read." });
            }
        };
    // A time grain cuts the metric's own date column unless another is named.
    let grain = text("grain");
    let group_by = match (text("groupBy"), grain) {
        (Some(g), _) => Some(g),
        (None, Some(_)) => found.time_column.as_deref(),
        (None, None) => None,
    };
    if grain.is_some() && group_by.is_none() {
        return json!({ "error": "this metric has no date column; name one in groupBy" });
    }
    let query = ValueQuery {
        metric: &found.name,
        source_kind: source.kind,
        source_id: &source.id,
        group_by,
        grain,
        filters,
    };
    match routes::value_query(state, principal, &source, catalog, &query).await {
        Ok(mut tile) => {
            tile["metric"] = json!(found.name);
            tile["label"] = json!(found.label);
            tile
        }
        Err(err) => api_result_to_value::<axum::Json<Value>>(Err(err.into())).await,
    }
}

pub(super) async fn run(
    state: &AppState,
    principal: Option<&Principal>,
    name: &str,
    args: &Map<String, Value>,
) -> Value {
    match name {
        "list_metrics" => list_metrics(state, args).await,
        "describe_metric" => describe_metric(state, args).await,
        "query_metric" => query_metric(state, principal, args).await,
        "create_metric" => {
            let who = principal.cloned().map(Extension);
            with_id(
                api_result_to_value(routes::create(State(state.clone()), who, body(args)).await)
                    .await,
            )
        }
        "update_metric" => {
            let who = principal.cloned().map(Extension);
            with_id(
                api_result_to_value(routes::update(State(state.clone()), who, body(args)).await)
                    .await,
            )
        }
        "delete_metric" => {
            let q = match serde_json::from_value(Value::Object(route_args(args))) {
                Ok(q) => q,
                Err(_unparsed) => return json!({ "error": "id is required" }),
            };
            api_result_to_value(routes::delete(State(state.clone()), Query(q)).await).await
        }
        other => json!({ "error": format!("unknown tool: {other}") }),
    }
}
