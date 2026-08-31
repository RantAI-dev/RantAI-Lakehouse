//! `GET/POST/PUT/DELETE /api/dashboard`, `/specs`, `/boards`, `/fields`,
//! `/records`, `/values`, `/export`, `/embed-info` — the BI dashboard
//! surface.
//!
//! Ports `src/app/api/dashboard/route.ts` and its seven sibling route files
//! under `src/app/api/dashboard/`. The heavy lifting (spec storage, board
//! CRUD, filtered `SQL` assembly) already lives in `lakehouse_bi::store` and
//! `lakehouse_bi::builder`; these handlers are thin HTTP wiring around that
//! crate, matching each TypeScript handler's status codes and JSON/error
//! shapes.

use std::collections::{HashMap, HashSet};

use crate::tenant::BUILTIN_DASHBOARD_ENABLED;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_bi::builder::sql_with_filters;
use lakehouse_bi::specs::{CHARTS, ChartKind, ChartSource, KPIS, to_render_spec};
use lakehouse_bi::store::{self, ChartInput, FilterDef, LayoutMap, StoredChartSpec};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::support::{
    is_numeric_type, mart_columns, render_stored_spec, run_spec_sql, strip_non_ident,
};
use crate::state::AppState;

// ── GET /api/dashboard ──────────────────────────────────────────────────

/// Query parameters accepted by `GET /api/dashboard`.
#[derive(Debug, Deserialize)]
pub struct DashboardQuery {
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    year: Option<String>,
    #[serde(default)]
    filters: Option<String>,
}

/// `GET /api/dashboard` — the combined tile data + metadata payload the
/// main dashboard view renders.
pub async fn get(State(state): State<AppState>, Query(q): Query<DashboardQuery>) -> Response {
    match get_body(&state.clickhouse, &q).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one straight-line port of a single large TS handler (spec \
              merge -> filter application -> parallel tile execution); \
              splitting it up would scatter one pipeline across helpers \
              with no independent reuse"
)]
async fn get_body(
    ch: &ChClient,
    q: &DashboardQuery,
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let board = q.board.clone().unwrap_or_else(|| "default".to_owned());
    let years: Vec<i64> = q
        .year
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect();
    let param_filters: Option<Vec<FilterDef>> = q
        .filters
        .as_deref()
        .and_then(|f| serde_json::from_str::<Vec<FilterDef>>(f).ok());

    let mut store_error: Option<String> = None;
    let (stored, boards) =
        match tokio::try_join!(store::list_stored_charts(ch), store::list_boards(ch)) {
            Ok(v) => v,
            Err(err) => {
                store_error = Some(format!("Error: {err}"));
                (Vec::new(), Vec::new())
            }
        };

    let board_obj = boards.iter().find(|b| b.id == board);
    let layout = board_obj.and_then(|b| b.layout.clone()).unwrap_or_default();
    let filters = param_filters
        .or_else(|| board_obj.and_then(|b| b.filters.clone()))
        .unwrap_or_default();

    // Tile bawaan hanya disajikan bila deployment ini memang punya mart-nya;
    // lihat `BUILTIN_DASHBOARD_ENABLED`.
    let on_default = (board == "default" || board == "all") && *BUILTIN_DASHBOARD_ENABLED;
    let stored_for_board: Vec<&StoredChartSpec> = if board == "all" {
        stored.iter().collect()
    } else {
        stored
            .iter()
            .filter(|c| c.board == board || (c.board.is_empty() && board == "default"))
            .collect()
    };

    let need_cols = !years.is_empty() || filters.iter().any(|f| !f.values.is_empty());
    let cols: HashMap<String, HashSet<String>> = if need_cols {
        mart_columns(ch).await?
    } else {
        HashMap::new()
    };

    let mut results = Map::new();
    if on_default {
        for k in KPIS.iter() {
            let (id, val) = run_spec_sql(ch, k.id, k.sql).await;
            results.insert(id, val);
        }
        for c in CHARTS.iter() {
            let (id, val) = run_spec_sql(ch, c.id, c.sql).await;
            results.insert(id, val);
        }
    }
    for c in &stored_for_board {
        let sql = sql_with_filters(c, &years, &filters, &cols);
        let (id, val) = run_spec_sql(ch, &c.spec.id, &sql).await;
        results.insert(id, val);
    }

    let filter_columns: Vec<String> = {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for c in &stored_for_board {
            let dim = &c.def.dimension;
            if !dim.is_empty() && seen.insert(dim.clone()) {
                out.push(dim.clone());
            }
        }
        out
    };

    let mut boards_out = vec![json!({ "id": "default", "name": "Main" })];
    for b in &boards {
        boards_out.push(json!({ "id": b.id, "name": b.name }));
    }

    let kpis_out: Vec<Value> = if on_default {
        KPIS.iter()
            .map(|k| json!({ "id": k.id, "title": k.title, "caption": k.caption, "format": k.format }))
            .collect()
    } else {
        Vec::new()
    };

    let mut charts_out: Vec<Value> = Vec::new();
    if on_default {
        for c in CHARTS.iter() {
            let mut rendered = serde_json::to_value(to_render_spec(c, ChartSource::Builtin))
                .unwrap_or_else(|_| json!({}));
            rendered["board"] = json!("default");
            charts_out.push(rendered);
        }
    }
    for c in &stored_for_board {
        let mut rendered = render_stored_spec(&c.spec, c.source);
        rendered["board"] = json!(c.board);
        rendered["def"] = serde_json::to_value(&c.def).unwrap_or_else(|_| json!({}));
        charts_out.push(rendered);
    }

    Ok(json!({
        "board": board,
        "years": years,
        "layout": layout_to_json(&layout),
        "filters": filters,
        "filterColumns": filter_columns,
        "boards": boards_out,
        "kpis": kpis_out,
        "charts": charts_out,
        "results": results,
        "storeError": store_error,
    }))
}

