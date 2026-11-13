//! Reports: a written page of text with live charts between the paragraphs
//! (`BI-18` part D, plan `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md`
//! T1 and T2; feature page `docs/core/features/reports-and-templates.md`,
//! decisions R1 to R4).
//!
//! - `GET /api/dashboard/reports` — the reports the caller may read.
//! - `POST /api/dashboard/reports` — make one (`dashboard:write`, `edit` on
//!   the folder).
//! - `GET /api/dashboard/reports/{id}` — one report with every chart block
//!   read as the caller.
//! - `PUT /api/dashboard/reports/{id}` — change its title, blocks or folder.
//! - `DELETE /api/dashboard/reports/{id}` — to the trash (30 days, restored
//!   through `POST /api/dashboard/trash/restore` with `kind: "report"`).
//! - `GET /api/dashboard/reports/{id}/pdf` — the server-made PDF (T2).
//!
//! # Who may do what (R2)
//!
//! A report lives in a dashboard folder and has no rules of its own: it takes
//! its folder's. Reading needs what reading a dashboard in that folder needs
//! (`dashboard:read` or a rule, through [`viewer_for`]), and a report at the
//! root is read by those with a starting point, as the built-in dashboard is.
//! A report the caller cannot read (unknown, in the trash, in a folder they
//! cannot see or in somebody else's personal space) is one 404,
//! [`REPORT_NOT_FOUND`], so an id guess learns nothing. Writing needs
//! `dashboard:write` at the router (feature `BI-18D-AC8`) and, here, `edit` on
//! the folder; moving a report out of a folder and deleting it need `manage`
//! there, as for a dashboard (`BI-19` B1).
//!
//! # A chart block is the reader's own read (R1)
//!
//! A chart block stores a chart's id and its dashboard, never data. Saving
//! checks the writer can see the chart and that it is a chart (not a text or
//! section tile), and takes the dashboard from the chart's own row. Reading
//! finds the chart again among the charts the *reader* may see, then reads it
//! through `dashboard::payload_for`, the function the dashboard page and the
//! PDF use, with only that chart: so every mask, row filter and access rule of
//! the reader applies exactly as on the dashboard. A chart the reader may not
//! see, or that is gone, is [`CHART_NOT_AVAILABLE`] and nothing else: no
//! title, no data.
//!
//! SEC-11: every store failure is classified; no upstream text reaches a
//! response.

use std::collections::{BTreeMap, HashMap};

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use futures::StreamExt;
use lakehouse_auth::Principal;
use lakehouse_bi::access::{Level, Viewer};
use lakehouse_bi::reports::{self, Block, ReportRule};
use lakehouse_bi::{folders, store};
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::dashboard_reports::{self as stored, Report, ReportInput, ReportSummary};
use serde::Deserialize;
use serde_json::{Value, json};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard::{DashboardQuery, payload_for};
use crate::routes::dashboard_access::{Scope, payload_scope, scope_for_write, viewer_for};
use crate::routes::dashboard_folders::{classify_ch_error, ensure_folder_usable};
use crate::state::AppState;
use crate::upstream_error;

/// The one answer for a report the caller may not read (see the module doc).
pub(crate) const REPORT_NOT_FOUND: &str = "report not found.";

/// What a chart block shows when the reader may not see its chart, or the
/// chart is gone. Never a title or a number.
pub(crate) const CHART_NOT_AVAILABLE: &str = "This chart is not available.";

/// What a chart block shows when its chart could not be read for a reason
/// that is not the reader's access (the store or the engine failed); the
/// cause is in the log under a reference.
pub(crate) const CHART_NOT_LOADED: &str = "This chart could not be loaded.";

/// Saving a chart block the writer cannot see.
pub(crate) const CHART_NOT_YOURS: &str = "A chart in this report is not one you can see.";

/// Saving a chart block that names a text, section, image or web tile.
pub(crate) const NOT_A_CHART: &str =
    "A text, section, image or web tile is not a chart; only charts can be placed in a report.";

/// Reports are rows in the console database; without it there are none.
const NEEDS_DATABASE: &str = "Reports need the console database, which is not reachable.";

/// The largest body a save reads: 300 blocks of 10,000 characters fit with
/// room; anything larger is refused before it is parsed.
const MAX_BODY_BYTES: usize = 4_000_000;

/// How many chart blocks are read at once. Each read also waits for the
/// process's `tile_slots`, so this only bounds one report's share.
const CHARTS_AT_ONCE: usize = 4;

