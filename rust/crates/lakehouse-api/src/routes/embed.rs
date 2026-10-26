//! `POST /api/embed/data`, `GET /api/public/dashboard/{token}` — read-only
//! dashboard views for external viewers (signed embed & public share link).
//!
//! Ports `src/app/api/embed/data/route.ts` and
//! `src/app/api/public/dashboard/[token]/route.ts`. Both assemble the same
//! `{ board, layout, charts, results }` payload as `dashboard::get`, but
//! scoped to one board and with dashboard-wide filters *locked* (a signed
//! embed's JWT `params`, or a public board's own stored filters) rather
//! than caller-supplied.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use lakehouse_bi::access::Viewer;
use lakehouse_bi::store::{self, Board, FilterDef, StoredChartSpec};
use lakehouse_clickhouse::{ChClient, ChError};
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::ApiResult;
use crate::json::ApiJson;
use crate::policy_engine::PolicyEngineObligations;
use crate::routes::support::{
    annotate_metric_labels, mart_columns, render_stored_spec, run_spec_sql, sources_for,
    stored_chart_sql,
};
use crate::state::AppState;
use crate::upstream_error;

/// WS7 item D1: neither `POST /api/embed/data` nor
/// `GET /api/public/dashboard/{token}` carries a real principal —
/// `Policy::Public`, checked in `route_auth.rs`'s own table-driven loop
/// against `policy.rs`'s `POLICY_TABLE`. Both are governed as if the
/// viewer held exactly this seeded, least-privileged, dashboard-only role
/// (`0002_seed_identity.sql:46`), never as an unrestricted principal — a
/// masking/row-filter policy authored against `"Dashboard Viewer"` (or
/// against no role at all, which matches nobody) is the only way to
/// restrict what an external embed/public-link viewer can see through
/// these two routes. This is a deliberate mapping, documented here, not
/// an accidental "no principal, so no obligations" fail-open path (WS7
/// plan Hard Requirement 2).
pub(crate) const EMBED_VIEWER_ROLE: &str = "Dashboard Viewer";

/// `{ jwt, filters? }` — the `POST /api/embed/data` body shape. `filters` is
/// the dashboard page's filter shape (`BI-26`, plan D4): the values a viewer
/// chose for the filters a manager made editable.
#[derive(Debug, Default, Deserialize)]
struct EmbedDataBody {
    #[serde(default)]
    jwt: Option<String>,
    #[serde(default)]
    filters: Option<Vec<FilterDef>>,
}

/// The one message every refused signed token gets (`SEC-12`): a bad
/// signature, a missing or too-long lifetime, an expired or withdrawn token
/// and a token without a dashboard all read the same.
pub(crate) const TOKEN_REFUSED: &str = "embed token is invalid or expired";

/// What every signed-embed surface says when `EMBED_SECRET` is unset
/// (`SEC-12`): `POST /api/embed/data` answers it with `503`, and
/// `GET /api/dashboard/embed-info` as the `reason` of `supported: false`.
pub(crate) const EMBEDDING_NOT_CONFIGURED: &str = "embedding is not configured";

fn token_refused() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        ApiJson(json!({ "error": TOKEN_REFUSED })),
    )
        .into_response()
}

/// Take the column of a hidden filter out of every tile's `filtersSkipped`
/// (`BI-26` review NOTE): a tile that says which active filters it could not
/// honour would otherwise name a filter the viewer is not meant to know of.
/// Locked and editable filters, which the viewer knows, stay. Signed embeds only.
fn strip_hidden_skips(body: &mut Value, access: &lakehouse_bi::embed_access::EmbedAccess) {
    use lakehouse_bi::embed_access::FilterMode;
    let Some(results) = body.get_mut("results").and_then(Value::as_object_mut) else {
        return;
    };
    for tile in results.values_mut() {
        let Some(list) = tile.get_mut("filtersSkipped").and_then(Value::as_array_mut) else {
            continue;
        };
        list.retain(|skipped| {
            skipped
                .get("column")
                .and_then(Value::as_str)
                .is_none_or(|column| access.mode_of(column) != FilterMode::Hidden)
        });
        if list.is_empty()
            && let Some(map) = tile.as_object_mut()
        {
            map.remove("filtersSkipped");
        }
    }
}

/// The filters of one signed-embed read (`BI-26` plan D3).
struct ResolvedFilters {
    /// What the tiles are filtered by.
    filters: Vec<FilterDef>,
    /// The columns whose filter the viewer may change here: editable, and not
    /// set by the token.
    editable: Vec<String>,
}

/// Work out the filters of a signed-embed read from the board's saved filters,
/// each one's embed mode, the token's `params` and the viewer's values.
///
/// - `locked`: the token must name the column, else the read is refused
///   (`Err(column)`; feature decision 6). A forgotten value never shows all rows.
///   The token's value replaces the saved filter on that column (R1).
/// - `editable`: the viewer's value replaces the saved one; with none the saved
///   value applies.
/// - `hidden` (and a column nobody chose a mode for): the saved value applies.
///
/// A viewer value for a column that is not editable, or that the token set, is
/// dropped. The token's `params` are applied last, as before, and only narrow
/// (feature decision 7): a legacy token that locks a column that is no filter
/// keeps working.
fn resolve_embed_filters(
    saved: &[FilterDef],
    access: &lakehouse_bi::embed_access::EmbedAccess,
    params: Option<&std::collections::HashMap<String, Value>>,
    viewer: &[FilterDef],
) -> Result<ResolvedFilters, String> {
    use lakehouse_bi::embed_access::FilterMode;
    let token_sets = |column: &str| params.is_some_and(|p| p.contains_key(column));
    let mode_of = |f: &FilterDef| {
        if f.column.is_empty() {
            FilterMode::Hidden
        } else {
            access.mode_of(&f.column)
        }
    };
    for f in saved {
        if mode_of(f) == FilterMode::Locked && !token_sets(&f.column) {
            return Err(f.column.clone());
        }
    }
    let mut editable: Vec<String> = Vec::new();
    for f in saved {
        if mode_of(f) == FilterMode::Editable
            && !token_sets(&f.column)
            && !editable.contains(&f.column)
        {
            editable.push(f.column.clone());
        }
    }
    // BI-26 review fix (BLOCKER) R1: for a locked column, and an editable one
    // the token sets, the token's value REPLACES the saved filter. Left in
    // place, the saved value was ANDed with the token's: a board saved with
    // `Bali` and a token for `Java` showed nothing, and a locked filter could
    // never show a value outside the saved one. A hidden column keeps being
    // ANDed with the saved filter, so tokens signed before filter modes (which
    // lock a column that is hidden or no filter at all) read exactly as before.
    let mut filters: Vec<FilterDef> = saved
        .iter()
        .filter(|f| {
            let replaced = !f.column.is_empty()
                && token_sets(&f.column)
                && matches!(mode_of(f), FilterMode::Locked | FilterMode::Editable);
            !replaced
        })
        .cloned()
        .collect();
    for v in viewer {
        if v.column.is_empty() || !editable.contains(&v.column) {
            tracing::info!(column = %v.column, "a viewer filter value for a column that is not editable was dropped");
            continue;
        }
        filters.retain(|f| f.column != v.column);
        filters.push(v.clone());
    }
    filters.extend(params_to_filters(params.cloned()));
    Ok(ResolvedFilters { filters, editable })
}

/// What a verified token names, resolved to its dashboard (`BI-26` plan D1).
pub(crate) struct Subject {
    /// The dashboard every board check (embedding on, withdrawal, allowed
    /// sites) is made on. For a chart token, the chart's own dashboard.
    pub board_id: String,
    /// The one chart a chart token may read; `None` for a dashboard token.
    pub only_chart: Option<String>,
}

/// Resolve what a token names. `None` is a refusal and the caller answers it
/// with the one token message: the token names nothing, or both a dashboard and
/// a chart, or a chart that is unknown, in the trash or on a personal board
/// (the chart is looked up as nobody in particular, like the payload). One
/// helper for `data`, `frame_origins` and `revoke_token` (AGENTS rule 4).
///
/// # Errors
///
/// [`lakehouse_clickhouse::ChError`] when the chart list cannot be read.
pub(crate) async fn subject_of(
    ch: &ChClient,
    resource: Option<&lakehouse_embed::EmbedResource>,
) -> Result<Option<Subject>, lakehouse_clickhouse::ChError> {
    match resource.and_then(lakehouse_embed::EmbedResource::target) {
        Some(lakehouse_embed::EmbedTarget::Dashboard(id)) => Ok(Some(Subject {
            board_id: id.to_owned(),
            only_chart: None,
        })),
        Some(lakehouse_embed::EmbedTarget::Chart(id)) => {
            let stored = store::list_stored_charts(ch, &Viewer::anonymous()).await?;
            Ok(stored.iter().find(|c| c.spec.id == id).map(|c| Subject {
                // A chart with no board is on the built-in dashboard.
                board_id: if c.board.is_empty() {
                    store::DEFAULT_BOARD_ID.to_owned()
                } else {
                    c.board.clone()
                },
                only_chart: Some(id.to_owned()),
            }))
        }
        None => Ok(None),
    }
}

