//! Dashboard templates (`BI-18` part D, plan
//! `docs/superpowers/plans/2026-11-13-bi-18d-reports-templates.md` T3; feature
//! page `docs/core/features/reports-and-templates.md`, decisions T1 to T4).
//!
//! - `GET /api/dashboard/templates` — every template.
//! - `POST /api/dashboard/templates { board, name }` — save a dashboard as one.
//! - `POST /api/dashboard/templates/{id}/use { name, folderId }` — start a new
//!   dashboard from one.
//! - `DELETE /api/dashboard/templates/{id}` — delete one for good.
//!
//! All four need `dashboard:write` at the router (T4, `BI-18D-AC8`).
//!
//! # What a template holds (T1)
//!
//! The dashboard's definition, never data: the dashboard file
//! (`lakehouse_bi::dashboard_file::export`, the serialisation `BI-19` part D
//! downloads, so a template holds exactly what a dashboard file holds: settings,
//! filters, tabs, layout, chart definitions and the tables they read) and, for
//! each chart, its stored document (`store::stored_document`). No snapshot, no
//! version history, no sharing state: the file refuses those fields, and the
//! stored document is the chart's own row. Saving needs the dashboard to be
//! one the person can see; the template is then for everyone with
//! `dashboard:write` on this deployment (T4), with no access rules of its own.
//!
//! # Starting from one (T2)
//!
//! The new dashboard is an independent copy, made the way a duplicate is: the
//! board is created in the folder the person names (`edit` there, as `POST
//! /api/dashboard/boards`), every chart gets a fresh id and is written from its
//! stored document, the layout is re-keyed to the new ids, and the settings go
//! through the checks of `PUT /api/dashboard/boards` (`apply_imported_content`).
//! Changing the copy changes nothing in the template, and the template is not
//! read again.
//!
//! A chart is copied as it was, not rebuilt against today's schema and not run:
//! a chart whose table has been dropped is still a tile, and reading it gives
//! the dashboard's own refusal for a table that is not there (the "source not
//! available" state, decision T2), never an empty chart. A chart over a SQL
//! source is the exception, as for an import (`BI-19` D5): without
//! `dashboard:sql`, or with a source the person may not use, it is left out
//! and named with a fixed reason. A failure after the board was made takes the
//! board back.
//!
//! SEC-11: every store failure is classified; no upstream text reaches a
//! response.

use std::collections::{BTreeMap, BTreeSet};

use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, State};
use lakehouse_auth::Principal;
use lakehouse_bi::access::{Level, Viewer};
use lakehouse_bi::dashboard_file::{self as file, DashboardFile};
use lakehouse_bi::store::{self, BiError};
use lakehouse_core::ApiError;
use lakehouse_store::dashboard_templates::{self as stored, TemplateInput, TemplateSummary};
use serde::Deserialize;
use serde_json::{Map, Value, json};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::routes::dashboard::{ImportedContent, apply_imported_content};
use crate::routes::dashboard_access::{require_board_level, scope_for_write};
use crate::routes::dashboard_folders::{
    classify_bi_error, classify_ch_error, ensure_folder_usable,
};
use crate::routes::dashboard_organise::{place_new_chart, snapshot};
use crate::routes::dashboard_reports::{pool_of, store_failure};
use crate::state::AppState;
use crate::upstream_error;

/// The one answer for a template id that is not one.
pub(crate) const TEMPLATE_NOT_FOUND: &str = "template not found.";

/// The built-in dashboard has no stored charts to save.
const BUILTIN_REFUSED: &str =
    "The built-in dashboard has no stored charts to save as a template. Duplicate it first.";

/// Why a chart of a template was left out of the new dashboard. Fixed
/// sentences, the same as an import's for the same cause (`BI-19` D5).
const NEEDS_SQL: &str = "It reads a SQL source, which needs the dashboard SQL permission.";
const SOURCE_UNAVAILABLE: &str = "The SQL source it reads is not available here.";
const NOT_READABLE: &str = "Its saved definition could not be read.";

/// The longest template name, in characters (as a report title).
const MAX_NAME_CHARS: usize = 200;