pub(crate) fn pool_of(state: &AppState) -> Result<&PgPool, ApiError> {
    state
        .pg
        .as_deref()
        .ok_or_else(|| ApiError::Unavailable(NEEDS_DATABASE.to_owned()))
}

pub(crate) fn store_failure(err: &lakehouse_store::StoreError) -> ApiError {
    upstream_error::internal_error(&upstream_error::STORE, err)
}

fn rule(err: ReportRule) -> ApiError {
    ApiError::BadRequest(err.message().to_owned())
}

fn rfc3339(at: OffsetDateTime) -> String {
    at.format(&Rfc3339).unwrap_or_default()
}

fn not_found() -> ApiError {
    ApiError::NotFound(REPORT_NOT_FOUND.to_owned())
}

/// A report id from the path; anything that is not a UUID is the same 404 as
/// an unknown id.
pub(crate) fn report_id(raw: &str) -> Result<Uuid, ApiError> {
    raw.trim().parse::<Uuid>().map_err(|_| not_found())
}

/// The JSON of a report, blocks and all.
pub(crate) fn report_json(report: &Report) -> Value {
    json!({
        "id": report.id,
        "title": report.title,
        "folderId": report.folder_id,
        "blocks": report.blocks,
        "createdBy": report.created_by,
        "createdAt": rfc3339(report.created_at),
        "updatedBy": report.updated_by,
        "updatedAt": rfc3339(report.updated_at),
    })
}

fn summary_json(report: &ReportSummary) -> Value {
    json!({
        "id": report.id,
        "title": report.title,
        "folderId": report.folder_id,
        "createdBy": report.created_by,
        "updatedBy": report.updated_by,
        "updatedAt": rfc3339(report.updated_at),
        "blockCount": report.block_count,
        "chartCount": report.chart_count,
    })
}

/// Whether `viewer` may read what is filed in `folder_id`: the folder is one
/// of the folders they see, or it is the root and they have a starting point.
/// `visible` is the viewer's folder list, read once by the caller.
fn folder_readable(viewer: &Viewer, visible: &[folders::Folder], folder_id: &str) -> bool {
    if folder_id.is_empty() {
        viewer.sees_builtin()
    } else {
        visible.iter().any(|f| f.id == folder_id)
    }
}

async fn visible_folders(
    state: &AppState,
    viewer: &Viewer,
) -> Result<Vec<folders::Folder>, ApiError> {
    folders::list_folders(&state.clickhouse, viewer)
        .await
        .map_err(|err| classify_ch_error(&err))
}

/// The live report `id` if `viewer` may read it.
///
/// # Errors
///
/// 404 [`REPORT_NOT_FOUND`] when it is unknown, in the trash or not readable;
/// the classified store error.
pub(crate) async fn readable_report(
    state: &AppState,
    viewer: &Viewer,
    id: Uuid,
) -> Result<Report, ApiError> {
    let pool = pool_of(state)?;
    let report = stored::get_live(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(not_found)?;
    let visible = visible_folders(state, viewer).await?;
    if !folder_readable(viewer, &visible, &report.folder_id) {
        return Err(not_found());
    }
    Ok(report)
}

// ── List ────────────────────────────────────────────────────────────────

/// `GET /api/dashboard/reports` — every live report the caller may read.
///
/// # Errors
///
/// 403 for a caller with neither `dashboard:read` nor a rule; 503 without
/// the console database; the classified store error.
pub async fn list(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> ApiResult<ApiJson<Value>> {
    let viewer = viewer_for(&state, &principal).await?;
    let pool = pool_of(&state)?;
    let (rows, truncated) = stored::list_live(pool)
        .await
        .map_err(|e| store_failure(&e))?;
    let visible = visible_folders(&state, &viewer).await?;
    let reports: Vec<Value> = rows
        .iter()
        .filter(|r| folder_readable(&viewer, &visible, &r.folder_id))
        .map(summary_json)
        .collect();
    Ok(ApiJson(json!({
        "reports": reports,
        "truncated": truncated,
        "maxCharts": reports::MAX_CHARTS,
    })))
}

// ── Saving ──────────────────────────────────────────────────────────────

/// `POST /api/dashboard/reports` body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CreateBody {
    title: String,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    blocks: Option<Value>,
}

/// `PUT /api/dashboard/reports/{id}` body: what is absent is kept.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UpdateBody {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    folder_id: Option<String>,
    #[serde(default)]
    blocks: Option<Value>,
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes, shape: &str) -> Result<T, ApiError> {
    if body.len() > MAX_BODY_BYTES {
        return Err(ApiError::BadRequest(
            "That report is too large to save.".to_owned(),
        ));
    }
    serde_json::from_slice(body).map_err(|_| ApiError::BadRequest(format!("body must be {shape}.")))
}