/// Verify a signed token (`SEC-12`): the secret is configuration only, and an
/// unset one means signed embedding is unavailable, said plainly (nothing is
/// generated or stored); every other failure is the one fixed message, the
/// reason going to the log only. Shared by `data` and `values`.
fn verified_claims(
    state: &AppState,
    jwt: &str,
) -> Result<lakehouse_embed::EmbedClaims, Box<Response>> {
    let Some(secret) = state.config.embed_secret.as_deref() else {
        return Err(Box::new(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                ApiJson(json!({ "error": EMBEDDING_NOT_CONFIGURED })),
            )
                .into_response(),
        ));
    };
    lakehouse_embed::verify_embed(
        jwt,
        secret,
        state.config.embed_token_max_lifetime_seconds,
        lakehouse_embed::unix_now(),
    )
    .map_err(|reason| {
        tracing::info!(%reason, "signed embed token refused");
        Box::new(token_refused())
    })
}

/// What every signed read checks about its token's dashboard, once: what the
/// token names ([`subject_of`]), that embedding is on for it, and that the
/// token has not been withdrawn. Returns the dashboard and the subject, or the
/// response to send.
async fn admit(
    state: &AppState,
    claims: &lakehouse_embed::EmbedClaims,
) -> Result<(Board, Subject), Box<Response>> {
    let subject = match subject_of(&state.clickhouse, claims.resource.as_ref()).await {
        Ok(Some(subject)) => subject,
        Ok(None) => return Err(Box::new(token_refused())),
        Err(err) => {
            return Err(Box::new(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
                )
                    .into_response(),
            ));
        }
    };
    let board = match store::get_board(&state.clickhouse, &subject.board_id, &Viewer::anonymous())
        .await
    {
        Ok(b) => b,
        Err(err) => {
            return Err(Box::new(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
                )
                    .into_response(),
            ));
        }
    };
    let Some(board) = board.filter(|b| b.embed_enabled.unwrap_or(false)) else {
        return Err(Box::new(
            (
                StatusCode::FORBIDDEN,
                ApiJson(json!({ "error": "embedding_disabled" })),
            )
                .into_response(),
        ));
    };
    // SEC-12: withdrawal is read on every request — the board's embed events
    // are aggregated on each read, with no cache in this process — so
    // "withdraw all" and "withdraw one" take effect on the next request. A
    // token with no `iat` cannot get here (`verify_embed` refuses it);
    // `NEG_INFINITY` would be refused if it did.
    // PR #201 review F SHOULD-FIX 4: the withdrawal is not read from the
    // board row any more (it lives in `console.bi_board_embed_event`), so
    // this is an aggregation of that table, not a `FINAL` read of the board.
    if board.embed_access.is_withdrawn(
        claims.iat.unwrap_or(f64::NEG_INFINITY),
        claims.jti.as_deref(),
    ) {
        return Err(Box::new(token_refused()));
    }
    // BI-26 review fix (SHOULD-FIX) R2: a chart token's own withdrawal follows the
    // chart. It is written on the dashboard the chart was on, so it is looked up
    // on every dashboard's events (a "withdraw all" is the dashboard's, and
    // applies to the dashboard the chart is on now).
    if subject.only_chart.is_some()
        && let Some(jti) = claims.jti.as_deref()
    {
        match store::jti_withdrawn_anywhere(&state.clickhouse, jti).await {
            Ok(false) => {}
            Ok(true) => return Err(Box::new(token_refused())),
            Err(err) => {
                return Err(Box::new(
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        ApiJson(
                            upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json(),
                        ),
                    )
                        .into_response(),
                ));
            }
        }
    }
    Ok((board, subject))
}

/// The board a signed embed token opens — or the refusal the token gets.
///
/// The checks and the answers are the ones `POST /api/embed/data` has always
/// given (`SEC-12`): a missing token is a 400, an unset secret a 503, a
/// token that does not verify (a bad signature, a missing or too-long
/// lifetime, an expired or withdrawn token, a token without a dashboard) the
/// one fixed 401, and a board that is not there or has embedding off a 403.
///
/// `PR #246 review BLOCKER 1`: the embed's map route reads the same token
/// the same way, so the two routes cannot disagree about what a token opens.
/// It is the same [`verified_claims`] and [`admit`] the data route uses, so a
/// chart token (`BI-26`) opens its own dashboard here too, exactly as it does
/// there.
///
/// # Errors
///
/// The refusal to send, as a ready response.
async fn embed_grant(state: &AppState, jwt: Option<&str>) -> Result<Subject, Box<Response>> {
    let Some(jwt) = jwt.filter(|s| !s.is_empty()) else {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                ApiJson(json!({ "error": "jwt is required" })),
            )
                .into_response(),
        ));
    };
    let claims = verified_claims(state, jwt)?;
    let (_board, subject) = admit(state, &claims).await?;
    Ok(subject)
}

/// `POST /api/embed/data` — signed-embed (Metabase-style) dashboard data.
pub async fn data(State(state): State<AppState>, body: Bytes) -> Response {
    // `try { jwt = String((await req.json())?.jwt ?? "") } catch { /* ignore */ }`
    // — an unparseable body is swallowed, not a 400; it just yields an
    // empty jwt, which the emptiness check below rejects the normal way.
    let parsed = serde_json::from_slice::<EmbedDataBody>(&body).ok();
    let viewer_filters = parsed.as_ref().and_then(|b| b.filters.clone());
    let jwt = parsed.and_then(|b| b.jwt).filter(|s| !s.is_empty());
    let Some(jwt) = jwt else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "jwt is required" })),
        )
            .into_response();
    };

    let claims = match verified_claims(&state, &jwt) {
        Ok(claims) => claims,
        Err(refusal) => return *refusal,
    };
    let (board, subject) = match admit(&state, &claims).await {
        Ok(admitted) => admitted,
        Err(refusal) => return *refusal,
    };
    let board_id = subject.board_id.clone();

    // BI-26 (D3): the saved filters, each by its embed mode, then the viewer's
    // values for the editable ones, then the token's values last.
    if let Some(viewer) = &viewer_filters
        && let Err(msg) = lakehouse_bi::filters::validate_filters(viewer)
    {
        return (StatusCode::BAD_REQUEST, ApiJson(json!({ "error": msg }))).into_response();
    }
    // A chart token gets that chart and nothing else, so it has no dropdown of
    // the dashboard's values: its editable filters read as hidden.
    let viewer_values = if subject.only_chart.is_some() {
        &[][..]
    } else {
        viewer_filters.as_deref().unwrap_or_default()
    };
    let resolved = match resolve_embed_filters(
        board.filters.as_deref().unwrap_or_default(),
        &board.embed_access,
        claims.params.as_ref(),
        viewer_values,
    ) {
        Ok(resolved) => resolved,
        Err(column) => {
            // The log says which column; the response never does.
            tracing::info!(%column, board = %board_id, "signed embed token refused: a locked filter has no value");
            return token_refused();
        }
    };
    let filters = resolved.filters;
    let editable = if subject.only_chart.is_some() {
        Vec::new()
    } else {
        resolved.editable
    };

    let time = match crate::routes::settings::time_context(&state).await {
        Ok(t) => t,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    // BI-17 part A: the number locale, company palette and fonts ride in the
    // payload's `reporting` object like the zone, so an embed formats as the
    // console does.
    let format = match crate::routes::settings::format_settings(&state).await {
        Ok(f) => f,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let payload = crate::result_cache::scoped(
        crate::result_cache::CacheScope::new(
            std::sync::Arc::clone(&state.result_cache),
            format.result_cache,
            crate::result_cache::Fresh::No,
        ),
        render_board_payload(
            &state.clickhouse,
            &PolicyEngineObligations::from_state(&state),
            &board,
            &board_id,
            &filters,
            (&time, &state.tile_slots),
            Narrowing {
                only_chart: subject.only_chart.as_deref(),
                editable: &editable,
            },
            &format,
        ),
    )
    .await;
    match payload {
        Ok((mut body, content_sites)) => {
            crate::routes::settings::add_formatting_for_anonymous(
                &mut body,
                &format,
                &content_sites,
            );
            strip_hidden_skips(&mut body, &board.embed_access);
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
}

/// `{ jwt, column }` — the `POST /api/embed/values` body shape.
#[derive(Debug, Default, Deserialize)]
struct EmbedValuesBody {
    #[serde(default)]
    jwt: Option<String>,
    #[serde(default)]
    column: Option<String>,
}

/// `POST /api/embed/values` — the values an editable filter's dropdown lists
/// (`BI-26` plan D5). Public: the token is the credential.
///
/// Every token check of `data` runs first (`verified_claims`, `admit`), then the
/// column must be editable for this token (a locked or hidden column, a column
/// the token set, and any column of a chart token are refused). The values come
/// from the same query as the dashboard picker, read as the fixed viewer role,
/// with the token's locked values and the board's saved values applied, so a
/// dropdown never lists a region the token locked away. At most 1,000 values.
///
/// # Errors
///
/// `400` without a token or a column; the one token message for a bad token;
/// `403 filter_not_editable` for a column the viewer may not change;
/// `503` without `EMBED_SECRET`.
pub async fn values(State(state): State<AppState>, body: Bytes) -> Response {
    let parsed = serde_json::from_slice::<EmbedValuesBody>(&body).unwrap_or_default();
    let (Some(jwt), Some(column)) = (
        parsed.jwt.filter(|s| !s.is_empty()),
        parsed.column.filter(|s| !s.is_empty()),
    ) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "jwt and column are required" })),
        )
            .into_response();
    };
    let claims = match verified_claims(&state, &jwt) {
        Ok(claims) => claims,
        Err(refusal) => return *refusal,
    };
    let (board, subject) = match admit(&state, &claims).await {
        Ok(admitted) => admitted,
        Err(refusal) => return *refusal,
    };
    // A chart token gets no dropdown of the dashboard's values, whatever else
    // its token lacks (checked first, so the answer does not depend on it).
    if subject.only_chart.is_some() {
        return (
            StatusCode::FORBIDDEN,
            ApiJson(json!({ "error": "filter_not_editable" })),
        )
            .into_response();
    }
    let Ok(resolved) = resolve_embed_filters(
        board.filters.as_deref().unwrap_or_default(),
        &board.embed_access,
        claims.params.as_ref(),
        &[],
    ) else {
        return token_refused();
    };
    if !resolved.editable.contains(&column) {
        return (
            StatusCode::FORBIDDEN,
            ApiJson(json!({ "error": "filter_not_editable" })),
        )
            .into_response();
    }
    let filters_json = serde_json::to_string(&resolved.filters).unwrap_or_else(|_| "[]".to_owned());
    let time = match crate::routes::settings::time_context(&state).await {
        Ok(t) => t,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let roles = [EMBED_VIEWER_ROLE.to_owned()];
    let obligations = PolicyEngineObligations::from_state(&state);
    match crate::routes::dashboard::values_with_limit(
        &state.clickhouse,
        crate::routes::dashboard::ValuesQuery::for_board(&column, &subject.board_id, filters_json),
        (&roles, &Viewer::anonymous()),
        (
            &crate::sql_rewrite::PlaceholderValues::none(),
            &obligations,
            &time,
        ),
        crate::routes::dashboard::EMBED_VALUES_LIMIT,
    )
    .await
    {
        Ok(found) => (StatusCode::OK, ApiJson(found)).into_response(),
        Err(err) => crate::error::ApiRejection(err).into_response(),
    }
}

