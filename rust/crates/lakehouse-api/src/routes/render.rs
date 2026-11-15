//! `POST /api/render/data` — the data behind the PDF's print view (`BI-1`,
//! plan decisions D2 and D3, feature decisions 2 and 3).
//!
//! The PDF is made by a headless browser (Gotenberg) that opens the console's
//! print view, `/render/dashboard/<token>`. That browser has no session, so
//! the view reads its data with a render token (`lakehouse_embed::RenderClaims`)
//! that names the person who asked, one dashboard and the filter, parameter
//! and grain values they had on screen. The token lives two minutes.
//!
//! # The data is the person's own, not the embed viewer's
//!
//! An embed reads as the seeded `Dashboard Viewer` role. A PDF made for a
//! person must show what that person is allowed to see and no more or less, so
//! this route loads the person named in the token from the identity store
//! *now* (their roles today, not the roles at the time the token was minted),
//! builds the `Principal`, and answers through the same function the dashboard
//! page uses (`dashboard::payload_for`): same masks, same row filters, same
//! personal-space rule. A disabled or deleted person is refused.
//!
//! # Public, token only
//!
//! The route is `Policy::Public` (the renderer has no cookie) and takes no
//! `Extension<Principal>`: a session cookie sent along is never read. The body
//! is `{ "token": ... }` and nothing else, so a caller cannot name another
//! board; the board is in the signed token. Every refusal reads the same.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::json::ApiJson;
use crate::routes::dashboard::{DashboardQuery, payload_for};
use crate::state::AppState;
use crate::upstream_error;

/// What every refused render token gets (one sentence, so a caller cannot tell
/// which check failed).
pub(crate) const RENDER_TOKEN_REFUSED: &str = "render token is invalid or expired";

/// What the route says when PDF export is not set up (`RENDER_TOKEN_SECRET` unset).
pub(crate) const PDF_NOT_SET_UP: &str = "PDF export is not set up on this deployment";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RenderBody {
    token: String,
}

fn refused() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        ApiJson(json!({ "error": RENDER_TOKEN_REFUSED })),
    )
        .into_response()
}

/// `POST /api/render/data { token }` — the dashboard payload for the person
/// the token names, with a `render` block for the print view's first page.
///
/// # Errors
///
/// 400 for a body that is not `{ "token": string }`; 503 when PDF export is
/// not set up; 401 (one sentence) for any token that does not verify, is
/// expired, or names a person who is disabled or gone; otherwise what
/// `GET /api/dashboard` would answer for that person.
pub async fn data(State(state): State<AppState>, body: Bytes) -> Response {
    let Ok(RenderBody { token }) = serde_json::from_slice::<RenderBody>(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "body must be { \"token\": string }" })),
        )
            .into_response();
    };
    let (claims, principal) = match admit(&state, &token).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    // BI-53 (QA LH-BI-1-01 F3): the print view's own read is a stage of one
    // PDF; durations only, never a token or a name.
    tracing::info!("render data requested");
    let read_started = std::time::Instant::now();
    let query = DashboardQuery::for_render(
        &claims.board,
        claims.filters.as_deref(),
        claims.params.as_deref(),
        claims.grain.as_deref(),
    );
    // `BI-39` D7: a chart image's data is that chart, so the board's other tiles
    // are not computed for it. `narrow_to_chart` below stays the guard on what
    // leaves, whatever was read.
    let query = match claims.chart.as_deref() {
        Some(chart) => query.only_chart(chart),
        None => query,
    };
    // `SEC-50` slice 2 M4/M5: the PDF reads as the person the token names —
    // their first tenant, because the token carries no tenant (M5).
    let read = payload_for(&state, &principal, &query, &HeaderMap::new()).await;
    tracing::info!(
        payload_ms = crate::routes::pdf_export::millis(read_started),
        ok = read.is_ok(),
        "render data read"
    );
    match read {
        Ok(mut payload) => {
            // BI-19 part D (plan D13): a chart token is good for that chart
            // only. The dashboard was read as the person (rules, masks, trash,
            // personal space), so a chart they cannot see is not in it; a
            // token naming another chart, or one that is gone, reads the same
            // as any refused token and nothing of the rest of the board leaves.
            if let Some(chart) = claims.chart.as_deref()
                && !narrow_to_chart(&mut payload, chart)
            {
                return refused();
            }
            if let Some(map) = payload.as_object_mut() {
                map.insert(
                    "render".to_owned(),
                    render_block(&principal.display_name, &claims),
                );
            }
            (StatusCode::OK, ApiJson(payload)).into_response()
        }
        Err(response) => response,
    }
}

