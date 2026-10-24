//! `BI-1` T4, AGENTS.md rule 8: the PDF client against a real Gotenberg
//! container, not a mock. Ignored by default (it needs Docker); run it as
//! described in the plan's handoff:
//!
//! ```text
//! docker run -d --rm --name bi1-gotenberg-scratch -p 127.0.0.1:13331:3000 \
//!   --add-host=host.docker.internal:host-gateway gotenberg/gotenberg:8.37.0 \
//!   gotenberg --api-timeout=70s --log-level=warn
//! GOTENBERG_TEST_URL=http://127.0.0.1:13331 cargo test -p lakehouse-api \
//!   --test pdf_real_gotenberg -- --ignored
//! docker stop bi1-gotenberg-scratch
//! ```
//!
//! The page the container prints is served by this test on a free local port
//! and sets `window.__lakehouseRenderReady` after a short script, so the test
//! also proves that `waitForExpression` is honoured: a PDF made before the
//! flag was set would still be a PDF, but Gotenberg would have returned at
//! once; the page here sets it after 500 ms and the test checks the call took
//! at least that long.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use lakehouse_api::pdf::{Orientation, Paper, convert_url};

const PAGE: &str = "<!doctype html><html><head><title>BI-1 proof</title></head><body>\
<h1>Print view proof</h1><script>\
setTimeout(function () { window.__lakehouseRenderReady = true; }, 500);\
</script></body></html>";

#[tokio::test]
#[ignore = "needs a Gotenberg container, see the module notes"]
async fn render_pdf_against_real_gotenberg() {
    let gotenberg = std::env::var("GOTENBERG_TEST_URL")
        .expect("GOTENBERG_TEST_URL is the base URL of the scratch Gotenberg");
    let host = std::env::var("GOTENBERG_TEST_PAGE_HOST")
        .unwrap_or_else(|_| "host.docker.internal".to_owned());

    let listener = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PAGE}",
                PAGE.len()
            );
        }
    });

    let started = Instant::now();
    let pdf = convert_url(
        &gotenberg,
        &format!("http://{host}:{port}/"),
        (Paper::A4, Orientation::Landscape),
        Duration::from_secs(60),
    )
    .await
    .expect("Gotenberg made a PDF");
    assert!(
        pdf.starts_with(b"%PDF"),
        "not a PDF: {:?}",
        &pdf[..pdf.len().min(16)]
    );
    assert!(pdf.len() > 1_000, "{} bytes", pdf.len());
    assert!(
        started.elapsed() >= Duration::from_millis(500),
        "Gotenberg did not wait for the ready flag: {:?}",
        started.elapsed()
    );
}

/// `BI-1` review fix (SHOULD-FIX) R3: a print view that raises an uncaught
/// exception fails the conversion (`failOnConsoleExceptions`), so the route does
/// not send a PDF of an error sentence.
#[tokio::test]
#[ignore = "needs a Gotenberg container, see the module notes"]
async fn a_page_that_raises_an_exception_fails_the_conversion_on_the_real_gotenberg() {
    const BROKEN: &str = "<!doctype html><html><body><p>This dashboard could not be loaded.</p>\
<script>window.__lakehouseRenderReady = true; setTimeout(function () { throw new Error('render data refused'); }, 0);</script>\
</body></html>";
    let gotenberg = std::env::var("GOTENBERG_TEST_URL").expect("GOTENBERG_TEST_URL");
    let host = std::env::var("GOTENBERG_TEST_PAGE_HOST")
        .unwrap_or_else(|_| "host.docker.internal".to_owned());
    let listener = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{BROKEN}",
                BROKEN.len()
            );
        }
    });
    let result = convert_url(
        &gotenberg,
        &format!("http://{host}:{port}/"),
        (Paper::A4, Orientation::Portrait),
        Duration::from_secs(60),
    )
    .await;
    assert!(
        matches!(result, Err(lakehouse_api::pdf::PdfError::Refused(_))),
        "an exception in the page must fail the conversion: {result:?}"
    );
}
