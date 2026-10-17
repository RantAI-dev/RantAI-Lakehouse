//! `GET /api/dashboard/export/pdf` — a dashboard as a PDF (`BI-1`, plan
//! decisions D5 and D8, feature decisions 1 to 6 and 9 to 10).
//!
//! The route mints a render token for the person who asked (two minutes, good
//! for this dashboard and these filter values only), asks Gotenberg to print
//! the console's print view at `<CONSOLE_INTERNAL_URL>/render/dashboard/<token>`
//! and returns the bytes. The data in the PDF is read by the print view through
//! `POST /api/render/data` as the same person, so the PDF shows what they may
//! see (`routes::render`).
//!
//! The address Gotenberg opens is built here from configuration and the token;
//! nothing from the query string reaches it, so a caller cannot point the
//! renderer anywhere else (`crate::pdf`).
//!
//! At most [`crate::state::PDF_AT_ONCE`] PDFs are made at once per process; a
//! further request waits for a slot. Every failure is a fixed sentence under a
//! reference (`upstream_error`, SEC-11), never Gotenberg's text.

use axum::Extension;
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use lakehouse_auth::Principal;
use lakehouse_bi::store;
use lakehouse_core::ApiError;
use serde::Deserialize;
use serde_json::json;

use crate::error::ApiResult;
use crate::pdf::{Orientation, Paper, PdfError, convert_url};
use crate::routes::dashboard::{
    classify_dashboard_ch_error, file_slug, pdf_supported, record_download_audit,
};
use crate::routes::dashboard_organise::viewer_of;
use crate::state::AppState;
use crate::upstream_error::{self, Context, FailedAs};

/// What the person is told when the deployment has no renderer (feature
/// decision 6). The browser's print stays available in the console.
pub(crate) const PDF_NOT_SET_UP: &str = "PDF export is not set up on this deployment";

/// What a service token is told (R3).
pub(crate) const PDF_FOR_PEOPLE: &str =
    "A PDF is made for a signed-in person; a service token cannot ask for one.";

/// A request waited a minute for a free PDF slot (`BI-1` review fix (BLOCKER) R2).
pub(crate) const PDF_BUSY: &str = "Too many PDFs are being made right now; try again in a minute.";

/// The PDF was not ready in time.
const PDF_TIMEOUT: Context = Context::new(
    "pdf render timeout",
    "The PDF took too long to make.",
    "The PDF took too long to make.",
);

/// Gotenberg could not be reached.
const PDF_UNREACHABLE: Context = Context::new(
    "pdf render unreachable",
    "The PDF service is not reachable.",
    "The PDF service is not reachable.",
);

/// Gotenberg answered, but not with a PDF.
const PDF_FAILED: Context = Context::new(
    "pdf render failed",
    "The PDF could not be made.",
    "The PDF could not be made.",
);

/// The query of `GET /api/dashboard/export/pdf`: the board and the paper, and
/// the filter, parameter and grain text exactly as `GET /api/dashboard` takes
/// them (the console sends the same strings it loaded the page with).
#[derive(Debug, Default, Deserialize)]
pub struct PdfQuery {
    #[serde(default)]
    board: Option<String>,
    #[serde(default)]
    paper: Option<String>,
    #[serde(default)]
    orientation: Option<String>,
    #[serde(default)]
    filters: Option<String>,
    #[serde(default)]
    params: Option<String>,
    #[serde(default)]
    grain: Option<String>,
}

/// The fixed answer for a failed conversion.
fn pdf_failure(err: &PdfError) -> ApiError {
    let ctx = match err {
        PdfError::Timeout => &PDF_TIMEOUT,
        PdfError::Unreachable(_) => &PDF_UNREACHABLE,
        PdfError::Refused(_) | PdfError::NotAPdf => &PDF_FAILED,
    };
    upstream_error::report(ctx, upstream_error::Class::Failed, err)
        .into_api_error(FailedAs::ServiceUnavailable)
}

