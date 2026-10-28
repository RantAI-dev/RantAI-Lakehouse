//! `/api/maps`: uploaded maps (`BI-16` part C2b, plan
//! `2026-10-24-bi-16c-advanced-charts-maps.md`, decision D5).
//!
//! An administrator (`settings:write`) uploads a `GeoJSON` boundary file; it is
//! checked and simplified for drawing by `lakehouse_bi::geo` and stored in
//! Postgres (`geo_map`, migration `0100`). Everyone signed in can list the
//! maps and read a file, because a map is visible to everyone who can make
//! charts (feature decision 4). A chart names a map by its server-made id
//! (`m_` and 12 hex digits) in its `map` field, as it names a bundled map.
//!
//! # The five routes and the contract with the console
//!
//! | route | who | answer |
//! |---|---|---|
//! | `GET /api/maps` | signed in | `{ maps: [{ id, name, nameProperty, regionCount, createdAt, createdBy }] }` |
//! | `POST /api/maps` | `settings:write` | 201 and the map; multipart `name`, `nameProperty`, `file` |
//! | `GET /api/maps/{id}/geojson` | signed in | the simplified file, `properties.name` on every feature, `ETag`, `Cache-Control: private, max-age=3600` |
//! | `PATCH /api/maps/{id}` | `settings:write` | `{ name }` renames; the map |
//! | `DELETE /api/maps/{id}` | `settings:write` | `{ ok: true }`, or 409 `{ error, charts: [...] }` while a chart uses it |
//!
//! A refused file is a 400 with the sentence from `lakehouse_bi::geo` (a
//! feature number or a region name at most, never more of the file). A file
//! over 50 MB is refused before it is parsed; `lakehouse_core::ApiError` has
//! no 413, so it is a 400 as `/api/uploads` answers (the console accepts 400 or
//! 413). Parsing and simplifying are CPU work on up to 50 MB, so they run on a
//! blocking thread, and the same receive limiter as `/api/uploads` bounds how
//! many files one person and the installation hold in memory at once.
//!
//! # Deleting a map in use
//!
//! The charts that name the map are read from the chart store with the system
//! viewer, so the check sees every chart. The 409 names only the charts the
//! caller may see (as the SQL-source delete does): a chart on somebody's
//! personal board still blocks the delete but is not named. A chart store
//! that cannot be read blocks the delete too (the check fails closed). A
//! calculated field whose `RegionOf` names the map, by id or by name, blocks it
//! the same way (`fields` in the 409; BI-16 C3 review S3). A rename is not
//! checked: it breaks fields that name the map by its label, so use the id.

use axum::Extension;
use axum::extract::multipart::{Multipart, MultipartError, MultipartRejection};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_bi::access::Viewer;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use lakehouse_store::PgPool;
use lakehouse_store::StoreError;
use lakehouse_store::audit::{self as store_audit, NewAuditEvent};
use lakehouse_store::geo_map::{self, GeoMap, NewGeoMap};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::format_description::well_known::Rfc3339;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::state::AppState;
use crate::upstream_error;

/// The largest file accepted, like an upload (plan D5).
pub(super) const MAX_MAP_BYTES: usize = 50 * 1024 * 1024;
/// The request body limit: the file plus the multipart framing.
pub(super) const MAX_REQUEST_BODY_BYTES: usize = MAX_MAP_BYTES + 1024 * 1024;
/// The longest label of a map, in characters (the table's `CHECK`).
const MAX_NAME_CHARS: usize = 80;

const NOT_MULTIPART: &str =
    "The request must be a multipart form with the parts name, nameProperty and file.";
const UNREADABLE: &str = "The upload could not be read.";
const TOO_LARGE: &str = "This file is over 50 MB. Simplify it or split it, then upload again.";
const NO_FILE: &str = "The form has no part named file.";
const EMPTY_FILE: &str = "The file is empty.";
const BAD_NAME: &str = "Give the map a name of 1 to 80 characters on one line.";
const NAME_TAKEN: &str = "A map with this name already exists.";
const NOT_FOUND: &str = "No such map.";
const BAD_BODY: &str = "The request body must be {\"name\": \"<new name>\"}.";
const TASK_FAILED: &str = "The file could not be processed.";

/// A map as the console reads it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MapView {
    id: String,
    name: String,
    name_property: String,
    region_count: i32,
    created_at: String,
    created_by: String,
}

impl MapView {
    fn from_row(row: GeoMap) -> Self {
        Self {
            created_at: row.created_at.format(&Rfc3339).unwrap_or_default(),
            id: row.id,
            name: row.name,
            name_property: row.name_property,
            region_count: row.region_count,
            created_by: row.created_by,
        }
    }
}

