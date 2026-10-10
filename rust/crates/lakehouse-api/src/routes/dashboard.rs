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

use std::collections::{BTreeMap, HashMap, HashSet};

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_bi::builder::{ReadContext, RelationColumns};
use lakehouse_bi::filters::{ColumnKind, validate_filters, without_inert};
use lakehouse_bi::grain::{Grain, TimeContext};
use lakehouse_bi::specs::{CHARTS, ChartKind, ChartSource, KPIS, to_render_spec};
use lakehouse_bi::store::{self, ChartInput, FilterDef, LayoutMap, StoredChartSpec};
use lakehouse_clickhouse::ChClient;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::policy_engine::PolicyEngineObligations;
use crate::routes::support::{
    is_numeric_type, mart_columns, render_stored_spec, run_spec_sql, sources_for, stored_chart_sql,
    strip_non_ident,
};
use crate::sql_guard::check_sql_source;
use crate::state::AppState;
use crate::upstream_error;

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
    /// BI-9: replaces the grain of every chart whose own grain is a
    /// truncation (`minute` to `year`). Absent: the board's saved grain, or
    /// each chart's own.
    #[serde(default)]
    grain: Option<String>,
}

/// What the caller asked of the dashboard's grain switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum GrainRequest {
    /// No `grain` parameter: the board's saved grain applies, if any.
    #[default]
    Saved,
    /// `grain=own`: each chart keeps its own grain, saved default or not.
    Own,
    /// One of the truncations.
    Switch(Grain),
}

impl From<Option<Grain>> for GrainRequest {
    fn from(g: Option<Grain>) -> Self {
        g.map_or(Self::Saved, Self::Switch)
    }
}

/// The `grain` query of `GET /api/dashboard`: absent or blank, `own`, or a
/// truncation.
///
/// # Errors
///
/// 400 with our message for any other text, a part of the date included.
fn parse_grain_request(raw: Option<&str>) -> Result<GrainRequest, ApiError> {
    if raw.map(str::trim) == Some("own") {
        return Ok(GrainRequest::Own);
    }
    parse_grain_param(raw).map(GrainRequest::from)
}

/// A `grain` query value as a dashboard switch: one of the truncations.
///
/// # Errors
///
/// 400 with our message for any other text, a part of the date included (a
/// part cannot replace every chart's grain).
fn parse_grain_param(raw: Option<&str>) -> Result<Option<Grain>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    Grain::parse(raw)
        .filter(|g| g.is_truncation())
        .map(Some)
        .ok_or_else(|| {
            ApiError::BadRequest(
                "grain must be one of minute, hour, day, week, month, quarter, year.".to_owned(),
            )
        })
}

/// `GET /api/dashboard` — the combined tile data + metadata payload the
/// main dashboard view renders.
///
/// WS7 item D1: `Policy::RequiresPermission("dashboard:read")` (see
/// `policy.rs`) guarantees a real `Extension<Principal>` is always
/// present here — its `role_names`/id/tenant ids are threaded into every
/// tile's `run_spec_sql` call, so a masking/row-filter policy applies to
/// the authenticated dashboard viewer, exactly as it would in Query
/// Studio.
pub async fn get(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<DashboardQuery>,
) -> Response {
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    // BI-18: a malformed `filters` used to be ignored without a word, which
    // showed an unfiltered dashboard for a filter the caller believed was on.
    let param_filters = match parse_filters_param(q.filters.as_deref()) {
        Ok(f) => f,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let requested = match parse_grain_request(q.grain.as_deref()) {
        Ok(g) => g,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let time = match crate::routes::settings::time_context(&state).await {
        Ok(t) => t,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let obligations = PolicyEngineObligations::from_state(&state);
    match get_body(
        &state.clickhouse,
        &q,
        param_filters,
        &principal.role_names,
        &placeholders,
        &obligations,
        (&time, requested),
    )
    .await
    {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
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
    param_filters: Option<Vec<FilterDef>>,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &PolicyEngineObligations<'_>,
    (time, requested_grain): (&TimeContext, GrainRequest),
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let board = q.board.clone().unwrap_or_else(|| "default".to_owned());
    let years: Vec<i64> = q
        .year
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter_map(|s| s.trim().parse::<i64>().ok())
        .collect();
    let mut store_error: Option<String> = None;
    let (stored, boards) =
        match tokio::try_join!(store::list_stored_charts(ch), store::list_boards(ch)) {
            Ok(v) => v,
            Err(err) => {
                store_error =
                    Some(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_string());
                (Vec::new(), Vec::new())
            }
        };

    // The built-in board's layout row is kept out of `list_boards`.
    let default_row = if board == store::DEFAULT_BOARD_ID {
        store::get_board(ch, store::DEFAULT_BOARD_ID)
            .await
            .unwrap_or_else(|err| {
                store_error.get_or_insert_with(|| {
                    upstream_error::report_ch(&upstream_error::DATABASE, &err).to_string()
                });
                None
            })
    } else {
        None
    };
    let board_obj = boards
        .iter()
        .find(|b| b.id == board)
        .or(default_row.as_ref());
    let layout = board_obj.and_then(|b| b.layout.clone()).unwrap_or_default();
    let filters = param_filters
        .or_else(|| board_obj.and_then(|b| b.filters.clone()))
        .unwrap_or_default();
    // BI-9: the caller's switch wins; otherwise the board's saved grain; a
    // stored value that is not a truncation is ignored, not trusted.
    let saved_grain = board_obj
        .and_then(|b| b.grain.as_deref())
        .and_then(Grain::parse)
        .filter(|g| g.is_truncation());
    let applied_grain = match requested_grain {
        GrainRequest::Switch(g) => Some(g),
        GrainRequest::Own => None,
        GrainRequest::Saved => saved_grain,
    };
    let read = ReadContext {
        time,
        grain: applied_grain,
    };

    // The built-in tiles are served only when this tenant's
    // BUILTIN_DASHBOARD_SPEC actually loaded a non-empty catalog — see
    // lakehouse_bi::specs's module doc for why an empty/missing spec means
    // "disabled" (this replaces the retired BUILTIN_DASHBOARD_ENABLED flag).
    let on_default =
        (board == "default" || board == "all") && (!KPIS.is_empty() || !CHARTS.is_empty());
    let stored_for_board: Vec<&StoredChartSpec> = if board == "all" {
        stored.iter().collect()
    } else {
        stored
            .iter()
            .filter(|c| c.board == board || (c.board.is_empty() && board == "default"))
            .collect()
    };

    // Always loaded now: `filterFields` lists every column of every relation
    // the board reads, not only the ones an active filter touches.
    let cols = mart_columns(ch).await?;

    let mut results = Map::new();
    if on_default {
        for k in KPIS.iter() {
            let sql =
                lakehouse_bi::builder::apply_builtin_year_filter(&k.sql, &k.mart, &years, &cols);
            let (id, val) = run_spec_sql(ch, &k.id, &sql, roles, placeholders, obligations).await;
            results.insert(id, val);
        }
        for c in CHARTS.iter() {
            let sql =
                lakehouse_bi::builder::apply_builtin_year_filter(&c.sql, &c.mart, &years, &cols);
            let (id, val) = run_spec_sql(ch, &c.id, &sql, roles, placeholders, obligations).await;
            results.insert(id, val);
        }
    }
    let sources = sources_for(ch, stored_for_board.iter().copied()).await?;
    for c in &stored_for_board {
        let filtered = match stored_chart_sql(c, &years, &filters, &cols, &sources, &read) {
            Ok(filtered) => filtered,
            Err(msg) => {
                results.insert(c.spec.id.clone(), json!({ "error": msg }));
                continue;
            }
        };
        let (id, mut val) = run_spec_sql(
            ch,
            &c.spec.id,
            &filtered.sql,
            roles,
            placeholders,
            obligations,
        )
        .await;
        let skipped = &filtered.skipped;
        crate::routes::support::annotate_grain(&mut val, &filtered, c);
        crate::routes::support::annotate_table(
            ch,
            &mut val,
            &filtered,
            (roles, placeholders, obligations),
        )
        .await;
        // The tile still renders; this only lets it say which active filters
        // its data cannot honour (no such column, or a type that does not fit).
        if !skipped.is_empty()
            && let Value::Object(tile) = &mut val
        {
            tile.insert("filtersSkipped".to_owned(), json!(skipped));
        }
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
        boards_out.push(json!({ "id": b.id, "name": b.name, "folderId": b.folder_id }));
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
        "filterFields": filter_fields(&stored_for_board, &cols, &sources),
        // The saved default, which `filters` is not when the caller passed
        // its own: the console compares the two to offer Save and Reset.
        "defaultFilters": board_obj.and_then(|b| b.filters.clone()).unwrap_or_default(),
        // BI-9: the grain an editor saved for the board (`""` when none), the
        // one this response applied, and the zone and first day it used, so
        // the console labels buckets the way the server cut them.
        "grain": saved_grain.map_or("", Grain::as_str),
        "appliedGrain": applied_grain.map(Grain::as_str),
        "reporting": {
            "timeZone": time.zone(),
            "weekStart": time.week_start().as_str(),
        },
        // BI-18·B: the interval an editor saved; 0 when none, so the
        // console starts from "off" without a second request.
        "refreshSeconds": board_obj.and_then(|b| b.refresh_seconds).unwrap_or(0),
        "boards": boards_out,
        "kpis": kpis_out,
        "charts": charts_out,
        "results": results,
        "storeError": store_error,
    }))
}

/// Parse the `filters` query parameter. Absent or blank means "use the
/// board's saved filters"; anything else must be a valid filter list.
///
/// # Errors
///
/// 400 with a message of ours when the text is not a JSON filter list or a
/// filter fails [`validate_filters`].
fn parse_filters_param(raw: Option<&str>) -> Result<Option<Vec<FilterDef>>, ApiError> {
    let Some(raw) = raw.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(None);
    };
    let filters: Vec<FilterDef> = serde_json::from_str(raw)
        .map_err(|err| ApiError::BadRequest(format!("filters is invalid: {err}")))?;
    validate_filters(&filters).map_err(ApiError::BadRequest)?;
    Ok(Some(filters))
}

/// Every column a dashboard filter can target: the union of the columns of
/// the relations the board's charts read (marts and SQL sources), with the
/// kind the filter editor should offer and how many tiles have the column.
/// A text tile reads nothing and a chart over unknown columns adds none.
///
/// When two relations disagree on a column's kind the first chart's wins;
/// the tiles it does not fit report `wrong_type` rather than being coerced.
fn filter_fields(
    charts: &[&StoredChartSpec],
    mart_cols: &HashMap<String, RelationColumns>,
    sources: &HashMap<String, lakehouse_bi::sources::SqlSource>,
) -> Vec<Value> {
    let mut by_column: BTreeMap<String, (ColumnKind, usize)> = BTreeMap::new();
    for c in charts {
        if c.spec.kind == ChartKind::Text {
            continue;
        }
        let source_cols;
        let cols = if let Some(id) = c.def.sql_source.as_deref() {
            let Some(source) = sources.get(id) else {
                continue;
            };
            source_cols = source.column_kinds();
            &source_cols
        } else if let Some(cols) = mart_cols.get(&c.def.mart) {
            cols
        } else {
            continue;
        };
        for (name, kind) in cols {
            by_column
                .entry(name.clone())
                .and_modify(|(_, tiles)| *tiles += 1)
                .or_insert((*kind, 1));
        }
    }
    by_column
        .into_iter()
        .map(|(column, (kind, tiles))| json!({ "column": column, "kind": kind, "tiles": tiles }))
        .collect()
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
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
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
        .map_err(|_err| ApiError::BadRequest("body JSON is invalid".to_owned()))
}

/// `POST /api/dashboard/specs` — create a chart from high-level input.
///
/// # Errors
///
/// 400 on an unparseable body or a `lakehouse_bi` validation/`ClickHouse`
/// failure, matching the `TypeScript`'s single `catch` around both.
pub async fn specs_create(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let input = parse_chart_input(&body)?;
    let time = crate::routes::settings::time_context(&state).await?;
    let spec = store::spec_from_input(
        &state.clickhouse,
        &input,
        ChartSource::Ui,
        "ui",
        None,
        &time,
    )
    .await
    .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    store::insert_chart(&state.clickhouse, &spec)
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::BadRequest,
            )
        })?;
    Ok(ApiJson(
        json!({ "ok": true, "chart": render_stored_spec(&spec.spec, ChartSource::Ui) }),
    ))
}

/// `POST /api/dashboard/specs/preview` — validate and execute a chart input
/// without persisting it. The builder uses this to show the user the actual
/// result before the chart is added to a dashboard.
///
/// Same posture as [`get`]: `Policy::RequiresPermission
/// ("dashboard:read")` guarantees a real `Extension<Principal>`, whose
/// `role_names`/id/tenant ids are threaded into [`run_spec_sql`] so a
/// preview is masked/row-filtered exactly like the persisted tile would be.
pub async fn specs_preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let input = parse_chart_input(&body)?;
    let time = crate::routes::settings::time_context(&state).await?;
    let spec = store::spec_from_input(
        &state.clickhouse,
        &input,
        ChartSource::Ui,
        "ui",
        None,
        &time,
    )
    .await
    .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations = PolicyEngineObligations::from_state(&state);
    let (_, mut result) = run_spec_sql(
        &state.clickhouse,
        &spec.spec.id,
        &spec.spec.sql,
        &principal.role_names,
        &placeholders,
        &obligations,
    )
    .await;
    crate::routes::support::annotate_saved_grain(&mut result, &spec);
    crate::routes::support::annotate_saved_table(&mut result, &spec);
    Ok(ApiJson(json!({
        "spec": render_stored_spec(&spec.spec, ChartSource::Ui),
        "result": result,
    })))
}

/// `PUT /api/dashboard/specs` — edit a stored chart, keeping its id.
///
/// The `TypeScript` handler (`specs/route.ts::PUT`) parses the body as
/// loose JSON, checks `id` first, and only THEN builds/validates a
/// `ChartInput` from it (`specFromInput` coerces missing fields with `??`
/// rather than failing). Deserializing straight into a `#[serde(flatten)]
/// ChartInput` here would invert that order — a body missing `mart`/`kind`/
/// etc. but ALSO missing `id` would fail on the strict `ChartInput` shape
/// before ever reaching the `id` check, reporting "body JSON is invalid"
/// instead of "id is required for edit" (caught by the parity corpus:
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
        .map_err(|_err| ApiError::BadRequest("body JSON is invalid".to_owned()))?;
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);
    let Some(id) = id else {
        return Err(ApiError::BadRequest("id is required for edit".to_owned()).into());
    };
    let input: ChartInput = serde_json::from_value(raw)
        .map_err(|_err| ApiError::BadRequest("body JSON is invalid".to_owned()))?;
    let time = crate::routes::settings::time_context(&state).await?;
    let spec = store::spec_from_input(
        &state.clickhouse,
        &input,
        ChartSource::Ui,
        "ui",
        Some(id),
        &time,
    )
    .await
    .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    store::insert_chart(&state.clickhouse, &spec)
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::BadRequest,
            )
        })?;
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
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
    };
    store::delete_chart(&state.clickhouse, &id)
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::Internal,
            )
        })?;
    Ok(ApiJson(json!({ "ok": true })))
}