/// A render token that verifies, and the person it names, or the response to
/// send instead (one fixed sentence for every refusal).
///
/// The one admission of every route that takes a render token: the data and an
/// uploaded map's boundary (`BI-36`), so they cannot disagree about what a token opens.
#[allow(
    clippy::result_large_err,
    reason = "the Err is a ready Response the caller returns verbatim; boxing it would change every call site for no gain on the success path"
)]
async fn admit(
    state: &AppState,
    token: &str,
) -> Result<(lakehouse_embed::RenderClaims, lakehouse_auth::Principal), Response> {
    let Some(secret) = state.config.render_token_secret.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": PDF_NOT_SET_UP })),
        )
            .into_response());
    };
    let claims = match lakehouse_embed::verify_render(
        token,
        secret.expose_secret(),
        lakehouse_embed::unix_now(),
    ) {
        Ok(claims) => claims,
        Err(reason) => {
            tracing::info!(%reason, "render token refused");
            return Err(refused());
        }
    };
    let principal = match principal_of(state, &claims.principal).await {
        Ok(Some(principal)) => principal,
        Ok(None) => return Err(refused()),
        Err(response) => return Err(response),
    };
    // BI-1 review fix (SHOULD-FIX) R9: the person's rights are read now, on every
    // data call, not taken from the moment the token was minted. A role removed,
    // or downloads switched off for it, inside the token's two minutes stops the
    // data at the next call. (Replaying the token inside those two minutes by
    // someone who holds it stays accepted, D2.)
    if !principal.has("dashboard:read") {
        return Err(refused());
    }
    match crate::routes::settings::format_settings(state).await {
        Ok(settings) => {
            if !crate::routes::settings::may_download(&principal, &settings.download_denied_roles) {
                return Err(refused());
            }
        }
        Err(err) => return Err(crate::error::ApiRejection(err).into_response()),
    }
    Ok((claims, principal))
}

/// Body of the route that takes only the token (the map
/// boundary): `{ "token": string }` and nothing else.
fn token_of(body: &Bytes) -> Option<String> {
    serde_json::from_slice::<RenderBody>(body)
        .ok()
        .map(|b| b.token)
}