/// What a manager is told when a filter that does not pick values is made
/// locked or editable (`BI-26` review fix R3).
const ONLY_VALUE_FILTERS: &str = "Only a filter that picks values can be locked or editable in an embed. Leave the others hidden.";

/// `{ board, filterModes }` — the `PUT /api/dashboard/embed-settings` body.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SettingsBody {
    board: String,
    filter_modes: std::collections::BTreeMap<String, lakehouse_bi::embed_access::FilterMode>,
}

/// `PUT /api/dashboard/embed-settings` — set each filter's embed mode
/// (`BI-26` plan D8). Needs `dashboard:write` until `BI-19` part B (feature
/// decision 11: manage on the dashboard). The columns must be the dashboard's
/// saved filters; the change is audited (`dashboard.embed_settings`) before it
/// is made.
///
/// # Errors
///
/// `400` for a body of another shape, the built-in dashboard, a column that is
/// no filter of the dashboard, or modes the store refuses; `404` for a
/// dashboard the caller cannot see; `503` when the change cannot be audited.
pub async fn put_settings(
    State(state): State<AppState>,
    axum::Extension(principal): axum::Extension<lakehouse_auth::Principal>,
    body: Bytes,
) -> ApiResult<ApiJson<Value>> {
    let parsed: SettingsBody = serde_json::from_slice(&body).map_err(|_| {
        ApiError::BadRequest(
            "body must be {\"board\": string, \"filterModes\": {column: \"locked\"|\"editable\"|\"hidden\"}}"
                .to_owned(),
        )
    })?;
    // BI-26 review fix (SHOULD-FIX) R6: the one guard for "a dashboard a manager
    // names and may see" (`public_links::managed_board`), not a second copy.
    let board =
        crate::routes::public_links::managed_board(&state, &principal, Some(&parsed.board)).await?;
    let known: std::collections::BTreeSet<&str> = board
        .filters
        .iter()
        .flatten()
        .map(|f| f.column.as_str())
        .collect();
    if parsed
        .filter_modes
        .keys()
        .any(|c| !known.contains(c.as_str()))
    {
        return Err(ApiError::BadRequest(
            "A mode can only be set for a filter the dashboard has.".to_owned(),
        )
        .into());
    }
    // BI-26 review fix (SHOULD-FIX) R3: locked and editable work for a filter that
    // picks values (`in`). A range, a relative date or a text match has no value a
    // token can set or a dropdown can offer, so it can only be hidden.
    let not_values: std::collections::BTreeSet<&str> = board
        .filters
        .iter()
        .flatten()
        .filter(|f| !f.op.is_in())
        .map(|f| f.column.as_str())
        .collect();
    if parsed.filter_modes.iter().any(|(column, mode)| {
        *mode != lakehouse_bi::embed_access::FilterMode::Hidden
            && not_values.contains(column.as_str())
    }) {
        return Err(ApiError::BadRequest(ONLY_VALUE_FILTERS.to_owned()).into());
    }
    // The audit helper is the download audit's single writer; the action name
    // and the arguments make the record this change's own (BI-26 review NOTE).
    crate::routes::dashboard::record_download_audit(
        &state,
        &principal,
        ("dashboard.embed_settings", "dashboard", &board.id),
        json!({ "filterModes": parsed.filter_modes }),
    )
    .await
    .map_err(|_| ApiError::Unavailable("the audit store is unavailable".to_owned()))?;
    store::set_board_embed_filter_modes(&state.clickhouse, &board.id, &parsed.filter_modes)
        .await
        .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    Ok(ApiJson(
        json!({ "ok": true, "filterModes": parsed.filter_modes }),
    ))
}