/// Check the blocks and fill each chart block's dashboard from the chart's own
/// row, refusing a chart the writer cannot see and a tile that is not a chart.
async fn resolve_blocks(
    state: &AppState,
    viewer: &Viewer,
    raw: Value,
) -> Result<Vec<Block>, ApiError> {
    let blocks = reports::check_blocks(reports::parse_blocks(raw).map_err(rule)?).map_err(rule)?;
    if reports::chart_ids(&blocks).is_empty() {
        return Ok(blocks);
    }
    let charts: HashMap<String, (String, bool)> =
        store::list_stored_charts(&state.clickhouse, viewer)
            .await
            .map_err(|err| classify_ch_error(&err))?
            .into_iter()
            .map(|c| (c.spec.id.clone(), (c.board, c.spec.kind.is_static())))
            .collect();
    blocks
        .into_iter()
        .map(|block| match block {
            Block::Chart { chart, .. } => match charts.get(&chart) {
                None => Err(ApiError::BadRequest(CHART_NOT_YOURS.to_owned())),
                Some((_, true)) => Err(ApiError::BadRequest(NOT_A_CHART.to_owned())),
                Some((board, false)) => Ok(Block::Chart {
                    chart,
                    board: board.clone(),
                }),
            },
            other => Ok(other),
        })
        .collect()
}

fn blocks_value(blocks: &[Block]) -> Result<Value, ApiError> {
    serde_json::to_value(blocks)
        .map_err(|err| upstream_error::internal_error(&upstream_error::SERVICE, &err))
}

/// Make a report for `principal`: the checks of `POST /api/dashboard/reports`,
/// shared with the assistant's `create_report` (`BI-18` D, T4) so the two
/// cannot drift.
///
/// # Errors
///
/// As [`create`].
pub(crate) async fn create_report(
    state: &AppState,
    principal: &Principal,
    title: &str,
    folder_id: &str,
    blocks: Value,
) -> Result<Report, ApiError> {
    let scope = scope_for_write(state, principal).await?;
    let title = reports::check_title(title).map_err(rule)?;
    let folder_id = folder_id.trim();
    ensure_folder_usable(&state.clickhouse, folder_id, &scope.viewer).await?;
    scope.require_on_folder(folder_id, Level::Edit)?;
    let blocks = resolve_blocks(state, &scope.viewer, blocks).await?;
    let pool = pool_of(state)?;
    stored::insert(
        pool,
        ReportInput {
            folder_id,
            title: &title,
            blocks: &blocks_value(&blocks)?,
            by: &principal.display_name,
        },
    )
    .await
    .map_err(|e| store_failure(&e))
}

/// `POST /api/dashboard/reports { title, folderId?, blocks? }`.
///
/// # Errors
///
/// 400 with a block rule's sentence (the 31st chart names the limit, R4), a
/// chart the writer cannot see or a tile that is not a chart, or a folder
/// they may not file into; 403 without `edit` on the folder; 404 for a folder
/// they cannot see; 503 without the console database.
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: CreateBody = parse(&body, "{ title, folderId, blocks }")?;
    let report = create_report(
        &state,
        &principal,
        &parsed.title,
        parsed.folder_id.as_deref().unwrap_or(""),
        parsed.blocks.unwrap_or_else(|| json!([])),
    )
    .await?;
    Ok(ApiJson(
        json!({ "ok": true, "report": report_json(&report) }),
    ))
}

/// The live report `id` for a write, with the caller's scope: 404 when they
/// cannot read it, 403 below `needed` on its folder.
async fn writable_report(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
    needed: Level,
) -> Result<(Scope, Report), ApiError> {
    let scope = scope_for_write(state, principal).await?;
    let report = readable_report(state, &scope.viewer, id).await?;
    scope.require_on_folder(&report.folder_id, needed)?;
    Ok((scope, report))
}

/// What a change asks for; `None` keeps the stored value.
pub(crate) struct ReportChange {
    pub(crate) title: Option<String>,
    pub(crate) folder_id: Option<String>,
    pub(crate) blocks: Option<Value>,
}

