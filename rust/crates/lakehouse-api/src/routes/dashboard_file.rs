//! A dashboard as a file, and import (`BI-19` part D, plan D14, feature D4/D5).
//!
//! - `GET /api/dashboard/export?format=file&board=` — one dashboard as JSON.
//!   Read through the requester's resolved viewer: a dashboard their rules
//!   leave out, in the trash or in somebody else's personal space is the same
//!   404 as an unknown id. The file describes the dashboard and the tables its
//!   charts read; it holds no row of data and no sharing state.
//! - `POST /api/dashboard/import/preview` — for each table in a file, the
//!   table here it most likely means (the same name, else none), so the
//!   console can show a mapping to confirm.
//! - `POST /api/dashboard/import` — make a new dashboard from a file.
//!
//! # Import is creation, by the importer
//!
//! The new dashboard is made by `create_board_in` in the folder the importer
//! names, which needs `edit` on that folder (part B2, as `POST
//! /api/dashboard/boards`). Nothing in the file chooses its owner, folder,
//! sharing or ids ([`lakehouse_bi::dashboard_file`] refuses those fields). Each
//! chart goes through `store::spec_from_input`, the function a chart saved by
//! hand goes through, after its tables are replaced by the importer's mapping,
//! so a column, calculated field, metric or parameter that is not here leaves
//! that chart out. A chart over a SQL source needs `dashboard:sql` (feature D5:
//! the permission that writes SQL) and a source the importer may see; no SQL
//! source is made from a file. The dashboard's own settings are saved through
//! the checks of `PUT /api/dashboard/boards`. A failure after the board was
//! made takes it back.
//!
//! Every left-out chart is reported with one of the fixed reasons below; the
//! store's own text never reaches the response.

use std::collections::{BTreeMap, BTreeSet};

use axum::Extension;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_bi::access::Level;
use lakehouse_bi::dashboard_file::{self as file, DashboardFile};
use lakehouse_bi::specs::ChartSource;
use lakehouse_bi::store::{self, BiError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard::{ImportedContent, apply_imported_content, file_slug};
use crate::routes::dashboard_access::{require_can_ever, scope_for_write, viewer_for};
use crate::routes::dashboard_folders::{classify_bi_error, classify_ch_error};
use crate::routes::dashboard_organise::{BOARD_NOT_FOUND, place_new_chart, snapshot};
use crate::state::AppState;
use crate::upstream_error;

/// Why a chart was left out (feature D5). Fixed sentences: they name the kind
/// of problem, never the file's or the database's text.
const NOT_MAPPED: &str = "Its table was not matched to a table here.";
const NEEDS_SQL: &str = "It reads a SQL source, which needs the dashboard SQL permission.";
const SOURCE_UNAVAILABLE: &str = "The SQL source it reads is not available here.";
const MISSING_PART: &str =
    "A column, calculated field, metric or parameter it uses is missing here.";
const QUERY_FAILED: &str = "Its query did not run here.";

/// `GET /api/dashboard/export?format=file&board=` (called by `dashboard::export`).
///
/// # Errors
///
/// 400 for a missing `board` or the built-in dashboard (duplicate it first: it
/// has no stored charts); 404 for a dashboard the person cannot see; 403 when
/// downloads are off for the person's roles (`BI-19` Q-D1); otherwise
/// the classified store error.
pub(crate) async fn export_file(
    state: &AppState,
    principal: &Principal,
    board: Option<&str>,
) -> ApiResult<Response> {
    let viewer = viewer_for(state, principal).await?;
    let id = board
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::BadRequest("board is required.".to_owned()))?;
    if id == store::DEFAULT_BOARD_ID {
        return Err(ApiError::BadRequest(
            "The built-in dashboard has no stored charts to export. Duplicate it first.".to_owned(),
        )
        .into());
    }
    let ch = &state.clickhouse;
    let board = store::get_board(ch, id, &viewer)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .ok_or_else(|| ApiError::NotFound(BOARD_NOT_FOUND.to_owned()))?;
    // BI-19 Q-D1 (owner decision 2026-10-27): the file holds table and column
    // names, filter and parameter values and KPI targets, so it follows the
    // per-role download switch (BI-1). After the visibility check above, so a
    // hidden dashboard stays the same 404 for a role with downloads off.
    crate::routes::settings::require_download(state, principal).await?;
    let charts: Vec<_> = store::list_stored_charts(ch, &viewer)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .into_iter()
        .filter(|c| c.board == id)
        .collect();
    let mut columns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let tables: BTreeSet<String> = charts
        .iter()
        .flat_map(|c| file::chart_tables(&c.def))
        .collect();
    for table in tables {
        match store::validated_mart_columns(ch, &table).await {
            Ok(cols) => {
                let mut names: Vec<String> = cols.into_keys().collect();
                names.sort();
                columns.insert(table, names);
            }
            // A table that is gone is listed without columns.
            Err(BiError::Validation(_)) => {}
            Err(err) => return Err(classify_bi_error(&err).into()),
        }
    }
    let document = file::export(&board, &charts, &columns);
    let body = serde_json::to_vec_pretty(&document)
        .map_err(|err| upstream_error::internal_error(&upstream_error::SERVICE, &err))?;
    let file_name = format!(
        "{}.rantai-dashboard.json",
        file_slug(&board.name, "dashboard")
    );
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        body,
    )
        .into_response())
}