fn pool(state: &AppState) -> Result<&PgPool, ApiError> {
    state.pg.as_deref().ok_or_else(|| {
        ApiError::Unavailable(
            "map store unavailable: no Postgres pool is configured \
             (DATABASE_URL is missing or not a valid Postgres connection string)"
                .to_owned(),
        )
    })
}

/// `BI-16` review SHOULD-FIX S1: one audit event per map write, with the map's
/// id and name and never the file. Best effort, like the upload events: a
/// failed audit write never fails, or undoes, the write it records.
async fn audit(pool: &PgPool, principal: &Principal, action: &str, id: &str, name: &str) {
    let event = NewAuditEvent {
        principal_id: Some(principal.id.uuid().to_string()),
        principal_kind: Some(principal.kind_for_audit().to_owned()),
        actor_label: Some(principal.display_name.clone()),
        action: action.to_owned(),
        resource_kind: Some("map".to_owned()),
        resource_id: Some(id.to_owned()),
        args: Some(json!({ "name": name })),
        outcome: "executed".to_owned(),
        detail: None,
        run_id: None,
        approval_id: None,
        session_id: None,
    };
    if let Err(err) = store_audit::insert(pool, event).await {
        tracing::warn!(%err, action, "failed to record a map audit event");
    }
}

fn store_failed(err: &dyn std::fmt::Display) -> ApiError {
    upstream_error::internal_error(&upstream_error::STORE, err)
}

