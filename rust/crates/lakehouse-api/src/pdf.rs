//! The PDF of a dashboard, made by a Gotenberg container (`BI-1`, plan
//! decisions D1 and D5, feature decision 1).
//!
//! Gotenberg runs Chromium. This module asks it to open one address, the
//! console's print view with a render token in it, wait until the page says it
//! has drawn every tile, and print it. Nothing here knows about dashboards: it
//! is the one HTTP client to Gotenberg, so the rules sit in one place:
//!
//! - The Gotenberg address and the page address come from configuration
//!   (`GOTENBERG_URL`, `CONSOLE_INTERNAL_URL`), never from a request: a caller
//!   cannot make this process, or the renderer, open an address of their
//!   choosing (the same stance as the Dagster and Lakekeeper clients). The
//!   resolver allowlist that guards user-supplied connector hosts does not
//!   apply, because the host is the operator's own service name.
//! - The client follows no redirects and uses no proxy, so a misconfigured
//!   address cannot be bounced elsewhere.
//! - Gotenberg's own text never reaches a caller: the failure is classified
//!   ([`PdfError`]) and the route answers a fixed sentence.
//!
//! The multipart body is written by hand (text fields and one small file), so
//! the `reqwest` `multipart` feature and its dependencies are not added.

use std::time::Duration;

/// How long a PDF may take before the person is told it did not work
/// (feature decision 5).
pub const PDF_DEADLINE: Duration = Duration::from_secs(60);

/// How long a PDF request waits for one of the process's slots
/// ([`crate::state::PDF_AT_ONCE`]) before it is told so (`BI-1` review fix
/// (BLOCKER) R2).
pub const PDF_SLOT_WAIT: Duration = Duration::from_secs(60);

/// Take a slot, waiting at most `wait`; `None` when none came free in time.
pub async fn acquire_slot(
    slots: &std::sync::Arc<tokio::sync::Semaphore>,
    wait: Duration,
) -> Option<tokio::sync::OwnedSemaphorePermit> {
    tokio::time::timeout(wait, slots.clone().acquire_owned())
        .await
        .ok()?
        .ok()
}

/// The total timeout of the HTTP client, a backstop above [`PDF_DEADLINE`]
/// that matches Gotenberg's `--api-timeout=70s` (D5).
const CLIENT_TIMEOUT: Duration = Duration::from_secs(70);

/// What the print view sets on `window` when every tile has drawn or shown its
/// error (plan D4). Gotenberg waits for this expression before it prints.
pub const READY_EXPRESSION: &str = "window.__lakehouseRenderReady === true";

/// `a4` or `letter`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    /// 210 by 297 mm.
    A4,
    /// 8.5 by 11 in.
    Letter,
}

impl Paper {
    /// The query value (`a4`, `letter`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "a4" => Some(Self::A4),
            "letter" => Some(Self::Letter),
            _ => None,
        }
    }

    /// The token and query value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::A4 => "a4",
            Self::Letter => "letter",
        }
    }

    /// Width and height in inches, portrait (Gotenberg's unit).
    const fn inches(self) -> (&'static str, &'static str) {
        match self {
            Self::A4 => ("8.27", "11.7"),
            Self::Letter => ("8.5", "11"),
        }
    }
}

/// `portrait` or `landscape`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Tall.
    Portrait,
    /// Wide.
    Landscape,
}

impl Orientation {
    /// The query value (`portrait`, `landscape`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "portrait" => Some(Self::Portrait),
            "landscape" => Some(Self::Landscape),
            _ => None,
        }
    }

    /// The token and query value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Portrait => "portrait",
            Self::Landscape => "landscape",
        }
    }
}

/// Why a PDF could not be made. For the route to choose a fixed sentence and
/// for the log; the display text is never sent to a caller.
#[derive(Debug)]
pub enum PdfError {
    /// Not ready within [`PDF_DEADLINE`].
    Timeout,
    /// Gotenberg could not be reached.
    Unreachable(String),
    /// Gotenberg answered with an error status (the text is for the log).
    Refused(u16),
    /// The answer was not a PDF.
    NotAPdf,
}

impl std::fmt::Display for PdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "the PDF was not ready in time"),
            Self::Unreachable(e) => write!(f, "Gotenberg is unreachable: {e}"),
            Self::Refused(status) => write!(f, "Gotenberg answered HTTP {status}"),
            Self::NotAPdf => write!(f, "Gotenberg's answer was not a PDF"),
        }
    }
}

/// A `multipart/form-data` body, written by hand.
struct Form {
    boundary: String,
    body: Vec<u8>,
}