/// The body of both import routes: the file, and for the import the mapping
/// and the folder.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ImportBody {
    file: Value,
    /// File table name to the table here.
    #[serde(default)]
    mapping: BTreeMap<String, String>,
    /// Where the new dashboard goes (`""` = the root).
    #[serde(default)]
    folder_id: Option<String>,
}

/// Room for the wrapper around a file of [`file::MAX_BYTES`].
const BODY_SLACK: usize = 64 * 1024;

/// Read and check a posted body: size first, then the file.
fn read_body(body: &Bytes) -> Result<(ImportBody, DashboardFile), ApiError> {
    if body.len() > file::MAX_BYTES + BODY_SLACK {
        return Err(ApiError::BadRequest(
            file::FileError::TooBig.message().to_owned(),
        ));
    }
    let parsed: ImportBody = serde_json::from_slice(body).map_err(|_| {
        ApiError::BadRequest("body must be { file, mapping, folderId }.".to_owned())
    })?;
    // Measured as the file itself, not as the body that carries it.
    let size = serde_json::to_vec(&parsed.file).map_or(usize::MAX, |v| v.len());
    let document = file::parse(parsed.file.clone(), size)
        .map_err(|err| ApiError::BadRequest(err.message().to_owned()))?;
    Ok((parsed, document))
}

/// The tables of the serving layer: the ones a chart can read (the list the
/// chart builder's picker shows).
async fn readable_tables(state: &AppState) -> Result<BTreeSet<String>, ApiError> {
    let rows = state
        .clickhouse
        .rows(
            "SELECT name FROM system.tables WHERE database='serving' \
             AND name NOT LIKE '%\\_baru' ORDER BY name",
            None,
        )
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(rows
        .iter()
        .filter_map(|r| r.get("name").and_then(Value::as_str).map(str::to_owned))
        .collect())
}

/// `POST /api/dashboard/import/preview { file }` — what the file holds and the
/// table here that matches each of its tables.
///
/// # Errors
///
/// 400 with the file's problem (too big, too many charts, not a dashboard file,
/// unknown version); 403 for a person who can edit nothing; a classified store
/// error.
pub async fn import_preview(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    require_can_ever(&state, &principal, Level::Edit).await?;
    let (_, document) = read_body(&body)?;
    let readable = readable_tables(&state).await?;
    let mut tables = Vec::new();
    for table in &document.tables {
        let suggested = readable.contains(&table.name).then(|| table.name.clone());
        let mut missing: Vec<String> = Vec::new();
        if let Some(target) = &suggested {
            match store::validated_mart_columns(&state.clickhouse, target).await {
                Ok(cols) => {
                    missing = table
                        .columns
                        .iter()
                        .filter(|c| !cols.contains_key(*c))
                        .cloned()
                        .collect();
                }
                Err(BiError::Validation(_)) => {}
                Err(err) => return Err(classify_bi_error(&err).into()),
            }
        }
        tables.push(json!({
            "name": table.name,
            "columns": table.columns,
            "suggested": suggested,
            "missingColumns": missing,
        }));
    }
    let charts: Vec<Value> = document
        .charts
        .iter()
        .map(|c| {
            json!({
                "id": c.id,
                "title": c.def.title,
                "kind": c.def.kind,
                "tables": file::chart_tables(&c.def),
                "readsSqlSource": !store::sql_sources_named(&c.def).is_empty(),
            })
        })
        .collect();
    Ok(ApiJson(json!({
        "name": document.board.name,
        "charts": charts,
        "tables": tables,
        "readableTables": readable,
        "canImportSql": principal.has("dashboard:sql"),
    })))
}