/// `POST /api/render/maps/{id}/geojson { token }` — the boundary file of an
/// uploaded map one of the board's charts draws on, for the print view
/// (`BI-36`).
///
/// The print view has no session, so the signed-in loader (`GET
/// /api/maps/{id}/geojson`) answered it with a redirect to the login page and
/// the map tile did not draw. The public link and the embed got their own token
/// routes in `PR #246`; this is the print view's, with the same rule: the token
/// opens its board's maps and nothing else, and a map no chart on that board
/// draws on is the same 404 as a map that does not exist. The token is checked
/// as [`data`] checks it (the person's rights are read now), and the charts are
/// read as that person.
///
/// # Errors
///
/// 400 for a body that is not `{ "token": string }`; 503 when PDF export is not
/// set up; 401 (one sentence) for a token that is refused; 404 for a map the
/// board does not use.
pub async fn map_geojson(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    body: Bytes,
) -> Response {
    let Some(token) = token_of(&body) else {
        return (
            StatusCode::BAD_REQUEST,
            ApiJson(json!({ "error": "body must be { \"token\": string }" })),
        )
            .into_response();
    };
    let (claims, principal) = match admit(&state, &token).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let viewer = match crate::routes::dashboard_access::viewer_for(&state, &principal).await {
        Ok(viewer) => viewer,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    crate::routes::embed::map_for_board(&state, &claims.board, &id, &headers, &viewer).await
}

/// The most bytes of an image card's picture the print view takes (`BI-42`).
const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024;

/// How long the fetch of one picture may take.
const IMAGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// What a picture that is not one of the board's allowed image cards gets: one
/// sentence, so the route is no oracle for what a board holds.
const IMAGE_NOT_AVAILABLE: &str = "image is not available";

/// What the route says for a body that is not a PNG, JPEG, GIF or `WebP`.
const IMAGE_NOT_SUPPORTED: &str = "the address does not give a supported image";

/// What the route says for a picture over [`IMAGE_MAX_BYTES`].
const IMAGE_TOO_LARGE: &str = "the image is too large to print";

/// The query of `GET /api/render/image`. Both fields are optional so a missing
/// one is the route's own 400, not the extractor's plain-text rejection.
#[derive(Debug, Deserialize)]
pub struct ImageQuery {
    /// The render token the print view was opened with.
    pub token: Option<String>,
    /// The address saved on the image card.
    pub src: Option<String>,
}

/// The media type of `bytes` by its magic numbers: PNG, JPEG, GIF or `WebP`.
/// SVG, and anything else, is `None`, whatever the server's header said.
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn image_refusal(status: StatusCode, sentence: &'static str) -> Response {
    (status, ApiJson(json!({ "error": sentence }))).into_response()
}

/// `GET /api/render/image?token=&src=` — one image card's picture for the print
/// view (`BI-42`, plan D6b). Gotenberg's Chromium may open only the console
/// (feature decision 3), so the print view points the card's `<img>` here and
/// the API fetches the picture from the card's site.
///
/// What it fetches is decided by the saved board, never by the request: `src`
/// must be exactly the address of an image card on the token's board (read as
/// the person the token names), and that address's site must be among the
/// board's content sites now. Anything else is a 404 with one sentence. The
/// fetch is `https` only, through a client pinned to the addresses
/// [`crate::connector_probe::resolve_checked`] approved (so a name that
/// resolves inside the deployment's network is refused), follows no redirect,
/// stops at 10 seconds and reads at most 5 MiB. Only PNG, JPEG, GIF and `WebP`
/// pass, judged by the first bytes; SVG never does. Upstream error text never
/// reaches the answer (`upstream_error`).
///
/// # Errors
///
/// 400 without `token` and `src`; the refusals of [`admit`] for a bad token; 404
/// for an address that is not an allowed image card of the board; 413 over the
/// size cap; 415 for a body that is not a supported image; a classified 500 when
/// the fetch fails.
pub async fn image(State(state): State<AppState>, Query(query): Query<ImageQuery>) -> Response {
    let (Some(token), Some(src)) = (query.token, query.src) else {
        return image_refusal(StatusCode::BAD_REQUEST, "token and src are required");
    };
    let (claims, principal) = match admit(&state, &token).await {
        Ok(admitted) => admitted,
        Err(response) => return response,
    };
    let viewer = match crate::routes::dashboard_access::viewer_for(&state, &principal).await {
        Ok(viewer) => viewer,
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    };
    match image_card_allowed(&state, &claims.board, &viewer, &src).await {
        Ok(true) => {}
        Ok(false) => return image_refusal(StatusCode::NOT_FOUND, IMAGE_NOT_AVAILABLE),
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    }
    fetch_image(&state, &src).await
}

/// Whether `src` is exactly the address of an image card on `board_id` whose
/// site is in the deployment's content sites now.
async fn image_card_allowed(
    state: &AppState,
    board_id: &str,
    viewer: &lakehouse_bi::access::Viewer,
    src: &str,
) -> Result<bool, lakehouse_core::ApiError> {
    let format = crate::routes::settings::format_settings(state).await?;
    if format.content_sites.is_empty() {
        return Ok(false);
    }
    let stored = lakehouse_bi::store::list_stored_charts(&state.clickhouse, viewer)
        .await
        .map_err(|err| {
            upstream_error::report_ch(&upstream_error::DATABASE, &err)
                .into_api_error(upstream_error::FailedAs::Internal)
        })?;
    let cards = crate::routes::embed::board_cards(&stored, board_id);
    Ok(cards.into_iter().any(|card| {
        card.spec.kind == lakehouse_bi::specs::ChartKind::Image
            && card.def.types.url.as_deref() == Some(src)
            && !crate::routes::settings::content_sites_of_cards(&[card], &format).is_empty()
    }))
}

/// Fetch `src` under the rules of [`image`] and answer with the picture.
async fn fetch_image(state: &AppState, src: &str) -> Response {
    // The error itself goes to `upstream_error`, which logs its text under a
    // reference id and answers with the fixed sentence (SEC-11); a reason of
    // this function's own is passed the same way.
    let failed = |cause: &dyn std::fmt::Display| {
        crate::error::ApiRejection(upstream_error::internal_error(
            &upstream_error::SERVICE,
            cause,
        ))
        .into_response()
    };
    let internal_hosts = state.config.connector_internal_hosts();
    let Some((host, port)) = crate::connector_probe::parse_endpoint_host_port(src)
        .filter(|_| src.starts_with("https://"))
    else {
        return image_refusal(StatusCode::NOT_FOUND, IMAGE_NOT_AVAILABLE);
    };
    let Ok(checked) = crate::connector_probe::resolve_checked(host, port, &internal_hosts).await
    else {
        return failed(&"the image host was refused or did not resolve");
    };
    let Ok(client) = reqwest::Client::builder()
        .timeout(IMAGE_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, checked.all())
        .build()
    else {
        return failed(&"the image client could not be built");
    };
    let mut response = match client.get(src).send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(_) => return failed(&"the image host did not answer with the image"),
        Err(err) => return failed(&err),
    };
    if response
        .content_length()
        .is_some_and(|n| n > u64::try_from(IMAGE_MAX_BYTES).unwrap_or(u64::MAX))
    {
        return image_refusal(StatusCode::PAYLOAD_TOO_LARGE, IMAGE_TOO_LARGE);
    }
    let mut body: Vec<u8> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() + chunk.len() > IMAGE_MAX_BYTES {
                    return image_refusal(StatusCode::PAYLOAD_TOO_LARGE, IMAGE_TOO_LARGE);
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(err) => return failed(&err),
        }
    }
    let Some(media_type) = sniff_image(&body) else {
        return image_refusal(StatusCode::UNSUPPORTED_MEDIA_TYPE, IMAGE_NOT_SUPPORTED);
    };
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, media_type),
            (axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (axum::http::header::CACHE_CONTROL, "private, max-age=300"),
        ],
        body,
    )
        .into_response()
}