// ── /api/dashboard/boards ───────────────────────────────────────────────

/// `GET /api/dashboard/boards` — every dashboard, `default` first.
///
/// Each board carries a `chartCount`. It is computed here rather than
/// stored: the number of tiles on a board is derived from `console.bi_chart`
/// and would go stale the moment a chart is added, moved between boards, or
/// deleted. The built-in board's count comes from the compiled-in specs,
/// which is where its tiles actually come from.
pub async fn boards_list(State(state): State<AppState>) -> Response {
    let ch = &state.clickhouse;
    let (boards, stored) =
        match tokio::try_join!(store::list_boards(ch), store::list_stored_charts(ch)) {
            Ok(pair) => pair,
            Err(err) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
                )
                    .into_response();
            }
        };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for c in &stored {
        *counts.entry(c.board.as_str()).or_default() += 1;
    }
    // Counted the same way the payload decides to serve them: an empty
    // catalogue means this tenant's BUILTIN_DASHBOARD_SPEC never loaded, so
    // the built-in board has no tiles to report.
    let builtin_tiles = CHARTS.len() + KPIS.len();
    let mut out = vec![json!({
        "id": store::DEFAULT_BOARD_ID,
        "name": "Main",
        "layout": {},
        "chartCount": builtin_tiles + counts.get(store::DEFAULT_BOARD_ID).copied().unwrap_or(0),
        "builtin": true,
    })];
    out.extend(boards.iter().map(|b| {
        let mut value = serde_json::to_value(b).unwrap_or_else(|_| json!({}));
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "chartCount".to_owned(),
                json!(counts.get(b.id.as_str()).copied().unwrap_or(0)),
            );
            obj.insert("builtin".to_owned(), json!(false));
        }
        value
    }));
    (StatusCode::OK, ApiJson(json!({ "boards": out }))).into_response()
}

/// `{name?, description?, duplicate?}` — the `POST /api/dashboard/boards`
/// body shape.
#[derive(Debug, Deserialize, Default)]
struct BoardCreateBody {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    duplicate: Option<String>,
    /// Folder to create the board in (`""`/absent = root).
    #[serde(default, rename = "folderId")]
    folder_id: Option<String>,
}

/// The signed-in caller's display name, for the `created_by` column.
///
/// Empty when nobody is attached — the AI tool surface creates boards with
/// no principal, and an empty author is the honest answer there. This is
/// recorded for display only; it grants nothing and is never read back as
/// an authorization input.
fn author_name(principal: Option<&Extension<Principal>>) -> String {
    principal.map_or_else(String::new, |Extension(p)| p.display_name.clone())
}

/// `POST /api/dashboard/boards` — create a board, or duplicate one.
///
/// # Errors
///
/// 400 on an unparseable body or a `ClickHouse`/validation failure.
pub async fn boards_create(
    State(state): State<AppState>,
    principal: Option<Extension<Principal>>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: BoardCreateBody = if body.is_empty() {
        BoardCreateBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|err| ApiError::BadRequest(format!("JSON is invalid: {err}")))?
    };
    let author = author_name(principal.as_ref());
    let folder_id = parsed.folder_id.unwrap_or_default().trim().to_owned();
    // Checked first, so an unknown folder creates nothing.
    crate::routes::dashboard_folders::ensure_folder_exists(&state.clickhouse, &folder_id).await?;
    let mut board = if let Some(dup) = parsed.duplicate {
        store::duplicate_board(&state.clickhouse, &dup, &author).await
    } else {
        store::create_board(
            &state.clickhouse,
            &parsed.name.unwrap_or_default(),
            &parsed.description.unwrap_or_default(),
            &author,
        )
        .await
    }
    .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    if !folder_id.is_empty() {
        store::move_board(&state.clickhouse, &board.id, &folder_id)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
        board.folder_id = Some(folder_id);
    }
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
    description: Option<String>,
    #[serde(default)]
    layout: Option<LayoutMap>,
    #[serde(default)]
    filters: Option<Vec<FilterDef>>,
    #[serde(default)]
    public: Option<bool>,
    #[serde(default)]
    embed: Option<bool>,
    /// `SEC-12`: `true` withdraws every signed embed token issued so far
    /// for this board ("withdraw all"). `false` is refused, not ignored.
    #[serde(default, rename = "embedRevokeAll")]
    embed_revoke_all: Option<bool>,
    /// `SEC-12`: replace the sites allowed to frame this board's embed pages.
    /// An empty list means no site may frame it.
    #[serde(default, rename = "embedOrigins")]
    embed_origins: Option<Vec<String>>,
    /// Move the board to this folder (`""` = root).
    #[serde(default, rename = "folderId")]
    folder_id: Option<String>,
    /// The board's saved auto-refresh in seconds (`BI-18`·B; `0` = off).
    /// Held as a signed number so `-1` is refused with our message, not by
    /// the parser's.
    #[serde(default, rename = "refreshSeconds")]
    refresh_seconds: Option<i64>,
    /// The board's saved time grain (`BI-9`): a truncation from `minute` to
    /// `year`, or `""` to go back to each chart's own.
    #[serde(default)]
    grain: Option<String>,
}

/// Whether a `PUT /api/dashboard/boards` body may be applied: it names a
/// board, and the built-in board is sent nothing but a layout.
fn board_edit_allowed(body: &BoardEditBody) -> bool {
    match body.id.as_deref() {
        None | Some("") => false,
        Some(store::DEFAULT_BOARD_ID) => {
            body.name.is_none()
                && body.description.is_none()
                && body.filters.is_none()
                && body.public.is_none()
                && body.embed.is_none()
                && body.embed_revoke_all.is_none()
                && body.embed_origins.is_none()
                && body.folder_id.is_none()
        }
        Some(_) => true,
    }
}

/// A board edit's `refreshSeconds` as a checked interval (`BI-18`·B).
///
/// # Errors
///
/// 400 with our message when it is not one of
/// [`lakehouse_bi::click::REFRESH_SECONDS`] (a negative or huge number
/// included).
fn checked_refresh(raw: Option<i64>) -> Result<Option<u32>, ApiError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let seconds = u32::try_from(raw).unwrap_or(u32::MAX);
    lakehouse_bi::click::validate_refresh_seconds(seconds).map_err(ApiError::BadRequest)?;
    Ok(Some(seconds))
}

/// The `SEC-12` part of [`boards_update`]: withdraw every embed token of the
/// board, and/or replace the sites allowed to frame it. Both need
/// `dashboard:write`, like the rest of that route (`POLICY_TABLE`). Returns
/// the keys to add to the response. Split out of `boards_update` when the
/// filter validation of `BI-18` and this met in one function and took it
/// over the line limit.
///
/// # Errors
///
/// 400 for `embedRevokeAll: false` (withdrawing cannot be undone) and for a
/// site the store refuses; a classified error when the write fails.
async fn apply_embed_access(
    ch: &ChClient,
    id: &str,
    parsed: &BoardEditBody,
) -> Result<Map<String, Value>, ApiError> {
    let mut embed_access = Map::new();
    match parsed.embed_revoke_all {
        Some(true) => {
            let now = crate::routes::embed::now_seconds(lakehouse_embed::unix_now());
            let revoked_before = store::revoke_all_embed_tokens(ch, id, now)
                .await
                .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
            embed_access.insert("embedRevokedBefore".to_owned(), json!(revoked_before));
        }
        Some(false) => {
            return Err(ApiError::BadRequest(
                "embedRevokeAll can only be true: withdrawing cannot be undone.".to_owned(),
            ));
        }
        None => {}
    }
    if let Some(origins) = &parsed.embed_origins {
        let stored = store::set_board_embed_origins(ch, id, origins)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
        embed_access.insert("embedOrigins".to_owned(), json!(stored));
    }
    Ok(embed_access)
}

/// `PUT /api/dashboard/boards` — rename/relayout/re-filter/publish/embed a
/// board.
///
/// The built-in `"default"` board accepts a layout only: it has no name,
/// filters, share link or embed of its own to change.
///
/// # Errors
///
/// 400 when `id` is missing, when `"default"` is sent anything but a
/// layout, or on a validation/`ClickHouse` failure.
pub async fn boards_update(
    State(state): State<AppState>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: BoardEditBody = if body.is_empty() {
        BoardEditBody::default()
    } else {
        serde_json::from_slice(&body)
            .map_err(|err| ApiError::BadRequest(format!("JSON is invalid: {err}")))?
    };
    if !board_edit_allowed(&parsed) {
        return Err(ApiError::BadRequest("invalid dashboard".to_owned()).into());
    }
    let id = parsed.id.clone().unwrap_or_default();
    let ch = &state.clickhouse;
    // Before the first write, so a bad filter list does not leave a board
    // half-edited (BI-18).
    if let Some(filters) = &parsed.filters {
        validate_filters(filters).map_err(ApiError::BadRequest)?;
    }
    let refresh = checked_refresh(parsed.refresh_seconds)?;
    if let Some(grain) = &parsed.grain
        && !grain.trim().is_empty()
    {
        parse_grain_param(Some(grain))?;
    }
    if let Some(name) = &parsed.name {
        store::rename_board(ch, &id, name)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    }
    if let Some(description) = &parsed.description {
        store::describe_board(ch, &id, description)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    }
    if let Some(layout) = &parsed.layout {
        store::update_board_layout(ch, &id, layout)
            .await
            .map_err(|err| {
                upstream_error::ch_error_as(
                    &upstream_error::DATABASE,
                    &err,
                    upstream_error::FailedAs::BadRequest,
                )
            })?;
    }
    if let Some(seconds) = refresh {
        store::update_board_refresh(ch, &id, seconds)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    }
    if let Some(grain) = &parsed.grain {
        store::update_board_grain(ch, &id, grain.trim())
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    }
    if let Some(filters) = &parsed.filters {
        // BI-18 review round 1: a save cleans the row of filters that filter
        // nothing. Not rejected (an older console still sends them) and not
        // rewritten on read.
        let filters = without_inert(filters);
        store::update_board_filters(ch, &id, &filters)
            .await
            .map_err(|err| {
                upstream_error::ch_error_as(
                    &upstream_error::DATABASE,
                    &err,
                    upstream_error::FailedAs::BadRequest,
                )
            })?;
    }
    if let Some(folder_id) = &parsed.folder_id {
        let folder_id = folder_id.trim();
        crate::routes::dashboard_folders::ensure_folder_exists(ch, folder_id).await?;
        store::move_board(ch, &id, folder_id)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    }
    // SEC-12. Merged into whichever response below is returned, so a body
    // that also sets `public` or `embed` still reports them.
    let embed_access = apply_embed_access(ch, &id, &parsed).await?;
    let with_embed_access = |mut body: Value| {
        if let Some(map) = body.as_object_mut() {
            map.extend(embed_access.clone());
        }
        body
    };
    if let Some(public) = parsed.public {
        let token = store::set_board_public(ch, &id, public)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
        return Ok(ApiJson(with_embed_access(
            json!({ "ok": true, "publicToken": token }),
        )));
    }
    if let Some(embed) = parsed.embed {
        let enabled = store::set_board_embed(ch, &id, embed)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
        return Ok(ApiJson(with_embed_access(
            json!({ "ok": true, "embedEnabled": enabled }),
        )));
    }
    Ok(ApiJson(with_embed_access(json!({ "ok": true }))))
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
    if id.is_empty() || id == store::DEFAULT_BOARD_ID {
        return Err(ApiError::BadRequest("invalid dashboard".to_owned()).into());
    }
    store::delete_board(&state.clickhouse, &id)
        .await
        .map_err(|err| {
            upstream_error::ch_error_as(
                &upstream_error::DATABASE,
                &err,
                upstream_error::FailedAs::Internal,
            )
        })?;
    Ok(ApiJson(json!({ "ok": true })))
}

// ── /api/dashboard/fields ───────────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/fields`.
#[derive(Debug, Deserialize)]
pub struct FieldsQuery {
    #[serde(default)]
    mart: Option<String>,
    /// A dashboard SQL source id; answers with its probed columns instead
    /// of a mart's.
    #[serde(default)]
    source: Option<String>,
}