/// Change a report for `principal`: the checks of `PUT
/// /api/dashboard/reports/{id}`, shared with the assistant's `update_report`.
///
/// # Errors
///
/// As [`update`].
pub(crate) async fn update_report(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
    change: ReportChange,
) -> Result<Report, ApiError> {
    let (scope, report) = writable_report(state, principal, id, Level::Edit).await?;
    let title = match change.title.as_deref() {
        Some(title) => reports::check_title(title).map_err(rule)?,
        None => report.title.clone(),
    };
    let folder_id = match change.folder_id.as_deref().map(str::trim) {
        Some(target) if target != report.folder_id => {
            // Moving is `manage` where it leaves and `edit` where it lands, as
            // for a dashboard (`BI-19` B1).
            scope.require_on_folder(&report.folder_id, Level::Manage)?;
            ensure_folder_usable(&state.clickhouse, target, &scope.viewer).await?;
            scope.require_on_folder(target, Level::Edit)?;
            target.to_owned()
        }
        _ => report.folder_id.clone(),
    };
    let blocks = match change.blocks {
        Some(raw) => blocks_value(&resolve_blocks(state, &scope.viewer, raw).await?)?,
        None => report.blocks.clone(),
    };
    let pool = pool_of(state)?;
    stored::update(
        pool,
        id,
        ReportInput {
            folder_id: &folder_id,
            title: &title,
            blocks: &blocks,
            by: &principal.display_name,
        },
    )
    .await
    .map_err(|e| store_failure(&e))?
    // Deleted between the read and the write.
    .ok_or_else(not_found)
}

/// `PUT /api/dashboard/reports/{id} { title?, folderId?, blocks? }`. The
/// blocks, when given, replace the report's blocks in their order.
///
/// # Errors
///
/// As [`create`]; 404 [`REPORT_NOT_FOUND`] for a report the caller cannot
/// read; 403 without `edit` on its folder, or without `manage` there when the
/// report moves.
pub async fn update(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let id = report_id(&id)?;
    let parsed: UpdateBody = parse(&body, "{ title, folderId, blocks }")?;
    let report = update_report(
        &state,
        &principal,
        id,
        ReportChange {
            title: parsed.title,
            folder_id: parsed.folder_id,
            blocks: parsed.blocks,
        },
    )
    .await?;
    Ok(ApiJson(
        json!({ "ok": true, "report": report_json(&report) }),
    ))
}

/// Move a report to the trash for `principal`: `manage` on its folder, as
/// deleting a dashboard is. Shared with the assistant's `delete_report`.
///
/// # Errors
///
/// As [`delete`].
pub(crate) async fn trash_report(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
) -> Result<Report, ApiError> {
    let (_, report) = writable_report(state, principal, id, Level::Manage).await?;
    let pool = pool_of(state)?;
    if !stored::trash(pool, id, &principal.display_name)
        .await
        .map_err(|e| store_failure(&e))?
    {
        return Err(not_found());
    }
    Ok(report)
}

/// `DELETE /api/dashboard/reports/{id}` — to the trash, restorable for 30
/// days.
///
/// # Errors
///
/// 404 [`REPORT_NOT_FOUND`] for a report the caller cannot read; 403 without
/// `manage` on its folder; 503 without the console database.
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let id = report_id(&id)?;
    let report = trash_report(&state, &principal, id).await?;
    Ok(ApiJson(json!({
        "ok": true,
        "id": report.id,
        "retentionDays": lakehouse_bi::trash::RETENTION_DAYS,
    })))
}

// ── Reading ─────────────────────────────────────────────────────────────

/// One chart of a dashboard payload, as a report's chart block shows it: its
/// render spec, its result and its dashboard's name. `None` when the payload
/// does not hold it.
fn chart_entry(payload: &Value, chart: &str, board: &str) -> Option<Value> {
    let spec = payload
        .get("charts")
        .and_then(Value::as_array)?
        .iter()
        .find(|c| c.get("id").and_then(Value::as_str) == Some(chart))?
        .clone();
    let cell = payload
        .get("results")
        .and_then(|r| r.get(chart))
        .cloned()
        .unwrap_or(Value::Null);
    let board_name = payload
        .get("boards")
        .and_then(Value::as_array)
        .and_then(|boards| {
            boards
                .iter()
                .find(|b| b.get("id").and_then(Value::as_str) == Some(board))
        })
        .and_then(|b| b.get("name"))
        .cloned()
        .unwrap_or_else(|| {
            json!(if board == store::DEFAULT_BOARD_ID {
                "Main"
            } else {
                ""
            })
        });
    Some(json!({ "spec": spec, "cell": cell, "board": board, "boardName": board_name }))
}

