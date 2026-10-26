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
use crate::routes::dashboard_access::viewer_for;
use crate::state::AppState;
use crate::upstream_error::{self, Context, FailedAs};

/// What the person is told when the deployment has no renderer (feature
/// decision 6). The browser's print stays available in the console.
pub(crate) const PDF_NOT_SET_UP: &str = "PDF export is not set up on this deployment";

/// What the person is told when the deployment has no renderer and asks for a
/// chart image (`BI-19` plan D13): the same three settings as the PDF.
pub(crate) const IMAGES_NOT_SET_UP: &str = "Images are not set up on this deployment.";

/// What a service token is told when it asks for an image.
const IMAGE_FOR_PEOPLE: &str =
    "An image is made for a signed-in person; a service token cannot ask for one.";

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
pub(crate) fn pdf_failure(err: &PdfError) -> ApiError {
    let ctx = match err {
        PdfError::Timeout => &PDF_TIMEOUT,
        PdfError::Unreachable(_) => &PDF_UNREACHABLE,
        PdfError::Refused(_) | PdfError::NotAPdf | PdfError::NotAnImage => &PDF_FAILED,
    };
    upstream_error::report(ctx, upstream_error::Class::Failed, err)
        .into_api_error(FailedAs::ServiceUnavailable)
}

/// What a PDF is made of: the dashboard, the person it is made as, the values
/// the print view reads it with and the paper.
pub(crate) struct PdfJob<'a> {
    pub board: &'a str,
    /// The person the render token names (the print view reads as them).
    pub person: uuid::Uuid,
    pub values: DashboardValues<'a>,
    pub paper: Paper,
    pub orientation: Orientation,
}

/// The filter, parameter and grain text a dashboard is read with, exactly as
/// `GET /api/dashboard` takes them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DashboardValues<'a> {
    pub filters: Option<&'a str>,
    pub params: Option<&'a str>,
    pub grain: Option<&'a str>,
}

/// Why no PDF came back. The route turns it into a fixed sentence under a
/// reference; the report runner turns it into the delivery's reason. Neither
/// carries Gotenberg's own text (SEC-11).
#[derive(Debug)]
pub(crate) enum PdfFailure {
    /// `GOTENBERG_URL`, `CONSOLE_INTERNAL_URL` or `RENDER_TOKEN_SECRET` unset.
    NotSetUp,
    /// No slot came free within [`crate::pdf::PDF_SLOT_WAIT`].
    Busy,
    /// Gotenberg did not give a PDF.
    Conversion(PdfError),
}

impl PdfFailure {
    /// The route's answer: the fixed sentences, Gotenberg's text only in the log.
    pub(crate) fn into_api_error(self) -> ApiError {
        match self {
            Self::NotSetUp => ApiError::Unavailable(PDF_NOT_SET_UP.to_owned()),
            Self::Busy => ApiError::Unavailable(PDF_BUSY.to_owned()),
            Self::Conversion(err) => pdf_failure(&err),
        }
    }
}

/// Make one dashboard's PDF: take a PDF slot, mint the render token for
/// `job.person`, and have Gotenberg print the console's print view. The one
/// copy of this path: the download route and the scheduled report (`RPT-1`)
/// both call it, so the slot limit, the token (`RENDER_TOKEN_SECRET` only,
/// `BI-1` review fix R1) and the address (built from configuration, never from
/// a request) cannot differ between them.
///
/// The caller has already checked that the person may download and that the
/// board is theirs to see; this function checks nothing about the person.
///
/// # Errors
///
/// [`PdfFailure`] as described there.
pub(crate) async fn make_pdf(state: &AppState, job: &PdfJob<'_>) -> Result<Vec<u8>, PdfFailure> {
    let _slot = take_slot(state).await?;
    let (page_url, gotenberg) = render_page(
        state,
        &RenderPage {
            kind: "dashboard",
            chart: None,
            board: job.board,
            person: job.person,
            values: job.values,
            paper: (job.paper.as_str(), job.orientation.as_str()),
        },
    )?;
    convert_url(
        gotenberg,
        &page_url,
        (job.paper, job.orientation),
        crate::pdf::PDF_DEADLINE,
    )
    .await
    .map_err(PdfFailure::Conversion)
}

/// What a chart image is made of (`BI-19` part D, plan D13): one chart of a
/// dashboard, the person it is made as, and the values the print view reads
/// the dashboard with.
pub(crate) struct PngJob<'a> {
    pub board: &'a str,
    pub chart: &'a str,
    pub person: uuid::Uuid,
    pub values: DashboardValues<'a>,
}