fn not_found() -> ApiError {
    ApiError::NotFound(TEMPLATE_NOT_FOUND.to_owned())
}

fn template_id(raw: &str) -> Result<Uuid, ApiError> {
    raw.trim().parse::<Uuid>().map_err(|_| not_found())
}

fn checked_name(raw: &str, what: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(ApiError::BadRequest(format!("A {what} needs a name.")));
    }
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(ApiError::BadRequest(format!(
            "A {what} name is at most 200 characters."
        )));
    }
    Ok(name.to_owned())
}

pub(crate) fn summary_json(t: &TemplateSummary) -> Value {
    json!({
        "id": t.id,
        "name": t.name,
        "sourceBoard": t.source_board,
        "sourceBoardName": t.source_board_name,
        "chartCount": t.chart_count,
        "createdBy": t.created_by,
        "createdAt": t.created_at.format(&Rfc3339).unwrap_or_default(),
    })
}

fn parse<T: serde::de::DeserializeOwned>(body: &Bytes, shape: &str) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::BadRequest(format!("body must be {shape}.")))
}

// ── List ────────────────────────────────────────────────────────────────

/// Every template, newest first.
///
/// # Errors
///
/// 503 without the console database; the classified store error.
pub(crate) async fn list_templates(state: &AppState) -> Result<Value, ApiError> {
    let pool = pool_of(state)?;
    let (rows, truncated) = stored::list(pool).await.map_err(|e| store_failure(&e))?;
    Ok(json!({
        "templates": rows.iter().map(summary_json).collect::<Vec<_>>(),
        "truncated": truncated,
    }))
}

/// `GET /api/dashboard/templates`.
///
/// # Errors
///
/// As [`list_templates`].
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    Ok(ApiJson(list_templates(&state).await?))
}

// ── Save ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveBody {
    board: String,
    name: String,
}

/// Save dashboard `board` as a template named `name`, for `principal`: the
/// checks of `POST /api/dashboard/templates`, shared with the assistant.
///
/// # Errors
///
/// As [`save`].
pub(crate) async fn save_template(
    state: &AppState,
    principal: &Principal,
    board: &str,
    name: &str,
) -> Result<TemplateSummary, ApiError> {
    let scope = scope_for_write(state, principal).await?;
    let name = checked_name(name, "template")?;
    let board_id = board.trim();
    if board_id.is_empty() || board_id == store::DEFAULT_BOARD_ID {
        return Err(ApiError::BadRequest(BUILTIN_REFUSED.to_owned()));
    }
    // Seeing the dashboard is enough to copy its definition; it is not changed.
    let Some(board) = require_board_level(state, &scope, board_id, Level::View).await? else {
        return Err(ApiError::BadRequest(BUILTIN_REFUSED.to_owned()));
    };
    let charts: Vec<_> = store::list_stored_charts(&state.clickhouse, &scope.viewer)
        .await
        .map_err(|err| classify_ch_error(&err))?
        .into_iter()
        .filter(|c| c.board == board.id)
        .collect();
    if charts.len() > file::MAX_CHARTS {
        return Err(ApiError::BadRequest(
            file::FileError::TooManyCharts.message().to_owned(),
        ));
    }
    // The file lists the tables the charts read without their columns: a
    // template is used on this deployment, so nothing is mapped from it.
    let document = file::export(&board, &charts, &BTreeMap::new());
    let documents: Map<String, Value> = charts
        .iter()
        .map(|c| (c.spec.id.clone(), store::stored_document(c)))
        .collect();
    let definition = json!({ "file": document, "charts": documents });
    let pool = pool_of(state)?;
    stored::insert(
        pool,
        TemplateInput {
            name: &name,
            source_board: &board.id,
            source_board_name: &board.name,
            chart_count: i32::try_from(charts.len()).unwrap_or(i32::MAX),
            definition: &definition,
            by: &principal.display_name,
        },
    )
    .await
    .map_err(|e| store_failure(&e))
}