/// One chart block's read: its entry and the payload's `reporting` context,
/// or the status the dashboard read answered with.
type ChartRead = Result<(Value, Option<Value>), StatusCode>;

/// Every chart block of `report` read as `principal`, keyed by chart id, and
/// the `reporting` context (time zone, number locale, palette) of the first
/// chart read. The one copy: the reader route and the PDF's print view
/// (`routes::render`, T2) both call it, so the PDF shows what the page shows.
pub(crate) async fn read_charts(
    state: &AppState,
    principal: &Principal,
    viewer: &Viewer,
    report: &Report,
    headers: &HeaderMap,
) -> Result<(BTreeMap<String, Value>, Option<Value>), ApiError> {
    let blocks: Vec<Block> = serde_json::from_value(report.blocks.clone()).unwrap_or_default();
    let wanted = reports::chart_ids(&blocks);
    let mut out = BTreeMap::new();
    if wanted.is_empty() {
        return Ok((out, None));
    }
    // Where each chart is *now*, among the charts this reader may see: a chart
    // they cannot see is not available without anything being run for it.
    let visible: HashMap<String, String> = store::list_stored_charts(&state.clickhouse, viewer)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .into_iter()
        .filter(|c| !c.spec.kind.is_static())
        .map(|c| (c.spec.id.clone(), c.board))
        .collect();
    let reads: Vec<(String, String)> = wanted
        .into_iter()
        .filter_map(|chart| {
            if let Some(board) = visible.get(&chart) {
                Some((chart, board.clone()))
            } else {
                out.insert(chart, json!({ "unavailable": CHART_NOT_AVAILABLE }));
                None
            }
        })
        .collect();
    let results: Vec<(String, ChartRead)> = futures::stream::iter(reads)
        .map(|(chart, board)| async move {
            let query = DashboardQuery::for_report(&board, &chart);
            let read = match payload_for(state, principal, &query, headers).await {
                Ok(payload) => chart_entry(&payload, &chart, &board)
                    .map(|entry| (entry, payload.get("reporting").cloned()))
                    .ok_or(StatusCode::NOT_FOUND),
                Err(response) => Err(response.status()),
            };
            (chart, read)
        })
        .buffered(CHARTS_AT_ONCE)
        .collect()
        .await;
    let mut reporting = None;
    for (chart, read) in results {
        let entry = match read {
            Ok((entry, context)) => {
                reporting = reporting.or(context);
                entry
            }
            Err(StatusCode::NOT_FOUND | StatusCode::FORBIDDEN) => {
                json!({ "unavailable": CHART_NOT_AVAILABLE })
            }
            // `payload_for` already logged the cause under a reference.
            Err(_) => json!({ "unavailable": CHART_NOT_LOADED }),
        };
        out.insert(chart, entry);
    }
    Ok((out, reporting))
}