fn layout_to_json(layout: &LayoutMap) -> Value {
    let mut m = Map::new();
    for (k, b) in layout {
        m.insert(k.clone(), json!({ "x": b.x, "y": b.y, "w": b.w, "h": b.h }));
    }
    Value::Object(m)
}

// ── /api/dashboard/specs ────────────────────────────────────────────────

/// `GET /api/dashboard/specs` — every stored chart, in render shape.
pub async fn specs_list(State(state): State<AppState>) -> Response {
    match store::list_stored_charts(&state.clickhouse).await {
        Ok(stored) => {
            let charts: Vec<Value> = stored.iter().map(render_stored_chart).collect();
            (StatusCode::OK, ApiJson(json!({ "charts": charts }))).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

fn render_stored_chart(c: &StoredChartSpec) -> Value {
    let mut rendered = render_stored_spec(&c.spec, c.source);
    rendered["board"] = json!(c.board);
    rendered["def"] = serde_json::to_value(&c.def).unwrap_or_else(|_| json!({}));
    rendered["hasYear"] = json!(c.has_year);
    rendered["createdBy"] = json!(c.created_by);
    rendered["createdAt"] = json!(c.created_at);
    rendered
}

fn parse_chart_input(body: &Bytes) -> Result<ChartInput, ApiError> {
    serde_json::from_slice(body)
        .map_err(|_err| ApiError::BadRequest("body JSON tidak valid".to_owned()))
}

/// `POST /api/dashboard/specs` — create a chart from high-level input.
///
/// # Errors
///
/// 400 on an unparseable body or a `lakehouse_bi` validation/`ClickHouse`
/// failure, matching the `TypeScript`'s single `catch` around both.
pub async fn specs_create(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let input = parse_chart_input(&body)?;
    let spec = store::spec_from_input(&state.clickhouse, &input, ChartSource::Ui, "ui", None)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    store::insert_chart(&state.clickhouse, &spec)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(
        json!({ "ok": true, "chart": render_stored_spec(&spec.spec, ChartSource::Ui) }),
    ))
}

/// `PUT /api/dashboard/specs` — edit a stored chart, keeping its id.
///
/// The `TypeScript` handler (`specs/route.ts::PUT`) parses the body as
/// loose JSON, checks `id` first, and only THEN builds/validates a
/// `ChartInput` from it (`specFromInput` coerces missing fields with `??`
/// rather than failing). Deserializing straight into a `#[serde(flatten)]
/// ChartInput` here would invert that order — a body missing `mart`/`kind`/
/// etc. but ALSO missing `id` would fail on the strict `ChartInput` shape
/// before ever reaching the `id` check, reporting "body JSON tidak valid"
/// instead of "id wajib untuk edit" (caught by the parity corpus:
/// `dashboard-specs-edit-missing-id` sends `{"title":"x"}`). Parsing to a
/// bare [`Value`] first and checking `id` before the strict decode restores
/// the TS precedence.
///
/// # Errors
///
/// 400 on an unparseable body, a missing `id`, or a validation/`ClickHouse`
/// failure.
pub async fn specs_update(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let raw: Value = serde_json::from_slice(&body)
        .map_err(|_err| ApiError::BadRequest("body JSON tidak valid".to_owned()))?;
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let Some(id) = id else {
        return Err(ApiError::BadRequest("id wajib untuk edit".to_owned()).into());
    };
    let input: ChartInput = serde_json::from_value(raw)
        .map_err(|_err| ApiError::BadRequest("body JSON tidak valid".to_owned()))?;
    let spec = store::spec_from_input(&state.clickhouse, &input, ChartSource::Ui, "ui", Some(id))
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    store::insert_chart(&state.clickhouse, &spec)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(
        json!({ "ok": true, "chart": render_stored_spec(&spec.spec, ChartSource::Ui) }),
    ))
}

/// Query parameters for `DELETE /api/dashboard/specs` / `/boards`
/// (`?id=`).
#[derive(Debug, Deserialize)]
pub struct IdQuery {
    #[serde(default)]
    id: Option<String>,
}

/// `DELETE /api/dashboard/specs?id=` — soft-delete a stored chart.
///
/// # Errors
///
/// 400 [`ApiError::BadRequest`] when `id` is missing; 500
/// [`ApiError::Internal`] on a `ClickHouse` failure.
pub async fn specs_delete(
    State(state): State<AppState>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let Some(id) = q.id.filter(|s| !s.is_empty()) else {
        return Err(ApiError::BadRequest("id wajib".to_owned()).into());
    };
    store::delete_chart(&state.clickhouse, &id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true })))
}

