//! `PUT /api/settings/general/images/{kind}`: the answer must reach a client
//! that is still sending the body (`ADM-2`, plan adm-120).
//!
//! What this proves:
//!
//! * an oversized logo or icon put over a REAL socket answers the API's own
//!   JSON error — the size sentence of the kind — and the client can finish
//!   sending the body and read the answer on the same connection;
//! * the same put without a session answers the 401 JSON, not a reset: the
//!   drain sits outside the auth gate, so the gate's own early answer is
//!   drained too;
//! * a body over the drain's cap is refused with the size sentence, and the
//!   API reads no more than the cap plus one frame;
//! * the drain touches that one route and nothing else: another route's
//!   body is not read at all, another method on the same path is not read
//!   either;
//! * a kind outside the list is drained all the same and keeps its own 404:
//!   it has no size sentence, so a body of its own over the cap keeps the
//!   404 too, never a 413;
//! * an upload the handler already stored and audited keeps its own 2xx
//!   when the drain gives up on what follows its form (`PR #638 review
//!   2`): a stored upload is never converted into a 413 — pinned by the
//!   unit tests on `answer_at_cap`, because through this router the
//!   route's own 1 MiB body limit always answers such a body first, so
//!   what runs end to end here is the form the trailing data refused.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use axum::body::{Body, Bytes, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use common::{create_principal_with_permissions, session_cookie_for_user};
use futures::stream::unfold;
use sqlx::PgPool;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::ServiceExt;

const PNG_HEAD: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
const BOUNDARY: &str = "PICTUREBOUNDARY";

/// The sentences of the feature page, written out here and not imported, so
/// a change to the server's text fails the test.
const LOGO_SIZE: &str = "The logo must be at most 512 KB.";
const ICON_RULE: &str = "The icon must be a PNG image of at most 64 KB.";

/// The cap the drain reads up to, 16 MiB. Written out here rather than read
/// from the server's constant, so a change to the cap fails this test.
const DRAIN_CAP: u64 = 16 * 1024 * 1024;

/// A frame size that does not divide the cap, so the frame the cap does not
/// cover shows up in the count of what the API read.
const ODD_FRAME: usize = 100 * 1024;

/// A PNG-headed buffer of `total` bytes.
fn png_bytes(total: usize) -> Vec<u8> {
    let mut bytes = PNG_HEAD.to_vec();
    bytes.resize(total, 0x41);
    bytes
}