/// Body of `POST /api/embed/frame`: the token the embed page was opened
/// with, either a signed `jwt` (`/embed/signed/<jwt>`) or the public link's
/// `token` (`/embed/dashboard/<token>`).
#[derive(Debug, Default, Deserialize)]
struct FrameBody {
    #[serde(default)]
    jwt: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// `POST /api/embed/frame` — the sites allowed to frame the embed page opened
/// with this token, and the content sites its image and web cards may load
/// from (`SEC-12`, `SEC-47`). Public, like the page itself: the token is the
/// credential.
///
/// The console's `proxy.ts` calls this before it serves `/embed/*` and turns
/// the answer into that page's `Content-Security-Policy`: `origins` is its
/// `frame-ancestors`, `contentSites` is what its `img-src` and `frame-src`
/// add to `'self' data: blob:` and `'none'`.
///
/// A token that is invalid, expired, withdrawn, for a board with embedding
/// off or for no board at all gets the same answer as a board with nothing
/// listed, `{"origins": [], "contentSites": []}`, so this route is no oracle
/// for whether a token is good.
///
/// `contentSites` is the deployment's list (`BI-16` part B) narrowed to the
/// sites of *this board's* image and web cards, with the same selection and
/// the same helper the board's payload uses
/// ([`crate::routes::settings::content_sites_of_cards`], `PR #233 review
/// SHOULD-FIX 1`): the whole deployment list would tell whoever reads a shared
/// link which sites the deployment allows (`PR #199 review M SHOULD-FIX BI-16
/// 1`, decision M6).
///
/// A store failure is a `500` with the fixed database message (a `503` for a
/// public link when there is no Postgres, `BI-19` part A), and the caller
/// treats anything but `200` as "no site". `BI-26` review fix R6: a signed
/// token is checked by the same `verified_claims` and `admit` the data route
/// uses, a public link by `public_links::resolve` (a link that is gone frames
/// nowhere; one with a password still answers, the page asks for it).
pub async fn frame_origins(State(state): State<AppState>, body: Bytes) -> Response {
    let parsed = serde_json::from_slice::<FrameBody>(&body).unwrap_or_default();
    let board = match board_for_frame(&state, parsed).await {
        Ok(board) => board,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let Some(board) = board else {
        return frame_answer(&[], &[]);
    };
    let content_sites = match board_content_sites(&state, &board).await {
        Ok(sites) => sites,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    frame_answer(&board.embed_access.origins, &content_sites)
}

/// The one `200` answer this route sends, with both lists in a fixed shape.
fn frame_answer(origins: &[String], content_sites: &[String]) -> Response {
    (
        StatusCode::OK,
        ApiJson(json!({ "origins": origins, "contentSites": content_sites })),
    )
        .into_response()
}

/// The board a frame request names, or `None` for anything that is not a good
/// token — every refused case gets the same empty answer.
async fn board_for_frame(state: &AppState, body: FrameBody) -> Result<Option<Board>, ApiError> {
    if let Some(jwt) = body.jwt.filter(|s| !s.is_empty()) {
        // BI-26 review fix (SHOULD-FIX) R6: the same checks `data` makes
        // (`verified_claims`, `admit`), not a second copy; a token that would be
        // refused there is given no framing list, whatever the reason.
        let Ok(claims) = verified_claims(state, &jwt) else {
            return Ok(None);
        };
        return Ok(admit(state, &claims).await.ok().map(|(board, _)| board));
    }
    if let Some(token) = body.token.filter(|s| !s.is_empty()) {
        // BI-19 part A (D3): a link that does not work frames nowhere, like a
        // token that is invalid.
        return Ok(
            match crate::routes::public_links::resolve(state, &token).await? {
                crate::routes::public_links::Resolved::Open(open) => Some(open.board),
                crate::routes::public_links::Resolved::Gone => None,
            },
        );
    }
    Ok(None)
}

/// The stored charts of `board_id` as an embed or a public link shows them: a
/// chart with no board is on the default board (`BI-18` part C).
///
/// `PR #233 review SHOULD-FIX 1`: the payload ([`render_board_payload`]) and
/// the frame answer's content sites ([`board_content_sites`]) select the
/// board's cards here and only here, so the page and its policy cannot
/// disagree about which cards are on the board.
fn board_cards<'a>(stored: &'a [StoredChartSpec], board_id: &str) -> Vec<&'a StoredChartSpec> {
    stored
        .iter()
        .filter(|c| {
            if c.board.is_empty() {
                board_id == store::DEFAULT_BOARD_ID
            } else {
                c.board == board_id
            }
        })
        .collect()
}

/// The deployment's content sites that one of `board`'s image or web cards
/// uses, in the order the settings list has them.
///
/// The chart read is the one `render_board_payload` already needs for an
/// embed; the selection and the narrowing are the payload's own
/// ([`board_cards`], [`crate::routes::settings::content_sites_of_cards`]), so
/// a card that is not on this board cannot widen this page's `img-src` and a
/// card the payload shows is never refused by the policy.
async fn board_content_sites(state: &AppState, board: &Board) -> Result<Vec<String>, ApiError> {
    let format = crate::routes::settings::format_settings(state).await?;
    if format.content_sites.is_empty() {
        return Ok(Vec::new());
    }
    let stored = store::list_stored_charts(&state.clickhouse, &Viewer::anonymous())
        .await
        .map_err(|err| {
            upstream_error::report_ch(&upstream_error::DATABASE, &err)
                .into_api_error(upstream_error::FailedAs::Internal)
        })?;
    Ok(crate::routes::settings::content_sites_of_cards(
        &board_cards(&stored, &board.id),
        &format,
    ))
}

/// Body of `POST /api/dashboard/embed-revoke`.
#[derive(Debug, Default, Deserialize)]
struct RevokeBody {
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    token: Option<String>,
}

/// `POST /api/dashboard/embed-revoke` — withdraw ONE signed embed token of a
/// dashboard (`SEC-12`). Needs `dashboard:write`.
///
/// The caller presents the whole token, never a bare `jti`: the server
/// verifies the signature and reads the `jti` and `exp` itself, so nobody can
/// withdraw (or pre-withdraw) an id they do not hold a signed token for. A
/// token without a `jti` cannot be withdrawn singly, and the answer says so.
///
/// # Errors
///
/// `400` for a missing board or token, a token that does not verify (the
/// same fixed message as everywhere), a token for another dashboard, a token
/// with no `jti`, an unknown dashboard or a full withdrawal list; `503` when
/// `EMBED_SECRET` is unset; the usual classified store errors otherwise.
pub async fn revoke_token(State(state): State<AppState>, body: Bytes) -> ApiResult<ApiJson<Value>> {
    let parsed: RevokeBody = serde_json::from_slice(&body)
        .map_err(|_| ApiError::BadRequest("Body must be JSON {board, token}.".to_owned()))?;
    let board = parsed.board.as_deref().map(str::trim).unwrap_or_default();
    let token = parsed.token.as_deref().map(str::trim).unwrap_or_default();
    if board.is_empty() || board == store::DEFAULT_BOARD_ID || token.is_empty() {
        return Err(ApiError::BadRequest("board and token are required.".to_owned()).into());
    }
    let Some(secret) = state.config.embed_secret.as_deref() else {
        return Err(ApiError::Unavailable(EMBEDDING_NOT_CONFIGURED.to_owned()).into());
    };
    let now = lakehouse_embed::unix_now();
    let claims = lakehouse_embed::verify_embed(
        token,
        secret,
        state.config.embed_token_max_lifetime_seconds,
        now,
    )
    .map_err(|reason| {
        tracing::info!(%reason, "embed token to withdraw refused");
        ApiError::BadRequest(TOKEN_REFUSED.to_owned())
    })?;
    // BI-26 (D1): a chart token is withdrawn through its chart's dashboard.
    let for_board = subject_of(&state.clickhouse, claims.resource.as_ref())
        .await
        .map_err(|err| crate::routes::dashboard_folders::classify_ch_error(&err))?
        .map(|s| s.board_id);
    if for_board.as_deref() != Some(board) {
        return Err(
            ApiError::BadRequest("that token is for a different dashboard.".to_owned()).into(),
        );
    }
    let Some(jti) = claims.jti.as_deref() else {
        return Err(ApiError::BadRequest(
            "that token has no id (jti), so it cannot be withdrawn on its own; withdraw all the dashboard's embed tokens instead.".to_owned(),
        )
        .into());
    };
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "exp is finite and within a day of now (verify_embed bounds it); rounded up, clamped at 0"
    )]
    let exp = claims.exp.unwrap_or(0.0).max(0.0).ceil() as u64;
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "tolerance is the constant 60.0"
    )]
    let tolerance = lakehouse_embed::CLOCK_TOLERANCE_SECONDS as u64;
    store::withdraw_embed_token(
        &state.clickhouse,
        board,
        jti,
        exp,
        now_seconds(now),
        tolerance,
    )
    .await
    .map_err(|err| crate::routes::dashboard_folders::classify_bi_error(&err))?;
    Ok(ApiJson(json!({ "ok": true, "withdrawn": jti })))
}

/// `now` (Unix seconds, fractional) as whole seconds, rounded down.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a Unix time in seconds is positive and far below u64::MAX"
)]
pub(crate) fn now_seconds(now: f64) -> u64 {
    now.max(0.0).floor() as u64
}