// ── /api/dashboard/boards ───────────────────────────────────────────────

/// `GET /api/dashboard/boards` — every dashboard, `default` first.
pub async fn boards_list(State(state): State<AppState>) -> Response {
    match store::list_boards(&state.clickhouse).await {
        Ok(boards) => {
            let mut out = vec![json!({ "id": "default", "name": "Main", "layout": {} })];
            out.extend(
                boards
                    .iter()
                    .map(|b| serde_json::to_value(b).unwrap_or_else(|_| json!({}))),
            );
            (StatusCode::OK, ApiJson(json!({ "boards": out }))).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

/// `{name?, duplicate?}` — the `POST /api/dashboard/boards` body shape.
#[derive(Debug, Deserialize, Default)]
struct BoardCreateBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    duplicate: Option<String>,
}

/// `POST /api/dashboard/boards` — create a board, or duplicate one.
///
/// # Errors
///
/// 400 on an unparseable body or a `ClickHouse`/validation failure.
pub async fn boards_create(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: BoardCreateBody = if body.is_empty() {
        BoardCreateBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|err| ApiError::BadRequest(format!("JSON tidak valid: {err}")))?
    };
    let board = if let Some(dup) = parsed.duplicate {
        store::duplicate_board(&state.clickhouse, &dup).await
    } else {
        store::create_board(&state.clickhouse, &parsed.name.unwrap_or_default()).await
    }
    .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true, "board": board })))
}

/// The `PUT /api/dashboard/boards` body shape.
#[derive(Debug, Deserialize, Default)]
struct BoardEditBody {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    layout: Option<LayoutMap>,
    #[serde(default)]
    filters: Option<Vec<FilterDef>>,
    #[serde(default)]
    public: Option<bool>,
    #[serde(default)]
    embed: Option<bool>,
}