/// `GET /api/dashboard/fields` — mart list, or one mart's columns split
/// into dimensions/measures.
pub async fn fields(State(state): State<AppState>, Query(q): Query<FieldsQuery>) -> Response {
    if let Some(id) = q.source.as_deref() {
        return match source_fields_body(&state.clickhouse, id).await {
            Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
            Err(err) => crate::error::ApiRejection(err).into_response(),
        };
    }
    match fields_body(&state.clickhouse, q.mart.as_deref()).await {
        Ok(body) => (StatusCode::OK, ApiJson(body)).into_response(),
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
}

/// `?source=` branch of [`fields`]: the SQL source's columns, split the same
/// way a mart's are. New shape (no parity corpus); errors are classified,
/// never `ClickHouse` text.
async fn source_fields_body(ch: &ChClient, id: &str) -> Result<Value, ApiError> {
    let source = lakehouse_bi::sources::get_source(ch, id.trim())
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?
        .ok_or_else(|| ApiError::NotFound(format!("SQL source '{}' not found.", id.trim())))?;
    let mut dimensions = Vec::new();
    let mut measures = Vec::new();
    for c in &source.columns {
        if is_numeric_type(&c.ty) {
            measures.push(c.name.clone());
        } else {
            dimensions.push(c.name.clone());
        }
    }
    Ok(json!({
        "source": source.id,
        "title": source.title,
        "dimensions": dimensions,
        "measures": measures,
        "columns": source.columns,
    }))
}

/// Schema metadata only — mart names, column names/types, row counts from
/// `system.tables`/`system.columns`, never row values — so no policy
/// rewrite: governance here masks and filters DATA, and the builder needs
/// the column list to exist at all. (`/values`, which reads data, is
/// rewritten.)
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
#[derive(Debug, Default, Deserialize)]
pub struct RecordsQuery {
    #[serde(default)]
    mart: Option<String>,
    /// The id of the dashboard SQL source a tile is built on, read instead
    /// of `mart` (BI-18·B: records over a SQL source).
    #[serde(default, rename = "sqlSource", alias = "source")]
    sql_source: Option<String>,
    /// With `value`: the clicked column. Without both, the whole tile's rows.
    #[serde(default)]
    column: Option<String>,
    #[serde(default)]
    value: Option<String>,
    /// Rows per page, at most [`RECORDS_PAGE_MAX`].
    #[serde(default)]
    limit: Option<String>,
    /// Rows to skip, for the next page.
    #[serde(default)]
    offset: Option<String>,
    /// The dashboard's active filters (JSON list, as for `GET /api/dashboard`),
    /// so the list agrees with the number that was clicked.
    #[serde(default)]
    filters: Option<String>,
    /// BI-9: with `column` and `value`, `value` is a bucket of a chart
    /// grouped by this grain (a truncation), and the rows are those whose raw
    /// `column` falls in it, in the report time zone.
    #[serde(default)]
    grain: Option<String>,
    /// `BI-16` part A: the columns of a raw table (comma-separated, in
    /// order). With it the route lists those columns of the whole tile (no
    /// `column` or `value`), sorted by `sortColumn` and `sortDir`, through the
    /// same filters, role rewrite and page size as any records list.
    #[serde(default)]
    columns: Option<String>,
    /// With `columns`: the column to sort by.
    #[serde(default, rename = "sortColumn")]
    sort_column: Option<String>,
    /// With `sortColumn`: `asc` (default) or `desc`.
    #[serde(default, rename = "sortDir")]
    sort_dir: Option<String>,
}

/// Rows per page: the default and the maximum (BI-18·B; the owner confirms 50
/// at QA).
const RECORDS_PAGE_MAX: u32 = 50;

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
///
/// Same posture as [`get`]: `Policy::RequiresPermission("dashboard:read")`
/// guarantees a real `Extension<Principal>`, and the row query goes through
/// `policy_engine::rewrite_sql_for_roles` before it reaches `ClickHouse`.
/// Drill-down review fix: this route used to run
/// `SELECT * FROM serving.<mart>` with plain `ch.query`, so a column masked
/// (or rows filtered) on the tile came back unmasked one click later.
pub async fn records(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<RecordsQuery>,
) -> ApiResult<ApiJson<Value>> {
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    let time = crate::routes::settings::time_context(&state).await?;
    records_for_roles(
        &state.clickhouse,
        q,
        &principal.role_names,
        &placeholders,
        &obligations,
        &time,
    )
    .await
    .map(ApiJson)
    .map_err(Into::into)
}

/// Fixed words for the dashboard's data routes (drill-down, filter values,
/// SQL-source fields).
const DASHBOARD_QUERY: upstream_error::Context = upstream_error::Context::new(
    "dashboard query",
    "The dashboard query failed.",
    "ClickHouse is unavailable.",
);

/// Classifies a `ClickHouse` failure on the dashboard's data routes
/// (drill-down, filter values, SQL-source fields) without forwarding its
/// text (AGENTS.md principle 4): a server-side error is a fixed 422,
/// anything else a fixed 503, both with a reference id; the real detail is
/// only logged.
fn classify_dashboard_ch_error(err: &lakehouse_clickhouse::ChError) -> ApiError {
    upstream_error::ch_error(&DASHBOARD_QUERY, err)
}

/// The relation a records list reads, with its columns.
struct RecordsRelation {
    from: lakehouse_bi::builder::Relation,
    cols: RelationColumns,
    /// What the response names it by: `("mart", name)` or `("sqlSource", id)`.
    label: (&'static str, String),
}

/// The relation named by `q`: a `serving` mart, or a SQL source passed
/// through the same `check_sql_source` gate it passed when saved.
async fn records_relation(ch: &ChClient, q: &RecordsQuery) -> Result<RecordsRelation, ApiError> {
    use lakehouse_bi::builder::Relation;
    if let Some(id) = q.sql_source.as_deref().filter(|s| !s.trim().is_empty()) {
        let source = lakehouse_bi::sources::get_source(ch, id)
            .await
            .map_err(|err| classify_dashboard_ch_error(&err))?
            .ok_or_else(|| {
                ApiError::NotFound("this chart's SQL source no longer exists".to_owned())
            })?;
        let sql = check_sql_source(&source.sql).map_err(|_| {
            ApiError::Unprocessable(
                "this chart's SQL source no longer passes the SQL check".to_owned(),
            )
        })?;
        return Ok(RecordsRelation {
            from: Relation::Sql(sql),
            cols: source.column_kinds(),
            label: ("sqlSource", source.id),
        });
    }
    let mart_raw = q.mart.clone().unwrap_or_default();
    let mart = mart_raw
        .strip_prefix("serving.")
        .unwrap_or(&mart_raw)
        .to_owned();
    if !is_strict_ident(&mart) {
        return Err(ApiError::BadRequest("invalid mart/column".to_owned()));
    }
    let cols_sql = format!(
        "SELECT name, type FROM system.columns WHERE database='serving' AND table='{}'",
        esc(&mart)
    );
    let rows = ch
        .rows(&cols_sql, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    if rows.is_empty() {
        return Err(ApiError::NotFound(format!("mart '{mart}' does not exist")));
    }
    let cols: RelationColumns = rows
        .iter()
        .filter_map(|r| {
            let name = r.get("name").and_then(Value::as_str)?;
            let ty = r.get("type").and_then(Value::as_str).unwrap_or("");
            Some((name.to_owned(), ColumnKind::from_clickhouse_type(ty)))
        })
        .collect();
    let ident = lakehouse_core::ident::Ident::new(mart.clone())
        .map_err(|_| ApiError::BadRequest("invalid mart/column".to_owned()))?;
    Ok(RecordsRelation {
        from: Relation::Mart(ident),
        cols,
        label: ("mart", mart),
    })
}

/// A page size or offset: absent means `default`; text that is not a
/// non-negative whole number is a 400 of ours.
fn records_number(raw: Option<&str>, name: &str, default: u64) -> Result<u64, ApiError> {
    match raw.map(str::trim).filter(|t| !t.is_empty()) {
        None => Ok(default),
        Some(t) => t.parse::<u64>().map_err(|_| {
            ApiError::BadRequest(format!("{name} must be a non-negative whole number"))
        }),
    }
}

/// What the records list is narrowed to (BI-9). With a `grain` the value is a
/// bucket the tile printed, and the rows are those whose raw column falls in
/// it, computed here in the report zone. A grain on a column it does not fit
/// (a plain date has no hour, a text column no date), or one that is a part
/// of the date, is refused, never read as "everything".
fn records_value<'a>(
    drill: Option<&'a (lakehouse_core::ident::Ident, &'a str)>,
    cols: &RelationColumns,
    grain: Option<&str>,
) -> Result<Option<lakehouse_bi::builder::RecordsValue<'a>>, ApiError> {
    use lakehouse_bi::builder::RecordsValue;
    let bucket_grain = parse_grain_param(grain)?;
    match (drill, bucket_grain) {
        (Some((ident, text)), Some(grain)) => {
            let kind = cols
                .get(ident.as_str())
                .copied()
                .unwrap_or(ColumnKind::Text);
            if !grain.supports(ChartKind::Line, Some(kind)) {
                return Err(ApiError::BadRequest(
                    "grain does not fit this column".to_owned(),
                ));
            }
            Ok(Some(RecordsValue::Bucket {
                column: ident,
                kind,
                grain,
                text,
            }))
        }
        (None, Some(_)) => Err(ApiError::BadRequest(
            "grain needs a column and a value".to_owned(),
        )),
        (Some((ident, text)), None) => Ok(Some(RecordsValue::Equals(ident, text))),
        (None, None) => Ok(None),
    }
}

/// [`records`]' body, taking the principal's roles and placeholders
/// explicitly so the enforcement path can be tested against a mock
/// `ClickHouse` and a real policy row.
///
/// BI-18·B: the statements come from [`lakehouse_bi::builder::records_sql`],
/// the builder's own relation and filter machinery, so a mart and a SQL
/// source share one path; both the page and the total go through the role
/// rewrite.
async fn records_for_roles(
    ch: &ChClient,
    q: RecordsQuery,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &PolicyEngineObligations<'_>,
    time: &TimeContext,
) -> Result<Value, ApiError> {
    let limit = u32::try_from(records_number(
        q.limit.as_deref(),
        "limit",
        u64::from(RECORDS_PAGE_MAX),
    )?)
    .unwrap_or(RECORDS_PAGE_MAX)
    .clamp(1, RECORDS_PAGE_MAX);
    let offset = records_number(q.offset.as_deref(), "offset", 0)?;
    let filters = parse_filters_param(q.filters.as_deref())?.unwrap_or_default();

    // A drill is a column and a value together; neither is the whole tile.
    let drill = match (
        q.column.as_deref().filter(|c| !c.is_empty()),
        q.value.as_deref(),
    ) {
        (Some(column), Some(value)) => Some((column, value)),
        (None, None) => None,
        _ => {
            return Err(ApiError::BadRequest(
                "column and value go together".to_owned(),
            ));
        }
    };
    let relation = records_relation(ch, &q).await?;
    if q.columns.is_some() {
        if drill.is_some() {
            return Err(ApiError::BadRequest(
                "columns cannot be combined with column and value".to_owned(),
            ));
        }
        let sql = rows_statements(&q, &relation, &filters, (limit, offset), time)?;
        return run_records(
            ch,
            &relation,
            sql,
            (limit, offset),
            None,
            (roles, placeholders, obligations),
        )
        .await;
    }
    let drill = match drill {
        Some((column, value)) => {
            let ident = lakehouse_core::ident::Ident::new(column.to_owned())
                .map_err(|_| ApiError::BadRequest("invalid mart/column".to_owned()))?;
            if !relation.cols.contains_key(column) {
                return Err(ApiError::BadRequest(format!(
                    "column '{column}' does not exist"
                )));
            }
            Some((ident, value))
        }
        None => None,
    };
    let value = records_value(drill.as_ref(), &relation.cols, q.grain.as_deref())?;
    let sql = lakehouse_bi::builder::records_sql(
        &relation.from,
        &relation.cols,
        &filters,
        value,
        limit,
        offset,
        time,
    );

    run_records(
        ch,
        &relation,
        sql,
        (limit, offset),
        drill.as_ref().map(|(c, v)| (c.as_str(), *v)),
        (roles, placeholders, obligations),
    )
    .await
}