/// `paramsToFilters` in `embed/data/route.ts`: a signed embed's locked JWT
/// `params` (`{ col: val | [vals] }`) become dashboard filters the viewer
/// cannot override.
fn params_to_filters(params: Option<std::collections::HashMap<String, Value>>) -> Vec<FilterDef> {
    let Some(params) = params else {
        return Vec::new();
    };
    params
        .into_iter()
        .map(|(column, v)| {
            let values = match v {
                Value::Array(items) => items.iter().map(value_to_string).collect(),
                other => vec![value_to_string(&other)],
            };
            FilterDef::in_values(column, values)
        })
        .collect()
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `GET /api/public/dashboard/{token}` — read-only public share view.
///
/// `BI-19` part A (plan D3): the link is read from Postgres. An unknown,
/// expired, switched-off (by the link's owner or by an administrator), trashed
/// or personal-space link is the same `404 not_found`; a link with a password
/// needs `X-Public-Link-Ticket` from the unlock route, and without a live one
/// answers `401 password_required` with no data and no dashboard name.
pub async fn public_dashboard(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    let board = match crate::routes::public_links::resolve(&state, &token).await {
        Ok(crate::routes::public_links::Resolved::Open(open)) => {
            if let Some(refusal) =
                crate::routes::public_links::check_ticket(&state, &open.link, &headers).await
            {
                return refusal;
            }
            open.board
        }
        Ok(crate::routes::public_links::Resolved::Gone) => {
            return crate::routes::public_links::not_found();
        }
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };

    let filters = board.filters.clone().unwrap_or_default();
    let board_id = board.id.clone();
    let time = match crate::routes::settings::time_context(&state).await {
        Ok(t) => t,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    // BI-17 part A: the number locale, company palette and fonts ride in the
    // payload's `reporting` object like the zone, so an embed formats as the
    // console does.
    let format = match crate::routes::settings::format_settings(&state).await {
        Ok(f) => f,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    let payload = crate::result_cache::scoped(
        crate::result_cache::CacheScope::new(
            std::sync::Arc::clone(&state.result_cache),
            format.result_cache,
            crate::result_cache::Fresh::No,
        ),
        render_board_payload(
            &state.clickhouse,
            &PolicyEngineObligations::from_state(&state),
            &board,
            &board_id,
            &filters,
            (&time, &state.tile_slots),
            Narrowing::default(),
            &format,
        ),
    )
    .await;
    match payload {
        Ok((mut body, content_sites)) => {
            crate::routes::settings::add_formatting_for_anonymous(
                &mut body,
                &format,
                &content_sites,
            );
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
}

/// `GET /api/public/dashboard/{token}/maps/{id}/geojson` — the boundary
/// file of a map one of this board's charts draws on.
///
/// `PR #246 review BLOCKER 1`: the public link reads a map's boundary
/// through its own token, the way it reads the board's data: a token that
/// is not a board is a 404, exactly as `GET /api/public/dashboard/{token}`
/// answers, and a map no chart on that board draws on is the same 404.
///
/// `BI-19` part A: the link is read from Postgres and a link with a
/// password needs its ticket, exactly as the board's data does — the map is
/// part of that board, so it is behind the same door.
pub async fn public_map_geojson(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((token, id)): Path<(String, String)>,
) -> Response {
    // BI-19 part A: the link is resolved the way `public_dashboard` resolves
    // it — from Postgres, with the password ticket — so a map behind a
    // password needs the same ticket the board's data does, and a link that
    // does not work is the same 404 as the board's.
    let board = match crate::routes::public_links::resolve(&state, &token).await {
        Ok(crate::routes::public_links::Resolved::Open(open)) => {
            if let Some(refusal) =
                crate::routes::public_links::check_ticket(&state, &open.link, &headers).await
            {
                return refusal;
            }
            open.board
        }
        Ok(crate::routes::public_links::Resolved::Gone) => {
            return crate::routes::public_links::not_found();
        }
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    map_for_board(&state, &board.id, &id, &headers).await
}

/// `POST /api/embed/maps/{id}/geojson` — the boundary file of a map one of
/// the embedded board's charts draws on. The body is `{ jwt }`, exactly as
/// `POST /api/embed/data` takes it, and it opens the same board.
///
/// `PR #246 review BLOCKER 1`: the console's `ensureMap` fetched a map
/// through `apiFetch`, whose 401 sends the browser to the login page — an
/// anonymous viewer of an embed has no account to log into. The embed reads
/// its boundary here instead, and a map no chart on that board draws on is a
/// 404, so the token reads the board it opens and nothing else.
pub async fn embed_map_geojson(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let jwt = serde_json::from_slice::<EmbedDataBody>(&body)
        .ok()
        .and_then(|b| b.jwt);
    // The same admission `POST /api/embed/data` applies, so the two routes
    // cannot disagree about what a token opens (`BI-26`'s `verified_claims`
    // and `admit`, which read a chart token as well as a board token).
    let subject = match embed_grant(&state, jwt.as_deref()).await {
        Ok(subject) => subject,
        Err(refusal) => return *refusal,
    };
    map_for_board(&state, &subject.board_id, &id, &headers).await
}

/// The map file `id` for a board that may open it.
///
/// The token is the credential, and the token's board is what it opens: the
/// file is served only when a chart on that board draws on that map, and the
/// answer is a 404 otherwise — the same answer for a map the board does not
/// use, for a map that does not exist and for a map id that is not
/// server-made, so the route is no oracle for what a board holds.
async fn map_for_board(
    state: &AppState,
    board_id: &str,
    id: &str,
    headers: &HeaderMap,
) -> Response {
    let maps = match board_map_ids(state, board_id).await {
        Ok(maps) => maps,
        Err(err) => return database_failure(&err),
    };
    if !maps.iter().any(|m| m == id) {
        return crate::routes::public_links::not_found();
    }
    match super::maps::map_file(state, id, headers).await {
        Ok(response) => response,
        Err(rejection) => rejection.into_response(),
    }
}

/// The map ids the charts on `board_id` draw on.
///
/// The charts are read as nobody in particular, exactly as the embed and the
/// public payload read them ([`board_cards`]): a chart in the trash or on a
/// personal board is not on this board.
async fn board_map_ids(state: &AppState, board_id: &str) -> Result<Vec<String>, ChError> {
    let stored = store::list_stored_charts(&state.clickhouse, &Viewer::anonymous()).await?;
    Ok(board_cards(&stored, board_id)
        .into_iter()
        .filter_map(|c| c.def.map.clone())
        .collect())
}

/// The fixed-500 answer a store failure gets on the map routes (`PR #246
/// review BLOCKER 1`: they read the store too). The engine's own text never
/// reaches the answer (SEC-11): it goes to the log under a reference id.
fn database_failure(err: &ChError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, err).to_json()),
    )
        .into_response()
}

/// A chart as an embed or a public link shows it: the render spec of the
/// chart as this read sees it (`BI-11`: the columns its parameters gave its
/// slots), the editor's definition, and no click setting. `BI-18`·B: embeds
/// and public links have no click menu and no destinations, so the editor's
/// click setting (a board id, a column or a URL) is not sent to someone who
/// cannot use it.
fn embed_rendered(original: &StoredChartSpec, effective: &StoredChartSpec) -> Value {
    let mut rendered = render_stored_spec(&effective.spec, effective.source);
    rendered["board"] = json!(original.board);
    rendered["def"] = serde_json::to_value(&original.def).unwrap_or_else(|_| json!({}));
    if let Some(def) = rendered["def"].as_object_mut() {
        def.remove("click");
    }
    rendered
}

/// The board's tile layout as the payload carries it.
fn layout_json(board: &Board) -> Value {
    board
        .layout
        .as_ref()
        .map_or_else(|| json!({}), crate::routes::dashboard::layout_to_json)
}

/// The calculated fields of every source.
///
/// BI-8: an embed or a public link shows a chart that uses a calculated field
/// exactly as the dashboard does (same builder, same role rewrite with the
/// embed viewer's role).
///
/// PR #104 CI fix: a catalog that cannot be read must not take the whole embed
/// down (it used to render, with a per-tile fixed message for a tile that
/// failed). Like the dashboard route, a failed read leaves the catalog empty: a
/// chart that names a calculated field then fails on its own statement with the
/// fixed tile message and a reference, never showing a wrong number, and the
/// failure is logged under its own reference.
async fn catalog_or_empty(ch: &ChClient) -> lakehouse_bi::fields::FieldCatalog {
    match lakehouse_bi::fields::list_catalog_fields(ch).await {
        Ok(found) => lakehouse_bi::fields::FieldCatalog::from_fields(found),
        Err(err) => {
            let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
            lakehouse_bi::fields::FieldCatalog::default()
        }
    }
}

/// The columns of every table, read only when something on the board needs
/// them. Column types are what decides whether a chart's grain still fits its
/// dimension (BI-9), and which filters apply.
///
/// BI-13: the questions of a combined chart need the columns of their own
/// sources. That read is optional for the board (PR #104 lesson): when it
/// fails the questions that need it say so and everything else renders.
async fn board_columns(
    ch: &ChClient,
    charts: &[&StoredChartSpec],
    filters: &[FilterDef],
    fields: &lakehouse_bi::fields::FieldCatalog,
    need_kinds: bool,
) -> Result<
    std::collections::HashMap<String, lakehouse_bi::builder::RelationColumns>,
    lakehouse_clickhouse::ChError,
> {
    let need_cols = need_kinds
        || filters.iter().any(FilterDef::is_active)
        || charts.iter().any(|c| {
            c.def.needs_columns()
                || fields.chart_uses(&c.def)
                || lakehouse_bi::params::changes_statement(&c.def)
        });
    if need_cols {
        return mart_columns(ch).await;
    }
    if charts.iter().any(|c| !c.def.shape.layers_list().is_empty()) {
        return Ok(match mart_columns(ch).await {
            Ok(found) => found,
            Err(err) => {
                let _logged = upstream_error::report_ch(&upstream_error::DATABASE, &err);
                std::collections::HashMap::new()
            }
        });
    }
    Ok(std::collections::HashMap::new())
}

/// The filters the tile of `chart_id` is given (`BI-18` part C): the tabs and
/// the layout choose, as on the dashboard. The locked embed `params` have no
/// tie and so apply on every tab.
fn tile_filters<'a>(
    board: &Board,
    filters: &'a [FilterDef],
    chart_id: &str,
) -> std::borrow::Cow<'a, [FilterDef]> {
    let no_tiles = lakehouse_bi::store::LayoutMap::new();
    lakehouse_bi::layout::filters_for_chart(
        filters,
        board.tabs.as_deref().unwrap_or_default(),
        board.layout.as_ref().unwrap_or(&no_tiles),
        chart_id,
    )
}

/// The charts a read covers: those of `board_id`, or only `only_chart` for a
/// chart token (`BI-26` plan D1: a chart token reads that chart and no other).
fn charts_of<'a>(
    stored: &'a [StoredChartSpec],
    board_id: &str,
    only_chart: Option<&str>,
) -> Vec<&'a StoredChartSpec> {
    // One selection of the board's cards (`board_cards`), narrowed to a chart
    // token's chart: the frame answer's content sites use the same helper.
    board_cards(stored, board_id)
        .into_iter()
        .filter(|c| only_chart.is_none_or(|only| c.spec.id == only))
        .collect()
}

/// A chart as the payload carries it, with `resolved` when its slots take a
/// parameter. `BI-19` part A QA fix P3: the columns the parameters gave this
/// read are sent as the dashboard payload sends them; without them the page
/// read `{param}` measures as column names.
fn rendered_chart(original: &StoredChartSpec, effective: &StoredChartSpec) -> Value {
    let mut rendered = embed_rendered(original, effective);
    if lakehouse_bi::params::changes_statement(&original.def) {
        rendered["resolved"] = crate::routes::support::resolved_json(effective);
    }
    rendered
}