/// Whether `id` has the shape the server makes (`m_` and 12 lower-case hex
/// digits). Anything else is an unknown map without asking the database.
fn well_formed(id: &str) -> bool {
    id.strip_prefix("m_").is_some_and(|rest| {
        rest.len() == 12
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// A label fit to store: trimmed, 1 to 80 characters, one line.
fn clean_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();
    if name.is_empty()
        || name.chars().count() > MAX_NAME_CHARS
        || name.chars().any(char::is_control)
    {
        return Err(ApiError::BadRequest(BAD_NAME.to_owned()));
    }
    Ok(name.to_owned())
}

/// `GET /api/maps`.
///
/// # Errors
///
/// 503 without a Postgres pool; 5xx through `upstream_error` on a database failure.
pub async fn list(State(state): State<AppState>) -> ApiResult<ApiJson<Value>> {
    let rows = geo_map::list(pool(&state)?)
        .await
        .map_err(|err| store_failed(&err))?;
    let maps: Vec<MapView> = rows.into_iter().map(MapView::from_row).collect();
    Ok(ApiJson(json!({ "maps": maps })))
}

/// A form that could not be read to its end.
fn refused_read(err: &MultipartError) -> ApiError {
    if err.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return ApiError::BadRequest(TOO_LARGE.to_owned());
    }
    tracing::warn!(%err, "a map upload form could not be read");
    ApiError::BadRequest(UNREADABLE.to_owned())
}

struct Form {
    name: String,
    name_property: String,
    file: Vec<u8>,
}

/// The three parts, read whole. Other parts are skipped, unread.
async fn read_form(mut form: Multipart) -> Result<Form, ApiError> {
    let (mut name, mut property, mut file) = (None, None, None);
    while let Some(field) = form.next_field().await.map_err(|err| refused_read(&err))? {
        match field.name() {
            Some("file") => file = Some(field.bytes().await.map_err(|err| refused_read(&err))?),
            Some("name") => name = Some(field.text().await.map_err(|err| refused_read(&err))?),
            Some("nameProperty") => {
                property = Some(field.text().await.map_err(|err| refused_read(&err))?);
            }
            _ => {}
        }
    }
    let file = file.ok_or_else(|| ApiError::BadRequest(NO_FILE.to_owned()))?;
    Ok(Form {
        name: name.unwrap_or_default(),
        name_property: property.unwrap_or_default(),
        file: file.to_vec(),
    })
}

/// What one upload produces: the file the console draws, the number of
/// regions, and the statements that put the full boundaries on the engine.
struct Processed {
    simplified: Vec<u8>,
    regions: usize,
    inserts: Vec<String>,
}

/// Validate and simplify on a blocking thread; the refusal's own sentence is the 400.
/// BI-16 C3 review S8: this comment had slid onto [`Processed`].
async fn check_and_simplify(
    file: Vec<u8>,
    property: String,
    id: String,
) -> Result<Processed, ApiError> {
    let done = tokio::task::spawn_blocking(move || {
        let checked = lakehouse_bi::geo::validate(&file, &property)?;
        drop(file);
        let inserts = checked
            .region_inserts(&id)
            .ok_or_else(|| TASK_FAILED.to_owned())?;
        lakehouse_bi::geo::simplify(&checked).map(|simplified| Processed {
            simplified,
            regions: checked.regions.len(),
            inserts,
        })
    })
    .await;
    match done {
        Ok(Ok(result)) => Ok(result),
        Ok(Err(sentence)) => Err(ApiError::BadRequest(sentence)),
        Err(err) => {
            tracing::warn!(%err, "a map processing task did not finish");
            Err(ApiError::Internal(TASK_FAILED.to_owned()))
        }
    }
}

/// `POST /api/maps`: multipart `name`, `nameProperty` and `file`. 201 and the map.
///
/// # Errors
///
/// 429 when the caller or the installation already holds the configured number
/// of files being received; 400 for a request that is not the form, no file, an
/// empty or oversized file, a name that is not 1 to 80 characters, or a file
/// `lakehouse_bi::geo` refuses (with its sentence); 409 when the name is taken;
/// 503 without a Postgres pool.
pub async fn create(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    form: Result<Multipart, MultipartRejection>,
) -> ApiResult<(StatusCode, ApiJson<MapViewOut>)> {
    let pool = pool(&state)?;
    // As for an upload: a place is taken before the body is read, because the
    // file is held in memory (up to 50 MB) until it is stored.
    let Some(_place) = state.upload_receive.try_acquire(&principal.display_name) else {
        return Err(ApiError::TooManyRequests {
            message: "Too many files are being received. Try again in a moment.".to_owned(),
            retry_after_secs: 5,
        }
        .into());
    };
    let form = form.map_err(|err| {
        tracing::warn!(%err, "a map upload was not a readable multipart form");
        ApiError::BadRequest(NOT_MULTIPART.to_owned())
    })?;
    let form = read_form(form).await?;
    let name = clean_name(&form.name)?;
    if form.file.is_empty() {
        return Err(ApiError::BadRequest(EMPTY_FILE.to_owned()).into());
    }
    // Refused before it is parsed.
    if form.file.len() > MAX_MAP_BYTES {
        return Err(ApiError::BadRequest(TOO_LARGE.to_owned()).into());
    }
    let property = form.name_property.trim().to_owned();
    let id = format!("m_{}", &Uuid::new_v4().simple().to_string()[..12]);
    let Processed {
        simplified: bytes,
        regions,
        inserts,
    } = check_and_simplify(form.file, property.clone(), id.clone()).await?;
    let created = geo_map::insert(
        pool,
        &NewGeoMap {
            id: &id,
            name: &name,
            name_property: &property,
            region_count: i32::try_from(regions).unwrap_or(i32::MAX),
            geojson: &bytes,
            created_by: &principal.display_name,
        },
    )
    .await
    .map_err(|err| match err {
        StoreError::Conflict => ApiError::Conflict(NAME_TAKEN.to_owned()),
        other => store_failed(&other),
    })?;
    audit(pool, &principal, "map_upload", &created.id, &created.name).await;
    // BI-16 part C3 (D8): the region lookup is set up after the map is saved
    // and never fails the upload; without it the map still draws and
    // `RegionOf` says it is not available for this map.
    let created = enable_region_lookup(&state, pool, created, &inserts).await;
    Ok((
        StatusCode::CREATED,
        ApiJson(MapViewOut(MapView::from_row(created))),
    ))
}

/// Creates the map's region lookup on the engine (`console.geo_region` rows,
/// then the polygon dictionary, loaded at once) and records that it exists.
///
/// Measured on a throwaway `ClickHouse` 26.8.30.8
/// (`docs/plans/BI-16C-RESULT.md`): the dictionary reads as the engine user
/// `lakehouse_geo`, which has NO password, may log in from the local host only
/// and may only SELECT the boundary table (`PR #246 review BLOCKER 2`), so no
/// credential is written into the dictionary's DDL or into the engine's own
/// metadata. Any failure is
/// logged under a reference id, undone as far as it can be, and leaves the
/// flag off: the map stays usable for drawing, and the formula compiler tells
/// `RegionOf` that the map has no lookup.
async fn enable_region_lookup(
    state: &AppState,
    pool: &PgPool,
    map: GeoMap,
    inserts: &[String],
) -> GeoMap {
    let ch = &state.clickhouse;
    let steps = async {
        ch.exec(lakehouse_bi::geo::REGION_TABLE_DDL, None).await?;
        // PR #246 review BLOCKER 2: the dictionary reads as a dedicated
        // engine user with NO password, which may log in from the local host
        // only and may only SELECT the boundary table. The API creates it
        // (and grants it) here, right where the table it reads is created
        // and at the same "first upload that needs it" moment; both
        // statements are idempotent.
        ch.exec(lakehouse_bi::geo::GEO_USER_DDL, None).await?;
        ch.exec(lakehouse_bi::geo::GEO_GRANT_DDL, None).await?;
        for statement in inserts {
            ch.exec(statement, None).await?;
        }
        for sql in [
            lakehouse_bi::geo::dictionary_ddl(&map.id),
            lakehouse_bi::geo::reload_dictionary_sql(&map.id),
        ]
        .into_iter()
        .flatten()
        {
            ch.exec(&sql, None).await?;
        }
        Ok::<(), lakehouse_clickhouse::ChError>(())
    };
    match steps.await {
        Ok(()) => match geo_map::set_region_lookup(pool, &map.id, true).await {
            Ok(true) => GeoMap {
                region_lookup: true,
                ..map
            },
            // BI-16 C3 review S4: the map was deleted while its upload was
            // still setting the lookup up; the delete could not drop what did
            // not exist yet, so drop it here and answer with the flag off.
            Ok(false) => {
                remove_region_lookup(state, &map.id).await;
                map
            }
            Err(err) => {
                tracing::warn!(%err, "a map's region lookup was created but could not be recorded");
                remove_region_lookup(state, &map.id).await;
                map
            }
        },
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            remove_region_lookup(state, &map.id).await;
            map
        }
    }
}