/// A one-part form whose `file` part holds `bytes`.
///
/// The closing boundary matches the one the `Content-Type` declares: the
/// old closing line was the literal `--BOUNDARY--`, which never matched
/// the declared `PICTUREBOUNDARY`, so no form parsed to its end — no
/// earlier test needed it to, since every one of them is answered before
/// the parser reaches the form's end (`PR #638 review 2`'s test is the
/// first that must read the `file` part and store it).
fn picture_form(bytes: &[u8]) -> Vec<u8> {
    let mut body = format!(
        "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"logo.png\"\r\nContent-Type: image/png\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());
    body
}

/// A request body that counts the bytes the API takes from it. The count is
/// what tells "the drain read the body" from "the API never touched it".
fn counted_body(total: usize, chunk: usize, taken: Arc<AtomicU64>) -> Body {
    let stream = unfold((0_usize, taken), move |(sent, taken)| async move {
        if sent >= total {
            return None;
        }
        let len = chunk.min(total - sent);
        taken.fetch_add(len as u64, Ordering::Relaxed);
        Some((
            Ok::<Bytes, std::convert::Infallible>(Bytes::from(vec![0x41_u8; len])),
            (sent + len, taken),
        ))
    });
    Body::from_stream(stream)
}

/// A body that is the form's own bytes and then `junk` bytes that are not
/// the form's: the multipart reader stops at the form's closing boundary,
/// so only the drain reads the junk. Counted, like [`counted_body`], so
/// the test can say where the API stopped reading.
fn form_then_junk(form: Vec<u8>, junk: usize, taken: Arc<AtomicU64>) -> Body {
    let stream = unfold(
        (Some(form), 0_usize, taken),
        move |(form, sent, taken)| async move {
            if let Some(chunk) = form {
                taken.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                return Some((
                    Ok::<Bytes, std::convert::Infallible>(chunk.into()),
                    (None, sent, taken),
                ));
            }
            if sent >= junk {
                return None;
            }
            let len = (64 * 1024).min(junk - sent);
            taken.fetch_add(len as u64, Ordering::Relaxed);
            Some((
                Ok::<Bytes, std::convert::Infallible>(Bytes::from(vec![0x41_u8; len])),
                (None, sent + len, taken),
            ))
        },
    );
    Body::from_stream(stream)
}

/// A session cookie for a principal that may change the workspace settings.
async fn admin_cookie(pool: &PgPool) -> String {
    let id = create_principal_with_permissions(pool, "settings:write").await;
    session_cookie_for_user(pool, id).await
}

/// Serves the router on a loopback socket and returns its address.
async fn serve(router: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    address
}

/// Where the head of an HTTP/1.1 answer ends.
fn head_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

/// How long the client waits for the peer. A peer that stops reading shows
/// up as a failed write; a peer that says nothing shows up as this timeout.
/// Neither may hang the test.
const PEER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// A client that speaks HTTP/1.1 over a real socket.
///
/// The point of the socket is the one `tower::ServiceExt::oneshot` cannot
/// show: a request the server answers before it has read the body is seen
/// here as the peer sees it — a closed connection, not an answer.
struct RawClient {
    stream: TcpStream,
}

impl RawClient {
    async fn connect(address: SocketAddr) -> Self {
        let stream = TcpStream::connect(address).await.unwrap();
        Self { stream }
    }

    /// Writes one request. The error is the peer's: `EPIPE` means the server
    /// closed the connection while the body was still going out.
    async fn write(
        &mut self,
        method: &str,
        path: &str,
        cookie: Option<&str>,
        body: &[u8],
    ) -> std::io::Result<()> {
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: multipart/form-data; boundary={BOUNDARY}\r\nContent-Length: {}\r\n",
            body.len()
        );
        if let Some(cookie) = cookie {
            head.push_str("Cookie: ");
            head.push_str(cookie);
            head.push_str("\r\n");
        }
        head.push_str("\r\n");
        self.stream.write_all(head.as_bytes()).await?;
        // The body write is the one that can block: without the drain the
        // server stops reading and closes the connection, and the write must
        // fail (or time out) rather than hang the test.
        match tokio::time::timeout(PEER_TIMEOUT, self.stream.write_all(body)).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the peer stopped reading the body",
                ));
            }
        }
        self.stream.flush().await
    }

    /// Reads what the peer sent. A closed connection fails with its own
    /// message: that is what a reset looks like from this side.
    async fn read_some(&mut self) -> Vec<u8> {
        let mut chunk = [0_u8; 8192];
        let read = tokio::time::timeout(PEER_TIMEOUT, self.stream.read(&mut chunk))
            .await
            .expect("the peer answers within the timeout")
            .expect("the socket is readable");
        assert!(
            read > 0,
            "the server closed the connection before its answer"
        );
        chunk[..read].to_vec()
    }

    /// Reads one answer: its status line, its headers and its body.
    async fn read_answer(&mut self) -> (StatusCode, String) {
        let mut buffer = Vec::new();
        let head_end = loop {
            if let Some(at) = head_end(&buffer) {
                break at;
            }
            buffer.extend_from_slice(&self.read_some().await);
        };
        let head = String::from_utf8_lossy(&buffer[..head_end]).to_string();
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .expect("a status line");
        let length = head
            .lines()
            .skip(1)
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.trim()
                    .eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().unwrap_or(0))
            })
            .unwrap_or(0);
        let mut body = buffer[head_end + 4..].to_vec();
        while body.len() < length {
            body.extend_from_slice(&self.read_some().await);
        }
        (
            StatusCode::from_u16(status).unwrap(),
            String::from_utf8_lossy(&body).to_string(),
        )
    }
}

/// The size rule broken by a logo that is too big is the size rule the
/// person is told, over a real socket, and the connection survives it.
///
/// The 2.4 MB body is over the route's 1 MiB body limit, so the handler
/// refuses it with the size sentence — but the answer must arrive on a
/// connection the client can still use, which is what the drain is for.
#[tokio::test]
async fn an_oversized_logo_put_answers_the_size_sentence_over_a_real_socket() {
    let app = common::spin_up().await;
    let cookie = admin_cookie(&app.pool).await;
    let address = serve(app.router.clone()).await;
    let mut client = RawClient::connect(address).await;

    let body = picture_form(&png_bytes(2_400_000));
    client
        .write(
            "PUT",
            "/api/settings/general/images/logo",
            Some(&cookie),
            &body,
        )
        .await
        .expect("the server must read the whole body before it answers");

    let (status, text) = client.read_answer().await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains(LOGO_SIZE), "{text}");

    // The connection is still good: the answer did not end it.
    client
        .write("GET", "/health", None, b"")
        .await
        .expect("the connection stays usable");
    let (status, _) = client.read_answer().await;
    assert_eq!(status, StatusCode::OK);
}