/// `GET /api/dashboard/export/pdf?board=&paper=&orientation=` — the PDF.
///
/// # Errors
///
/// 400 for a missing board, paper or orientation outside their two words, or
/// filters that do not parse; 403 when every role of the person is switched
/// off from downloading; 404 for a board the person cannot see; 503 with a
/// fixed sentence when PDF export is not set up, Gotenberg is down or too
/// slow, or the audit store is down.
pub async fn export_pdf(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
    Query(q): Query<PdfQuery>,
) -> ApiResult<Response> {
    // BI-1 review fix (SHOULD-FIX) R3: the print view reads as a person (their
    // roles, their masks), and a service identity is not one; refuse it here,
    // in a sentence, instead of printing a page that cannot load.
    if matches!(principal.id, lakehouse_auth::PrincipalId::Service(_)) {
        return Err(ApiError::BadRequest(PDF_FOR_PEOPLE.to_owned()).into());
    }
    crate::routes::settings::require_download(&state, &principal).await?;
    if !pdf_supported(&state.config) {
        return Err(ApiError::Unavailable(PDF_NOT_SET_UP.to_owned()).into());
    }
    let paper = q
        .paper
        .as_deref()
        .map_or(Some(Paper::A4), Paper::parse)
        .ok_or_else(|| ApiError::BadRequest("paper must be a4 or letter.".to_owned()))?;
    let orientation = q
        .orientation
        .as_deref()
        .map_or(Some(Orientation::Portrait), Orientation::parse)
        .ok_or_else(|| {
            ApiError::BadRequest("orientation must be portrait or landscape.".to_owned())
        })?;
    let board_id = q
        .board
        .as_deref()
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .ok_or_else(|| ApiError::BadRequest("board is required".to_owned()))?;
    // The same checks the page applies, so a malformed filter is a 400 here
    // and not a blank page in the PDF.
    crate::routes::dashboard::check_filters_text(q.filters.as_deref())?;

    // The board must be one this person can see (a trashed board and another
    // person's personal board read as missing). The built-in board has no row
    // until someone saves it.
    let viewer = viewer_of(&principal);
    let board = store::get_board(&state.clickhouse, board_id, &viewer)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    if board.is_none() && board_id != store::DEFAULT_BOARD_ID {
        return Err(ApiError::NotFound("this dashboard does not exist".to_owned()).into());
    }
    let name = board.as_ref().map_or("Main", |b| b.name.as_str());

    // The slots are taken for the whole conversion. A further request waits for
    // one, but not longer than a minute (BI-1 review fix (BLOCKER) R2): then it
    // is told, in a sentence, instead of meeting the route's deadline.
    let _slot = crate::pdf::acquire_slot(&state.pdf_slots, crate::pdf::PDF_SLOT_WAIT)
        .await
        .ok_or_else(|| ApiError::Unavailable(PDF_BUSY.to_owned()))?;

    let (Some(secret), Some(gotenberg), Some(console)) = (
        state.config.render_token_secret.as_ref(),
        state.config.gotenberg_url.as_deref(),
        state.config.console_internal_url.as_deref(),
    ) else {
        return Err(ApiError::Unavailable(PDF_NOT_SET_UP.to_owned()).into());
    };
    let now = lakehouse_embed::unix_now().floor();
    let token = lakehouse_embed::sign_render(
        &lakehouse_embed::RenderClaims {
            aud: lakehouse_embed::RENDER_AUDIENCE.to_owned(),
            board: board_id.to_owned(),
            principal: principal.id.uuid().to_string(),
            filters: q.filters.clone(),
            params: q.params.clone(),
            grain: q.grain.clone(),
            paper: paper.as_str().to_owned(),
            orientation: orientation.as_str().to_owned(),
            iat: now,
            exp: lakehouse_embed::render_expiry(now),
            jti: uuid::Uuid::new_v4().to_string(),
        },
        secret.expose_secret(),
    );
    // The address comes from configuration and the token only.
    let page_url = format!("{}/render/dashboard/{token}", console.trim_end_matches('/'));
    let pdf = convert_url(
        gotenberg,
        &page_url,
        (paper, orientation),
        crate::pdf::PDF_DEADLINE,
    )
    .await
    .map_err(|err| pdf_failure(&err))?;

    record_download_audit(
        &state,
        &principal,
        ("dashboard.pdf_export", "dashboard", board_id),
        json!({
            "board": board_id,
            "paper": paper.as_str(),
            "orientation": orientation.as_str(),
            "bytes": pdf.len(),
        }),
    )
    .await?;

    let date = crate::routes::settings::report_date(&state).await;
    let file_name = format!("{}-{date}.pdf", file_slug(name, "dashboard"));
    Ok((
        [
            (header::CONTENT_TYPE, "application/pdf".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        pdf,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R2: the sentence a request gets when no slot came free in a minute, and
    /// that it is not the "took too long" one (that is Gotenberg's).
    #[test]
    fn a_request_that_waited_for_a_slot_is_told_so() {
        assert_eq!(
            PDF_BUSY,
            "Too many PDFs are being made right now; try again in a minute."
        );
        assert_ne!(PDF_BUSY, "The PDF took too long to make.");
    }

    #[test]
    fn each_failure_has_its_own_fixed_sentence_and_never_the_raw_text() {
        let cases = [
            (PdfError::Timeout, "The PDF took too long to make."),
            (
                PdfError::Unreachable("http://gotenberg:3000 refused".to_owned()),
                "The PDF service is not reachable.",
            ),
            (PdfError::Refused(500), "The PDF could not be made."),
            (PdfError::NotAPdf, "The PDF could not be made."),
        ];
        for (err, sentence) in cases {
            let text = pdf_failure(&err).to_string();
            assert!(text.starts_with(sentence), "{text}");
            assert!(text.contains("Reference:"), "{text}");
            assert!(!text.contains("gotenberg:3000"), "{text}");
        }
    }
}