/// The `embedFilters` of a payload: for each editable column its kind (from the
/// relation's columns, `null` when unknown) and the filter now applied to it.
fn describe_editable(editable: &[String], fields: &[Value], filters: &[FilterDef]) -> Value {
    let described: Vec<Value> = editable
        .iter()
        .map(|column| {
            json!({
                "column": column,
                "kind": fields
                    .iter()
                    .find(|f| f["column"] == json!(column))
                    .map_or(Value::Null, |f| f["kind"].clone()),
                "filter": filters.iter().find(|f| &f.column == column),
            })
        })
        .collect();
    json!(described)
}

/// The `{ board, layout, charts, results }` payload of an embed or a public
/// link, with the board's tabs and phone layout when it has them (`BI-18` part
/// C; left off otherwise so an older board's payload is unchanged).
fn board_body(
    board: &Board,
    charts: &[Value],
    results: &serde_json::Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
    only_chart: Option<&str>,
) -> Value {
    if let Some(chart) = only_chart {
        return chart_body(board, charts, results, time, chart);
    }
    let mut body = json!({
        "board": { "id": board.id, "name": board.name },
        "layout": layout_json(board),
        "charts": charts,
        "results": results,
        // BI-9: the zone and first day the buckets were cut with, so the
        // page labels them the same way.
        "reporting": { "timeZone": time.zone(), "weekStart": time.week_start().as_str() },
    });
    // BI-17 part A (D13): the theme and font apply wherever the dashboard is
    // shown, so a public link and a signed embed carry them.
    if let Some(board_obj) = body["board"].as_object_mut() {
        if let Some(theme) = &board.theme {
            board_obj.insert("theme".to_owned(), json!(theme));
        }
        if let Some(font) = &board.font {
            board_obj.insert("font".to_owned(), json!(font));
        }
    }
    if let Some(map) = body.as_object_mut() {
        if let Some(tabs) = board.tabs.as_ref().filter(|t| !t.is_empty()) {
            map.insert("tabs".to_owned(), json!(tabs));
        }
        if let Some(phone) = board.phone_layout.as_ref().filter(|p| !p.is_empty()) {
            map.insert("phoneLayout".to_owned(), json!(phone));
        }
    }
    body
}

/// The payload of a chart token (`BI-26` plan D1): that chart's spec and result
/// and nothing that names another: no dashboard name, no other chart id in a
/// layout, no tabs, no phone layout. `onlyChart` tells the page to draw the one
/// chart filling the frame. The dashboard's theme and font still apply.
fn chart_body(
    board: &Board,
    charts: &[Value],
    results: &serde_json::Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
    chart: &str,
) -> Value {
    let mut body = json!({
        "board": { "id": board.id },
        "layout": {},
        "charts": charts,
        "results": results,
        "onlyChart": chart,
        "reporting": { "timeZone": time.zone(), "weekStart": time.week_start().as_str() },
    });
    if let Some(board_obj) = body["board"].as_object_mut() {
        if let Some(theme) = &board.theme {
            board_obj.insert("theme".to_owned(), json!(theme));
        }
        if let Some(font) = &board.font {
            board_obj.insert("font".to_owned(), json!(font));
        }
    }
    body
}

/// What a read is narrowed to beyond the board (`BI-26`): a chart token's one
/// chart, and the columns whose filter the viewer may change. A struct so
/// [`render_board_payload`] stays within the argument limit once it also takes
/// the format settings (`SEC-47`).
#[derive(Debug, Default, Clone, Copy)]
struct Narrowing<'a> {
    only_chart: Option<&'a str>,
    editable: &'a [String],
}

/// Shared `{ board, layout, charts, results }` assembly for both
/// [`data`] and [`public_dashboard`]: every stored chart belonging to
/// `board_id`, filtered by `filters` (never by caller-supplied years —
/// neither route accepts a `year` parameter).
///
/// Also returns the content sites of the board's image and web cards
/// ([`crate::routes::settings::content_sites_of_cards`], over the same
/// selection), so the payload and the frame answer for that board narrow the
/// deployment's list the same way (`PR #233 review SHOULD-FIX 1`).
#[allow(
    clippy::too_many_lines,
    reason = "one pipeline (read the board's charts and sources, run every tile \
              through `tile_pool`, assemble the payload); the per-tile future \
              borrows eleven locals, so a helper would need as many \
              parameters as the body has lines it saves"
)]
async fn render_board_payload(
    ch: &ChClient,
    obligations: &PolicyEngineObligations<'_>,
    board: &Board,
    board_id: &str,
    filters: &[FilterDef],
    (time, tile_slots): (&lakehouse_bi::grain::TimeContext, &tokio::sync::Semaphore),
    narrowing: Narrowing<'_>,
    format: &crate::routes::settings::FormatSettings,
) -> Result<(Value, Vec<String>), lakehouse_clickhouse::ChError> {
    let roles = [EMBED_VIEWER_ROLE.to_owned()];
    let placeholders = crate::sql_rewrite::PlaceholderValues::none();
    // An embed or a public link is read by nobody in particular: a chart in the
    // trash or on a personal board is not in it (BI-18 part C, part 2).
    let stored = store::list_stored_charts(ch, &Viewer::anonymous()).await?;
    let field_catalog = catalog_or_empty(ch).await;
    let stored_for_board = charts_of(&stored, board_id, narrowing.only_chart);

    let cols = board_columns(
        ch,
        &stored_for_board,
        filters,
        &field_catalog,
        !narrowing.editable.is_empty(),
    )
    .await?;

    let mut results = serde_json::Map::new();
    let mut charts_out = Vec::with_capacity(stored_for_board.len());
    let sources = sources_for(ch, stored_for_board.iter().copied()).await?;
    // BI-9: embeds and public links offer no grain switch; they use the one
    // the board saved, and each chart's own where there is none.
    // BI-11: an embed or a public link offers no control; it uses the values
    // the board saved, else each parameter's default. A store that cannot be
    // read leaves the values empty, so only a chart that names a parameter
    // shows an error (fail closed per tile).
    let values = crate::routes::params::saved_or_default(ch, board).await;
    let time_with_params = time.clone().with_params(values.clone());
    let read = lakehouse_bi::builder::ReadContext {
        time: &time_with_params,
        grain: board
            .grain
            .as_deref()
            .and_then(lakehouse_bi::grain::Grain::parse)
            .filter(|g| g.is_truncation()),
        fields: &field_catalog,
    };
    let layer_sources =
        crate::routes::layers::layer_sources(ch, stored_for_board.iter().copied()).await;
    let layer_runner = crate::routes::layers::RoleRunner {
        ch,
        roles: &roles,
        placeholders: &placeholders,
        obligations,
    };
    let layer_ctx = crate::routes::layers::LayerContext {
        runner: &layer_runner,
        years: &[],
        filters,
        cols: &cols,
        chart_sources: &sources,
        layer_sources: &layer_sources,
        read: &read,
    };
    // BI-4 part A (plan D1/D2): the tiles run up to 6 at once for this
    // request and under the process's `tile_slots`, through the one shared
    // mechanism (`tile_pool`). Each tile keeps the masks of the embed viewer
    // role and its own `error` value; the outputs come back in chart order,
    // the order the sequential loop produced.
    let (cols_ref, sources_ref, read_ref, values_ref) = (&cols, &sources, &read, &values);
    let (roles_ref, placeholders_ref, catalog_ref) = (&roles, &placeholders, &field_catalog);
    let layer_ctx_ref = &layer_ctx;
    let tiles: Vec<futures::future::BoxFuture<'_, (String, Value, Value)>> = stored_for_board
        .iter()
        .map(|original| {
            let original: &StoredChartSpec = original;
            // BI-4 part B: the chart's own freshness rule narrows the board's.
            Box::pin(crate::result_cache::in_chart(
                (Some(original.spec.id.as_str()), original.def.shape.cache),
                async move {
                    let tile_filters = tile_filters(board, filters, &original.spec.id);
                    let effective = crate::routes::support::effective_chart(
                        original,
                        cols_ref,
                        sources_ref,
                        read_ref,
                    );
                    let c = match &effective {
                        Ok(e) => &**e,
                        Err(msg) => {
                            return (
                                original.spec.id.clone(),
                                json!({ "error": msg }),
                                embed_rendered(original, original),
                            );
                        }
                    };
                    let val = match stored_chart_sql(
                        c,
                        &[],
                        &tile_filters,
                        cols_ref,
                        sources_ref,
                        read_ref,
                    ) {
                        Ok(filtered) => {
                            let filtered =
                                crate::routes::support::with_map_limit(filtered, c.spec.kind).await;
                            // BI-5 review B1: the rows this kind may read.
                            let (id, mut val) = crate::routes::support::with_tile_rows(
                                c.spec.kind,
                                &filtered.sql,
                                run_spec_sql(
                                    ch,
                                    &c.spec.id,
                                    &filtered.sql,
                                    roles_ref,
                                    placeholders_ref,
                                    obligations,
                                ),
                            )
                            .await;
                            crate::routes::support::annotate_params(&mut val, c, values_ref);
                            crate::routes::support::annotate_grain(&mut val, &filtered, c);
                            // BI-16 part A: the first page of a raw table, with
                            // its total; an embed and a public link offer no
                            // paging.
                            crate::routes::support::annotate_table(
                                ch,
                                &mut val,
                                &filtered,
                                (roles_ref, placeholders_ref, obligations),
                            )
                            .await;
                            let tile_ctx = crate::routes::layers::LayerContext {
                                filters: &tile_filters,
                                ..*layer_ctx_ref
                            };
                            crate::routes::layers::annotate_layers(
                                &tile_ctx,
                                &mut val,
                                c,
                                filtered.grain,
                            )
                            .await;
                            annotate_metric_labels(
                                &mut val,
                                c,
                                catalog_ref,
                                Some((cols_ref, sources_ref)),
                            );
                            (id, val)
                        }
                        Err(msg) => (c.spec.id.clone(), json!({ "error": msg })),
                    };
                    (val.0, val.1, rendered_chart(original, c))
                },
            )) as futures::future::BoxFuture<'_, (String, Value, Value)>
        })
        .collect();
    // BI-5 (plan D4): the same board deadline as the dashboard payload.
    let run = crate::query_limits::QueryRun::current_or_detached();
    let limit = run
        .limits()
        .await
        .map_or(crate::query_limits::BOARD_DEADLINE_CAP, |limits| {
            limits.timeout
        });
    let deadline =
        tokio::time::Instant::now() + crate::query_limits::board_deadline(tiles.len(), limit);
    let unreached = |i: usize| {
        let original: &StoredChartSpec = stored_for_board[i];
        (
            original.spec.id.clone(),
            json!({ "error": crate::query_limits::NOT_REACHED_SENTENCE }),
            embed_rendered(original, original),
        )
    };
    // BI-4 part B: the scope the handler opened, narrowed to the board's own
    // rule; the embed viewer's masks are in each statement, so equal viewers
    // share a result and different ones never do.
    let running = crate::tile_pool::run_tiles_until(tile_slots, tiles, deadline, &unreached);
    let finished = match crate::result_cache::current() {
        Some(scope) => {
            let candidates = cols.keys().map(|mart| format!("serving.{mart}")).collect();
            crate::result_cache::scoped(
                scope
                    .for_board(board.cache, candidates)
                    .with_zone(time.zone()),
                running,
            )
            .await
        }
        None => running.await,
    };
    for (id, val, rendered) in finished {
        results.insert(id, val);
        charts_out.push(rendered);
    }

    let mut body = board_body(board, &charts_out, &results, time, narrowing.only_chart);
    // BI-26 (D4): the editable filters only (column, kind, current value).
    // Locked and hidden filters are not described.
    if !narrowing.editable.is_empty() {
        let fields = crate::routes::dashboard::filter_fields(&stored_for_board, &cols, &sources);
        body["embedFilters"] = describe_editable(narrowing.editable, &fields, filters);
    }
    let sites = crate::routes::settings::content_sites_of_cards(&stored_for_board, format);
    Ok((body, sites))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// `BI-26` review NOTE: a tile does not name a hidden filter in `filtersSkipped`.
    #[test]
    fn a_hidden_filters_column_is_taken_out_of_the_skipped_list_and_the_others_stay() {
        use lakehouse_bi::embed_access::{EmbedAccess, FilterMode};
        let mut access = EmbedAccess::default();
        access
            .filter_modes
            .insert("region".to_owned(), FilterMode::Locked);
        access
            .filter_modes
            .insert("year".to_owned(), FilterMode::Editable);
        let mut body = json!({ "results": {
            "c1": { "rows": [], "filtersSkipped": [
                { "column": "region", "reason": "no_column" },
                { "column": "year", "reason": "no_column" },
                { "column": "secret_column", "reason": "no_column" } ] },
            "c2": { "rows": [], "filtersSkipped": [{ "column": "secret_column", "reason": "no_column" }] },
            "c3": { "rows": [] }
        }});
        strip_hidden_skips(&mut body, &access);
        let kept: Vec<&str> = body["results"]["c1"]["filtersSkipped"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["column"].as_str().unwrap())
            .collect();
        assert_eq!(kept, ["region", "year"]);
        assert!(body["results"]["c2"].get("filtersSkipped").is_none());
        assert!(body["results"]["c3"].get("filtersSkipped").is_none());
    }

    #[test]
    fn params_to_filters_none_yields_empty() {
        assert_eq!(
            params_to_filters(None),
            [] as [lakehouse_bi::filters::FilterDef; 0]
        );
    }

    #[test]
    fn params_to_filters_wraps_single_value_in_array() {
        let mut params = std::collections::HashMap::new();
        params.insert("kawasan".to_owned(), json!("Asia"));
        let filters = params_to_filters(Some(params));
        assert_eq!(filters.len(), 1);
        assert_eq!(filters[0].column, "kawasan");
        assert_eq!(filters[0].values, vec!["Asia".to_owned()]);
    }

    #[test]
    fn params_to_filters_passes_through_array_values() {
        let mut params = std::collections::HashMap::new();
        params.insert("tahun".to_owned(), json!(["2023", "2024"]));
        let filters = params_to_filters(Some(params));
        assert_eq!(
            filters[0].values,
            vec!["2023".to_owned(), "2024".to_owned()]
        );
    }
}