/// Keep `chart` and nothing else of the board's tiles: its spec, its result and
/// its place. Returns `false` when the payload has no such chart. The board's
/// other tiles' data never reaches a chart image's view.
fn narrow_to_chart(payload: &mut Value, chart: &str) -> bool {
    let Some(map) = payload.as_object_mut() else {
        return false;
    };
    let Some(charts) = map.get_mut("charts").and_then(Value::as_array_mut) else {
        return false;
    };
    charts.retain(|c| c.get("id").and_then(Value::as_str) == Some(chart));
    if charts.is_empty() {
        return false;
    }
    if let Some(results) = map.get_mut("results").and_then(Value::as_object_mut) {
        results.retain(|id, _| id == chart);
    }
    if let Some(layout) = map.get_mut("layout").and_then(Value::as_object_mut) {
        layout.retain(|id, _| id == chart);
    }
    // The tab list and the phone order describe the other tiles.
    map.remove("tabs");
    map.remove("phoneLayout");
    true
}

/// What the print view's first page lists beyond the dashboard itself (feature
/// decision 4): who asked, when, and the paper.
fn render_block(requested_by: &str, claims: &lakehouse_embed::RenderClaims) -> Value {
    json!({
        "requestedBy": requested_by,
        "issuedAt": claims.iat,
        "paper": claims.paper,
        "orientation": claims.orientation,
        "chart": claims.chart,
    })
}

/// The person a token names, as they are now: `None` for a disabled or deleted
/// person (or an id that is not a user's), a response for a store failure.
// `result_large_err` now reports `async fn`s too (clippy 1.98,
// rust-clippy#17130). The `Err` is a ready `Response` the caller returns
// verbatim, so boxing it would change this contract and every call site for
// an allocation only on the error path (PR #204 CI, REL-9 decision D2).
#[allow(
    clippy::result_large_err,
    reason = "the Err is a ready Response the caller returns verbatim; boxing it would change every call site for no gain on the success path"
)]
pub(crate) async fn principal_of(
    state: &AppState,
    id: &str,
) -> Result<Option<lakehouse_auth::Principal>, Response> {
    let Ok(user_id) = id.parse::<uuid::Uuid>() else {
        return Ok(None);
    };
    let Some(pool) = state.pg.as_deref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": "the identity store is unavailable" })),
        )
            .into_response());
    };
    let store_failed = |err: &dyn std::fmt::Display| {
        crate::error::ApiRejection(upstream_error::internal_error(&upstream_error::STORE, err))
            .into_response()
    };
    match lakehouse_store::identity::user_status(pool, user_id).await {
        Ok(Some(status)) if status == "active" => {}
        Ok(_) => return Ok(None),
        Err(err) => return Err(store_failed(&err)),
    }
    match lakehouse_auth::repository::load_principal_for_user(
        pool,
        user_id,
        "render".to_owned(),
        false,
    )
    .await
    {
        Ok(principal) => Ok(Some(principal)),
        // Raced with a delete between the two reads.
        Err(lakehouse_auth::AuthError::NotFound) => Ok(None),
        // As `authored_pipelines` reports it: the `sqlx` source goes to the
        // log under a reference, the response is the fixed message.
        Err(lakehouse_auth::AuthError::Database(source)) => Err(crate::error::ApiRejection(
            upstream_error::report_sqlx(&upstream_error::STORE, &source)
                .into_api_error(upstream_error::FailedAs::Internal),
        )
        .into_response()),
        Err(other) => Err(store_failed(&other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `BI-42`: the type is judged by the first bytes, never by a header; SVG,
    /// HTML and an empty body are refused.
    #[test]
    fn an_image_is_known_by_its_magic_numbers_and_svg_is_refused() {
        assert_eq!(
            sniff_image(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            Some("image/png")
        );
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a\x01\x00"), Some("image/gif"));
        assert_eq!(sniff_image(b"GIF87a"), Some("image/gif"));
        assert_eq!(
            sniff_image(b"RIFF\x10\x00\x00\x00WEBPVP8 "),
            Some("image/webp")
        );
        assert_eq!(sniff_image(b"RIFF\x10\x00\x00\x00WAVEfmt "), None);
        assert_eq!(
            sniff_image(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"),
            None
        );
        assert_eq!(sniff_image(b"<?xml version=\"1.0\"?><svg/>"), None);
        assert_eq!(sniff_image(b"<html></html>"), None);
        assert_eq!(sniff_image(b""), None);
    }
}