impl Form {
    fn new() -> Self {
        Self {
            boundary: format!("lakehouse-{}", uuid::Uuid::new_v4().simple()),
            body: Vec::new(),
        }
    }

    fn text(&mut self, name: &str, value: &str) {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n",
                self.boundary
            )
            .as_bytes(),
        );
    }

    fn file(&mut self, name: &str, file_name: &str, content_type: &str, content: &str) {
        self.body.extend_from_slice(
            format!(
                "--{}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file_name}\"\r\nContent-Type: {content_type}\r\n\r\n{content}\r\n",
                self.boundary
            )
            .as_bytes(),
        );
    }

    fn finish(mut self) -> (String, Vec<u8>) {
        self.body
            .extend_from_slice(format!("--{}--\r\n", self.boundary).as_bytes());
        (
            format!("multipart/form-data; boundary={}", self.boundary),
            self.body,
        )
    }
}

/// The page footer: page number and page count, small and centred.
const FOOTER_HTML: &str = "<html><head><style>\
body{font-size:8px;width:100%;text-align:center;color:#666;margin:0;\
font-family:sans-serif}</style></head>\
<body><span class=\"pageNumber\"></span> / <span class=\"totalPages\"></span></body></html>";

/// Print `page_url` as a PDF through the Gotenberg at `gotenberg_url`.
///
/// `deadline` is [`PDF_DEADLINE`] in the service; a test passes a short one.
///
/// # Errors
///
/// [`PdfError`]: not ready in time, Gotenberg unreachable or refusing, or an
/// answer that is not a PDF.
pub async fn convert_url(
    gotenberg_url: &str,
    page_url: &str,
    (paper, orientation): (Paper, Orientation),
    deadline: Duration,
) -> Result<Vec<u8>, PdfError> {
    let (width, height) = paper.inches();
    let mut form = Form::new();
    form.text("url", page_url);
    form.text("paperWidth", width);
    form.text("paperHeight", height);
    form.text(
        "landscape",
        &(orientation == Orientation::Landscape).to_string(),
    );
    for margin in ["marginTop", "marginBottom", "marginLeft", "marginRight"] {
        form.text(
            margin,
            if margin == "marginBottom" {
                "0.7"
            } else {
                "0.5"
            },
        );
    }
    form.text("printBackground", "true");
    form.text("emulatedMediaType", "print");
    form.text("waitForExpression", READY_EXPRESSION);
    // BI-1 review fix (SHOULD-FIX) R3: a print view that could not load its data
    // raises an uncaught exception (`render-view.tsx`, `renderFailure`); with this
    // Gotenberg fails the conversion (HTTP 409) and the route says "The PDF could
    // not be made." instead of sending a PDF of an error sentence.
    form.text("failOnConsoleExceptions", "true");
    form.file("files", "footer.html", "text/html", FOOTER_HTML);
    let (content_type, body) = form.finish();

    let client = reqwest::Client::builder()
        .timeout(CLIENT_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()
        .map_err(|e| PdfError::Unreachable(e.to_string()))?;
    let endpoint = format!(
        "{}/forms/chromium/convert/url",
        gotenberg_url.trim_end_matches('/')
    );
    let send = client
        .post(endpoint)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .body(body)
        .send();
    let response = match tokio::time::timeout(deadline, send).await {
        Err(_) => return Err(PdfError::Timeout),
        Ok(Err(e)) if e.is_timeout() => return Err(PdfError::Timeout),
        Ok(Err(e)) => return Err(PdfError::Unreachable(e.to_string())),
        Ok(Ok(response)) => response,
    };
    let status = response.status();
    if !status.is_success() {
        return Err(PdfError::Refused(status.as_u16()));
    }
    let bytes = match tokio::time::timeout(deadline, response.bytes()).await {
        Err(_) => return Err(PdfError::Timeout),
        Ok(Err(e)) if e.is_timeout() => return Err(PdfError::Timeout),
        Ok(Err(e)) => return Err(PdfError::Unreachable(e.to_string())),
        Ok(Ok(bytes)) => bytes,
    };
    if !bytes.starts_with(b"%PDF") {
        return Err(PdfError::NotAPdf);
    }
    Ok(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const PAGE: &str = "http://console.test:3000/render/dashboard/t.o.k";
    const SHORT: Duration = Duration::from_millis(300);

    async fn gotenberg(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/forms/chromium/convert/url"))
            .respond_with(response)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn the_request_carries_the_page_the_paper_the_wait_and_a_footer() {
        let server =
            gotenberg(ResponseTemplate::new(200).set_body_bytes(b"%PDF-1.7 x".to_vec())).await;
        let pdf = convert_url(
            &server.uri(),
            PAGE,
            (Paper::Letter, Orientation::Landscape),
            SHORT,
        )
        .await
        .unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        let sent = server.received_requests().await.unwrap();
        let request = &sent[0];
        assert!(
            request.headers["content-type"]
                .to_str()
                .unwrap()
                .starts_with("multipart/form-data; boundary=")
        );
        let body = String::from_utf8_lossy(&request.body);
        for expected in [
            format!("name=\"url\"\r\n\r\n{PAGE}\r\n"),
            "name=\"paperWidth\"\r\n\r\n8.5\r\n".to_owned(),
            "name=\"paperHeight\"\r\n\r\n11\r\n".to_owned(),
            "name=\"landscape\"\r\n\r\ntrue\r\n".to_owned(),
            "name=\"printBackground\"\r\n\r\ntrue\r\n".to_owned(),
            "name=\"failOnConsoleExceptions\"\r\n\r\ntrue\r\n".to_owned(),
            "name=\"emulatedMediaType\"\r\n\r\nprint\r\n".to_owned(),
            format!("name=\"waitForExpression\"\r\n\r\n{READY_EXPRESSION}\r\n"),
            "name=\"files\"; filename=\"footer.html\"".to_owned(),
            "class=\"pageNumber\"".to_owned(),
        ] {
            assert!(body.contains(&expected), "{expected:?} in {body}");
        }
    }

    #[tokio::test]
    async fn a_slow_gotenberg_is_a_timeout_not_a_hang() {
        let server = gotenberg(
            ResponseTemplate::new(200)
                .set_body_bytes(b"%PDF".to_vec())
                .set_delay(Duration::from_secs(5)),
        )
        .await;
        let err = convert_url(
            &server.uri(),
            PAGE,
            (Paper::A4, Orientation::Portrait),
            SHORT,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PdfError::Timeout), "{err}");
    }

    #[tokio::test]
    async fn an_error_status_a_redirect_and_a_non_pdf_are_refusals_without_their_text() {
        let server =
            gotenberg(ResponseTemplate::new(500).set_body_string("/secret/path exploded")).await;
        let err = convert_url(
            &server.uri(),
            PAGE,
            (Paper::A4, Orientation::Portrait),
            SHORT,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PdfError::Refused(500)));
        assert!(!err.to_string().contains("secret"));

        let redirect = gotenberg(
            ResponseTemplate::new(302).insert_header("location", "http://elsewhere.invalid/"),
        )
        .await;
        let err = convert_url(
            &redirect.uri(),
            PAGE,
            (Paper::A4, Orientation::Portrait),
            SHORT,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, PdfError::Refused(302)),
            "no redirect is followed: {err}"
        );

        let html = gotenberg(ResponseTemplate::new(200).set_body_string("<html>")).await;
        let err = convert_url(&html.uri(), PAGE, (Paper::A4, Orientation::Portrait), SHORT)
            .await
            .unwrap_err();
        assert!(matches!(err, PdfError::NotAPdf));
    }

    #[tokio::test]
    async fn a_gotenberg_that_is_down_is_unreachable() {
        let err = convert_url(
            "http://127.0.0.1:1",
            PAGE,
            (Paper::A4, Orientation::Portrait),
            Duration::from_secs(5),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, PdfError::Unreachable(_)), "{err}");
    }

    /// `BI-1` review fix (BLOCKER) R2: a request that cannot get a slot gives up
    /// when told to, instead of waiting for the route's deadline.
    #[tokio::test]
    async fn a_slot_wait_that_runs_out_gives_none_and_a_free_slot_is_taken() {
        let slots = std::sync::Arc::new(tokio::sync::Semaphore::new(1));
        let first = acquire_slot(&slots, Duration::from_millis(50)).await;
        assert!(first.is_some());
        let started = std::time::Instant::now();
        assert!(
            acquire_slot(&slots, Duration::from_millis(100))
                .await
                .is_none()
        );
        assert!(started.elapsed() >= Duration::from_millis(100));
        drop(first);
        assert!(
            acquire_slot(&slots, Duration::from_millis(50))
                .await
                .is_some()
        );
    }

    #[test]
    fn paper_and_orientation_parse_only_their_two_words() {
        assert_eq!(Paper::parse("a4"), Some(Paper::A4));
        assert_eq!(Paper::parse("letter"), Some(Paper::Letter));
        assert_eq!(Paper::parse("A4"), None);
        assert_eq!(
            Orientation::parse("landscape"),
            Some(Orientation::Landscape)
        );
        assert_eq!(Orientation::parse("sideways"), None);
    }
}