/// `PUT /api/dashboard/boards` — rename/relayout/re-filter/publish/embed a
/// board.
///
/// # Errors
///
/// 400 when `id` is missing/`"default"`, or on a validation/`ClickHouse`
/// failure.
pub async fn boards_update(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: BoardEditBody = if body.is_empty() {
        BoardEditBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|err| ApiError::BadRequest(format!("JSON tidak valid: {err}")))?
    };
    let id = parsed.id.unwrap_or_default();
    if id.is_empty() || id == "default" {
        return Err(ApiError::BadRequest("dashboard tidak valid".to_owned()).into());
    }
    let ch = &state.clickhouse;
    if let Some(name) = &parsed.name {
        store::rename_board(ch, &id, name)
            .await
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    }
    if let Some(layout) = &parsed.layout {
        store::update_board_layout(ch, &id, layout)
            .await
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    }
    if let Some(filters) = &parsed.filters {
        store::update_board_filters(ch, &id, filters)
            .await
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    }
    if let Some(public) = parsed.public {
        let token = store::set_board_public(ch, &id, public)
            .await
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
        return Ok(ApiJson(json!({ "ok": true, "publicToken": token })));
    }
    if let Some(embed) = parsed.embed {
        let enabled = store::set_board_embed(ch, &id, embed)
            .await
            .map_err(|err| ApiError::BadRequest(err.to_string()))?;
        return Ok(ApiJson(json!({ "ok": true, "embedEnabled": enabled })));
    }
    Ok(ApiJson(json!({ "ok": true })))
}

/// `DELETE /api/dashboard/boards?id=` — delete a board and its charts.
///
/// # Errors
///
/// 400 when `id` is missing/`"default"`; 500 on a `ClickHouse` failure.
pub async fn boards_delete(
    State(state): State<AppState>,
    Query(q): Query<IdQuery>,
) -> ApiResult<ApiJson<Value>> {
    let id = q.id.unwrap_or_default();
    if id.is_empty() || id == "default" {
        return Err(ApiError::BadRequest("dashboard tidak valid".to_owned()).into());
    }
    store::delete_board(&state.clickhouse, &id)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    Ok(ApiJson(json!({ "ok": true })))
}

// ── /api/dashboard/fields ───────────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/fields`.
#[derive(Debug, Deserialize)]
pub struct FieldsQuery {
    #[serde(default)]
    mart: Option<String>,
}

/// `GET /api/dashboard/fields` — mart list, or one mart's columns split
/// into dimensions/measures.
pub async fn fields(State(state): State<AppState>, Query(q): Query<FieldsQuery>) -> Response {
    match fields_body(&state.clickhouse, q.mart.as_deref()).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn fields_body(
    ch: &ChClient,
    mart: Option<&str>,
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let Some(mart) = mart else {
        let rows = ch
            .rows(
                "SELECT name, toString(total_rows) AS total_rows FROM system.tables \
                 WHERE database='serving' AND name NOT LIKE '%\\_baru' ORDER BY name",
                None,
            )
            .await?;
        let marts: Vec<Value> = rows
            .iter()
            .map(|r| {
                let name = r.get("name").and_then(Value::as_str).unwrap_or("");
                let rows_n: i64 = r
                    .get("total_rows")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                json!({ "name": name, "rows": rows_n })
            })
            .collect();
        return Ok(json!({ "marts": marts }));
    };

    let safe = strip_non_ident(mart);
    let sql = format!(
        "SELECT name, type FROM system.columns WHERE database='serving' AND table='{safe}' \
         ORDER BY position"
    );
    let cols = ch.rows(&sql, None).await?;
    let mut dimensions = Vec::new();
    let mut measures = Vec::new();
    let mut columns_out = Vec::new();
    for c in &cols {
        let name = c.get("name").and_then(Value::as_str).unwrap_or("");
        let ty = c.get("type").and_then(Value::as_str).unwrap_or("");
        if is_numeric_type(ty) {
            measures.push(name.to_owned());
        } else {
            dimensions.push(name.to_owned());
        }
        columns_out.push(json!({ "name": name, "type": ty }));
    }
    Ok(json!({
        "mart": safe,
        "dimensions": dimensions,
        "measures": measures,
        "columns": columns_out,
    }))
}

// ── /api/dashboard/records ──────────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/records`.
#[derive(Debug, Deserialize)]
pub struct RecordsQuery {
    #[serde(default)]
    mart: Option<String>,
    #[serde(default)]
    column: Option<String>,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    limit: Option<String>,
}

/// `/^[a-zA-Z_][a-zA-Z0-9_]*$/` — the exact `IDENT` pattern used by
/// `records/route.ts` (distinct from the strip-only pattern in
/// `fields`/`values`).
fn is_strict_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// `s.replace(/\\/g, "\\\\").replace(/'/g, "''")`.
fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "''")
}