/// What one chart came to.
enum Outcome {
    Made(String),
    LeftOut(&'static str),
}

/// `POST /api/dashboard/import { file, mapping, folderId }` — make a new
/// dashboard from a file.
///
/// # Errors
///
/// As [`import_preview`]; 404 for a folder the person cannot see; 403 without
/// `edit` on the folder; 400 with the editor's own sentence when the file's
/// dashboard settings are not ones it could save (the half-made dashboard is
/// taken back). A chart that cannot be made is not an error: it is in
/// `leftOut` with its reason.
pub async fn import(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    // The same gate as `POST /api/dashboard/boards`: `edit` somewhere, then
    // `edit` on the folder the dashboard is filed in.
    require_can_ever(&state, &principal, Level::Edit).await?;
    let scope = scope_for_write(&state, &principal).await?;
    let (parsed, document) = read_body(&body)?;
    let ch = &state.clickhouse;
    let folder_id = parsed.folder_id.unwrap_or_default().trim().to_owned();
    crate::routes::dashboard_folders::ensure_folder_usable(ch, &folder_id, &scope.viewer).await?;
    scope.require_on_folder(&folder_id, Level::Edit)?;
    let folder_owner = if folder_id.is_empty() {
        String::new()
    } else {
        lakehouse_bi::folders::folder_owner(ch, &folder_id)
            .await
            .map_err(|err| classify_ch_error(&err))?
    };

    // The mapping: only file tables can be mapped, and only to a table that is
    // here. A chart that needs any other table is left out below.
    let readable = readable_tables(&state).await?;
    let in_file: BTreeSet<&str> = document.tables.iter().map(|t| t.name.as_str()).collect();
    let mapping: BTreeMap<String, String> = parsed
        .mapping
        .iter()
        .filter(|(from, _)| in_file.contains(from.as_str()))
        .filter(|(_, to)| {
            readable.contains(to.trim().strip_prefix("serving.").unwrap_or(to.trim()))
        })
        .map(|(from, to)| {
            (
                from.clone(),
                to.trim()
                    .strip_prefix("serving.")
                    .unwrap_or(to.trim())
                    .to_owned(),
            )
        })
        .collect();

    let author = principal.display_name.clone();
    let board = store::create_board_in(
        ch,
        &document.board.name,
        &document.board.description,
        &author,
        &folder_id,
        &folder_owner,
    )
    .await
    .map_err(|err| classify_bi_error(&err))?;

    // BI-19 D review SF-D2: the build and the take-back run in a task of their
    // own, which the handler awaits. A client that goes away mid-import drops
    // this handler's future, not the task, so the half-made board is always
    // either finished or wiped.
    let task = tokio::spawn({
        let (state, principal, viewer) = (state.clone(), principal.clone(), scope.viewer.clone());
        let board_id = board.id.clone();
        async move {
            let built = build(
                &state,
                &principal,
                &viewer,
                &board_id,
                (&document, &mapping),
            )
            .await;
            match built {
                Ok(result) => {
                    snapshot(&state.clickhouse, &board_id, &principal).await;
                    Ok(result)
                }
                Err(err) => {
                    // The half-made dashboard is taken back; a cleanup that
                    // fails as well goes to the log under a reference, and the
                    // caller gets the error that stopped the import.
                    if let Err(cleanup) = store::wipe_board(&state.clickhouse, &board_id).await {
                        let _logged =
                            upstream_error::report_ch(&upstream_error::DATABASE, &cleanup);
                    }
                    Err(err)
                }
            }
        }
    });
    let result = task
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::SERVICE, &err))??;
    let mut shown = board;
    shown.folder_id = Some(folder_id);
    Ok(ApiJson(json!({
        "ok": true,
        "board": shown,
        "imported": result.made,
        "leftOut": result.left_out,
    })))
}

