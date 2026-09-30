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

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
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
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    match get_body(
        &state.clickhouse,
        &q,
        &principal.role_names,
        &placeholders,
        &obligations,
    )
    .await
    {
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
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &PolicyEngineObligations<'_>,
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

    // The built-in board's layout row is kept out of `list_boards`.
    let default_row = if board == store::DEFAULT_BOARD_ID {
        store::get_board(ch, store::DEFAULT_BOARD_ID)
            .await
            .unwrap_or_else(|err| {
                store_error.get_or_insert_with(|| format!("Error: {err}"));
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

    let need_cols = !years.is_empty() || filters.iter().any(|f| !f.values.is_empty());
    let cols: HashMap<String, HashSet<String>> = if need_cols {
        mart_columns(ch).await?
    } else {
        HashMap::new()
    };

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
        let sql = match stored_chart_sql(c, &years, &filters, &cols, &sources) {
            Ok(sql) => sql,
            Err(msg) => {
                results.insert(c.spec.id.clone(), json!({ "error": msg }));
                continue;
            }
        };
        let (id, val) = run_spec_sql(ch, &c.spec.id, &sql, roles, placeholders, obligations).await;
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
    let spec = store::spec_from_input(&state.clickhouse, &input, ChartSource::Ui, "ui", None)
        .await
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    let placeholders = crate::sql_rewrite::PlaceholderValues {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_tenant_ids: principal
            .tenant_ids
            .iter()
            .map(ToString::to_string)
            .collect(),
    };
    let obligations = PolicyEngineObligations::new(state.pg.as_deref(), &state.clickhouse);
    let (_, result) = run_spec_sql(
        &state.clickhouse,
        &spec.spec.id,
        &spec.spec.sql,
        &principal.role_names,
        &placeholders,
        &obligations,
    )
    .await;
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
        return Err(ApiError::BadRequest("id is required".to_owned()).into());
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
    /// Folder to create the board in (`""`/absent = root).
    #[serde(default, rename = "folderId")]
    folder_id: Option<String>,
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
            .map_err(|err| ApiError::BadRequest(format!("JSON is invalid: {err}")))?
    };
    let folder_id = parsed.folder_id.unwrap_or_default().trim().to_owned();
    // Checked first, so an unknown folder creates nothing.
    crate::routes::dashboard_folders::ensure_folder_exists(&state.clickhouse, &folder_id).await?;
    let mut board = if let Some(dup) = parsed.duplicate {
        store::duplicate_board(&state.clickhouse, &dup).await
    } else {
        store::create_board(&state.clickhouse, &parsed.name.unwrap_or_default()).await
    }
    .map_err(|err| ApiError::BadRequest(err.to_string()))?;
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
    layout: Option<LayoutMap>,
    #[serde(default)]
    filters: Option<Vec<FilterDef>>,
    #[serde(default)]
    public: Option<bool>,
    #[serde(default)]
    embed: Option<bool>,
    /// Move the board to this folder (`""` = root).
    #[serde(default, rename = "folderId")]
    folder_id: Option<String>,
}

/// Whether a `PUT /api/dashboard/boards` body may be applied: it names a
/// board, and the built-in board is sent nothing but a layout.
fn board_edit_allowed(body: &BoardEditBody) -> bool {
    match body.id.as_deref() {
        None | Some("") => false,
        Some(store::DEFAULT_BOARD_ID) => {
            body.name.is_none()
                && body.filters.is_none()
                && body.public.is_none()
                && body.embed.is_none()
                && body.folder_id.is_none()
        }
        Some(_) => true,
    }
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
    if let Some(folder_id) = &parsed.folder_id {
        let folder_id = folder_id.trim();
        crate::routes::dashboard_folders::ensure_folder_exists(ch, folder_id).await?;
        store::move_board(ch, &id, folder_id)
            .await
            .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
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
        return Err(ApiError::BadRequest("invalid dashboard".to_owned()).into());
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
            ApiJson(json!({ "error": err.to_string() })),
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
#[derive(Debug, Deserialize)]
pub struct RecordsQuery {
    #[serde(default)]
    mart: Option<String>,
    /// Set by a tile built on a dashboard SQL source. Drill-down over a
    /// source is not supported yet (plan §3.4); answered honestly instead of
    /// being treated as a missing mart.
    #[serde(default)]
    source: Option<String>,
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
    records_for_roles(
        &state.clickhouse,
        q,
        &principal.role_names,
        &placeholders,
        &obligations,
    )
    .await
    .map(ApiJson)
    .map_err(Into::into)
}

/// Classifies a `ClickHouse` failure on the dashboard's data routes
/// (drill-down, filter values, SQL-source fields) without forwarding its
/// text (AGENTS.md principle 4): a server-side error is a fixed 422,
/// anything else a fixed 503; the real detail is only logged.
fn classify_dashboard_ch_error(err: &lakehouse_clickhouse::ChError) -> ApiError {
    match err {
        lakehouse_clickhouse::ChError::Server(_) => {
            tracing::warn!(%err, "dashboard query failed");
            ApiError::Unprocessable("dashboard query failed".to_owned())
        }
        lakehouse_clickhouse::ChError::Transport(_) | lakehouse_clickhouse::ChError::Cancelled => {
            tracing::warn!(%err, "clickhouse unreachable for a dashboard query");
            ApiError::Unavailable("clickhouse unavailable".to_owned())
        }
    }
}

/// [`records`]' body, taking the principal's roles and placeholders
/// explicitly so the enforcement path can be tested against a mock
/// `ClickHouse` and a real policy row.
async fn records_for_roles(
    ch: &ChClient,
    q: RecordsQuery,
    roles: &[String],
    placeholders: &crate::sql_rewrite::PlaceholderValues,
    obligations: &PolicyEngineObligations<'_>,
) -> Result<Value, ApiError> {
    if q.source.as_deref().is_some_and(|s| !s.trim().is_empty()) {
        return Ok(json!({
            "supported": false,
            "message": "Drill-down is not available yet for charts built on a SQL source.",
            "columns": [],
            "rows": [],
        }));
    }
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
        return Err(ApiError::BadRequest("invalid mart/column".to_owned()));
    }

    let cols_sql = format!(
        "SELECT name FROM system.columns WHERE database='serving' AND table='{}'",
        esc(&mart)
    );
    let cols = ch
        .rows(&cols_sql, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    if cols.is_empty() {
        return Err(ApiError::NotFound(format!("mart '{mart}' does not exist")));
    }
    let has_column = cols
        .iter()
        .any(|c| c.get("name").and_then(Value::as_str) == Some(column.as_str()));
    if !has_column {
        return Err(ApiError::BadRequest(format!(
            "column '{column}' does not exist"
        )));
    }

    let sql = format!(
        "SELECT * FROM serving.{mart} WHERE {column} = '{}' LIMIT {limit}",
        esc(&value)
    );
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
    let result = ch
        .query(&rewritten, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    let columns: Vec<String> = result.meta.iter().map(|m| m.name.clone()).collect();
    Ok(json!({
        "columns": columns,
        "rows": result.data,
        "mart": mart,
        "column": column,
        "value": value,
    }))
}

// ── /api/dashboard/values ───────────────────────────────────────────────

/// Query parameters for `GET /api/dashboard/values`.
#[derive(Debug, Deserialize)]
pub struct ValuesQuery {
    #[serde(default)]
    column: Option<String>,
}

/// `GET /api/dashboard/values?column=` — the distinct values of `column`
/// across every `serving` mart that has it (the dashboard filter picker).
///
/// Unlike `/fields` (schema metadata only), this reads DATA, so it goes
/// through `policy_engine::rewrite_sql_for_roles` for the caller's roles
/// like the tiles do: a masked column's values, or rows a row filter hides,
/// used to be listed here in the clear (plan §8 item 3). `ClickHouse`
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
    values_for_roles(
        &state.clickhouse,
        q,
        &principal.role_names,
        &placeholders,
        &obligations,
    )
    .await
    .map(ApiJson)
    .map_err(Into::into)
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
) -> Result<Value, ApiError> {
    let column = strip_non_ident(q.column.as_deref().unwrap_or(""));
    if column.is_empty() {
        return Err(ApiError::BadRequest("column is required".to_owned()));
    }
    let marts_sql = format!(
        "SELECT table FROM system.columns WHERE database='serving' AND name='{column}' AND \
         table NOT LIKE '%\\_baru'"
    );
    let marts = ch
        .rows(&marts_sql, None)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    if marts.is_empty() {
        return Ok(json!({ "column": column, "values": Vec::<String>::new() }));
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
    let values: Vec<String> = rows
        .iter()
        .filter_map(|r| r.get("v").and_then(Value::as_str).map(ToOwned::to_owned))
        .collect();
    Ok(json!({ "column": column, "values": values }))
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
    .map_err(|err| ApiError::Internal(err.to_string()))?;

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
            limit: None,
            source: None,
        }
    }

    /// Answers both `system.columns` lookups the drill-down makes: the
    /// route's own column check (reads `name`) and the policy engine's
    /// column resolution (reads `name`/`default_*`).
    async fn mount_mart_columns(server: &MockServer) {
        Mock::given(method("POST"))
            .and(body_string_contains("system.columns"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "meta": [
                    {"name": "name", "type": "String"},
                    {"name": "default_kind", "type": "String"},
                    {"name": "default_expression", "type": "String"},
                ],
                "data": [
                    {"name": "id", "default_kind": "", "default_expression": ""},
                    {"name": "email", "default_kind": "", "default_expression": ""},
                    {"name": "region", "default_kind": "", "default_expression": ""},
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
        Ok(())
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn drill_down_on_a_sql_source_tile_is_unsupported_and_never_queries(
        pool: PgPool,
    ) -> sqlx::Result<()> {
        // No mocks: any ClickHouse request would fail the call.
        let server = MockServer::start().await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let obligations = PolicyEngineObligations::new(Some(&pool), &ch);
        let mut q = drill("", "material_group", "x");
        q.source = Some("s_1234abcd".to_owned());

        let body = records_for_roles(&ch, q, &[], &PlaceholderValues::none(), &obligations)
            .await
            .expect("an honest unsupported answer, not an error");

        assert_eq!(body["supported"], false);
        assert!(server.received_requests().await.unwrap().is_empty());
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
        )
        .await
        .expect_err("a failing row query is an error");

        let text = format!("{err:?}");
        assert!(
            !text.contains("upstream-secret-detail"),
            "ClickHouse's own text leaked: {text}"
        );
        assert!(
            matches!(err, ApiError::Unprocessable(ref m) if m == "dashboard query failed"),
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
            },
            &["Analyst".to_owned()],
            &PlaceholderValues::none(),
            &obligations,
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
}