/// `GET /api/dashboard/records` — drill-down: raw Gold rows behind one
/// category value.
pub async fn records(
    State(state): State<AppState>,
    Query(q): Query<RecordsQuery>,
) -> ApiResult<ApiJson<Value>> {
    let mart_raw = q.mart.unwrap_or_default();
    let mart = mart_raw
        .strip_prefix("serving.")
        .unwrap_or(&mart_raw)
        .to_owned();
    let column = q.column.unwrap_or_default();
    let value = q.value.unwrap_or_default();
    let limit: i64 = q
        .limit
        .as_deref()
        .and_then(|s| s.parse::<i64>().ok())
        .filter(|n| *n != 0)
        .unwrap_or(50)
        .clamp(1, 200);

    if !is_strict_ident(&mart) || !is_strict_ident(&column) {
        return Err(ApiError::BadRequest("mart/column tidak valid".to_owned()).into());
    }

    let ch = &state.clickhouse;
    let cols_sql = format!(
        "SELECT name FROM system.columns WHERE database='serving' AND table='{}'",
        esc(&mart)
    );
    let cols = ch
        .rows(&cols_sql, None)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    if cols.is_empty() {
        return Err(ApiError::NotFound(format!("mart '{mart}' tidak ada")).into());
    }
    let has_column = cols
        .iter()
        .any(|c| c.get("name").and_then(Value::as_str) == Some(column.as_str()));
    if !has_column {
        return Err(ApiError::BadRequest(format!("kolom '{column}' tidak ada")).into());
    }

    let sql = format!(
        "SELECT * FROM serving.{mart} WHERE {column} = '{}' LIMIT {limit}",
        esc(&value)
    );
    let result = ch
        .query(&sql, None)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
    Ok(ApiJson(json!({
        "columns": columns,
        "rows": result.data,
        "mart": mart,
        "column": column,
        "value": value,
    })))
}

// ── /api/dashboard/values ───────────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/values`.
#[derive(Debug, Deserialize)]
pub struct ValuesQuery {
    #[serde(default)]
    column: Option<String>,
}