/// `GET /api/dashboard/reports/{id}` — the report, every chart block read as
/// the caller, what they may do with it, and whether a PDF can be made.
///
/// # Errors
///
/// 403 for a caller with neither `dashboard:read` nor a rule; 404
/// [`REPORT_NOT_FOUND`] for a report they cannot read; 503 without the
/// console database; the classified store error.
pub async fn get(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<ApiJson<Value>> {
    let scope = payload_scope(&state, &principal).await?;
    let id = report_id(&id)?;
    let report = readable_report(&state, &scope.viewer, id).await?;
    let (charts, reporting) =
        read_charts(&state, &principal, &scope.viewer, &report, &headers).await?;
    let format = crate::routes::settings::format_settings(&state).await?;
    // The level decides what the page offers; the write routes check it again.
    let level = if principal.has("dashboard:write") {
        scope.folder_level(&report.folder_id).unwrap_or(Level::View)
    } else {
        Level::View
    };
    Ok(ApiJson(json!({
        "report": report_json(&report),
        "charts": charts,
        "reporting": reporting,
        "access": { "level": level.as_str() },
        "maxCharts": reports::MAX_CHARTS,
        "pdfSupported": crate::routes::dashboard::pdf_supported(&state.config),
        "canDownload": crate::routes::settings::may_download(&principal, &format.download_denied_roles),
    })))
}

// ── The trash ───────────────────────────────────────────────────────────

/// The reports in the trash that `viewer` may see, shaped as
/// `lakehouse_bi::trash::TrashItem` with `kind: "report"`, for `GET
/// /api/dashboard/trash`. A report whose folder is gone comes back to the
/// root, so it is judged at the root.
///
/// # Errors
///
/// The classified store error. Without the console database there are none.
pub(crate) async fn trash_items(state: &AppState, viewer: &Viewer) -> Result<Vec<Value>, ApiError> {
    let Some(pool) = state.pg.as_deref() else {
        return Ok(Vec::new());
    };
    let rows = stored::list_trashed(pool)
        .await
        .map_err(|e| store_failure(&e))?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let visible = visible_folders(state, viewer).await?;
    let live = folders::list_folders(&state.clickhouse, &Viewer::system())
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let retention = time::Duration::days(i64::from(lakehouse_bi::trash::RETENTION_DAYS));
    let clickhouse_time = |at: OffsetDateTime| {
        let format =
            time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
        at.to_offset(time::UtcOffset::UTC)
            .format(&format)
            .unwrap_or_default()
    };
    Ok(rows
        .iter()
        .filter(|r| {
            let folder = if live.iter().any(|f| f.id == r.folder_id) {
                r.folder_id.as_str()
            } else {
                ""
            };
            folder_readable(viewer, &visible, folder)
        })
        .map(|r| {
            json!({
                "kind": "report",
                "id": r.id,
                "name": r.title,
                "board": "",
                "boardName": "",
                "boardState": "",
                "deletedBy": r.deleted_by,
                "deletedAt": clickhouse_time(r.deleted_at),
                "expiresAt": clickhouse_time(r.deleted_at + retention),
                "personal": live.iter().any(|f| f.id == r.folder_id && !f.owner_id.is_empty()),
            })
        })
        .collect())
}

/// Bring report `id` back from the trash, for `POST
/// /api/dashboard/trash/restore` with `kind: "report"`: `manage` on its folder
/// (undoing a delete), or `edit` at the root when that folder is gone, where
/// it then goes.
///
/// # Errors
///
/// 404 when it is not in the trash for the caller; 403 below the level; the
/// classified store error.
pub(crate) async fn restore_from_trash(
    state: &AppState,
    scope: &Scope,
    id: &str,
) -> Result<Value, ApiError> {
    let missing = || ApiError::NotFound("That item is not in the trash.".to_owned());
    let id = id.trim().parse::<Uuid>().map_err(|_| missing())?;
    let pool = pool_of(state)?;
    let trashed = stored::get_trashed(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(missing)?;
    let live = folders::list_folders(&state.clickhouse, &Viewer::system())
        .await
        .map_err(|err| classify_ch_error(&err))?;
    let folder_live =
        trashed.folder_id.is_empty() || live.iter().any(|f| f.id == trashed.folder_id);
    let target = if folder_live {
        trashed.folder_id.as_str()
    } else {
        ""
    };
    let visible = visible_folders(state, &scope.viewer).await?;
    if !folder_readable(&scope.viewer, &visible, target) {
        return Err(missing());
    }
    if folder_live {
        scope.require_on_folder(target, Level::Manage)?;
    } else {
        scope.require_root(Level::Edit)?;
    }
    if !stored::restore(pool, id, target)
        .await
        .map_err(|e| store_failure(&e))?
    {
        return Err(missing());
    }
    Ok(json!({ "kind": "report", "id": id, "charts": 0, "boardState": "" }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_chart_entry_is_the_one_chart_its_result_and_its_dashboard_name() {
        let payload = json!({
            "charts": [{ "id": "c_1", "title": "A" }, { "id": "c_2", "title": "B" }],
            "results": { "c_1": { "rows": [1] }, "c_2": { "rows": [2] } },
            "boards": [{ "id": "b_1", "name": "Sales" }],
        });
        let entry = chart_entry(&payload, "c_2", "b_1").unwrap();
        assert_eq!(entry["spec"]["title"], "B");
        assert_eq!(entry["cell"], json!({ "rows": [2] }));
        assert_eq!(entry["boardName"], "Sales");
        assert!(chart_entry(&payload, "c_3", "b_1").is_none());
    }

    #[test]
    fn a_report_id_that_is_not_a_uuid_is_the_same_not_found() {
        let err = report_id("../etc").unwrap_err();
        assert_eq!(err.to_string(), REPORT_NOT_FOUND);
    }

    #[test]
    fn the_root_is_readable_only_with_a_starting_point() {
        assert!(folder_readable(&Viewer::user("u1", false), &[], ""));
        assert!(!folder_readable(&Viewer::nothing(), &[], ""));
        assert!(!folder_readable(&Viewer::user("u1", false), &[], "f_gone"));
    }
}