/// Make one chart's PNG: the same slot, the same token secret and the same
/// configured address as [`make_pdf`], with a token that names the chart, and
/// Gotenberg's screenshot route in place of the print route.
///
/// # Errors
///
/// [`PdfFailure`] as for [`make_pdf`]; `NotSetUp` reads "Images are not set up
/// on this deployment" at the route (it is the same three settings).
pub(crate) async fn make_png(state: &AppState, job: &PngJob<'_>) -> Result<Vec<u8>, PdfFailure> {
    let _slot = take_slot(state).await?;
    let (page_url, gotenberg) = render_page(
        state,
        &RenderPage {
            kind: "chart",
            chart: Some(job.chart),
            board: job.board,
            person: job.person,
            values: job.values,
            // The claims carry a paper and an orientation for a PDF; an image
            // has neither, and says so rather than naming a paper it is not on.
            paper: ("image", "none"),
        },
    )?;
    crate::pdf::screenshot_url(gotenberg, &page_url, crate::pdf::PDF_DEADLINE)
        .await
        .map_err(PdfFailure::Conversion)
}

/// The slots are taken for the whole conversion. A further request waits for
/// one, but not longer than a minute (BI-1 review fix (BLOCKER) R2): then it
/// is told, in a sentence, instead of meeting the route's deadline. The PDF and
/// the image share the slots, so Chromium never runs more than `PDF_AT_ONCE`
/// conversions together.
async fn take_slot(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, PdfFailure> {
    crate::pdf::acquire_slot(&state.pdf_slots, crate::pdf::PDF_SLOT_WAIT)
        .await
        .ok_or(PdfFailure::Busy)
}

/// The print view one token opens.
struct RenderPage<'a> {
    /// `dashboard` or `chart`: the path under `/render/`.
    kind: &'static str,
    chart: Option<&'a str>,
    board: &'a str,
    person: uuid::Uuid,
    values: DashboardValues<'a>,
    paper: (&'a str, &'a str),
}