/// `GET /api/dashboard/values` — distinct values of one column, across
/// every Gold mart that has it (for a dashboard filter dropdown).
pub async fn values(
    State(state): State<AppState>,
    Query(q): Query<ValuesQuery>,
) -> ApiResult<ApiJson<Value>> {
    let column = strip_non_ident(q.column.as_deref().unwrap_or(""));
    if column.is_empty() {
        return Err(ApiError::BadRequest("column wajib".to_owned()).into());
    }
    let ch = &state.clickhouse;
    let marts_sql = format!(
        "SELECT table FROM system.columns WHERE database='serving' AND name='{column}' AND \
         table NOT LIKE '%\\_baru'"
    );
    let marts = ch
        .rows(&marts_sql, None)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    if marts.is_empty() {
        return Ok(ApiJson(
            json!({ "column": column, "values": Vec::<String>::new() }),
        ));
    }
    let union = marts
        .iter()
        .map(|m| {
            let table = strip_non_ident(m.get("table").and_then(Value::as_str).unwrap_or(""));
            format!("SELECT DISTINCT toString({column}) AS v FROM serving.{table}")
        })
        .collect::<Vec<_>>()
        .join(" UNION DISTINCT ");
    let sql = format!("SELECT v FROM ({union}) WHERE v != '' ORDER BY v LIMIT 200");
    let rows = ch
        .rows(&sql, None)
        .await
        .map_err(|err| ApiError::Internal(err.to_string()))?;
    let values: Vec<String> = rows
        .iter()
        .filter_map(|r| r.get("v").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    Ok(ApiJson(json!({ "column": column, "values": values })))
}

// ── /api/dashboard/export ───────────────────────────────────────────────

/// `GET /api/dashboard/export` — every board + stored chart, as a minimal
/// hand-rolled `YAML` document. The only non-`JSON` response in this crate:
/// returned directly as `text/yaml`, bypassing [`ApiJson`].
pub async fn export(State(state): State<AppState>) -> ApiResult<Response> {
    let (charts, boards) = tokio::try_join!(
        store::list_stored_charts(&state.clickhouse),
        store::list_boards(&state.clickhouse)
    )
    .map_err(|err| ApiError::Internal(err.to_string()))?;

    let mut out = String::new();
    out.push_str("# RantAI Lakehouse — dashboard as code\n");
    out.push_str("# boards & chart specs, diekspor dari console.bi_chart\n\n");
    out.push_str("boards:\n");
    out.push_str(&yaml_board("default", "Main", None));
    for b in &boards {
        out.push_str(&yaml_board(&b.id, &b.name, b.layout.as_ref()));
    }
    out.push('\n');
    out.push_str("charts:\n");
    for c in &charts {
        out.push_str(&yaml_chart(c));
        out.push('\n');
    }

    let mut response = out.into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        header::HeaderValue::from_static("text/yaml; charset=utf-8"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        header::HeaderValue::from_static("attachment; filename=\"dashboards.yaml\""),
    );
    Ok(response)
}

/// `/^[\w.\-/]+$/.test(s) ? s : JSON.stringify(s)` — bare if it's a
/// "plain" token, quoted (`JSON.stringify`) otherwise. `~` for
/// null/undefined; numbers/booleans render as-is.
fn yaml_value(v: &Value) -> String {
    match v {
        Value::Null => "~".to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => {
            let s = match other {
                Value::String(s) => s.clone(),
                _ => other.to_string(),
            };
            let plain = !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/'));
            if plain {
                s
            } else {
                serde_json::to_string(&s).unwrap_or(s)
            }
        }
    }
}

fn yaml_board(id: &str, name: &str, layout: Option<&LayoutMap>) -> String {
    let mut lines = vec![
        format!("  - id: {id}"),
        format!("    name: {}", yaml_value(&json!(name))),
    ];
    if let Some(layout) = layout
        && !layout.is_empty()
    {
        lines.push("    layout:".to_owned());
        for (cid, bx) in layout {
            lines.push(format!(
                "      {cid}: {{ x: {}, y: {}, w: {}, h: {} }}",
                bx.x, bx.y, bx.w, bx.h
            ));
        }
    }
    lines.join("\n") + "\n"
}

/// Renders `n` the way `JSON.stringify` renders a JS `number`: a
/// whole-valued float becomes the bare integer (`3000000`, not
/// `3000000.0`, which is what `serde_json`'s own `Value::Number` `Display`
/// gives an `f64`-backed number — verified directly, not assumed).
fn js_number(n: f64) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
        json!(n as i64)
    } else {
        json!(n)
    }
}

fn yaml_chart(c: &StoredChartSpec) -> String {
    let mut lines = vec![
        format!("- id: {}", c.spec.id),
        format!("  board: {}", yaml_value(&json!(c.board))),
    ];
    for (k, val) in chart_def_fields(&c.def) {
        match &val {
            Value::Null => {}
            Value::Array(items) => {
                let rendered = items.iter().map(yaml_value).collect::<Vec<_>>().join(", ");
                lines.push(format!("  {k}: [{rendered}]"));
            }
            other => lines.push(format!("  {k}: {}", yaml_value(other))),
        }
    }
    lines.join("\n")
}