/// The same put without a session: the gate's own early answer is drained
/// too, so the caller reads the 401 JSON instead of a reset.
#[tokio::test]
async fn an_unauthenticated_oversized_logo_put_answers_the_unauthorized_json_over_a_real_socket() {
    let app = common::spin_up().await;
    let address = serve(app.router.clone()).await;
    let mut client = RawClient::connect(address).await;

    let body = picture_form(&png_bytes(2_400_000));
    client
        .write("PUT", "/api/settings/general/images/logo", None, &body)
        .await
        .expect("the server must read the whole body before it answers");

    let (status, text) = client.read_answer().await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(text.contains("unauthorized"), "{text}");

    client
        .write("GET", "/health", None, b"")
        .await
        .expect("the connection stays usable");
    let (status, _) = client.read_answer().await;
    assert_eq!(status, StatusCode::OK);
}

/// The icon has its own rule and its own sentence.
#[tokio::test]
async fn an_oversized_icon_put_answers_the_icon_sentence_over_a_real_socket() {
    let app = common::spin_up().await;
    let cookie = admin_cookie(&app.pool).await;
    let address = serve(app.router.clone()).await;
    let mut client = RawClient::connect(address).await;

    let body = picture_form(&png_bytes(2_400_000));
    client
        .write(
            "PUT",
            "/api/settings/general/images/icon",
            Some(&cookie),
            &body,
        )
        .await
        .expect("the server must read the whole body before it answers");

    let (status, text) = client.read_answer().await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(text.contains(ICON_RULE), "{text}");

    // The connection is still good: the answer did not end it.
    client
        .write("GET", "/health", None, b"")
        .await
        .expect("the connection stays usable");
    let (status, _) = client.read_answer().await;
    assert_eq!(status, StatusCode::OK);
}

/// A body over the cap is refused with the size sentence, and the API reads
/// no more than the cap.
///
/// This one runs in process: over a socket the peer may see the connection
/// go away before it reads the answer, because the server stopped reading
/// the body on purpose. What is deterministic — and what the cap exists for
/// — is how much the API read: with frames that divide the cap, exactly the
/// cap, not one byte more.
#[tokio::test]
async fn a_body_over_the_cap_is_refused_with_the_size_sentence_and_the_api_reads_no_more_than_the_cap()
 {
    let app = common::spin_up().await;
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/logo")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(counted_body(20 * 1024 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let text = String::from_utf8_lossy(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
        .to_string();
    assert!(text.contains(LOGO_SIZE), "{text}");
    assert_eq!(
        taken.load(Ordering::Relaxed),
        DRAIN_CAP,
        "the API must stop reading at the cap"
    );
}

/// The bound the cap really is: the drain reads a frame and only then looks
/// at the counter, so the API reads at most the cap plus one frame. A frame
/// size that does not divide the cap shows the frame the cap does not
/// cover: the API reads past the cap, but never past the cap plus a frame.
#[tokio::test]
async fn a_body_over_the_cap_is_read_up_to_the_cap_plus_one_frame() {
    let app = common::spin_up().await;
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/logo")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(counted_body(20 * 1024 * 1024, ODD_FRAME, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let text = String::from_utf8_lossy(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
        .to_string();
    assert!(text.contains(LOGO_SIZE), "{text}");
    let read = taken.load(Ordering::Relaxed);
    assert!(
        read > DRAIN_CAP,
        "the API read {read} bytes, not past the cap"
    );
    assert!(
        read <= DRAIN_CAP + ODD_FRAME as u64,
        "the API read {read} bytes, past the cap plus one frame"
    );
}

/// The drain acts on the picture route and on nothing else.
///
/// The count is the proof: a body the API never reads stays at zero, and the
/// picture route's body is read to its end.
#[tokio::test]
async fn the_drain_reads_the_picture_route_and_no_other() {
    let app = common::spin_up().await;

    // Another route: the body is not read at all.
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(counted_body(1024 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        taken.load(Ordering::Relaxed),
        0,
        "another route's body must not be read"
    );

    // The same path, another method: the drain is for the `PUT` alone.
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::DELETE)
        .uri("/api/settings/general/images/logo")
        .body(counted_body(1024 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        taken.load(Ordering::Relaxed),
        0,
        "another method's body must not be read"
    );

    // The picture route: what the gate left behind is read and thrown away.
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/logo")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(counted_body(64 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        taken.load(Ordering::Relaxed),
        64 * 1024,
        "the drain must read what the gate left behind"
    );
}