/// Mint the render token and build the address Gotenberg opens, from
/// configuration and the token only (`BI-1` review fix R1: `RENDER_TOKEN_SECRET`
/// only). Returns the page address and the Gotenberg address.
fn render_page<'s>(
    state: &'s AppState,
    page: &RenderPage<'_>,
) -> Result<(String, &'s str), PdfFailure> {
    let (Some(secret), Some(gotenberg), Some(console)) = (
        state.config.render_token_secret.as_ref(),
        state.config.gotenberg_url.as_deref(),
        state.config.console_internal_url.as_deref(),
    ) else {
        return Err(PdfFailure::NotSetUp);
    };
    let now = lakehouse_embed::unix_now().floor();
    let token = lakehouse_embed::sign_render(
        &lakehouse_embed::RenderClaims {
            aud: lakehouse_embed::RENDER_AUDIENCE.to_owned(),
            board: page.board.to_owned(),
            chart: page.chart.map(str::to_owned),
            principal: page.person.to_string(),
            filters: page.values.filters.map(str::to_owned),
            params: page.values.params.map(str::to_owned),
            grain: page.values.grain.map(str::to_owned),
            paper: page.paper.0.to_owned(),
            orientation: page.paper.1.to_owned(),
            iat: now,
            exp: lakehouse_embed::render_expiry(now),
            jti: uuid::Uuid::new_v4().to_string(),
        },
        secret.expose_secret(),
    );
    // The address comes from configuration and the token only.
    Ok((
        format!(
            "{}/render/{}/{token}",
            console.trim_end_matches('/'),
            page.kind
        ),
        gotenberg,
    ))
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
    // BI-19 part B1 (plan D8): the router floor is "signed in", so the entry
    // check comes first and a caller with neither `dashboard:read` nor a rule
    // is refused before anything else is looked at.
    let viewer = viewer_for(&state, &principal).await?;
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
    // Through the caller's rules (resolved above), and the built-in board only
    // for somebody with a starting point.
    let board = store::get_board(&state.clickhouse, board_id, &viewer)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?;
    let missing = if board_id == store::DEFAULT_BOARD_ID {
        !viewer.sees_builtin()
    } else {
        board.is_none()
    };
    if missing {
        return Err(ApiError::NotFound("this dashboard does not exist".to_owned()).into());
    }
    let name = board.as_ref().map_or("Main", |b| b.name.as_str());

    let pdf = make_pdf(
        &state,
        &PdfJob {
            board: board_id,
            person: principal.id.uuid(),
            values: DashboardValues {
                filters: q.filters.as_deref(),
                params: q.params.as_deref(),
                grain: q.grain.as_deref(),
            },
            paper,
            orientation,
        },
    )
    .await
    .map_err(PdfFailure::into_api_error)?;

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

/// `GET /api/dashboard/chart-export?chart=&format=png` — one chart as a PNG
/// (`BI-19` part D, plan D13), called by `chart_export` for that format.
///
/// The chart is found through the caller's own reads
/// (`list_stored_charts` with their resolved viewer), so a chart on a
/// dashboard their rules leave out, in the trash, or in somebody else's
/// personal space is the same 404 as one that does not exist. The renderer
/// then reads the dashboard as the same person with a token that names only
/// this chart (`routes::render`). Same `require_download`, same slots and fixed
/// sentences as the PDF; the audit entry is `dashboard.chart_png`.
///
/// # Errors
///
/// 400 for a missing chart, filters that do not parse, a tile that shows no
/// data, or a service token; 403 when every role of the person is switched off
/// from downloading; 404 for a chart the person cannot see; 503 with a fixed
/// sentence when images are not set up, Gotenberg is down or too slow, or the
/// audit store is down.
pub(crate) async fn export_chart_png(
    state: &AppState,
    principal: &Principal,
    q: &crate::routes::dashboard::ChartExportQuery,
) -> ApiResult<Response> {
    // The entry check (and the resolved viewer) first, as the PDF route does.
    let viewer = viewer_for(state, principal).await?;
    if matches!(principal.id, lakehouse_auth::PrincipalId::Service(_)) {
        return Err(ApiError::BadRequest(IMAGE_FOR_PEOPLE.to_owned()).into());
    }
    crate::routes::settings::require_download(state, principal).await?;
    if !pdf_supported(&state.config) {
        return Err(ApiError::Unavailable(IMAGES_NOT_SET_UP.to_owned()).into());
    }
    let chart_id = q
        .chart
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| ApiError::BadRequest("chart is required".to_owned()))?;
    crate::routes::dashboard::check_filters_text(q.filters.as_deref())?;
    let chart = store::list_stored_charts(&state.clickhouse, &viewer)
        .await
        .map_err(|err| classify_dashboard_ch_error(&err))?
        .into_iter()
        .find(|c| c.spec.id == chart_id)
        .ok_or_else(|| ApiError::NotFound("this tile does not exist".to_owned()))?;
    if chart.spec.kind.is_static() {
        return Err(ApiError::BadRequest(
            "this tile shows no data, so there is nothing to download".to_owned(),
        )
        .into());
    }
    let board = if chart.board.is_empty() {
        store::DEFAULT_BOARD_ID
    } else {
        chart.board.as_str()
    };
    let png = make_png(
        state,
        &PngJob {
            board,
            chart: chart_id,
            person: principal.id.uuid(),
            values: DashboardValues {
                filters: q.filters.as_deref(),
                params: q.params.as_deref(),
                grain: q.grain.as_deref(),
            },
        },
    )
    .await
    .map_err(|failure| match failure {
        PdfFailure::NotSetUp => ApiError::Unavailable(IMAGES_NOT_SET_UP.to_owned()),
        other => other.into_api_error(),
    })?;

    record_download_audit(
        state,
        principal,
        ("dashboard.chart_png", "dashboard_tile", chart_id),
        json!({
            "board": board,
            "tile": chart_id,
            "width": crate::pdf::PNG_WIDTH,
            "scale": crate::pdf::PNG_SCALE,
            "bytes": png.len(),
        }),
    )
    .await?;

    let date = crate::routes::settings::report_date(state).await;
    let file_name = format!("{}-{date}.png", file_slug(&chart.spec.title, "chart"));
    Ok((
        [
            (header::CONTENT_TYPE, "image/png".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{file_name}\""),
            ),
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        png,
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
            (PdfError::NotAnImage, "The PDF could not be made."),
        ];
        for (err, sentence) in cases {
            let text = pdf_failure(&err).to_string();
            assert!(text.starts_with(sentence), "{text}");
            assert!(text.contains("Reference:"), "{text}");
            assert!(!text.contains("gotenberg:3000"), "{text}");
        }
    }
}