/// `def`'s field order, mirroring the `TypeScript` object-literal
/// construction order in `bi-store.ts::specFromInput` — NOT `ChartInput`'s
/// Rust struct declaration order, which the TS deliberately does not
/// follow. Each `kind` branch there builds its own literal with its own key
/// order (`mart` comes before `kind` for charts/tables, but after it for
/// text/kpi/gauge), and `dashboard-export`'s captured corpus — taken from a
/// live, TS-created board — reflects that exact order. Iterating
/// `serde_json::to_value(def)` instead would silently reorder every field
/// to match declaration order and fail parity nondeterministically (`kind`
/// vs `mart`, `caption` vs `span`/`board`, ...).
fn chart_def_fields(def: &ChartInput) -> Vec<(&'static str, Value)> {
    let mut out: Vec<(&'static str, Value)> = Vec::new();
    match def.kind {
        ChartKind::Text => {
            out.push(("title", json!(def.title)));
            out.push(("kind", json!(def.kind)));
            out.push(("mart", json!(def.mart)));
            out.push(("dimension", json!(def.dimension)));
            out.push(("measures", json!(def.measures)));
            if let Some(text) = &def.text {
                out.push(("text", json!(text)));
            }
            if let Some(span) = def.span {
                out.push(("span", json!(span)));
            }
            if let Some(board) = &def.board {
                out.push(("board", json!(board)));
            }
        }
        ChartKind::Kpi | ChartKind::Gauge => {
            out.push(("title", json!(def.title)));
            out.push(("kind", json!(def.kind)));
            out.push(("mart", json!(def.mart)));
            out.push(("dimension", json!(def.dimension)));
            out.push(("measures", json!(def.measures)));
            if let Some(aggregate) = &def.aggregate {
                out.push(("aggregate", json!(aggregate)));
            }
            if let Some(caption) = &def.caption {
                out.push(("caption", json!(caption)));
            }
            if let Some(target) = def.target {
                out.push(("target", js_number(target)));
            }
            if let Some(span) = def.span {
                out.push(("span", json!(span)));
            }
            if let Some(board) = &def.board {
                out.push(("board", json!(board)));
            }
        }
        _ => {
            out.push(("title", json!(def.title)));
            if let Some(subtitle) = &def.subtitle {
                out.push(("subtitle", json!(subtitle)));
            }
            out.push(("mart", json!(def.mart)));
            out.push(("kind", json!(def.kind)));
            out.push(("dimension", json!(def.dimension)));
            out.push(("measures", json!(def.measures)));
            if let Some(breakdown) = &def.breakdown {
                out.push(("breakdown", json!(breakdown)));
            }
            if let Some(aggregate) = &def.aggregate {
                out.push(("aggregate", json!(aggregate)));
            }
            if let Some(limit) = def.limit {
                out.push(("limit", json!(limit)));
            }
            if let Some(order) = &def.order {
                out.push(("order", json!(order)));
            }
            if let Some(span) = def.span {
                out.push(("span", json!(span)));
            }
            if let Some(board) = &def.board {
                out.push(("board", json!(board)));
            }
        }
    }
    out
}

// ── /api/dashboard/embed-info ───────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/embed-info`.
#[derive(Debug, Deserialize)]
pub struct EmbedInfoQuery {
    #[serde(default)]
    board: Option<String>,
}

/// `GET /api/dashboard/embed-info` — this board's embed status and a
/// freshly-signed sample token.
///
/// # D2 (post-cutover): the signing secret is no longer returned
///
/// The `TypeScript` original (and this route, pre-fix) returned the raw
/// HMAC signing secret in the response body — `{"secret": "<64-hex>",
/// "enabled": bool, "sampleToken": "<jwt>"}`. That treated the console as
/// the only trusted surface, which stopped being true once this route sat
/// behind real authentication: any authenticated caller (not just an
/// admin) could read the key that signs EVERY embed JWT for EVERY
/// dashboard and forge tokens offline, bypassing `routes::embed::data`'s
/// verification entirely. A signing key must never cross the wire.
///
/// The fix keeps `enabled` and `sampleToken` — minting a sample token
/// server-side for an authenticated console user is the legitimate use
/// case the secret was being exposed for — and simply omits `secret`.
/// `rust/tests/parity/corpus/dashboard-embed-info.json` and
/// `rust/tests/parity/README.md` are updated accordingly; this is a
/// deliberate, documented parity divergence, not drift.
///
/// # Errors
///
/// 400 when `board` is missing/`"default"`; 404 when the board doesn't
/// exist; 500 on any other `ClickHouse` failure.
pub async fn embed_info(
    State(state): State<AppState>,
    Query(q): Query<EmbedInfoQuery>,
) -> Response {
    let id = q.board.unwrap_or_default();
    if id.is_empty() || id == "default" {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "dashboard tidak valid" })),
        )
            .into_response();
    }
    match embed_info_body(&state, &id).await {
        Ok(Some(body)) => (StatusCode::OK, ApiJson(body)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": "not_found" })),
        )
            .into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(json!({ "error": err.to_string() })),
        )
            .into_response(),
    }
}