struct Built {
    made: Vec<Value>,
    left_out: Vec<Value>,
}

/// The charts, then the dashboard's settings.
async fn build(
    state: &AppState,
    principal: &Principal,
    viewer: &lakehouse_bi::access::Viewer,
    board: &str,
    (document, mapping): (&DashboardFile, &BTreeMap<String, String>),
) -> Result<Built, ApiError> {
    let ch = &state.clickhouse;
    let time = crate::routes::settings::time_context(state).await?;
    let may_sql = principal.has("dashboard:sql");
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let mut made = Vec::new();
    let mut left_out = Vec::new();
    for chart in &document.charts {
        let outcome = make_chart(state, viewer, board, chart, (mapping, may_sql), &time).await?;
        match outcome {
            Outcome::Made(new_id) => {
                made.push(json!({ "id": new_id, "title": chart.def.title }));
                ids.insert(chart.id.clone(), new_id);
            }
            Outcome::LeftOut(reason) => {
                left_out.push(json!({ "title": chart.def.title, "reason": reason }));
            }
        }
    }
    let (layout, phone_layout) =
        file::rekey(&document.board.layout, &document.board.phone_layout, &ids);
    let placed: BTreeSet<String> = layout.keys().cloned().collect();
    apply_imported_content(
        state,
        board,
        ImportedContent {
            filters: document.board.filters.clone(),
            tabs: document.board.tabs.clone(),
            layout,
            phone_layout,
            params: document.board.params.clone(),
            grain: document.board.grain.clone(),
            refresh_seconds: document.board.refresh_seconds,
            theme: document.board.theme.clone(),
            font: document.board.font.clone(),
        },
    )
    .await?;
    for new_id in ids.values().filter(|id| !placed.contains(*id)) {
        place_new_chart(ch, board, new_id).await;
    }
    Ok(Built { made, left_out })
}

/// One chart: retarget, check, save. A problem with the chart itself is a
/// reason; a store that cannot answer stops the import.
async fn make_chart(
    state: &AppState,
    viewer: &lakehouse_bi::access::Viewer,
    board: &str,
    chart: &file::FileChart,
    (mapping, may_sql): (&BTreeMap<String, String>, bool),
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<Outcome, ApiError> {
    let ch = &state.clickhouse;
    let mut input = chart.def.clone();
    if !store::sql_sources_named(&input).is_empty() && !may_sql {
        return Ok(Outcome::LeftOut(NEEDS_SQL));
    }
    // A table that was never matched, or was matched to one that is not here
    // (the mapping above drops those), leaves the chart out.
    if file::retarget(&mut input, mapping, board).is_err() {
        return Ok(Outcome::LeftOut(NOT_MAPPED));
    }
    match store::ensure_sources_visible(ch, &input, viewer, &[]).await {
        Ok(()) => {}
        Err(BiError::Validation(_)) => return Ok(Outcome::LeftOut(SOURCE_UNAVAILABLE)),
        Err(err) => return Err(classify_bi_error(&err)),
    }
    let spec = match store::spec_from_input(ch, &input, ChartSource::Ui, "ui", None, time).await {
        Ok(spec) => spec,
        Err(BiError::Validation(_)) => return Ok(Outcome::LeftOut(MISSING_PART)),
        Err(err) => return Err(classify_bi_error(&err)),
    };
    // BI-19 D review SF-D1: only the chart's own statement failing is "its query
    // did not run here"; a store that cannot write stops the import (which then
    // takes the board back) instead of reading as a chart left out.
    if let Err(err) = store::smoke_test_chart(ch, &spec).await {
        // The engine's text goes to the log under a reference.
        let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
        return Ok(Outcome::LeftOut(QUERY_FAILED));
    }
    store::insert_chart_unchecked(ch, &spec)
        .await
        .map_err(|err| classify_ch_error(&err))?;
    Ok(Outcome::Made(spec.spec.id))
}