/// The name route the Auditor named: `PUT /api/settings/general` is not the
/// picture route, so the drain does not touch it. The gate answers its own
/// `401` and the API never reads the body at all — the count stays at zero.
#[tokio::test]
async fn the_general_settings_put_is_not_drained_by_the_picture_drain() {
    let app = common::spin_up().await;
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general")
        .body(counted_body(1024 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        taken.load(Ordering::Relaxed),
        0,
        "the name route's body must not be read"
    );
}

/// A kind outside the list has no size sentence, so the drain cannot refuse
/// it with one: the route's own 404 stands, and the body is read all the
/// same — a client that is still sending it must be able to finish.
#[tokio::test]
async fn a_kind_outside_the_list_is_drained_and_keeps_its_own_404() {
    let app = common::spin_up().await;
    let cookie = admin_cookie(&app.pool).await;
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/banner")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("cookie", cookie)
        .body(counted_body(1024 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        taken.load(Ordering::Relaxed),
        1024 * 1024,
        "the drain must read the body of a kind outside the list too"
    );
}

/// A body over the cap for a kind outside the list: the cap stops the
/// reading, but there is no size sentence to refuse the body with, so the
/// route's own 404 stands — never a 413.
#[tokio::test]
async fn a_kind_outside_the_list_over_the_cap_keeps_its_own_404() {
    let app = common::spin_up().await;
    let cookie = admin_cookie(&app.pool).await;
    let taken = Arc::new(AtomicU64::new(0));
    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/banner")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("cookie", cookie)
        .body(counted_body(20 * 1024 * 1024, 64 * 1024, taken.clone()))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        taken.load(Ordering::Relaxed),
        DRAIN_CAP,
        "the API must stop reading at the cap"
    );
}

/// A valid upload on the drained route: the form parses, the picture is
/// stored, the audit row is written and the answer is the handler's own
/// 200. The drain keeps it: the body is small, so the drain finds nothing
/// over the cap in it.
///
/// This is also the first test that reads a [`picture_form`] to its end,
/// which is why the form's closing boundary had to be fixed to match the
/// one the `Content-Type` declares (the old literal `--BOUNDARY--` never
/// matched the declared `PICTUREBOUNDARY`; no earlier test parsed past the
/// refusal it was sent to assert, so it never showed).
#[tokio::test]
async fn a_valid_upload_on_the_drained_route_is_stored_and_answers_200() {
    let app = common::spin_up().await;
    let cookie = admin_cookie(&app.pool).await;
    let png = png_bytes(64);

    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/logo")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("cookie", cookie)
        .body(Body::from(picture_form(&png)))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // The picture really was stored: the 200 is about a save that happened.
    let stored = lakehouse_store::workspace_settings::get(&app.pool)
        .await
        .unwrap()
        .expect("the settings row exists");
    let logo = stored.logo.expect("the logo was saved");
    assert_eq!(&*logo.bytes, &png[..]);
    assert_eq!(logo.content_type, "image/png");
}

/// PR #638 review 2's shape — a valid small form followed by over-cap
/// trailing data — pinned to the answer it can actually reach today.
///
/// A 2xx answer whose leftover body then crosses the drain's cap cannot be
/// driven through this router: the multipart parser drains every byte the
/// shared body hands it until the route's own body limit (1 MiB,
/// `PICTURE_BODY_LIMIT_BYTES`, far below the drain's 16 MiB cap) refuses
/// the rest, so for this body the inner answer is the 422 with the size
/// sentence, the picture is never stored, and the drain's cap answers the
/// 413 over it. The success-stands half of the cap decision — the store
/// happened, so the 2xx must survive the cap — is pinned where it lives,
/// by the unit tests on `answer_at_cap` in `routes::picture_drain`
/// (`PR #638 review 2`): the day a route change lets a stored upload meet
/// the cap (a raised body limit, an answer that reads less), the decision
/// is already the right one.
///
/// What this test also pins: the drain still reads the trailing data to
/// its cap and no further, and the form at the head of the body is not
/// kept alive by the refusal.
#[tokio::test]
async fn a_valid_form_with_over_cap_trailing_data_is_refused_and_the_reading_stops_at_the_cap() {
    let app = common::spin_up().await;
    let cookie = admin_cookie(&app.pool).await;
    let taken = Arc::new(AtomicU64::new(0));
    let png = png_bytes(64);

    let request = Request::builder()
        .method(Method::PUT)
        .uri("/api/settings/general/images/logo")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .header("cookie", cookie)
        .body(form_then_junk(
            picture_form(&png),
            20 * 1024 * 1024,
            taken.clone(),
        ))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let text = String::from_utf8_lossy(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
        .to_string();
    assert!(text.contains(LOGO_SIZE), "{text}");

    // Nothing was stored: the refusal the caller can act on is honest.
    assert!(
        lakehouse_store::workspace_settings::get(&app.pool)
            .await
            .unwrap()
            .and_then(|row| row.logo)
            .is_none(),
        "an upload whose body the route's own limit refused must not be stored"
    );

    // The drain still stopped at the cap: the trailing data was never read
    // to its end.
    let read = taken.load(Ordering::Relaxed);
    assert!(read > DRAIN_CAP, "the drain did reach the cap: {read}");
    assert!(
        read <= DRAIN_CAP + 64 * 1024,
        "the drain read past the cap plus one junk frame: {read}"
    );
}