/// `POST /api/dashboard/templates { board, name }`.
///
/// # Errors
///
/// 400 for a blank or long name, the built-in dashboard, or a dashboard of more
/// than 200 charts; 404 for a dashboard the person cannot see; 503 without the
/// console database; the classified store error.
pub async fn save(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: SaveBody = parse(&body, "{ board, name }")?;
    let saved = save_template(&state, &principal, &parsed.board, &parsed.name).await?;
    Ok(ApiJson(
        json!({ "ok": true, "template": summary_json(&saved) }),
    ))
}

// ── Use ─────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UseBody {
    name: String,
    #[serde(default)]
    folder_id: Option<String>,
}

/// A template's definition read back: the file (checked again by the file's
/// own parser) and each chart's stored document.
fn read_definition(definition: &Value) -> Result<(DashboardFile, Map<String, Value>), ApiError> {
    let raw = definition.get("file").cloned().unwrap_or(Value::Null);
    let size = serde_json::to_vec(&raw).map_or(usize::MAX, |v| v.len());
    let document = file::parse(raw, size).map_err(|err| {
        // A stored template that no longer reads is the server's problem.
        upstream_error::internal_error(&upstream_error::SERVICE, &err.message())
    })?;
    let charts = definition
        .get("charts")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    Ok((document, charts))
}

/// What starting from a template made.
pub(crate) struct Started {
    pub(crate) board: store::Board,
    pub(crate) left_out: Vec<Value>,
}