async fn embed_info_body(
    state: &AppState,
    id: &str,
) -> Result<Option<Value>, lakehouse_clickhouse::ChError> {
    let Some(board) = store::get_board(&state.clickhouse, id).await? else {
        return Ok(None);
    };
    let secret = state.embed_secret.get_embed_secret().await?;
    let exp = now_unix_seconds() + 3600.0;
    let claims = lakehouse_embed::EmbedClaims {
        resource: Some(lakehouse_embed::EmbedResource {
            dashboard: Some(id.to_owned()),
        }),
        params: Some(HashMap::new()),
        exp: Some(exp),
    };
    let sample_token = lakehouse_embed::sign_embed(&claims, &secret);
    // D2: `secret` is deliberately NOT included in the response — see this
    // function's doc comment. Only `enabled` and a freshly-signed
    // `sampleToken` (the legitimate use case the secret used to serve) go
    // over the wire.
    Ok(Some(json!({
        "enabled": board.embed_enabled.unwrap_or(false),
        "sampleToken": sample_token,
    })))
}

fn now_unix_seconds() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
        .floor()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn strip_non_ident_removes_everything_but_word_chars() {
        assert_eq!(strip_non_ident("mart_wisman"), "mart_wisman");
        assert_eq!(strip_non_ident("a b'; DROP--"), "abDROP");
    }

    #[test]
    fn is_numeric_type_matches_int_float_decimal() {
        assert!(is_numeric_type("UInt16"));
        assert!(is_numeric_type("Float64"));
        assert!(is_numeric_type("Decimal(10,2)"));
        assert!(!is_numeric_type("String"));
    }

    #[test]
    fn is_strict_ident_requires_leading_letter_or_underscore() {
        assert!(is_strict_ident("mart_wisman"));
        assert!(is_strict_ident("_hidden"));
        assert!(!is_strict_ident("2024col"));
        assert!(!is_strict_ident("not!valid"));
        assert!(!is_strict_ident(""));
    }

    #[test]
    fn esc_doubles_backslashes_and_quotes() {
        assert_eq!(esc("O'Brien\\x"), "O''Brien\\\\x");
    }

    #[test]
    fn yaml_value_quotes_non_plain_strings() {
        assert_eq!(yaml_value(&json!("mart_wisman")), "mart_wisman");
        assert_eq!(
            yaml_value(&json!("Sample — Visitors")),
            "\"Sample — Visitors\""
        );
        assert_eq!(yaml_value(&json!(null)), "~");
        assert_eq!(yaml_value(&json!(3_000_000)), "3000000");
    }

    #[test]
    fn yaml_board_omits_layout_when_absent() {
        let out = yaml_board("default", "Main", None);
        assert_eq!(out, "  - id: default\n    name: Main\n");
    }

    #[test]
    fn yaml_board_renders_nonempty_layout() {
        let mut layout = LayoutMap::new();
        layout.insert(
            "c1".to_owned(),
            store::TileBox {
                x: 0,
                y: 0,
                w: 3,
                h: 5,
            },
        );
        let out = yaml_board("b1", "Board 1", Some(&layout));
        assert!(out.contains("    layout:\n"));
        assert!(out.contains("      c1: { x: 0, y: 0, w: 3, h: 5 }"));
    }
}