/// Removes a map's dictionary and region rows, best effort: a map that is gone
/// (or whose lookup failed halfway) must not leave boundaries behind.
async fn remove_region_lookup(state: &AppState, id: &str) {
    for sql in [
        lakehouse_bi::geo::drop_dictionary_sql(id),
        lakehouse_bi::geo::delete_regions_sql(id),
    ]
    .into_iter()
    .flatten()
    {
        if let Err(err) = state.clickhouse.exec(&sql, None).await {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
        }
    }
}

/// The body of a map answer: the map itself, not wrapped.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct MapViewOut(MapView);

/// `GET /api/maps/{id}/geojson`: the simplified file. `ETag` is the file's
/// `md5` from Postgres; a matching `If-None-Match` answers 304.
///
/// # Errors
///
/// 404 for an unknown id; 503 without a Postgres pool.
pub async fn geojson(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    map_file(&state, &id, &headers).await
}

/// The file of a map, as every reader of it gets it: the simplified
/// `GeoJSON`, an `ETag` that changes with the file, and a `Cache-Control`
/// that keeps it out of shared caches. A matching `If-None-Match` answers
/// 304.
///
/// `PR #246 review BLOCKER 1`: the public link and the embed read a map's
/// boundary through their own token, so they come here too — the bytes are
/// the same file the console draws, and the map's own route keeps the
/// signed-in reader.
///
/// # Errors
///
/// 404 for an id that is not server-made or that names no map; 503 without a
/// Postgres pool; 5xx through `upstream_error` on a database failure.
pub(super) async fn map_file(
    state: &AppState,
    id: &str,
    headers: &HeaderMap,
) -> ApiResult<Response> {
    if !well_formed(id) {
        return Err(ApiError::NotFound(NOT_FOUND.to_owned()).into());
    }
    let file = geo_map::file(pool(state)?, id)
        .await
        .map_err(|err| store_failed(&err))?
        .ok_or_else(|| ApiError::NotFound(NOT_FOUND.to_owned()))?;
    let etag = format!("\"{}\"", file.etag);
    let cache = "private, max-age=3600";
    let unchanged = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|sent| {
            sent.split(',')
                .any(|t| t.trim().trim_start_matches("W/") == etag)
        });
    // A raw GeoJSON body, not `ApiJson`: the console loads it as a map file.
    if unchanged {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, cache.to_owned()),
            ],
        )
            .into_response());
    }
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "application/geo+json".to_owned()),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache.to_owned()),
        ],
        file.geojson,
    )
        .into_response())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenameBody {
    name: String,
}