/// Run a records list's two statements through the caller's role rewrite and
/// shape the response: one page of rows, the total, and the filters the
/// relation could not honour.
async fn run_records(
    ch: &ChClient,
    relation: &RecordsRelation,
    sql: lakehouse_bi::builder::RecordsSql,
    (limit, offset): (u32, u64),
    drill: Option<(&str, &str)>,
    (roles, placeholders, obligations): (
        &[String],
        &crate::sql_rewrite::PlaceholderValues,
        &PolicyEngineObligations<'_>,
    ),
) -> Result<Value, ApiError> {
    let rewrite = |statement: String| async move {
        crate::policy_engine::rewrite_sql_for_roles(
            &statement,
            &sqlparser::dialect::ClickHouseDialect {},
            roles,
            placeholders,
            obligations,
        )
        .await
        .map_err(|err| {
            ApiError::Unprocessable(
                crate::policy_engine::enforcement_error_message(&err).to_owned(),
            )
        })
    };
    let rows_sql = rewrite(sql.rows).await?;
    let count_sql = rewrite(sql.count).await?;
    let result = ch
        .query(&rows_sql, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    let counted = ch
        .rows(&count_sql, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    // `count()` is a UInt64, which ClickHouse quotes in JSON; a count that is
    // neither text nor number is not invented as 0.
    let total = counted
        .first()
        .and_then(|r| r.get("n"))
        .and_then(|n| {
            n.as_u64()
                .or_else(|| n.as_str().and_then(|t| t.parse().ok()))
        })
        .ok_or_else(|| ApiError::Internal("database error".to_owned()))?;
    let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
    let mut body = json!({
        "columns": columns,
        "rows": result.data,
        "total": total,
        "limit": limit,
        "offset": offset,
        "column": drill.map(|(c, _)| c),
        "value": drill.map(|(_, v)| v),
        "filtersSkipped": sql.skipped,
    });
    body[relation.label.0] = json!(relation.label.1);
    Ok(body)
}

/// The statements of a raw table's page (`BI-16` part A): its columns and sort
/// from the query, checked against the relation like a saved definition is.
fn rows_statements(
    q: &RecordsQuery,
    relation: &RecordsRelation,
    filters: &[FilterDef],
    page: (u32, u64),
    time: &TimeContext,
) -> Result<lakehouse_bi::builder::RecordsSql, ApiError> {
    let fields = lakehouse_bi::tables::TableFields {
        columns: Some(
            q.columns
                .as_deref()
                .unwrap_or_default()
                .split(',')
                .map(str::to_owned)
                .collect(),
        ),
        sort_column: q.sort_column.clone().filter(|c| !c.is_empty()),
        sort_dir: q.sort_dir.clone().filter(|d| !d.is_empty()),
        ..lakehouse_bi::tables::TableFields::default()
    };
    let known: std::collections::HashSet<String> = relation.cols.keys().cloned().collect();
    let plan = lakehouse_bi::tables::plan_rows(&fields, &known).map_err(ApiError::BadRequest)?;
    Ok(lakehouse_bi::builder::rows_page_sql(
        &relation.from,
        &relation.cols,
        filters,
        &plan,
        page,
        time,
    ))
}

// ── /api/dashboard/values ───────────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/values`.
#[derive(Debug, Default, Deserialize)]
pub struct ValuesQuery {
    #[serde(default)]
    column: Option<String>,
    /// Case-insensitive substring to look for in the values.
    #[serde(default)]
    q: Option<String>,
    /// Read the values from the relations this board's charts read, SQL
    /// sources included. Absent: every `serving` mart with the column.
    #[serde(default)]
    board: Option<String>,
    /// The dashboard's other active filters (JSON list), applied to every
    /// relation that has their column so the list narrows. Only read with
    /// `board`; the filter on `column` itself is ignored.
    #[serde(default)]
    filters: Option<String>,
}

/// Values a list returns at most; one more is fetched to know whether the
/// list was cut.
const VALUES_LIMIT: usize = 200;
/// Longest search text, in characters.
const VALUES_SEARCH_MAX_CHARS: usize = 200;

/// `GET /api/dashboard/values?column=` — the distinct values of `column`
/// (the dashboard filter picker): across every `serving` mart that has it,
/// or, with `board`, across the relations that board's charts read.
///
/// Unlike `/fields` (schema metadata only), this reads DATA, so it goes
/// through `policy_engine::rewrite_sql_for_roles` for the caller's roles
/// like the tiles do: a masked column's values, or rows a row filter hides,
/// used to be listed here in the clear (plan §8 item 3). The search text and
/// the linked filters are part of the statement that is rewritten, so they
/// compare against the masked value, never the clear one. `ClickHouse`
/// errors are classified, never forwarded.
pub async fn values(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<ValuesQuery>,
) -> ApiResult<ApiJson<Value>> {
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    let time = crate::routes::settings::time_context(&state).await?;
    values_for_roles(
        &state.clickhouse,
        q,
        &principal.role_names,
        &placeholders,
        &obligations,
        &time,
    )
    .await
    .map(ApiJson)
    .map_err(Into::into)
}

/// `SELECT DISTINCT toString(column) AS v FROM <from> [WHERE ...]`, with the
/// search text and the relation's applicable filter predicates.
fn values_select(
    column: &str,
    from: &str,
    search: Option<&str>,
    mut predicates: Vec<String>,
    other_columns: &[String],
) -> String {
    if let Some(text) = search {
        predicates.push(format!(
            "positionCaseInsensitiveUTF8(toString({column}), {}) > 0",
            lakehouse_core::ident::SqlLiteral::from(text)
        ));
    }
    if predicates.is_empty() {
        return format!("SELECT DISTINCT toString({column}) AS v FROM {from}");
    }
    let where_sql = predicates.join(" AND ");
    // BI-18·A review BLOCKER 1: the output is aliased `v`, so a predicate on
    // a column that is itself named `v` would resolve to the alias in WHERE.
    // Only then are the predicates applied in an inner relation.
    let reads_v = column == "v" || other_columns.iter().any(|c| c == "v");
    if reads_v {
        return format!(
            "SELECT DISTINCT toString({column}) AS v FROM (SELECT * FROM {from} WHERE {where_sql}) AS flt"
        );
    }
    format!("SELECT DISTINCT toString({column}) AS v FROM {from} WHERE {where_sql}")
}

/// The statement over every relation's `SELECT`, fetching one value more
/// than [`VALUES_LIMIT`] so the caller can tell the list was cut.
fn values_statement(selects: &[String], settings: &str) -> String {
    let union = selects.join(" UNION DISTINCT ");
    format!(
        "SELECT v FROM ({union}) WHERE v != '' ORDER BY v LIMIT {}{settings}",
        VALUES_LIMIT + 1
    )
}

/// The relations a board's charts read that have `column`, as `SELECT`s for
/// [`values_statement`], and whether one is a SQL source (which brings the
/// source `SETTINGS` caps along).
///
/// A mart is read as `serving.<mart>`; a source goes through the same
/// `check_sql_source` gate a tile's source passed when it was saved and is
/// wrapped as the tile wraps it ([`lakehouse_bi::builder::Relation::render`]),
/// so the later role rewrite governs it identically. A source the gate
/// refuses is left out, never run.
async fn board_value_selects(
    ch: &ChClient,
    board: &str,
    column: &str,
    search: Option<&str>,
    other_filters: &[FilterDef],
    time: &TimeContext,
) -> Result<(Vec<String>, bool), ApiError> {
    let charts = store::list_stored_charts(ch)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    let on_board: Vec<&StoredChartSpec> = charts
        .iter()
        .filter(|c| board == "all" || c.board == board)
        .filter(|c| c.spec.kind != ChartKind::Text)
        .collect();
    let mart_cols = mart_columns(ch)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    let sources = sources_for(ch, on_board.iter().copied())
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;

    let mut selects = Vec::new();
    let mut has_source = false;
    let mut seen: HashSet<String> = HashSet::new();
    for c in on_board {
        let (key, from, cols) = if let Some(id) = c.def.sql_source.as_deref() {
            let Some(source) = sources.get(id) else {
                continue;
            };
            let Ok(sql) = check_sql_source(&source.sql) else {
                continue;
            };
            let from = lakehouse_bi::builder::Relation::Sql(sql).render();
            (format!("s:{id}"), from, source.column_kinds())
        } else {
            let mart = c.def.mart.as_str();
            let (Ok(ident), Some(cols)) = (
                lakehouse_core::ident::Ident::new(mart.to_owned()),
                mart_cols.get(mart),
            ) else {
                continue;
            };
            (
                format!("m:{mart}"),
                lakehouse_bi::builder::Relation::Mart(ident).render(),
                cols.clone(),
            )
        };
        if !cols.contains_key(column) || !seen.insert(key) {
            continue;
        }
        has_source |= c.def.sql_source.is_some();
        let outcome = lakehouse_bi::builder::filter_predicates(&cols, &[], other_filters, time);
        selects.push(values_select(
            column,
            &from,
            search,
            outcome.predicates,
            &outcome.columns,
        ));
    }
    Ok((selects, has_source))
}

/// [`values`]' body, taking roles and placeholders explicitly so the
/// enforcement path can be tested against a mock `ClickHouse` and a real
/// policy row.
async fn values_for_roles(
    ch: &ChClient,
    q: ValuesQuery,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &PolicyEngineObligations<'_>,
    time: &TimeContext,
) -> Result<Value, ApiError> {
    let column = strip_non_ident(q.column.as_deref().unwrap_or(""));
    if column.is_empty() {
        return Err(ApiError::BadRequest("column is required".to_owned()));
    }
    let search = q.q.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if search.is_some_and(|t| t.chars().count() > VALUES_SEARCH_MAX_CHARS) {
        return Err(ApiError::BadRequest(format!(
            "q is at most {VALUES_SEARCH_MAX_CHARS} characters"
        )));
    }
    let linked = parse_filters_param(q.filters.as_deref())?.unwrap_or_default();
    let empty = || json!({ "column": column, "values": Vec::<String>::new(), "truncated": false });

    let (selects, settings) = if let Some(board) = q.board.as_deref().filter(|b| !b.is_empty()) {
        let others: Vec<FilterDef> = linked.into_iter().filter(|f| f.column != column).collect();
        let (selects, has_source) =
            board_value_selects(ch, board, &column, search, &others, time).await?;
        let settings = if has_source {
            lakehouse_bi::builder::SQL_SOURCE_SETTINGS
        } else {
            ""
        };
        (selects, settings)
    } else {
        let marts_sql = format!(
            "SELECT table FROM system.columns WHERE database='serving' AND name='{column}' AND \
             table NOT LIKE '%\\_baru'"
        );
        let marts = ch
            .rows(&marts_sql, None)
            .await
            .map_err(|err| classify_dashboard_ch_error(&err))?;
        let selects = marts
            .iter()
            .map(|m| {
                let table = strip_non_ident(m.get("table").and_then(Value::as_str).unwrap_or(""));
                values_select(
                    &column,
                    &format!("serving.{table}"),
                    search,
                    Vec::new(),
                    &[],
                )
            })
            .collect();
        (selects, "")
    };
    if selects.is_empty() {
        return Ok(empty());
    }
    let sql = values_statement(&selects, settings);
    let rewritten = crate::policy_engine::rewrite_sql_for_roles(
        &sql,
        &sqlparser::dialect::ClickHouseDialect {},
        roles,
        placeholders,
        obligations,
    )
    .await
    .map_err(|err| {
        ApiError::Unprocessable(crate::policy_engine::enforcement_error_message(&err).to_owned())
    })?;
    let rows = ch
        .rows(&rewritten, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    let mut values: Vec<String> = rows
        .iter()
        .filter_map(|r| r.get("v").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    let truncated = values.len() > VALUES_LIMIT;
    values.truncate(VALUES_LIMIT);
    Ok(json!({ "column": column, "values": values, "truncated": truncated }))
}

// ── /api/dashboard/export ───────────────────────────────────────────────

/// `GET /api/dashboard/export` — every board + stored chart, as a minimal
/// hand-rolled `YAML` document. The only non-`JSON` response in this crate:
/// returned directly as `text/yaml`, bypassing [`ApiJson`].
///
/// # Errors
///
/// 500 (`ApiError::Internal`) if either `ClickHouse` listing fails.
pub async fn export(State(state): State<AppState>) -> ApiResult<Response> {
    let (charts, boards, default_row) = tokio::try_join!(
        store::list_stored_charts(&state.clickhouse),
        store::list_boards(&state.clickhouse),
        store::get_board(&state.clickhouse, store::DEFAULT_BOARD_ID)
    )
    .map_err(|err| {
        upstream_error::ch_error_as(
            &upstream_error::DATABASE,
            &err,
            upstream_error::FailedAs::Internal,
        )
    })?;

    let mut out = String::new();
    out.push_str("# RantAI Lakehouse — dashboard as code\n");
    out.push_str("# boards & chart specs, exported from console.bi_chart\n\n");
    out.push_str("boards:\n");
    out.push_str(&yaml_board(
        store::DEFAULT_BOARD_ID,
        "Main",
        default_row.as_ref().and_then(|b| b.layout.as_ref()),
    ));
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
                // BI-16 part A: a pivot's `values` are objects. Compact JSON
                // is a valid YAML flow mapping, so they are written as is.
                let rendered = items
                    .iter()
                    .map(|i| match i {
                        Value::Object(_) => i.to_string(),
                        _ => yaml_value(i),
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                lines.push(format!("  {k}: [{rendered}]"));
            }
            Value::Object(_) => lines.push(format!("  {k}: {val}")),
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
            push_table_fields(def, &mut out);
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
            // Map kinds only; `None` for every other chart, so the exported
            // shape of existing charts does not change.
            if let Some(map) = &def.map {
                out.push(("map", json!(map)));
            }
            if let Some(lat) = &def.lat {
                out.push(("lat", json!(lat)));
            }
            if let Some(lon) = &def.lon {
                out.push(("lon", json!(lon)));
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
            // BI-9: only grained charts, so the exported shape of every
            // earlier chart does not change.
            if let Some(grain) = &def.grain {
                out.push(("grain", json!(grain)));
            }
            push_table_fields(def, &mut out);
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

/// The `BI-16` part A fields of a definition (raw table, pivot, comparison),
/// present only on the charts that carry them, so the exported shape of every
/// earlier chart does not change.
fn push_table_fields(def: &ChartInput, out: &mut Vec<(&'static str, Value)>) {
    const NAMES: [&str; 10] = [
        "tableMode",
        "columns",
        "rows",
        "values",
        "totals",
        "sortColumn",
        "sortDir",
        "columnSettings",
        "compare",
        "goodDirection",
    ];
    let Ok(Value::Object(fields)) = serde_json::to_value(&def.tables) else {
        return;
    };
    for name in NAMES {
        if let Some(value) = fields.get(name) {
            out.push((name, value.clone()));
        }
    }
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
            ApiJson(json!({ "error": "invalid dashboard" })),
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
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
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
    let enabled = board.embed_enabled.unwrap_or(false);
    let max_lifetime = state.config.embed_token_max_lifetime_seconds;
    // SEC-12: what the Share dialog shows beside the sample token. `null`
    // when nothing has been withdrawn, so the dialog does not print "1970".
    let revoked_before = if board.embed_access.revoked_before == 0 {
        Value::Null
    } else {
        json!(board.embed_access.revoked_before)
    };
    let access = json!({
        "maxLifetimeSeconds": max_lifetime,
        "revokedBefore": revoked_before,
        "allowedOrigins": board.embed_access.origins,
    });
    let with_access = |mut body: Value| {
        if let (Some(map), Some(extra)) = (body.as_object_mut(), access.as_object()) {
            map.extend(extra.clone());
        }
        body
    };
    // SEC-12: no secret, no signed embedding. Said as `supported: false` with
    // the reason, and no sample token (nothing can sign one).
    let Some(secret) = state.config.embed_secret.as_deref() else {
        return Ok(Some(with_access(json!({
            "enabled": enabled,
            "supported": false,
            "reason": crate::routes::embed::EMBEDDING_NOT_CONFIGURED,
        }))));
    };
    let iat = now_unix_seconds();
    // The sample lives an hour, or the configured maximum when that is lower:
    // a sample the API would refuse is no sample.
    #[allow(
        clippy::cast_precision_loss,
        reason = "a lifetime limit in seconds; far below 2^53"
    )]
    let lifetime = 3600.0_f64.min(max_lifetime as f64);
    let claims = lakehouse_embed::EmbedClaims {
        resource: Some(lakehouse_embed::EmbedResource {
            dashboard: Some(id.to_owned()),
        }),
        params: Some(HashMap::new()),
        exp: Some(iat + lifetime),
        // SEC-12: a token must carry `iat` and `exp`; the sample also gets a
        // `jti` so the Share dialog can show a token that can be withdrawn.
        iat: Some(iat),
        jti: Some(uuid::Uuid::new_v4().simple().to_string()),
    };
    let sample_token = lakehouse_embed::sign_embed(&claims, secret);
    // D2: `secret` is deliberately NOT included in the response — see this
    // function's doc comment. Only `enabled` and a freshly-signed
    // `sampleToken` (the legitimate use case the secret used to serve) go
    // over the wire.
    Ok(Some(with_access(json!({
        "enabled": enabled,
        "supported": true,
        "sampleToken": sample_token,
    }))))
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
    fn board_edit_allows_only_a_layout_on_the_default_board() {
        let body = |json: &str| serde_json::from_str::<BoardEditBody>(json).unwrap();
        assert!(board_edit_allowed(&body(r#"{"id":"default","layout":{}}"#)));
        assert!(!board_edit_allowed(&body(r#"{"id":"default","name":"x"}"#)));
        assert!(!board_edit_allowed(&body(
            r#"{"id":"default","public":true}"#
        )));
        assert!(!board_edit_allowed(&body(
            r#"{"id":"default","layout":{},"embed":true}"#
        )));
        assert!(board_edit_allowed(&body(
            r#"{"id":"b_1","name":"x","public":true}"#
        )));
        assert!(!board_edit_allowed(&body(r#"{"layout":{}}"#)));
        // The built-in board lives outside the folder tree.
        assert!(!board_edit_allowed(&body(
            r#"{"id":"default","folderId":"f_1"}"#
        )));
        assert!(board_edit_allowed(&body(
            r#"{"id":"b_1","folderId":"f_1"}"#
        )));
    }

    #[test]
    fn the_saved_refresh_is_a_listed_interval_or_a_400_of_ours() {
        for ok in [0_u32, 60, 300, 600, 900, 1800, 3600] {
            assert_eq!(checked_refresh(Some(i64::from(ok))).unwrap(), Some(ok));
        }
        assert_eq!(checked_refresh(None).unwrap(), None);
        for bad in [-1, 1, 59, 3601, i64::MAX, i64::MIN] {
            let err = checked_refresh(Some(bad)).unwrap_err();
            assert!(
                matches!(err, ApiError::BadRequest(ref m) if m.starts_with("refreshSeconds must be one of 0, 60, 300")),
                "{bad}: {err:?}"
            );
        }
    }

    #[test]
    fn the_built_in_board_may_save_a_refresh_as_it_may_save_a_layout() {
        let body: BoardEditBody =
            serde_json::from_str(r#"{"id":"default","refreshSeconds":300}"#).unwrap();
        assert!(board_edit_allowed(&body));
        assert_eq!(body.refresh_seconds, Some(300));
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

#[cfg(test)]
mod records_enforcement {
    //! Drill-down review fix: `/api/dashboard/records` must go through the
    //! same `policy_engine::rewrite_sql_for_roles` path as the tiles. Same
    //! two-harness shape as `support::run_spec_sql_enforcement`: a real
    //! (ephemeral) Postgres with a real policy row, and a wiremock
    //! `ClickHouse`.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use lakehouse_core::ApiError;
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::{self, CreatePolicyInput};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{RecordsQuery, records_for_roles};
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    fn drill(mart: &str, column: &str, value: &str) -> RecordsQuery {
        RecordsQuery {
            mart: Some(mart.to_owned()),
            column: Some(column.to_owned()),
            value: Some(value.to_owned()),
            ..RecordsQuery::default()
        }
    }

    /// Answers both `system.columns` lookups the drill-down makes: the
    /// route's own column check (reads `name`/`type`) and the policy engine's
    /// column resolution (reads `name`/`default_*`), and the total (`count()`).
    async fn mount_mart_columns(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("count() AS n"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "n", "type": "UInt64"}],
                "data": [{"n": "1"}],
                "rows": 1,
            })))
            .mount(server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "type", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "id", "type": "UInt64", "default_kind": "", "default_expression": ""},
                    {"name": "email", "type": "String", "default_kind": "", "default_expression": ""},
                    {"name": "region", "type": "String", "default_kind": "", "default_expression": ""},
                ],
                "rows": 3,
            })))
            .mount(server)
            .await;
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn drill_down_rows_are_masked_like_the_tile_they_came_from(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "drill-down-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");

        let server = MockServer::start().await;
        mount_mart_columns(&server).await;
        Mock::given(method("POST"))
            .and(body_string_contains("replaceRegexpOne"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "id", "type": "UInt64"},
                    {"name": "email", "type": "String"},
                    {"name": "region", "type": "String"},
                ],
                "data": [{"id": "1", "email": "***", "region": "north"}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let body = records_for_roles(
            &ch,
            drill("mart_x", "region", "north"),
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect("a masked drill-down still answers");

        assert_eq!(body["rows"][0]["email"], "***");
        let requests = server
            .received_requests()
            .await
            .expect("mock server records requests");
        let row_queries: Vec<String> = requests
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| b.contains("north"))
            .collect();
        assert!(
            !row_queries.is_empty(),
            "the row query must reach ClickHouse"
        );
        assert!(
            row_queries
                .iter()
                .all(|b| b.contains("replaceRegexpOne(toString(`email`)")),
            "every drill-down row query must be the masked rewrite, never the raw SELECT *: {row_queries:?}"
        );
        // BI-18·B: the page order survives the rewrite, and the total is
        // masked and row-filtered through the same path.
        assert!(
            row_queries
                .iter()
                .any(|b| b.contains("count() AS n") && b.contains("replaceRegexpOne")),
            "{row_queries:?}"
        );
        assert!(
            row_queries
                .iter()
                .filter(|b| !b.contains("count() AS n"))
                .all(|b| b.contains("ORDER BY ALL")),
            "{row_queries:?}"
        );
        Ok(())
    }

    /// BI-16 part A, plan section 4: a raw table that lists a masked column,
    /// or sorts by it, still gets the mask. The page and the total go through
    /// the same rewrite as a drill-down.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_raw_table_page_masks_a_listed_and_a_sorted_column(pool: PgPool) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "raw-table-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");

        let server = MockServer::start().await;
        mount_mart_columns(&server).await;
        Mock::given(method("POST"))
            .and(body_string_contains("replaceRegexpOne"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "id", "type": "UInt64"}, {"name": "email", "type": "String"}],
                "data": [{"id": "1", "email": "***"}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
        let q = RecordsQuery {
            mart: Some("mart_x".to_owned()),
            columns: Some("id,email".to_owned()),
            sort_column: Some("email".to_owned()),
            sort_dir: Some("desc".to_owned()),
            ..RecordsQuery::default()
        };

        let body = records_for_roles(
            &ch,
            q,
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect("a masked raw-table page still answers");

        assert_eq!(body["rows"][0]["email"], "***");
        let requests = server.received_requests().await.expect("requests recorded");
        let bodies: Vec<String> = requests
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| b.contains("serving.mart_x") && !b.contains("system."))
            .collect();
        assert!(!bodies.is_empty(), "the page must reach ClickHouse");
        assert!(
            bodies
                .iter()
                .all(|b| b.contains("replaceRegexpOne(toString(`email`)")),
            "every statement must carry the mask, listed or sorted: {bodies:?}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn drill_down_clickhouse_error_text_never_reaches_the_response(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        let server = MockServer::start().await;
        mount_mart_columns(&server).await;
        Mock::given(method("POST"))
            .and(body_string_contains("north"))
            .respond_with(
                ResponseTemplate::new(500)
                    .set_body_string("Code: 60. DB::Exception: upstream-secret-detail"),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let err = records_for_roles(
            &ch,
            drill("mart_x", "region", "north"),
            &[],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect_err("a failing row query is an error");

        let text = format!("{err:?}");
        assert!(
            !text.contains("upstream-secret-detail"),
            "ClickHouse's own text leaked: {text}"
        );
        assert!(
            // SEC-11: the fixed message now carries a reference id after it.
            matches!(err, ApiError::Unprocessable(ref m) if m.starts_with("The dashboard query failed. Reference: ")),
            "expected the fixed 422, got {err:?}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod values_enforcement {
    //! `/api/dashboard/values` reads data, so it is governed like the tiles
    //! (plan §8 item 3). Same two-harness shape as `records_enforcement`.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use lakehouse_store::PgPool;
    use lakehouse_store::governance::{self, CreatePolicyInput};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{ValuesQuery, values_for_roles};
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    #[sqlx::test(migrations = "../../migrations")]
    async fn filter_values_of_a_masked_column_are_masked(pool: PgPool) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "values-masking-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
        let server = MockServer::start().await;
        // Which marts have the column.
        Mock::given(method("POST"))
            .and(body_string_contains("SELECT table FROM system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "table", "type": "String"}],
                "data": [{"table": "mart_x"}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        // The policy engine's column resolution.
        Mock::given(method("POST"))
            .and(body_string_contains("default_kind"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "region", "default_kind": "", "default_expression": ""},
                ],
                "rows": 2,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("replaceRegexpOne"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "v", "type": "String"}],
                "data": [{"v": "***"}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let body = values_for_roles(
            &ch,
            ValuesQuery {
                column: Some("email".to_owned()),
                ..ValuesQuery::default()
            },
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect("masked values still answer");

        assert_eq!(body["values"], serde_json::json!(["***"]));
        let distinct_queries: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| b.contains("DISTINCT"))
            .collect();
        assert!(!distinct_queries.is_empty());
        assert!(
            distinct_queries
                .iter()
                .all(|b| b.contains("replaceRegexpOne(toString(`email`)")),
            "every DISTINCT read must be the masked rewrite: {distinct_queries:?}"
        );
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_search_over_a_masked_column_compares_the_masked_value_not_the_clear_one(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "values-masking-search-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"]}"#.to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
        let server = MockServer::start().await;
        // Which marts have the column.
        Mock::given(method("POST"))
            .and(body_string_contains("SELECT table FROM system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "table", "type": "String"}],
                "data": [{"table": "mart_x"}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        // The policy engine's column resolution.
        Mock::given(method("POST"))
            .and(body_string_contains("default_kind"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "region", "default_kind": "", "default_expression": ""},
                ],
                "rows": 2,
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(body_string_contains("replaceRegexpOne"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [{"name": "v", "type": "String"}],
                "data": [{"v": "***"}],
                "rows": 1,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);

        let body = values_for_roles(
            &ch,
            ValuesQuery {
                column: Some("email".to_owned()),
                q: Some("secret".to_owned()),
                ..ValuesQuery::default()
            },
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
        .expect("masked values still answer");

        assert_eq!(body["values"], serde_json::json!(["***"]));
        let reads: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .filter(|b| b.contains("DISTINCT"))
            .collect();
        assert!(!reads.is_empty());
        for sql in &reads {
            // The search runs over the masked source, so typing a clear value
            // cannot confirm that it exists behind the mask.
            let mask = sql.find("replaceRegexpOne").expect("masked");
            let search = sql.find("'secret'").expect("search literal");
            assert!(
                mask < search,
                "the mask must wrap the column before the search: {sql}"
            );
        }
        Ok(())
    }

    /// BI-18·A review BLOCKER 1: a tile whose filter sits in an inner
    /// `SELECT *` (the shape built when a filtered column is also an output
    /// alias) still has its row filter and mask applied to the base table.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_tile_with_its_filter_in_an_inner_select_is_still_masked_and_row_filtered(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        governance::create_policy(
            &pool,
            &CreatePolicyInput {
                name: "wrapped-tile-test".to_owned(),
                kind: "Row filter".to_owned(),
                subjects: "Analyst".to_owned(),
                resources: "serving.mart_x".to_owned(),
                effect: "Permit with obligation".to_owned(),
                conditions: Some(
                    r#"{"roles":["Analyst"],"table":"serving.mart_x","mask":["email"],"rowFilter":"region = 'north'"}"#
                        .to_owned(),
                ),
                activate: true,
                owner: None,
            },
        )
        .await
        .expect("seeding the governing policy must succeed");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("default_kind"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "region", "default_kind": "", "default_expression": ""},
                    {"name": "visitors", "default_kind": "", "default_expression": ""},
                ],
                "rows": 3,
            })))
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
        let sql = "SELECT region, round(sum(visitors)) AS visitors FROM \
                   (SELECT * FROM serving.mart_x WHERE visitors >= 2000) AS flt \
                   GROUP BY region ORDER BY region LIMIT 20";

        let out = crate::policy_engine::rewrite_sql_for_roles(
            sql,
            &sqlparser::dialect::ClickHouseDialect {},
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
        )
        .await
        .expect("the wrapped shape must rewrite");

        assert!(
            out.contains("region = 'north'"),
            "row filter missing: {out}"
        );
        assert!(out.contains("replaceRegexpOne"), "mask missing: {out}");
        assert!(out.contains("visitors >= 2000"), "tile filter lost: {out}");
        // The policy sits on the base table, i.e. inside the inner SELECT.
        let inner = out.find("AS flt").expect("wrapper kept");
        assert!(out.find("region = 'north'").unwrap() < inner, "{out}");
        Ok(())
    }
}

#[cfg(test)]
mod typed_filters {
    //! BI-18 part A: typed filters on the dashboard payload, and the filter
    //! value list's search / linking / truncation. A wiremock `ClickHouse`
    //! answers the store reads and records every statement it is sent; there
    //! is no Postgres, so no governance policy applies (the masking test
    //! lives in `values_enforcement`).

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use lakehouse_core::ApiError;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        DashboardQuery, GrainRequest, ValuesQuery, get_body, parse_filters_param, values_for_roles,
        values_select,
    };
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    async fn answer(server: &MockServer, statement_has: &str, meta: &[&str], data: Vec<Value>) {
        let meta: Vec<Value> = meta
            .iter()
            .map(|n| json!({ "name": n, "type": "String" }))
            .collect();
        let rows = data.len();
        Mock::given(method("POST"))
            .and(body_string_contains(statement_has))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": meta, "data": data, "rows": rows })),
            )
            .mount(server)
            .await;
    }

    /// Anything not matched above (DDL, tile queries): success, no rows.
    async fn answer_rest(server: &MockServer) {
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
    }

    async fn statements(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    fn chart_row(id: &str, board: &str, mart: &str, source: Option<&str>) -> Value {
        let mut spec = json!({
            "id": id, "title": id, "kind": "bar", "mart": mart,
            "sql": "SELECT 1", "x": "kab", "y": "visitors"
        });
        let mut def = json!({
            "title": id, "mart": mart, "kind": "bar",
            "dimension": "kab", "measures": ["visitors"]
        });
        if let Some(s) = source {
            spec["sqlSource"] = json!(s);
            def["sqlSource"] = json!(s);
        }
        json!({
            "id": id, "board": board, "created_by": "ui", "created_at": "2026-01-01 00:00:00",
            "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
        })
    }

    fn mart_column_rows(table: &str, cols: &[(&str, &str)]) -> Vec<Value> {
        cols.iter()
            .map(|(name, ty)| json!({ "table": table, "name": name, "type": ty }))
            .collect()
    }

    fn client(server: &MockServer) -> ChClient {
        ChClient::new(server.uri(), "default".to_owned(), String::new())
    }

    /// Board `b1`: a tile on `mart_a` (has `day`, `visitors`, `provinsi`,
    /// `kab`), one on `mart_b` (no `day`), one on SQL source `s_1`
    /// (`day`, `visitors`, `channel`).
    async fn mount_board(server: &MockServer) {
        answer(
            server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![
                chart_row("c_a", "b1", "mart_a", None),
                chart_row("c_b", "b1", "mart_b", None),
                chart_row("c_s", "b1", "", Some("s_1")),
            ],
        )
        .await;
        answer(server, "FROM console.bi_board FINAL", &["id"], vec![]).await;
        answer(
            server,
            "FROM console.bi_source FINAL",
            &["id"],
            vec![json!({
                "id": "s_1", "title": "src", "sql": "SELECT day, visitors, channel FROM serving.mart_a",
                "columns_json": json!([
                    {"name": "day", "type": "Date"},
                    {"name": "visitors", "type": "UInt32"},
                    {"name": "channel", "type": "String"},
                ]).to_string(),
                "folder_id": "", "created_by": "u", "updated_at": "2026-01-01 00:00:00",
            })],
        )
        .await;
        let mut cols = mart_column_rows(
            "mart_a",
            &[
                ("day", "Date"),
                ("visitors", "UInt32"),
                ("provinsi", "String"),
                ("kab", "LowCardinality(String)"),
            ],
        );
        cols.extend(mart_column_rows(
            "mart_b",
            &[("visitors", "UInt32"), ("kab", "String")],
        ));
        answer(
            server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            cols,
        )
        .await;
    }

    async fn dashboard(server: &MockServer, filters: &str) -> Value {
        let ch = client(server);
        let obligations = PolicyEngineObligations::new(None, &ch);
        let q = DashboardQuery {
            board: Some("b1".to_owned()),
            year: None,
            filters: Some(filters.to_owned()),
            grain: None,
        };
        let parsed = parse_filters_param(q.filters.as_deref()).expect("valid filters");
        get_body(
            &ch,
            &q,
            parsed,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            (
                &lakehouse_bi::grain::TimeContext::default(),
                GrainRequest::Saved,
            ),
        )
        .await
        .expect("the payload builds")
    }

    #[test]
    fn a_malformed_filters_query_is_a_400_with_our_message_not_an_ignored_filter() {
        for bad in [
            "not json",
            r#"[{"column":"c","op":"regex","values":[]}]"#,
            r#"[{"column":"c","op":"between","min":"2024-02-30"}]"#,
            r#"[{"column":"bad col","values":["a"]}]"#,
            // BI-18 round two: `next` needs `n`, an exclusive end needs its
            // bound, `not_contains` needs text.
            r#"[{"column":"d","op":"relative","anchor":"next","unit":"day"}]"#,
            r#"[{"column":"d","op":"between","max":"3","minExclusive":true}]"#,
            r#"[{"column":"c","op":"not_contains"}]"#,
        ] {
            let err = parse_filters_param(Some(bad)).expect_err(bad);
            assert!(matches!(err, ApiError::BadRequest(_)), "{bad}");
        }
        assert!(parse_filters_param(None).unwrap().is_none());
        assert!(parse_filters_param(Some("  ")).unwrap().is_none());
        assert!(
            parse_filters_param(Some(
                r#"[{"column":"d","op":"relative","anchor":"next","n":7,"unit":"day","required":true},{"column":"p","op":"between","min":"5","minExclusive":true}]"#
            ))
            .unwrap()
            .is_some()
        );
        assert!(
            parse_filters_param(Some(r#"[{"column":"kab","values":["a"]}]"#))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_values_filter_on_a_column_named_v_is_applied_inside_the_relation() {
        // BI-18·A review BLOCKER 1: the output alias is `v`.
        let plain = values_select(
            "kab",
            "serving.m",
            None,
            vec!["x > 1".to_owned()],
            &["x".to_owned()],
        );
        assert_eq!(
            plain,
            "SELECT DISTINCT toString(kab) AS v FROM serving.m WHERE x > 1"
        );
        let wrapped = values_select(
            "kab",
            "serving.m",
            None,
            vec!["v > 1".to_owned()],
            &["v".to_owned()],
        );
        assert_eq!(
            wrapped,
            "SELECT DISTINCT toString(kab) AS v FROM (SELECT * FROM serving.m WHERE v > 1) AS flt"
        );
        let searched = values_select("v", "serving.m", Some("a"), Vec::new(), &[]);
        assert!(searched.contains("(SELECT * FROM serving.m WHERE positionCaseInsensitiveUTF8(toString(v), 'a') > 0) AS flt"), "{searched}");
    }

    #[tokio::test]
    async fn typed_filters_narrow_a_mart_tile_and_a_sql_source_tile() {
        let server = MockServer::start().await;
        mount_board(&server).await;
        answer_rest(&server).await;
        let body = dashboard(
            &server,
            r#"[{"column":"visitors","op":"between","min":"100"},
                {"column":"day","op":"relative","anchor":"last","n":30,"unit":"day"}]"#,
        )
        .await;

        let sent = statements(&server).await;
        let tile = |needle: &str| {
            sent.iter()
                .find(|s| s.contains(needle))
                .cloned()
                .unwrap_or_else(|| panic!("no statement for {needle}: {sent:?}"))
        };
        // BI-9: "today" is the date in the report time zone, not the server's.
        let range = "(day > subtractDays(toDate(now('Asia/Jakarta')), 30) AND day <= toDate(now('Asia/Jakarta')))";
        let mart_a = tile("FROM serving.mart_a WHERE");
        assert!(mart_a.contains("visitors >= 100"), "{mart_a}");
        assert!(mart_a.contains(range), "{mart_a}");
        let source = tile("AS src WHERE");
        assert!(source.contains("visitors >= 100"), "{source}");
        assert!(source.contains(range), "{source}");
        let mart_b = tile("FROM serving.mart_b WHERE");
        assert!(mart_b.contains("visitors >= 100"), "{mart_b}");
        assert!(!mart_b.contains("subtractDays"), "{mart_b}");
        // `mart_b` has no `day`: that tile gets the range only, and says so.
        assert_eq!(
            body["results"]["c_b"]["filtersSkipped"],
            json!([{ "column": "day", "reason": "no_column" }])
        );
        assert!(body["results"]["c_a"].get("filtersSkipped").is_none());
    }

    /// `SEC` review finding from BI-18·A: filter values holding backslashes
    /// and quotes reach `ClickHouse` as the literals they were, after the role
    /// rewrite, and the tile statement keeps its shape.
    #[tokio::test]
    async fn awkward_filter_values_survive_the_rewrite_on_every_tile() {
        let server = MockServer::start().await;
        mount_board(&server).await;
        answer_rest(&server).await;
        // A literal as the rewrite writes it: `\\` for a backslash, `\'` for a quote.
        let written = |v: &str| format!("'{}'", v.replace('\\', "\\\\").replace('\'', "\\'"));
        let values = ["x\\", "\\'", "a\\\\'b", "50%\\_"];
        let filters = json!([
            { "column": "kab", "values": values },
            { "column": "kab", "op": "contains", "text": "q\\" },
        ])
        .to_string();
        let _ = dashboard(&server, &filters).await;

        let list = values
            .iter()
            .map(|v| written(v))
            .collect::<Vec<_>>()
            .join(", ");
        let contains = written("q\\");
        let sent = statements(&server).await;
        let tiles: Vec<&String> = sent
            .iter()
            .filter(|s| s.contains("GROUP BY kab") && s.contains("WHERE kab IN"))
            .collect();
        assert!(!tiles.is_empty());
        for sql in tiles {
            assert!(sql.contains(&format!("kab IN ({list})")), "{sql}");
            assert!(
                sql.contains(&format!(
                    "positionCaseInsensitiveUTF8(toString(kab), {contains}) > 0"
                )),
                "{sql}"
            );
            assert_eq!(sql.matches("GROUP BY").count(), 1, "{sql}");
            assert_eq!(sql.matches("kab IN (").count(), 1, "{sql}");
        }
    }

    #[tokio::test]
    async fn a_filter_that_does_not_fit_the_column_type_is_reported_as_wrong_type() {
        let server = MockServer::start().await;
        mount_board(&server).await;
        answer_rest(&server).await;
        let body = dashboard(
            &server,
            r#"[{"column":"visitors","op":"contains","text":"1"}]"#,
        )
        .await;
        assert_eq!(
            body["results"]["c_a"]["filtersSkipped"],
            json!([{ "column": "visitors", "reason": "wrong_type" }])
        );
    }

    #[tokio::test]
    async fn filter_fields_list_every_column_the_board_reads_with_kind_and_tile_count() {
        let server = MockServer::start().await;
        mount_board(&server).await;
        answer_rest(&server).await;
        let body = dashboard(&server, "[]").await;
        let fields = body["filterFields"].as_array().unwrap();
        let find = |name: &str| fields.iter().find(|f| f["column"] == name).cloned();
        // `visitors` is a measure, not a dimension: it is offered all the same.
        assert_eq!(
            find("visitors"),
            Some(json!({ "column": "visitors", "kind": "number", "tiles": 3 }))
        );
        assert_eq!(
            find("day"),
            Some(json!({ "column": "day", "kind": "date", "tiles": 2 }))
        );
        // A source-only column is offered too.
        assert_eq!(
            find("channel"),
            Some(json!({ "column": "channel", "kind": "text", "tiles": 1 }))
        );
        assert_eq!(
            find("kab"),
            Some(json!({ "column": "kab", "kind": "text", "tiles": 2 }))
        );
        // The legacy list is still there for older consoles.
        assert!(body["filterColumns"].is_array());
    }

    #[tokio::test]
    async fn the_saved_default_is_reported_apart_from_the_filters_in_force() {
        let server = MockServer::start().await;
        answer(
            &server,
            "FROM console.bi_board FINAL",
            &["id", "name", "filters_json"],
            vec![json!({
                "id": "b1", "name": "B",
                "filters_json": r#"[{"column":"kab","values":["Ubud"]}]"#,
            })],
        )
        .await;
        mount_board(&server).await;
        answer_rest(&server).await;
        let body = dashboard(
            &server,
            r#"[{"column":"visitors","op":"between","min":"5"}]"#,
        )
        .await;
        assert_eq!(body["filters"][0]["column"], "visitors");
        assert_eq!(
            body["defaultFilters"],
            json!([{ "column": "kab", "values": ["Ubud"] }])
        );
    }

    fn values_query(column: &str) -> ValuesQuery {
        ValuesQuery {
            column: Some(column.to_owned()),
            ..ValuesQuery::default()
        }
    }

    async fn run_values(server: &MockServer, q: ValuesQuery) -> Result<Value, ApiError> {
        let ch = client(server);
        let obligations = PolicyEngineObligations::new(None, &ch);
        values_for_roles(
            &ch,
            q,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
    }

    /// The statements that read values (`DISTINCT`), after the role rewrite.
    async fn value_reads(server: &MockServer) -> Vec<String> {
        statements(server)
            .await
            .into_iter()
            .filter(|s| s.contains("DISTINCT"))
            .collect()
    }

    async fn mount_marts_with(server: &MockServer, tables: &[&str]) {
        answer(
            server,
            "SELECT table FROM system.columns",
            &["table"],
            tables.iter().map(|t| json!({ "table": t })).collect(),
        )
        .await;
    }

    #[tokio::test]
    async fn a_values_search_is_a_case_insensitive_substring_with_the_text_as_one_literal() {
        let server = MockServer::start().await;
        mount_marts_with(&server, &["mart_a"]).await;
        answer(&server, "DISTINCT", &["v"], vec![json!({ "v": "50%_x" })]).await;
        // Covers a search text holding `%`, `_`, a quote and backslashes (one
        // before the quote, one at the end): the text reaches ClickHouse as
        // one literal with its value intact after the role rewrite, and the
        // statement keeps its shape (`SEC` review finding from BI-18·A).
        let needle = "50%_\\'x\\";
        let body = run_values(
            &server,
            ValuesQuery {
                q: Some(needle.to_owned()),
                ..values_query("kab")
            },
        )
        .await
        .unwrap();
        assert_eq!(body["values"], json!(["50%_x"]));
        // The rewrite writes a literal in its own escaping (`\\` and `\'`).
        let literal = "'50%_\\\\\\'x\\\\'";
        let reads = value_reads(&server).await;
        assert!(!reads.is_empty());
        for sql in &reads {
            assert!(sql.contains("positionCaseInsensitiveUTF8"), "{sql}");
            assert!(sql.contains(literal), "{sql} lacks {literal}");
            assert_eq!(
                sql.matches("positionCaseInsensitiveUTF8").count(),
                1,
                "{sql}"
            );
            assert!(!sql.contains("LIKE"), "no wildcard matching: {sql}");
        }
    }

    #[tokio::test]
    async fn a_values_search_longer_than_200_characters_is_a_400() {
        let server = MockServer::start().await;
        let err = run_values(
            &server,
            ValuesQuery {
                q: Some("x".repeat(201)),
                ..values_query("kab")
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_value_list_is_cut_at_200_and_says_so_only_when_there_were_more() {
        for (rows, truncated, returned) in [(201, true, 200), (200, false, 200), (3, false, 3)] {
            let server = MockServer::start().await;
            mount_marts_with(&server, &["mart_a"]).await;
            answer(
                &server,
                "DISTINCT",
                &["v"],
                (0..rows)
                    .map(|i| json!({ "v": format!("v{i:03}") }))
                    .collect(),
            )
            .await;
            let body = run_values(&server, values_query("kab")).await.unwrap();
            assert_eq!(body["truncated"], truncated, "{rows} rows");
            assert_eq!(body["values"].as_array().unwrap().len(), returned);
            let reads = value_reads(&server).await;
            assert!(reads.iter().all(|s| s.contains("LIMIT 201")), "{reads:?}");
        }
    }

    #[tokio::test]
    async fn another_active_filter_narrows_the_list_and_the_filter_on_the_column_itself_is_ignored()
    {
        let server = MockServer::start().await;
        mount_board(&server).await;
        answer(&server, "DISTINCT", &["v"], vec![json!({ "v": "Ubud" })]).await;
        answer_rest(&server).await;
        let body = run_values(
            &server,
            ValuesQuery {
                board: Some("b1".to_owned()),
                filters: Some(
                    r#"[{"column":"provinsi","values":["Bali"]},
                        {"column":"kab","values":["Ignored"]}]"#
                        .to_owned(),
                ),
                ..values_query("kab")
            },
        )
        .await
        .unwrap();
        assert_eq!(body["values"], json!(["Ubud"]));
        let reads = value_reads(&server).await;
        assert!(!reads.is_empty());
        // `mart_a` has `provinsi`, so it is narrowed; `mart_b` lacks it and
        // is read whole.
        assert!(
            reads
                .iter()
                .any(|s| s.contains("provinsi IN ('Bali')") && s.contains("mart_a")),
            "{reads:?}"
        );
        assert!(reads.iter().all(|s| !s.contains("Ignored")), "{reads:?}");
    }

    #[tokio::test]
    async fn a_malformed_linked_filter_on_the_values_endpoint_is_a_400() {
        let server = MockServer::start().await;
        let err = run_values(
            &server,
            ValuesQuery {
                board: Some("b1".to_owned()),
                filters: Some("nope".to_owned()),
                ..values_query("kab")
            },
        )
        .await
        .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)));
    }

    #[tokio::test]
    async fn a_column_only_a_sql_source_returns_is_listed_through_the_source_guard() {
        let server = MockServer::start().await;
        mount_board(&server).await;
        answer(&server, "DISTINCT", &["v"], vec![json!({ "v": "email" })]).await;
        answer_rest(&server).await;
        let body = run_values(
            &server,
            ValuesQuery {
                board: Some("b1".to_owned()),
                ..values_query("channel")
            },
        )
        .await
        .unwrap();
        assert_eq!(body["values"], json!(["email"]));
        let reads = value_reads(&server).await;
        assert_eq!(reads.len(), 1, "only the source has `channel`: {reads:?}");
        assert!(reads[0].contains("AS src"), "{}", reads[0]);
        assert!(
            reads[0].contains("SETTINGS"),
            "source caps ride along: {}",
            reads[0]
        );
    }

    #[tokio::test]
    async fn a_source_the_sql_guard_refuses_is_left_out_of_the_value_list() {
        let server = MockServer::start().await;
        answer(
            &server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![chart_row("c_s", "b1", "", Some("s_1"))],
        )
        .await;
        answer(
            &server,
            "FROM console.bi_source FINAL",
            &["id"],
            vec![json!({
                "id": "s_1", "title": "bad", "sql": "DROP TABLE serving.mart_a",
                "columns_json": json!([{"name": "channel", "type": "String"}]).to_string(),
                "folder_id": "", "created_by": "u", "updated_at": "2026-01-01 00:00:00",
            })],
        )
        .await;
        answer(
            &server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            vec![],
        )
        .await;
        answer_rest(&server).await;
        let body = run_values(
            &server,
            ValuesQuery {
                board: Some("b1".to_owned()),
                ..values_query("channel")
            },
        )
        .await
        .unwrap();
        assert_eq!(body["values"], json!([]));
        assert!(value_reads(&server).await.is_empty());
    }
}

#[cfg(test)]
mod records_pages {
    //! BI-18·B: the records list is paged, counted, filtered and works on a
    //! SQL source. A wiremock `ClickHouse` answers and records every
    //! statement; there is no Postgres, so the masking test stays in
    //! `records_enforcement`.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_clickhouse::ChClient;
    use lakehouse_core::ApiError;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{RecordsQuery, records_for_roles};
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    async fn answer(server: &MockServer, statement_has: &str, data: Vec<Value>) {
        let rows = data.len();
        Mock::given(method("POST"))
            .and(body_string_contains(statement_has))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({ "meta": [{"name": "n", "type": "String"}], "data": data, "rows": rows }),
            ))
            .mount(server)
            .await;
    }

    /// Anything not matched above (the store's DDL): success, no rows.
    async fn answer_rest(server: &MockServer) {
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
    }

    /// A mart `mart_x` (`region` text, `visitors` number) with a page of rows
    /// and a total of 120; every other statement answers with no rows.
    async fn mount_mart(server: &MockServer) {
        answer(server, "count() AS n", vec![json!({ "n": "120" })]).await;
        answer(
            server,
            "FROM serving.mart_x",
            vec![json!({ "region": "north" }), json!({ "region": "north" })],
        )
        .await;
        answer(
            server,
            "FROM system.columns",
            vec![
                json!({ "name": "region", "type": "String" }),
                json!({ "name": "visitors", "type": "UInt32" }),
            ],
        )
        .await;
    }

    async fn statements(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    /// The row statement and the count statement the route sent.
    async fn page_and_count(server: &MockServer) -> (String, String) {
        let all = statements(server).await;
        let rows = all.iter().find(|s| s.contains("ORDER BY ALL")).unwrap();
        let count = all.iter().find(|s| s.contains("count() AS n")).unwrap();
        (rows.clone(), count.clone())
    }

    fn mart_query(column: Option<&str>, value: Option<&str>) -> RecordsQuery {
        RecordsQuery {
            mart: Some("mart_x".to_owned()),
            column: column.map(ToOwned::to_owned),
            value: value.map(ToOwned::to_owned),
            ..RecordsQuery::default()
        }
    }

    async fn run(server: &MockServer, q: RecordsQuery) -> Result<Value, ApiError> {
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(None, &ch);
        records_for_roles(
            &ch,
            q,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            &lakehouse_bi::grain::TimeContext::default(),
        )
        .await
    }

    #[tokio::test]
    async fn a_mart_answers_a_page_and_the_total_and_offset_moves_the_page() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        let q = RecordsQuery {
            offset: Some("100".to_owned()),
            ..mart_query(Some("region"), Some("north"))
        };

        let body = run(&server, q).await.unwrap();

        assert_eq!(body["total"], 120);
        assert_eq!(body["limit"], 50);
        assert_eq!(body["offset"], 100);
        assert_eq!(body["rows"].as_array().unwrap().len(), 2);
        let (rows, count) = page_and_count(&server).await;
        assert!(rows.contains("LIMIT 50 OFFSET 100"), "{rows}");
        assert!(rows.contains("region = 'north'"), "{rows}");
        assert!(count.contains("region = 'north'"), "{count}");
    }

    #[tokio::test]
    async fn a_limit_over_the_page_size_is_clamped() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        let q = RecordsQuery {
            limit: Some("500".to_owned()),
            ..mart_query(Some("region"), Some("north"))
        };

        let body = run(&server, q).await.unwrap();

        assert_eq!(body["limit"], 50);
        let (rows, _) = page_and_count(&server).await;
        assert!(rows.contains("LIMIT 50 OFFSET 0"), "{rows}");
    }

    #[tokio::test]
    async fn the_active_filters_are_in_both_the_row_and_the_count_statement() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        let q = RecordsQuery {
            filters: Some(r#"[{"column":"visitors","op":"between","min":"7"}]"#.to_owned()),
            ..mart_query(Some("region"), Some("north"))
        };

        run(&server, q).await.unwrap();

        let (rows, count) = page_and_count(&server).await;
        assert!(rows.contains("visitors >= 7"), "{rows}");
        assert!(count.contains("visitors >= 7"), "{count}");
    }

    #[tokio::test]
    async fn a_filter_the_relation_cannot_honour_is_reported_not_applied() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        let q = RecordsQuery {
            filters: Some(r#"[{"column":"absent","values":["a"]}]"#.to_owned()),
            ..mart_query(None, None)
        };

        let body = run(&server, q).await.unwrap();

        assert_eq!(body["filtersSkipped"][0]["column"], "absent");
        let (rows, _) = page_and_count(&server).await;
        assert!(!rows.contains("absent"), "{rows}");
    }

    #[tokio::test]
    async fn a_whole_tile_needs_no_column_or_value() {
        let server = MockServer::start().await;
        mount_mart(&server).await;

        let body = run(&server, mart_query(None, None)).await.unwrap();

        assert_eq!(body["total"], 120);
        assert!(body["value"].is_null());
        let (rows, _) = page_and_count(&server).await;
        assert!(!rows.contains("WHERE"), "{rows}");
    }

    fn rows_query(columns: &str, sort: Option<(&str, &str)>) -> RecordsQuery {
        RecordsQuery {
            mart: Some("mart_x".to_owned()),
            columns: Some(columns.to_owned()),
            sort_column: sort.map(|(c, _)| c.to_owned()),
            sort_dir: sort.map(|(_, d)| d.to_owned()),
            ..RecordsQuery::default()
        }
    }

    #[tokio::test]
    async fn a_raw_table_page_lists_its_columns_sorted_with_the_filters_and_a_total() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        let q = RecordsQuery {
            offset: Some("50".to_owned()),
            filters: Some(r#"[{"column":"visitors","op":"between","min":"7"}]"#.to_owned()),
            ..rows_query("region,visitors", Some(("visitors", "desc")))
        };

        let body = run(&server, q).await.unwrap();

        assert_eq!(body["total"], 120);
        assert_eq!(body["offset"], 50);
        let all = statements(&server).await;
        let rows = all
            .iter()
            .find(|s| s.contains("LIMIT 50 OFFSET 50"))
            .unwrap();
        assert!(
            rows.contains("SELECT region, visitors FROM serving.mart_x WHERE visitors >= 7 ORDER BY visitors DESC, region, visitors"),
            "{rows}"
        );
        let count = all.iter().find(|s| s.contains("count() AS n")).unwrap();
        assert!(count.contains("visitors >= 7"), "{count}");
    }

    #[tokio::test]
    async fn a_raw_table_page_refuses_a_column_the_relation_lacks_and_a_drill_beside_it() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        for q in [
            rows_query("region,absent", None),
            rows_query("", None),
            rows_query("region", Some(("absent", "asc"))),
            rows_query("region", Some(("region", "sideways"))),
            RecordsQuery {
                column: Some("region".to_owned()),
                value: Some("north".to_owned()),
                ..rows_query("region", None)
            },
        ] {
            let err = run(&server, q).await.unwrap_err();
            assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
        }
        assert!(
            !statements(&server)
                .await
                .iter()
                .any(|s| s.contains("serving.mart_x")),
            "nothing may reach the data when the request is refused"
        );
    }

    #[tokio::test]
    async fn a_column_without_a_value_is_a_400() {
        let server = MockServer::start().await;
        mount_mart(&server).await;

        let err = run(&server, mart_query(Some("region"), None))
            .await
            .unwrap_err();

        assert!(matches!(err, ApiError::BadRequest(ref m) if m == "column and value go together"));
    }

    #[tokio::test]
    async fn a_column_the_relation_lacks_is_a_400_with_our_message() {
        let server = MockServer::start().await;
        mount_mart(&server).await;

        let err = run(&server, mart_query(Some("nope"), Some("x")))
            .await
            .unwrap_err();

        assert!(
            matches!(err, ApiError::BadRequest(ref m) if m == "column 'nope' does not exist"),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_offset_that_is_not_a_number_is_a_400() {
        let server = MockServer::start().await;
        mount_mart(&server).await;
        let q = RecordsQuery {
            offset: Some("-1".to_owned()),
            ..mart_query(None, None)
        };

        let err = run(&server, q).await.unwrap_err();

        assert!(matches!(err, ApiError::BadRequest(ref m) if m.starts_with("offset ")));
    }

    #[tokio::test]
    async fn a_value_with_a_quote_and_a_backslash_is_one_escaped_literal() {
        let server = MockServer::start().await;
        mount_mart(&server).await;

        run(&server, mart_query(Some("region"), Some(r"a'b\c")))
            .await
            .unwrap();

        let (rows, count) = page_and_count(&server).await;
        // The policy rewrite re-prints the literal with ClickHouse's backslash
        // escapes; `'a\'b\\c'` is the same value `a'b\c`.
        assert!(rows.contains(r"region = 'a\'b\\c'"), "{rows}");
        assert!(count.contains(r"region = 'a\'b\\c'"), "{count}");
    }

    #[tokio::test]
    async fn a_sql_source_answers_a_page_and_a_total_through_its_own_guarded_sql() {
        let server = MockServer::start().await;
        answer(&server, "count() AS n", vec![json!({ "n": "9" })]).await;
        answer(&server, "AS src", vec![json!({ "channel": "web" })]).await;
        answer(
            &server,
            "FROM console.bi_source FINAL",
            vec![json!({
                "id": "s_1", "title": "src",
                "sql": "SELECT channel FROM serving.mart_x",
                "columns_json": json!([{"name": "channel", "type": "String"}]).to_string(),
                "folder_id": "", "created_by": "u", "updated_at": "2026-01-01 00:00:00",
            })],
        )
        .await;
        answer_rest(&server).await;
        let q = RecordsQuery {
            sql_source: Some("s_1".to_owned()),
            column: Some("channel".to_owned()),
            value: Some("web".to_owned()),
            ..RecordsQuery::default()
        };

        let body = run(&server, q).await.unwrap();

        assert_eq!(body["total"], 9);
        assert_eq!(body["sqlSource"], "s_1");
        let (rows, count) = page_and_count(&server).await;
        assert!(rows.contains("channel = 'web'"), "{rows}");
        // The source's cap rides on the count too.
        assert!(count.contains("max_execution_time = 30"), "{count}");
        assert!(rows.contains("max_execution_time = 30"), "{rows}");
    }

    #[tokio::test]
    async fn a_deleted_sql_source_is_a_404_of_ours_and_runs_nothing_else() {
        let server = MockServer::start().await;
        answer(&server, "FROM console.bi_source FINAL", vec![]).await;
        answer_rest(&server).await;
        let q = RecordsQuery {
            sql_source: Some("s_gone".to_owned()),
            ..RecordsQuery::default()
        };

        let err = run(&server, q).await.unwrap_err();

        assert!(matches!(err, ApiError::NotFound(_)), "{err:?}");
        assert!(
            statements(&server)
                .await
                .iter()
                .all(|s| !s.contains("ORDER BY ALL"))
        );
    }
}

#[cfg(test)]
mod time_grain {
    //! BI-9: the dashboard's grain switch, the board's saved grain, the
    //! `truncated` and `grainSkipped` tile marks, and records by bucket. A
    //! wiremock `ClickHouse` answers the store reads and records every
    //! statement it is sent (no Postgres, so no governance policy applies).

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_bi::grain::{Grain, TimeContext, WeekStart};
    use lakehouse_clickhouse::ChClient;
    use lakehouse_core::ApiError;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{
        DashboardQuery, GrainRequest, RecordsQuery, get_body, parse_grain_param, records_for_roles,
    };
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    async fn answer(
        server: &MockServer,
        statement_has: &str,
        meta: &[(&str, &str)],
        data: Vec<Value>,
    ) {
        let meta: Vec<Value> = meta
            .iter()
            .map(|(n, t)| json!({ "name": n, "type": t }))
            .collect();
        let rows = data.len();
        Mock::given(method("POST"))
            .and(body_string_contains(statement_has))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": meta, "data": data, "rows": rows })),
            )
            .mount(server)
            .await;
    }

    fn chart_row(id: &str, grain: &str, kind: &str, dimension: &str) -> Value {
        let spec = json!({
            "id": id, "title": id, "kind": kind, "mart": "mart_a",
            "sql": "SELECT stored", "x": dimension, "y": "visitors"
        });
        let def = json!({
            "title": id, "mart": "mart_a", "kind": kind, "dimension": dimension,
            "measures": ["visitors"], "limit": 3, "order": "none", "grain": grain,
        });
        json!({
            "id": id, "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
            "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
        })
    }

    /// Board `b1` with a day-grained line, a month-grained bar on a plain
    /// date, and a day-of-week bar; `mart_a` has a `Date` `day` and a
    /// `DateTime` `seen_at`.
    async fn mount_board(server: &MockServer, board_grain: &str) {
        let col = |n: &str| (n.to_owned(), "String".to_owned());
        let _ = col;
        answer(
            server,
            "FROM console.bi_chart FINAL",
            &[
                ("id", "String"),
                ("board", "String"),
                ("created_by", "String"),
                ("created_at", "String"),
                ("spec_json", "String"),
            ],
            vec![
                chart_row("c_line", "day", "line", "day"),
                chart_row("c_month", "month", "bar", "day"),
                chart_row("c_dow", "day_of_week", "bar", "day"),
            ],
        )
        .await;
        answer(
            server,
            "FROM console.bi_board FINAL",
            &[("id", "String")],
            vec![json!({
                "id": "b1", "name": "B", "description": "", "created_by": "", "layout_json": "{}",
                "filters_json": "[]", "public_token": "", "embed_enabled": 0, "folder_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]", "refresh_seconds": 0, "grain": board_grain,
                "created_at": "2026-01-01 00:00:00",
            })],
        )
        .await;
        answer(
            server,
            "SELECT table, name, type FROM system.columns",
            &[("table", "String"), ("name", "String"), ("type", "String")],
            vec![
                json!({"table": "mart_a", "name": "day", "type": "Date"}),
                json!({"table": "mart_a", "name": "seen_at", "type": "DateTime"}),
                json!({"table": "mart_a", "name": "visitors", "type": "UInt32"}),
            ],
        )
        .await;
        // The tiles: every grained statement carries `AS bkt`; three buckets
        // more than the limit of 3 come back (the builder asked for 4).
        answer(
            server,
            "AS bkt",
            &[("day", "Date"), ("visitors", "UInt64")],
            vec![
                json!({"day": "2026-01-01", "visitors": "1"}),
                json!({"day": "2026-02-01", "visitors": "2"}),
                json!({"day": "2026-03-01", "visitors": "3"}),
                json!({"day": "2026-04-01", "visitors": "4"}),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
    }

    async fn statements(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    async fn dashboard(server: &MockServer, requested: Option<Grain>, time: &TimeContext) -> Value {
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(None, &ch);
        let q = DashboardQuery {
            board: Some("b1".to_owned()),
            year: None,
            filters: None,
            grain: requested.map(|g| g.as_str().to_owned()),
        };
        get_body(
            &ch,
            &q,
            None,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            (time, requested.into()),
        )
        .await
        .expect("the payload builds")
    }

    fn tile_statement(all: &[String], needle: &str) -> String {
        all.iter()
            .find(|s| s.contains("AS bkt") && s.contains(needle))
            .unwrap_or_else(|| panic!("no tile statement with {needle}: {all:#?}"))
            .clone()
    }

    #[tokio::test]
    async fn the_switch_regroups_every_truncated_chart_and_leaves_a_part_alone() {
        let server = MockServer::start().await;
        mount_board(&server, "").await;
        let body = dashboard(&server, Some(Grain::Quarter), &TimeContext::default()).await;
        let sent = statements(&server).await;
        // c_line (own: day) and c_month (own: month) follow the switch.
        let quarter = sent
            .iter()
            .filter(|s| s.contains("date_trunc('quarter', day)"))
            .count();
        assert_eq!(quarter, 2, "{sent:#?}");
        // c_dow (a part of the date) does not.
        let dow = tile_statement(&sent, "toDayOfWeek(day)");
        assert!(!dow.contains("date_trunc('quarter'"), "{dow}");
        assert_eq!(body["appliedGrain"], json!("quarter"));
        assert_eq!(body["grain"], json!(""));
        assert_eq!(body["reporting"]["timeZone"], json!("Asia/Jakarta"));
        assert_eq!(body["reporting"]["weekStart"], json!("monday"));
    }

    #[tokio::test]
    async fn without_a_switch_the_boards_saved_grain_applies_and_is_returned() {
        let server = MockServer::start().await;
        mount_board(&server, "year").await;
        let body = dashboard(&server, None, &TimeContext::default()).await;
        let sent = statements(&server).await;
        assert!(
            sent.iter().any(|s| s.contains("date_trunc('year', day)")),
            "{sent:#?}"
        );
        assert_eq!(body["grain"], json!("year"));
        assert_eq!(body["appliedGrain"], json!("year"));

        // The caller's own switch wins over the saved one.
        let body = dashboard(&server, Some(Grain::Month), &TimeContext::default()).await;
        assert_eq!(body["appliedGrain"], json!("month"));
    }

    #[tokio::test]
    async fn own_sets_the_saved_grain_aside_for_this_view() {
        let server = MockServer::start().await;
        mount_board(&server, "year").await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(None, &ch);
        let q = DashboardQuery {
            board: Some("b1".to_owned()),
            year: None,
            filters: None,
            grain: Some("own".to_owned()),
        };
        let body = get_body(
            &ch,
            &q,
            None,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            (&TimeContext::default(), super::GrainRequest::Own),
        )
        .await
        .unwrap();
        assert_eq!(
            body["grain"],
            json!("year"),
            "the saved default is still reported"
        );
        assert_eq!(body["appliedGrain"], Value::Null);
        let sent = statements(&server).await;
        assert!(
            sent.iter().all(|s| !s.contains("date_trunc('year'")),
            "{sent:#?}"
        );
        assert_eq!(
            super::parse_grain_request(Some("own")).unwrap(),
            GrainRequest::Own
        );
        assert_eq!(
            super::parse_grain_request(None).unwrap(),
            GrainRequest::Saved
        );
        assert!(super::parse_grain_request(Some("hour_of_day")).is_err());
    }

    #[tokio::test]
    async fn every_grained_tile_reports_the_grain_it_was_cut_with_and_its_columns_kind() {
        let server = MockServer::start().await;
        mount_board(&server, "").await;
        let body = dashboard(&server, Some(Grain::Quarter), &TimeContext::default()).await;
        assert_eq!(body["results"]["c_line"]["grain"], json!("quarter"));
        assert_eq!(body["results"]["c_line"]["grainColumn"], json!("date"));
        assert_eq!(body["results"]["c_dow"]["grain"], json!("day_of_week"));
    }

    #[tokio::test]
    async fn a_chart_that_cannot_take_the_switch_keeps_its_own_and_says_so() {
        let server = MockServer::start().await;
        mount_board(&server, "").await;
        // `hour` on a plain date.
        let body = dashboard(&server, Some(Grain::Hour), &TimeContext::default()).await;
        assert_eq!(body["results"]["c_line"]["grainSkipped"], json!("hour"));
        assert_eq!(body["results"]["c_month"]["grainSkipped"], json!("hour"));
        assert!(
            body["results"]["c_dow"].get("grainSkipped").is_none(),
            "a part is not offered the switch, so it has nothing to skip"
        );
    }

    #[tokio::test]
    async fn a_result_longer_than_the_limit_keeps_the_latest_buckets_and_is_marked() {
        let server = MockServer::start().await;
        mount_board(&server, "").await;
        let body = dashboard(&server, None, &TimeContext::default()).await;
        let tile = &body["results"]["c_month"];
        assert_eq!(tile["truncated"], json!(true), "{tile}");
        let days: Vec<&str> = tile["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["day"].as_str().unwrap())
            .collect();
        assert_eq!(days, ["2026-02-01", "2026-03-01", "2026-04-01"]);
    }

    #[tokio::test]
    async fn the_report_zone_and_week_start_reach_the_statements() {
        let server = MockServer::start().await;
        mount_board(&server, "week").await;
        let time = TimeContext::new("Europe/Berlin", WeekStart::Sunday).unwrap();
        dashboard(&server, None, &time).await;
        let sent = statements(&server).await;
        assert!(
            sent.iter()
                .any(|s| s.contains("date_trunc('week', day + toIntervalDay(1))")),
            "{sent:#?}"
        );
    }

    #[test]
    fn the_grain_query_value_must_be_a_truncation() {
        assert_eq!(parse_grain_param(None).unwrap(), None);
        assert_eq!(parse_grain_param(Some(" ")).unwrap(), None);
        assert_eq!(
            parse_grain_param(Some("month")).unwrap(),
            Some(Grain::Month)
        );
        for bad in ["fortnight", "day_of_week", "Month", "month;--"] {
            assert!(
                matches!(parse_grain_param(Some(bad)), Err(ApiError::BadRequest(_))),
                "{bad}"
            );
        }
    }

    async fn records(
        server: &MockServer,
        q: RecordsQuery,
        time: &TimeContext,
    ) -> Result<Value, ApiError> {
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(None, &ch);
        records_for_roles(&ch, q, &[], &PlaceholderValues::none(), &obligations, time).await
    }

    fn bucket_query(column: &str, value: &str, grain: &str) -> RecordsQuery {
        RecordsQuery {
            mart: Some("mart_a".to_owned()),
            column: Some(column.to_owned()),
            value: Some(value.to_owned()),
            grain: Some(grain.to_owned()),
            ..RecordsQuery::default()
        }
    }

    async fn mount_records(server: &MockServer) {
        answer(
            server,
            "FROM system.columns",
            &[("name", "String"), ("type", "String")],
            vec![
                json!({"name": "day", "type": "Date"}),
                json!({"name": "seen_at", "type": "DateTime"}),
                json!({"name": "place", "type": "String"}),
            ],
        )
        .await;
        answer(
            server,
            "count()",
            &[("n", "UInt64")],
            vec![json!({"n": "2"})],
        )
        .await;
        answer(
            server,
            "ORDER BY ALL",
            &[("day", "Date")],
            vec![json!({"day": "2026-03-04"}), json!({"day": "2026-03-20"})],
        )
        .await;
    }

    #[tokio::test]
    async fn records_for_a_clicked_month_are_selected_by_the_bucket_in_the_report_zone() {
        let server = MockServer::start().await;
        mount_records(&server).await;
        let time = TimeContext::new("Europe/Berlin", WeekStart::Monday).unwrap();
        let body = records(
            &server,
            bucket_query("seen_at", "2026-03-01", "month"),
            &time,
        )
        .await
        .unwrap();
        assert_eq!(body["total"], json!(2));
        let sent = statements(&server).await;
        let rows = sent.iter().find(|s| s.contains("ORDER BY ALL")).unwrap();
        assert!(
            rows.contains(
                "toString(date_trunc('month', toTimeZone(seen_at, 'Europe/Berlin'))) = '2026-03-01'"
            ),
            "{rows}"
        );
        let count = sent.iter().find(|s| s.contains("count()")).unwrap();
        assert!(
            count.contains("date_trunc('month'"),
            "the total counts the same bucket: {count}"
        );
    }

    #[tokio::test]
    async fn a_bucket_on_a_column_that_is_not_a_date_or_without_a_value_is_refused() {
        let server = MockServer::start().await;
        mount_records(&server).await;
        let t = TimeContext::default();
        let err = records(&server, bucket_query("place", "x", "month"), &t)
            .await
            .unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
        let err = records(&server, bucket_query("day", "x", "hour"), &t)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::BadRequest(_)),
            "a plain date has no hour: {err:?}"
        );
        let err = records(&server, bucket_query("day", "x", "day_of_week"), &t)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ApiError::BadRequest(_)),
            "a part has no records: {err:?}"
        );
        let q = RecordsQuery {
            mart: Some("mart_a".to_owned()),
            grain: Some("month".to_owned()),
            ..RecordsQuery::default()
        };
        let err = records(&server, q, &t).await.unwrap_err();
        assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
    }
}

#[cfg(test)]
mod table_tiles {
    //! `BI-16` part A on the dashboard payload: a raw table's first page with
    //! its total, a pivot cut at the cell cap, and a KPI that compares. A
    //! wiremock `ClickHouse` answers the store reads and records every
    //! statement it is sent.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use lakehouse_bi::grain::TimeContext;
    use lakehouse_clickhouse::ChClient;
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{DashboardQuery, GrainRequest, get_body};
    use crate::policy_engine::PolicyEngineObligations;
    use crate::sql_rewrite::PlaceholderValues;

    async fn answer(
        server: &MockServer,
        statement_has: &str,
        meta: &[(&str, &str)],
        data: Vec<Value>,
    ) {
        let meta: Vec<Value> = meta
            .iter()
            .map(|(n, t)| json!({ "name": n, "type": t }))
            .collect();
        let rows = data.len();
        Mock::given(method("POST"))
            .and(body_string_contains(statement_has))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": meta, "data": data, "rows": rows })),
            )
            .mount(server)
            .await;
    }

    fn chart_row(id: &str, kind: &str, def: &Value) -> Value {
        let spec = json!({
            "id": id, "title": id, "kind": kind, "mart": "mart_a",
            "sql": "SELECT stored", "x": "", "y": "visitors"
        });
        let mut def = def.clone();
        def["title"] = json!(id);
        def["mart"] = json!("mart_a");
        def["kind"] = json!(kind);
        json!({
            "id": id, "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
            "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
        })
    }

    /// Board `b1` with a raw table, a five-value pivot and a KPI that compares
    /// with its previous month, all on `mart_a`.
    async fn mount_board(server: &MockServer, pivot_rows: usize) {
        let values: Vec<Value> = ["sum", "avg", "min", "max", "count"]
            .iter()
            .map(|a| json!({ "column": "visitors", "aggregate": a }))
            .collect();
        let charts = vec![
            chart_row(
                "c_rows",
                "table",
                &json!({ "tableMode": "rows", "columns": ["place", "visitors"] }),
            ),
            chart_row(
                "c_pivot",
                "pivot",
                &json!({ "rows": ["place"], "values": values }),
            ),
            chart_row(
                "c_kpi",
                "kpi",
                &json!({
                    "measures": ["visitors"], "aggregate": "sum",
                    "compare": { "kind": "previous", "dateColumn": "day", "period": "month" },
                }),
            ),
        ];
        let meta = [
            ("id", "String"),
            ("board", "String"),
            ("created_by", "String"),
            ("created_at", "String"),
            ("spec_json", "String"),
        ];
        answer(server, "FROM console.bi_chart FINAL", &meta, charts).await;
        answer(
            server,
            "FROM console.bi_board FINAL",
            &[("id", "String")],
            vec![json!({
                "id": "b1", "name": "B", "description": "", "created_by": "", "layout_json": "{}",
                "filters_json": "[]", "public_token": "", "embed_enabled": 0, "folder_id": "",
                "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
                "embed_origins_json": "[]", "refresh_seconds": 0, "grain": "",
                "created_at": "2026-01-01 00:00:00",
            })],
        )
        .await;
        answer(
            server,
            "SELECT table, name, type FROM system.columns",
            &[("table", "String"), ("name", "String"), ("type", "String")],
            vec![
                json!({"table": "mart_a", "name": "day", "type": "Date"}),
                json!({"table": "mart_a", "name": "place", "type": "String"}),
                json!({"table": "mart_a", "name": "visitors", "type": "UInt32"}),
            ],
        )
        .await;
        answer(
            server,
            "count() AS n FROM",
            &[("n", "UInt64")],
            vec![json!({ "n": "237" })],
        )
        .await;
        answer(
            server,
            "GROUPING SETS",
            &[("place", "String"), ("__v0", "UInt64")],
            (0..pivot_rows)
                .map(|i| json!({ "place": format!("p{i}"), "__v0": "1" }))
                .collect(),
        )
        .await;
        answer(
            server,
            "ORDER BY day DESC LIMIT 12",
            &[("day", "Date"), ("v", "UInt64")],
            vec![
                json!({"day": "2026-02-01", "v": "5"}),
                json!({"day": "2026-03-01", "v": "7"}),
            ],
        )
        .await;
        answer(
            server,
            "LIMIT 50 OFFSET 0",
            &[("place", "String"), ("visitors", "UInt32")],
            vec![json!({ "place": "a", "visitors": 1 })],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(server)
            .await;
    }

    async fn dashboard(server: &MockServer) -> Value {
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(None, &ch);
        let q = DashboardQuery {
            board: Some("b1".to_owned()),
            year: None,
            filters: None,
            grain: None,
        };
        get_body(
            &ch,
            &q,
            None,
            &[],
            &PlaceholderValues::none(),
            &obligations,
            (&TimeContext::default(), GrainRequest::Saved),
        )
        .await
        .expect("the payload builds")
    }

    async fn statements(server: &MockServer) -> Vec<String> {
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect()
    }

    #[tokio::test]
    async fn a_raw_tables_tile_carries_its_first_page_and_the_total_of_the_rows_behind_it() {
        let server = MockServer::start().await;
        mount_board(&server, 3).await;
        let body = dashboard(&server).await;
        let tile = &body["results"]["c_rows"];
        assert_eq!(tile["total"], json!(237));
        assert_eq!(tile["limit"], json!(50));
        assert_eq!(tile["offset"], json!(0));
        assert_eq!(tile["rows"].as_array().unwrap().len(), 1);
        let sent = statements(&server).await;
        assert!(
            sent.iter().any(|s| s.contains(
                "SELECT place, visitors FROM serving.mart_a ORDER BY place, visitors LIMIT 50 OFFSET 0"
            )),
            "{sent:#?}"
        );
        assert!(
            sent.iter()
                .any(|s| s.contains("SELECT count() AS n FROM serving.mart_a")),
            "{sent:#?}"
        );
        assert_eq!(body["charts"][0]["def"]["tableMode"], json!("rows"));
    }

    #[tokio::test]
    async fn a_pivot_is_cut_at_its_cell_cap_and_says_so_only_when_it_was() {
        // Five values: 10 000 cells / 5 = 2 000 long-format rows.
        let server = MockServer::start().await;
        mount_board(&server, 2001).await;
        let body = dashboard(&server).await;
        let tile = &body["results"]["c_pivot"];
        assert_eq!(tile["rows"].as_array().unwrap().len(), 2000);
        assert_eq!(tile["truncated"], json!(true));

        let server = MockServer::start().await;
        mount_board(&server, 2000).await;
        let body = dashboard(&server).await;
        let tile = &body["results"]["c_pivot"];
        assert_eq!(tile["rows"].as_array().unwrap().len(), 2000);
        assert!(tile.get("truncated").is_none(), "{tile}");
    }

    #[tokio::test]
    async fn a_kpi_that_compares_returns_its_periods_and_the_grain_that_labels_them() {
        let server = MockServer::start().await;
        mount_board(&server, 3).await;
        let body = dashboard(&server).await;
        let tile = &body["results"]["c_kpi"];
        assert_eq!(tile["grain"], json!("month"));
        assert_eq!(tile["rows"].as_array().unwrap().len(), 2);
        assert_eq!(tile["rows"][1]["v"], json!("7"));
    }
}
