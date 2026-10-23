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
use axum::http::StatusCode;
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

/// `{ jwt }` — the `POST /api/embed/data` body shape.
#[derive(Debug, Default, Deserialize)]
struct EmbedDataBody {
    #[serde(default)]
    jwt: Option<String>,
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

/// `POST /api/embed/data` — signed-embed (Metabase-style) dashboard data.
pub async fn data(State(state): State<AppState>, body: Bytes) -> Response {
    // `try { jwt = String((await req.json())?.jwt ?? "") } catch { /* ignore */ }`
    // — an unparseable body is swallowed, not a 400; it just yields an
    // empty jwt, which the emptiness check below rejects the normal way.
    let jwt = serde_json::from_slice::<EmbedDataBody>(&body)
        .ok()
        .and_then(|b| b.jwt)
        .filter(|s| !s.is_empty());
    let Some(jwt) = jwt else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "jwt is required" })),
        )
            .into_response();
    };

    // SEC-12: the secret is configuration only. Unset means signed embedding
    // is unavailable, said plainly; nothing is generated or stored.
    let Some(secret) = state.config.embed_secret.as_deref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": EMBEDDING_NOT_CONFIGURED })),
        )
            .into_response();
    };
    let claims = match lakehouse_embed::verify_embed(
        &jwt,
        secret,
        state.config.embed_token_max_lifetime_seconds,
        lakehouse_embed::unix_now(),
    ) {
        Ok(claims) => claims,
        Err(reason) => {
            // SEC-12: the reason is for the log only; the response is one
            // fixed message so a caller cannot tell which check failed.
            tracing::info!(%reason, "signed embed token refused");
            return token_refused();
        }
    };

    let Some(board_id) = claims
        .resource
        .and_then(|r| r.dashboard)
        .filter(|d| !d.is_empty())
    else {
        return token_refused();
    };

    let board = match store::get_board(&state.clickhouse, &board_id, &Viewer::anonymous()).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
            )
                .into_response();
        }
    };
    let Some(board) = board.filter(|b| b.embed_enabled.unwrap_or(false)) else {
        return (
            StatusCode::FORBIDDEN,
            ApiJson(json!({ "error": "embedding_disabled" })),
        )
            .into_response();
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
        return token_refused();
    }

    let mut filters = board.filters.clone().unwrap_or_default();
    filters.extend(params_to_filters(claims.params));

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
    let payload = render_board_payload(
        &state.clickhouse,
        &PolicyEngineObligations::from_state(&state),
        &board,
        &board_id,
        &filters,
        &time,
    )
    .await;
    match payload {
        Ok(mut body) => {
            crate::routes::settings::add_formatting_for_anonymous(&mut body, &format);
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
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
/// sites of *this board's* image and web cards, the same list the board's
/// payload carries to an embed (`add_formatting_for_anonymous`): the whole
/// deployment list would tell whoever reads a shared link which sites the
/// deployment allows (`PR #199 review M SHOULD-FIX BI-16 1`, decision M6).
/// A store failure is a `500` with the fixed database message, and the caller
/// treats anything but `200` as "no site".
pub async fn frame_origins(State(state): State<AppState>, body: Bytes) -> Response {
    let parsed = serde_json::from_slice::<FrameBody>(&body).unwrap_or_default();
    let board = match board_for_frame(&state, parsed).await {
        Ok(board) => board,
        Err(err) => return database_failure(&err),
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

/// The fixed-500 answer a store failure gets, as the other embed routes send it.
fn database_failure(err: &ChError) -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, err).to_json()),
    )
        .into_response()
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
async fn board_for_frame(state: &AppState, body: FrameBody) -> Result<Option<Board>, ChError> {
    if let Some(jwt) = body.jwt.filter(|s| !s.is_empty()) {
        // Same verification `data` applies, so a token that would be refused
        // there is not given a framing list here.
        let Some(secret) = state.config.embed_secret.as_deref() else {
            return Ok(None);
        };
        let Ok(claims) = lakehouse_embed::verify_embed(
            &jwt,
            secret,
            state.config.embed_token_max_lifetime_seconds,
            lakehouse_embed::unix_now(),
        ) else {
            return Ok(None);
        };
        let Some(board_id) = claims.resource.and_then(|r| r.dashboard) else {
            return Ok(None);
        };
        let board = store::get_board(&state.clickhouse, &board_id, &Viewer::anonymous()).await?;
        return Ok(board.filter(|b| {
            b.embed_enabled.unwrap_or(false)
                && !b.embed_access.is_withdrawn(
                    claims.iat.unwrap_or(f64::NEG_INFINITY),
                    claims.jti.as_deref(),
                )
        }));
    }
    if let Some(token) = body.token.filter(|s| !s.is_empty()) {
        return store::get_board_by_token(&state.clickhouse, &token).await;
    }
    Ok(None)
}

/// The deployment's content sites that one of `board`'s image or web cards
/// uses, in the order the settings list has them.
///
/// The chart read is the one `render_board_payload` already needs for an
/// embed, and the same "no board on the chart is the default board" rule:
/// a card that is not on this board must not widen this page's `img-src`.
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
    let mut used: Vec<String> = Vec::new();
    for chart in stored.iter().filter(|c| {
        if c.board.is_empty() {
            board.id == store::DEFAULT_BOARD_ID
        } else {
            c.board == board.id
        }
    }) {
        let Some(url) = chart.def.types.url.as_deref() else {
            continue;
        };
        let Some(origin) = lakehouse_bi::embed_access::https_origin_of(url) else {
            continue;
        };
        if !used.contains(&origin) {
            used.push(origin);
        }
    }
    Ok(format
        .content_sites
        .into_iter()
        .filter(|site| used.contains(site))
        .collect())
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
    let for_board = claims
        .resource
        .as_ref()
        .and_then(|r| r.dashboard.as_deref());
    if for_board != Some(board) {
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
pub async fn public_dashboard(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Response {
    let board = match store::get_board_by_token(&state.clickhouse, &token).await {
        Ok(b) => b,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
            )
                .into_response();
        }
    };
    let Some(board) = board else {
        return (
            StatusCode::NOT_FOUND,
            ApiJson(json!({ "error": "not_found" })),
        )
            .into_response();
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
    let payload = render_board_payload(
        &state.clickhouse,
        &PolicyEngineObligations::from_state(&state),
        &board,
        &board_id,
        &filters,
        &time,
    )
    .await;
    match payload {
        Ok(mut body) => {
            crate::routes::settings::add_formatting_for_anonymous(&mut body, &format);
            (StatusCode::OK, ApiJson(body)).into_response()
        }
        Err(err) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiJson(upstream_error::report_ch(&upstream_error::DATABASE, &err).to_json()),
        )
            .into_response(),
    }
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
) -> Result<
    std::collections::HashMap<String, lakehouse_bi::builder::RelationColumns>,
    lakehouse_clickhouse::ChError,
> {
    let need_cols = filters.iter().any(FilterDef::is_active)
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

/// The `{ board, layout, charts, results }` payload of an embed or a public
/// link, with the board's tabs and phone layout when it has them (`BI-18` part
/// C; left off otherwise so an older board's payload is unchanged).
fn board_body(
    board: &Board,
    charts: &[Value],
    results: &serde_json::Map<String, Value>,
    time: &lakehouse_bi::grain::TimeContext,
) -> Value {
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

/// Shared `{ board, layout, charts, results }` assembly for both
/// [`data`] and [`public_dashboard`]: every stored chart belonging to
/// `board_id`, filtered by `filters` (never by caller-supplied years —
/// neither route accepts a `year` parameter).
async fn render_board_payload(
    ch: &ChClient,
    obligations: &PolicyEngineObligations<'_>,
    board: &Board,
    board_id: &str,
    filters: &[FilterDef],
    time: &lakehouse_bi::grain::TimeContext,
) -> Result<Value, lakehouse_clickhouse::ChError> {
    let roles = [EMBED_VIEWER_ROLE.to_owned()];
    let placeholders = crate::sql_rewrite::PlaceholderValues::none();
    // An embed or a public link is read by nobody in particular: a chart in the
    // trash or on a personal board is not in it (BI-18 part C, part 2).
    let stored = store::list_stored_charts(ch, &Viewer::anonymous()).await?;
    let field_catalog = catalog_or_empty(ch).await;
    let stored_for_board: Vec<&StoredChartSpec> = stored
        .iter()
        .filter(|c| {
            if c.board.is_empty() {
                board_id == "default"
            } else {
                c.board == board_id
            }
        })
        .collect();

    let cols = board_columns(ch, &stored_for_board, filters, &field_catalog).await?;

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
    for original in &stored_for_board {
        let tile_filters = tile_filters(board, filters, &original.spec.id);
        let effective = crate::routes::support::effective_chart(original, &cols, &sources, &read);
        let c = match &effective {
            Ok(e) => &**e,
            Err(msg) => {
                results.insert(original.spec.id.clone(), json!({ "error": msg }));
                charts_out.push(embed_rendered(original, original));
                continue;
            }
        };
        match stored_chart_sql(c, &[], &tile_filters, &cols, &sources, &read) {
            Ok(filtered) => {
                let (id, mut val) = run_spec_sql(
                    ch,
                    &c.spec.id,
                    &filtered.sql,
                    &roles,
                    &placeholders,
                    obligations,
                )
                .await;
                crate::routes::support::annotate_params(&mut val, c, &values);
                crate::routes::support::annotate_grain(&mut val, &filtered, c);
                // BI-16 part A: the first page of a raw table, with its
                // total; an embed and a public link offer no paging.
                crate::routes::support::annotate_table(
                    ch,
                    &mut val,
                    &filtered,
                    (&roles, &placeholders, obligations),
                )
                .await;
                let tile_ctx = crate::routes::layers::LayerContext {
                    filters: &tile_filters,
                    ..layer_ctx
                };
                crate::routes::layers::annotate_layers(&tile_ctx, &mut val, c, filtered.grain)
                    .await;
                annotate_metric_labels(&mut val, c, &field_catalog, Some((&cols, &sources)));
                results.insert(id, val);
            }
            Err(msg) => {
                results.insert(c.spec.id.clone(), json!({ "error": msg }));
            }
        }

        charts_out.push(embed_rendered(original, c));
    }

    Ok(board_body(board, &charts_out, &results, time))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn params_to_filters_none_yields_empty() {
        assert!(params_to_filters(None).is_empty());
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
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            owner_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };

        let body = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &filters,
            &lakehouse_bi::grain::TimeContext::default(),
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
            created_at: None,
            updated_at: None,
            public_token: None,
            embed_enabled: None,
            folder_id: None,
            owner_id: None,
            embed_access: lakehouse_bi::embed_access::EmbedAccess::default(),
        };
        let body = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &filters,
            &lakehouse_bi::grain::TimeContext::default(),
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

        let body = render_board_payload(
            &ch,
            &PolicyEngineObligations::new(None, &ch),
            &board,
            "b1",
            &locked,
            &lakehouse_bi::grain::TimeContext::default(),
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
            &time,
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
