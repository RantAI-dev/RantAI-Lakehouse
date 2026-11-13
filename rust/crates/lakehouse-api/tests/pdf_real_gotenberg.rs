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

/// `BI-19` part D (plan D13): a chart image against the real container. The
/// bytes start with the PNG signature, and the picture is the box the selector
/// names at the asked width and scale (1200 CSS px at 2x is 2400 px across).
#[tokio::test]
#[ignore = "needs a Gotenberg container, see the module notes"]
async fn render_png_against_real_gotenberg() {
    const CHART: &str = "<!doctype html><html><body style=\"margin:0\">\
<div data-render-chart style=\"width:1200px;height:300px;background:#369\"><h1>Chart image proof</h1></div>\
<script>setTimeout(function () { window.__lakehouseRenderReady = true; }, 300);</script></body></html>";
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
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{CHART}",
                CHART.len()
            );
        }
    });
    let png = lakehouse_api::pdf::screenshot_url(
        &gotenberg,
        &format!("http://{host}:{port}/"),
        Duration::from_secs(60),
    )
    .await
    .expect("Gotenberg made a PNG");
    assert!(
        png.starts_with(b"\x89PNG\r\n\x1a\n"),
        "not a PNG: {:?}",
        &png[..png.len().min(16)]
    );
    // IHDR: width and height are the big-endian u32s at bytes 16..24.
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!(width, 2400, "1200 CSS px at a scale of 2");
    assert_eq!(height, 600, "the box is 300 CSS px tall");
}

/// `BI-53` (QA `LH-BI-1-01` F1, `LH-BI-6-22`, `LH-BI-6-23`): what the renderer
/// itself says about its window, its media and how long a conversion takes.
///
/// The page reports `window.innerWidth`, the width of a full-width box and
/// `matchMedia("print").matches` at load and again 1.5 s later, so the two
/// moments a chart could be created in are both seen. The test asserts that
/// each conversion returned a PDF and that both reports arrived; it asserts
/// **nothing** about the numbers, because reading them is the purpose: they go
/// to `docs/plans/BI-53-RESULT.md`, and T12 of the plan is gated on them.
#[tokio::test]
#[ignore = "needs a Gotenberg container, see the module notes"]
async fn the_renderer_reports_its_window_its_media_and_how_long_a_conversion_takes() {
    use std::sync::{Arc, Mutex};

    const PROBE_PAGE: &str = "<!doctype html><html><body style=\"margin:0\"><div id=\"box\" style=\"width:100%;height:10px\"></div><script>
function report(stage) {
  var box = document.getElementById('box').getBoundingClientRect().width;
  return fetch('/probe?stage=' + stage + '&w=' + window.innerWidth + '&box=' + box + '&print=' + window.matchMedia('print').matches);
}
report('start').then(function () { setTimeout(function () { report('later').then(function () { window.__lakehouseRenderReady = true; }); }, 1500); });
</script></body></html>";

    let gotenberg = std::env::var("GOTENBERG_TEST_URL").expect("GOTENBERG_TEST_URL");
    let host = std::env::var("GOTENBERG_TEST_PAGE_HOST")
        .unwrap_or_else(|_| "host.docker.internal".to_owned());
    let listener = TcpListener::bind("0.0.0.0:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let seen = Arc::clone(&lines);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let head = String::from_utf8_lossy(&request[..read]).into_owned();
            let first = head.lines().next().unwrap_or("").to_owned();
            if let Ok(mut all) = seen.lock() {
                all.push(first.clone());
            }
            if first.contains("/probe?") {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            } else {
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{PROBE_PAGE}",
                    PROBE_PAGE.len()
                );
            }
        }
    });

    for (orientation, label) in [
        (Orientation::Landscape, "landscape"),
        (Orientation::Portrait, "portrait"),
    ] {
        let before = lines.lock().unwrap().len();
        let started = Instant::now();
        let pdf = convert_url(
            &gotenberg,
            &format!("http://{host}:{port}/probe"),
            (Paper::A4, orientation),
            Duration::from_secs(60),
        )
        .await
        .expect("Gotenberg made a PDF");
        let elapsed = started.elapsed();
        assert!(
            pdf.starts_with(b"%PDF"),
            "not a PDF: {:?}",
            &pdf[..pdf.len().min(16)]
        );
        let probes: Vec<String> = lines.lock().unwrap()[before..]
            .iter()
            .filter(|line| line.contains("/probe?"))
            .cloned()
            .collect();
        println!(
            "A4 {label}: {} ms for the conversion, {} reports",
            elapsed.as_millis(),
            probes.len()
        );
        for probe in &probes {
            println!("  {probe}");
        }
        assert_eq!(probes.len(), 2, "two reports for {label}: {probes:?}");
    }
}
