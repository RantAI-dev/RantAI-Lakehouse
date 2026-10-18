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
use axum::extract::State;
use axum::http::StatusCode;
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
    let Some(secret) = state.config.render_token_secret.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            ApiJson(json!({ "error": PDF_NOT_SET_UP })),
        )
            .into_response();
    };
    let claims = match lakehouse_embed::verify_render(
        &token,
        secret.expose_secret(),
        lakehouse_embed::unix_now(),
    ) {
        Ok(claims) => claims,
        Err(reason) => {
            tracing::info!(%reason, "render token refused");
            return refused();
        }
    };
    let principal = match principal_of(&state, &claims.principal).await {
        Ok(Some(principal)) => principal,
        Ok(None) => return refused(),
        Err(response) => return response,
    };
    // BI-1 review fix (SHOULD-FIX) R9: the person's rights are read now, on every
    // data call, not taken from the moment the token was minted. A role removed,
    // or downloads switched off for it, inside the token's two minutes stops the
    // data at the next call. (Replaying the token inside those two minutes by
    // someone who holds it stays accepted, D2.)
    if !principal.has("dashboard:read") {
        return refused();
    }
    match crate::routes::settings::format_settings(&state).await {
        Ok(settings) => {
            if !crate::routes::settings::may_download(&principal, &settings.download_denied_roles) {
                return refused();
            }
        }
        Err(err) => return crate::error::ApiRejection(err).into_response(),
    }
    let query = DashboardQuery::for_render(
        &claims.board,
        claims.filters.as_deref(),
        claims.params.as_deref(),
        claims.grain.as_deref(),
    );
    match payload_for(&state, &principal, &query).await {
        Ok(mut payload) => {
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

/// What the print view's first page lists beyond the dashboard itself (feature
/// decision 4): who asked, when, and the paper.
fn render_block(requested_by: &str, claims: &lakehouse_embed::RenderClaims) -> Value {
    json!({
        "requestedBy": requested_by,
        "issuedAt": claims.iat,
        "paper": claims.paper,
        "orientation": claims.orientation,
    })
}

/// The person a token names, as they are now: `None` for a disabled or deleted
/// person (or an id that is not a user's), a response for a store failure.
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