/// `PATCH /api/maps/{id}` with `{ "name": "…" }`. The map.
///
/// # Errors
///
/// 400 for a bad body or name; 404 for an unknown id; 409 when the name is
/// taken; 503 without a Postgres pool.
pub async fn rename(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> ApiResult<ApiJson<MapViewOut>> {
    if !well_formed(&id) {
        return Err(ApiError::NotFound(NOT_FOUND.to_owned()).into());
    }
    let body: RenameBody =
        serde_json::from_slice(&body).map_err(|_| ApiError::BadRequest(BAD_BODY.to_owned()))?;
    let name = clean_name(&body.name)?;
    let pool = pool(&state)?;
    let renamed = geo_map::rename(pool, &id, &name)
        .await
        .map_err(|err| match err {
            StoreError::Conflict => ApiError::Conflict(NAME_TAKEN.to_owned()),
            other => store_failed(&other),
        })?
        .ok_or_else(|| ApiError::NotFound(NOT_FOUND.to_owned()))?;
    audit(pool, &principal, "map_rename", &renamed.id, &renamed.name).await;
    Ok(ApiJson(MapViewOut(MapView::from_row(renamed))))
}

/// `DELETE /api/maps/{id}`: `{ ok: true }`, or 409 with the charts that use it.
///
/// # Errors
///
/// 404 for an unknown id; 409 while a chart uses the map; 5xx through
/// `upstream_error` when the chart store cannot be read; 503 without a
/// Postgres pool.
pub async fn delete(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, ApiJson<Value>)> {
    if !well_formed(&id) {
        return Err(ApiError::NotFound(NOT_FOUND.to_owned()).into());
    }
    let pool = pool(&state)?;
    let Some(existing) = geo_map::get(pool, &id)
        .await
        .map_err(|err| store_failed(&err))?
    else {
        return Err(ApiError::NotFound(NOT_FOUND.to_owned()).into());
    };
    let ch = &state.clickhouse;
    let uses = |charts: &[store::StoredChartSpec]| -> Vec<(String, String, String)> {
        charts
            .iter()
            .filter(|c| c.def.map.as_deref() == Some(id.as_str()))
            .map(|c| (c.spec.id.clone(), c.spec.title.clone(), c.board.clone()))
            .collect()
    };
    let all = store::list_stored_charts(ch, &Viewer::system())
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
    if !uses(&all).is_empty() {
        let viewer = crate::routes::dashboard_organise::viewer_of(&principal);
        let mine = store::list_stored_charts(ch, &viewer)
            .await
            .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
        let boards = store::list_boards(ch, &viewer)
            .await
            .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?;
        let title_of = |board: &str| {
            boards
                .iter()
                .find(|b| b.id == board)
                .map_or_else(|| board.to_owned(), |b| b.name.clone())
        };
        let charts: Vec<Value> = uses(&mine)
            .into_iter()
            .map(|(chart_id, chart_title, board)| {
                json!({
                    "chartId": chart_id, "chartTitle": chart_title,
                    "dashboardId": board, "dashboardTitle": title_of(&board),
                })
            })
            .collect();
        let message = if charts.is_empty() {
            "This map is still used by a dashboard you cannot see.".to_owned()
        } else {
            format!(
                "This map is still used by {} chart(s). Change or delete them first.",
                charts.len()
            )
        };
        return Ok((
            StatusCode::CONFLICT,
            ApiJson(json!({ "error": message, "charts": charts })),
        ));
    }
    // BI-16 C3 review S3: a calculated field whose `RegionOf` names the map
    // (by id or by name) would fall back to its stored SQL and fail with a
    // generic database error once the dictionary is gone. A field store that
    // cannot be read blocks the delete too (fail closed).
    let named_by: Vec<String> = lakehouse_bi::fields::list_fields(ch)
        .await
        .map_err(|err| upstream_error::ch_error(&upstream_error::DATABASE, &err))?
        .into_iter()
        .filter(lakehouse_bi::fields::FieldDef::is_formula_field)
        .filter(|f| {
            lakehouse_bi::formula::compile::region_maps_in(&f.formula)
                .iter()
                .any(|m| m == &id || m.eq_ignore_ascii_case(&existing.name))
        })
        .map(|f| f.name)
        .collect();
    if !named_by.is_empty() {
        return Ok((
            StatusCode::CONFLICT,
            ApiJson(json!({
                "error": format!(
                    "This map is still used by {} calculated field(s) through RegionOf: {}. Change them first.",
                    named_by.len(),
                    named_by.join(", ")
                ),
                "fields": named_by,
            })),
        ));
    }
    geo_map::delete(pool, &id)
        .await
        .map_err(|err| store_failed(&err))?;
    audit(pool, &principal, "map_delete", &id, &existing.name).await;
    remove_region_lookup(&state, &id).await;
    Ok((StatusCode::OK, ApiJson(json!({ "ok": true }))))
}