/// Start a new dashboard from template `id`, for `principal`: the checks of
/// `POST /api/dashboard/templates/{id}/use`, shared with the assistant's
/// `create_dashboard_from_template`.
///
/// # Errors
///
/// As [`use_template`].
pub(crate) async fn start_from_template(
    state: &AppState,
    principal: &Principal,
    id: Uuid,
    name: &str,
    folder_id: &str,
) -> Result<Started, ApiError> {
    let scope = scope_for_write(state, principal).await?;
    let name = checked_name(name, "dashboard")?;
    let ch = &state.clickhouse;
    let folder_id = folder_id.trim().to_owned();
    ensure_folder_usable(ch, &folder_id, &scope.viewer).await?;
    scope.require_on_folder(&folder_id, Level::Edit)?;
    let pool = pool_of(state)?;
    let template = stored::get(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
        .ok_or_else(not_found)?;
    let (document, documents) = read_definition(&template.definition)?;
    let folder_owner = if folder_id.is_empty() {
        String::new()
    } else {
        lakehouse_bi::folders::folder_owner(ch, &folder_id)
            .await
            .map_err(|err| classify_ch_error(&err))?
    };
    let board = store::create_board_in(
        ch,
        &name,
        &document.board.description,
        &principal.display_name,
        &folder_id,
        &folder_owner,
    )
    .await
    .map_err(|err| classify_bi_error(&err))?;

    // As an import (`BI-19` D review SF-D2): the build and the take-back run in
    // a task of their own, so a client that goes away mid-copy leaves either a
    // finished dashboard or none.
    let task = tokio::spawn({
        let (state, principal, viewer) = (state.clone(), principal.clone(), scope.viewer.clone());
        let board_id = board.id.clone();
        async move {
            let built = build(
                &state,
                &principal,
                &viewer,
                &board_id,
                (&document, &documents),
            )
            .await;
            match built {
                Ok(left_out) => {
                    snapshot(&state.clickhouse, &board_id, &principal).await;
                    Ok(left_out)
                }
                Err(err) => {
                    if let Err(cleanup) = store::wipe_board(&state.clickhouse, &board_id).await {
                        let _logged =
                            upstream_error::report_ch(&upstream_error::DATABASE, &cleanup);
                    }
                    Err(err)
                }
            }
        }
    });
    let left_out = task
        .await
        .map_err(|err| upstream_error::internal_error(&upstream_error::SERVICE, &err))??;
    let mut shown = board;
    shown.folder_id = Some(folder_id);
    Ok(Started {
        board: shown,
        left_out,
    })
}

/// The charts, then the dashboard's settings. Returns the charts left out.
async fn build(
    state: &AppState,
    principal: &Principal,
    viewer: &Viewer,
    board: &str,
    (document, documents): (&DashboardFile, &Map<String, Value>),
) -> Result<Vec<Value>, ApiError> {
    let ch = &state.clickhouse;
    let may_sql = principal.has("dashboard:sql");
    let mut ids: BTreeMap<String, String> = BTreeMap::new();
    let mut left_out = Vec::new();
    for chart in &document.charts {
        let left = |reason: &str| json!({ "title": chart.def.title, "reason": reason });
        let Some(mut copy) = documents
            .get(&chart.id)
            .and_then(|doc| store::stored_from_document(doc, board, "ui"))
        else {
            left_out.push(left(NOT_READABLE));
            continue;
        };
        if !store::sql_sources_named(&copy.def).is_empty() {
            if !may_sql {
                left_out.push(left(NEEDS_SQL));
                continue;
            }
            match store::ensure_sources_visible(ch, &copy.def, viewer, &[]).await {
                Ok(()) => {}
                Err(BiError::Validation(_)) => {
                    left_out.push(left(SOURCE_UNAVAILABLE));
                    continue;
                }
                Err(err) => return Err(classify_bi_error(&err)),
            }
        }
        copy.spec.id = store::new_chart_id();
        // Not run first (decision T2): a chart whose table is gone keeps its
        // tile, and reading it says the table is not there.
        store::insert_chart_unchecked(ch, &copy)
            .await
            .map_err(|err| classify_ch_error(&err))?;
        ids.insert(chart.id.clone(), copy.spec.id);
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
    Ok(left_out)
}

/// `POST /api/dashboard/templates/{id}/use { name, folderId? }` — a new
/// dashboard from the template, in the folder named (`""` = the root).
///
/// # Errors
///
/// 400 for a blank or long name, a folder the person may not file into, or
/// settings the editor would refuse (the half-made dashboard is taken back);
/// 403 without `edit` on the folder; 404 [`TEMPLATE_NOT_FOUND`]; 503 without
/// the console database. A chart left out is not an error: it is in
/// `leftOut` with its reason.
pub async fn use_template(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let id = template_id(&id)?;
    let parsed: UseBody = parse(&body, "{ name, folderId }")?;
    let started = start_from_template(
        &state,
        &principal,
        id,
        &parsed.name,
        parsed.folder_id.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(ApiJson(json!({
        "ok": true,
        "board": started.board,
        "leftOut": started.left_out,
    })))
}

// ── Delete ──────────────────────────────────────────────────────────────

/// Delete template `id` for good. Shared with the assistant's
/// `delete_template`.
///
/// # Errors
///
/// 404 [`TEMPLATE_NOT_FOUND`]; 503 without the console database.
pub(crate) async fn delete_template(state: &AppState, id: Uuid) -> Result<(), ApiError> {
    let pool = pool_of(state)?;
    if stored::delete(pool, id)
        .await
        .map_err(|e| store_failure(&e))?
    {
        Ok(())
    } else {
        Err(not_found())
    }
}

/// `DELETE /api/dashboard/templates/{id}`. Dashboards made from it stay.
///
/// # Errors
///
/// As [`delete_template`].
pub async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ApiJson<Value>> {
    let id = template_id(&id)?;
    delete_template(&state, id).await?;
    Ok(ApiJson(json!({ "ok": true, "id": id })))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn a_name_is_trimmed_and_bounded_with_a_sentence_for_the_thing_named() {
        assert_eq!(checked_name("  Sales ", "template").unwrap(), "Sales");
        assert_eq!(
            checked_name(" ", "template").unwrap_err().to_string(),
            "A template needs a name."
        );
        assert_eq!(
            checked_name(&"x".repeat(201), "dashboard")
                .unwrap_err()
                .to_string(),
            "A dashboard name is at most 200 characters."
        );
    }

    #[test]
    fn a_template_id_that_is_not_a_uuid_is_the_same_not_found() {
        assert_eq!(
            template_id("x").unwrap_err().to_string(),
            TEMPLATE_NOT_FOUND
        );
    }
}