#[cfg(test)]
mod typed_filters {
    //! A board saved with a typed filter renders publicly with it applied
    //! (BI-18): the public and embed paths share `render_board_payload`.

    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::json;
    use wiremock::matchers::{body_string_contains, method};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    async fn answer(server: &MockServer, has: &str, meta: &[&str], data: Vec<Value>) {
        let meta: Vec<Value> = meta
            .iter()
            .map(|n| json!({ "name": n, "type": "String" }))
            .collect();
        let rows = data.len();
        Mock::given(method("POST"))
            .and(body_string_contains(has))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": meta, "data": data, "rows": rows })),
            )
            .mount(server)
            .await;
    }

    #[tokio::test]
    #[allow(
        clippy::too_many_lines,
        reason = "one fixture board and its assertions in a single scenario; \
                  it was 100 lines on BI-26's branch and the BI-4 tile-slot \
                  tuple in its call took it to 103"
    )]
    async fn a_board_saved_with_a_typed_filter_renders_publicly_with_it_applied() {
        let server = MockServer::start().await;
        let spec = json!({
            "id": "c1", "title": "T", "kind": "bar", "mart": "mart_a",
            "sql": "SELECT 1", "x": "kab", "y": "visitors"
        });
        let def = json!({
            "title": "T", "mart": "mart_a", "kind": "bar",
            "dimension": "kab", "measures": ["visitors"],
            "click": { "kind": "url", "url": "/somewhere/{value}" }
        });
        answer(
            &server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![json!({
                "id": "c1", "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })],
        )
        .await;
        answer(
            &server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            vec![
                json!({ "table": "mart_a", "name": "kab", "type": "String" }),
                json!({ "table": "mart_a", "name": "visitors", "type": "UInt32" }),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let filters: Vec<FilterDef> = serde_json::from_str(
            r#"[{"column":"visitors","op":"between","min":"100","max":"900"},
                {"column":"kab","op":"contains","text":"ub"}]"#,
        )
        .unwrap();
        let board = Board {
            id: "b1".to_owned(),
            name: "B".to_owned(),
            description: None,
            created_by: None,
            layout: None,
            filters: Some(filters.clone()),
            refresh_seconds: None,
            grain: None,
            params: None,
            tabs: None,
            phone_layout: None,
            theme: None,
            font: None,
            cache: None,
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            owner_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };

        let (body, _) = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &filters,
            (
                &lakehouse_bi::grain::TimeContext::default(),
                &tokio::sync::Semaphore::new(crate::tile_pool::TILES_PER_PROCESS),
            ),
            Narrowing::default(),
            &crate::routes::settings::FormatSettings::default(),
        )
        .await
        .unwrap();

        assert_eq!(body["board"]["id"], "b1");
        // BI-18·B: the editor's click setting is not sent to an embed.
        assert!(body["charts"][0]["def"].get("click").is_none());
        assert_eq!(body["charts"][0]["def"]["dimension"], "kab");
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let tile = sent
            .iter()
            .find(|s| s.contains("FROM serving.mart_a WHERE"))
            .unwrap_or_else(|| panic!("tile query not sent: {sent:?}"));
        assert_chart_read_as_nobody(&sent);
        assert!(
            tile.contains("(visitors >= 100 AND visitors <= 900)"),
            "{tile}"
        );
        assert!(
            tile.contains("positionCaseInsensitiveUTF8(toString(kab), 'ub') > 0"),
            "{tile}"
        );
    }

    /// BI-18 part C, part 2: an embed reads as nobody in particular, so the chart
    /// read leaves out a personal board's charts and the trash.
    fn assert_chart_read_as_nobody(sent: &[String]) {
        let read = sent
            .iter()
            .find(|s| s.contains("FROM console.bi_chart FINAL"))
            .expect("the charts were read");
        assert!(read.contains("owner_id != '' AND owner_id != ''"), "{read}");
        assert!(read.contains("is_deleted = 0"), "{read}");
        assert!(read.contains("is_deleted = 2"), "{read}");
    }

    /// Two charts, `c1` on `mart_a` and `c2` on `mart_b`, both with `kab` and
    /// `visitors`; every other statement succeeds with no rows.
    async fn mount_two_marts(server: &MockServer) {
        let row = |id: &str, mart: &str| {
            let spec = json!({
                "id": id, "title": id, "kind": "bar", "mart": mart,
                "sql": "SELECT 1", "x": "kab", "y": "visitors"
            });
            let def = json!({
                "title": id, "mart": mart, "kind": "bar",
                "dimension": "kab", "measures": ["visitors"]
            });
            json!({
                "id": id, "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })
        };
        answer(
            server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![row("c1", "mart_a"), row("c2", "mart_b")],
        )
        .await;
        let mut cols = Vec::new();
        for table in ["mart_a", "mart_b"] {
            cols.push(json!({ "table": table, "name": "kab", "type": "String" }));
            cols.push(json!({ "table": table, "name": "visitors", "type": "UInt32" }));
        }
        answer(
            server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            cols,
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

    /// BI-18 part C: on an embed or a public link a filter tied to one tab is
    /// not given to the tiles of the other, and the payload carries the tabs,
    /// the tile tabs and the phone layout.
    #[tokio::test]
    async fn a_tied_filter_and_the_tabs_reach_a_public_link() {
        let server = MockServer::start().await;
        mount_two_marts(&server).await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let filters: Vec<FilterDef> = serde_json::from_str(
            r#"[{"column":"kab","op":"contains","text":"ub","tabs":["t_a"]}]"#,
        )
        .unwrap();
        let layout: lakehouse_bi::store::LayoutMap = serde_json::from_value(json!({
            "c1": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_a" },
            "c2": { "x": 0, "y": 0, "w": 6, "h": 4, "tab": "t_b" },
        }))
        .unwrap();
        let tabs = vec![
            lakehouse_bi::layout::Tab {
                id: "t_a".to_owned(),
                title: "A".to_owned(),
            },
            lakehouse_bi::layout::Tab {
                id: "t_b".to_owned(),
                title: "B".to_owned(),
            },
        ];
        let board = Board {
            id: "b1".to_owned(),
            name: "B".to_owned(),
            description: None,
            created_by: None,
            layout: Some(layout),
            filters: Some(filters.clone()),
            refresh_seconds: None,
            grain: None,
            params: None,
            tabs: Some(tabs),
            phone_layout: Some(vec![lakehouse_bi::layout::PhoneTile {
                id: "c2".to_owned(),
                h: 5,
            }]),
            theme: Some("dark".to_owned()),
            font: Some("inter".to_owned()),
            cache: None,
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            owner_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };
        let (body, _) = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &filters,
            (
                &lakehouse_bi::grain::TimeContext::default(),
                &tokio::sync::Semaphore::new(crate::tile_pool::TILES_PER_PROCESS),
            ),
            Narrowing::default(),
            &crate::routes::settings::FormatSettings::default(),
        )
        .await
        .unwrap();
        assert_eq!(body["tabs"][1]["id"], "t_b");
        assert_eq!(body["phoneLayout"][0], json!({ "id": "c2", "h": 5 }));
        assert_eq!(body["layout"]["c2"]["tab"], "t_b");
        // BI-17 part A (D13): the theme and font go out with the embed.
        assert_eq!(body["board"]["theme"], "dark");
        assert_eq!(body["board"]["font"], "inter");
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let on_a = sent
            .iter()
            .find(|s| s.contains("FROM serving.mart_a WHERE"))
            .unwrap();
        assert!(
            on_a.contains("positionCaseInsensitiveUTF8(toString(kab), 'ub')"),
            "{on_a}"
        );
        // c2 is on t_b: the filter tied to t_a is not given, so it runs its stored SQL.
        assert!(
            sent.iter()
                .all(|s| !s.contains("FROM serving.mart_b WHERE")),
            "{sent:?}"
        );
    }

    /// BI-6: a chart's own filters apply on an embed or a public link whatever
    /// the board's saved filters are, and the tile can list them.
    #[tokio::test]
    async fn an_embed_applies_a_charts_own_filters_with_the_boards() {
        let server = MockServer::start().await;
        let spec = json!({
            "id": "c1", "title": "T", "kind": "bar", "mart": "mart_a",
            "sql": "SELECT stored_sql_marker", "x": "kab", "y": "visitors"
        });
        let def = json!({
            "title": "T", "mart": "mart_a", "kind": "bar", "dimension": "kab",
            "measures": ["visitors"],
            "filters": [{ "column": "kab", "values": ["a", "b"] }]
        });
        answer(
            &server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![json!({
                "id": "c1", "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })],
        )
        .await;
        answer(
            &server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            vec![
                json!({ "table": "mart_a", "name": "kab", "type": "String" }),
                json!({ "table": "mart_a", "name": "visitors", "type": "UInt32" }),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        // No filter is active on the board: the chart's own still rebuild its statement.
        let board = Board {
            id: "b1".to_owned(),
            name: "B".to_owned(),
            description: None,
            created_by: None,
            layout: None,
            filters: None,
            refresh_seconds: None,
            grain: None,
            params: None,
            tabs: None,
            phone_layout: None,
            theme: None,
            font: None,
            cache: None,
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            owner_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };
        let locked: Vec<FilterDef> =
            serde_json::from_str(r#"[{"column":"kab","values":["b","c"]}]"#).unwrap();

        let (body, _) = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &locked,
            (
                &lakehouse_bi::grain::TimeContext::default(),
                &tokio::sync::Semaphore::new(crate::tile_pool::TILES_PER_PROCESS),
            ),
            Narrowing::default(),
            &crate::routes::settings::FormatSettings::default(),
        )
        .await
        .unwrap();

        assert_eq!(body["charts"][0]["def"]["filters"][0]["column"], "kab");
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        let tile = sent
            .iter()
            .find(|s| s.contains("FROM serving.mart_a WHERE"))
            .unwrap_or_else(|| panic!("tile query not sent: {sent:?}"));
        assert!(
            tile.contains("kab IN ('a', 'b') AND kab IN ('b', 'c')"),
            "{tile}"
        );
        assert!(!tile.contains("stored_sql_marker"), "{tile}");
    }

    /// BI-9: an embed or public link has no grain switch; it uses the grain
    /// the board saved, in the deployment's time zone, and a grained chart is
    /// rebuilt rather than served from its stored SQL.
    #[tokio::test]
    async fn an_embed_uses_the_boards_saved_grain_and_never_the_stored_sql() {
        let server = MockServer::start().await;
        let spec = json!({
            "id": "c1", "title": "T", "kind": "line", "mart": "mart_a",
            "sql": "SELECT stored_sql_marker", "x": "seen_at", "y": "visitors"
        });
        let def = json!({
            "title": "T", "mart": "mart_a", "kind": "line", "dimension": "seen_at",
            "measures": ["visitors"], "limit": 20, "order": "none", "grain": "day"
        });
        answer(
            &server,
            "FROM console.bi_chart FINAL",
            &["id", "board", "created_by", "created_at", "spec_json"],
            vec![json!({
                "id": "c1", "board": "b1", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
                "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
            })],
        )
        .await;
        answer(
            &server,
            "SELECT table, name, type FROM system.columns",
            &["table", "name", "type"],
            vec![
                json!({ "table": "mart_a", "name": "seen_at", "type": "DateTime" }),
                json!({ "table": "mart_a", "name": "visitors", "type": "UInt32" }),
            ],
        )
        .await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({ "meta": [], "data": [], "rows": 0 })),
            )
            .mount(&server)
            .await;
        let ch = ChClient::new(server.uri(), "default".to_owned(), String::new());
        let board = Board {
            id: "b1".to_owned(),
            name: "B".to_owned(),
            description: None,
            created_by: None,
            layout: None,
            filters: None,
            refresh_seconds: None,
            grain: Some("month".to_owned()),
            params: None,
            tabs: None,
            phone_layout: None,
            theme: None,
            font: None,
            cache: None,
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            owner_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };
        let time = lakehouse_bi::grain::TimeContext::new(
            "Europe/Berlin",
            lakehouse_bi::grain::WeekStart::Monday,
        )
        .unwrap();
        render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &[],
            (
                &time,
                &tokio::sync::Semaphore::new(crate::tile_pool::TILES_PER_PROCESS),
            ),
            Narrowing::default(),
            &crate::routes::settings::FormatSettings::default(),
        )
        .await
        .unwrap();
        let sent: Vec<String> = server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        assert!(
            sent.iter().any(|s| s
                .contains("date_trunc('month', toTimeZone(seen_at, 'Europe/Berlin')) AS seen_at")),
            "{sent:?}"
        );
        assert!(
            sent.iter().all(|s| !s.contains("stored_sql_marker")),
            "{sent:?}"
        );
    }
}
