//! The picture route's drain: an early answer must not leave the client
//! writing into a closed connection (`ADM-2`, plan adm-120).
//!
//! # What broke
//!
//! `PUT /api/settings/general/images/{kind}` can know its answer before it
//! has read the request body: the auth gate's `401`, the route's own `404`
//! for a kind outside the list, and the handler's `422` after
//! `DefaultBodyLimit` refused the body are all answers that arrive while the
//! client is still sending. Hyper closes a connection whose request body was
//! dropped before its end, so the peer — the console's Next.js `/api` proxy
//! — is still writing when the API goes away: it logs `write EPIPE` and
//! answers its own plain-text `500 Internal Server Error` instead of the
//! API's JSON. A person uploading a 2.4 MB logo saw a server error, not the
//! sentence that says the logo must be at most 512 KB.
//!
//! # What this layer does
//!
//! On that one route, and only for a `PUT`, the layer reads what is left of
//! the request body after the inner service answered and throws it away —
//! never buffered, only counted — so the client finishes its upload and
//! reads the answer on a connection that is still good. The kind of the
//! picture does not decide whether the layer acts: a kind outside the list
//! is drained all the same, and its own `404` stands. The answer itself
//! does not change: the `422` with the size sentence, the `401`, the `404`,
//! all as they were.
//!
//! The layer sits OUTSIDE the auth gate on purpose: the gate's `401` is one
//! of the early answers, and an unauthenticated caller must get it the same
//! way an authenticated one does — a JSON error on a connection that is
//! still usable, not a reset. It sits inside the request timeout, so the
//! time it spends draining counts against the request's deadline.
//!
//! # Why the cap is hard, for every caller
//!
//! Draining a body is work the API does for a caller it may have just
//! refused, so the drain stops at [`PICTURE_DRAIN_MAX_BYTES`] (16 MiB) and
//! answers `413` with the size sentence of the kind. A kind outside the
//! list has no size sentence, so a body of its own over the cap keeps the
//! inner answer — the `404`, never a `413`. The cap is far above the 512 KB
//! logo rule and the 64 KB icon rule — every picture the feature accepts is
//! well inside it — and far below a size that could pin memory. The
//! amplification it allows is therefore small and bounded, and it is
//! accepted on purpose: an unauthenticated caller can make the API read up
//! to 16 MiB per request. The alternative — refusing without reading — is
//! what produced the reset the proxy turned into a server error.
//!
//! # What the cap does not promise
//!
//! The drain reads a frame and only then looks at the counter, so the API
//! reads at most the cap plus one frame: the bytes of the frame that
//! crossed the cap are already in memory when the cap is noticed. A frame
//! is the transport's unit, not ours — hyper decides how large a frame is —
//! so its size is not ours to fix. What the cap does promise is a bound
//! that holds: the drain never reads an unbounded body and never buffers
//! one.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use axum::body::{Body, Bytes, HttpBody};
use axum::extract::{MatchedPath, Request};
use axum::http::{Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use futures::Stream;
use futures::stream::unfold;
use lakehouse_store::workspace_settings::ImageKind;

use crate::json::ApiJson;
use crate::routes::FixedErrorBody;
use crate::routes::settings::{picture_kind, picture_size_sentence};

/// The route the drain acts on, as the pattern the router registers it
/// under. One constant: the route registration and this layer's own check
/// read it, so the two cannot drift apart.
pub(crate) const PICTURE_ROUTE_PATTERN: &str = "/api/settings/general/images/{kind}";

/// The cap the drain reads up to, 16 MiB. The API reads at most the cap
/// plus one frame: it reads a frame and only then looks at the counter, so
/// the bytes of the frame that crossed the cap are already in memory when
/// the cap is noticed. A frame is the transport's unit, not ours, so its
/// size is not ours to fix.
///
/// Why it exists: the drain reads a body the API has already answered, so
/// without a bound an unauthenticated caller could make the API read an
/// unbounded body. Why 16 MiB: far above the 512 KB logo rule and the 64 KB
/// icon rule (every picture the feature accepts is well inside it) and far
/// below a size that could pin memory. The cap is hard for every caller,
/// signed in or not (the Auditor's ruling, 2026-11-19); the amplification it
/// allows — up to 16 MiB plus one frame read per request — is small and
/// bounded and is the accepted cost of answering instead of resetting.
pub(crate) const PICTURE_DRAIN_MAX_BYTES: usize = 16 * 1024 * 1024;

/// The request body, shared between the inner service and the drain.
///
/// The slot holds the one owner of the body. The inner service reads through
/// a stream that pulls from the slot; when it drops its stream, the body is
/// still in the slot, so hyper's connection stays open for the drain. The
/// counter counts every byte handed out, so the cap bounds what the API read
/// in total, up to one frame — the handler's reads included, the drain's
/// included.
#[derive(Clone)]
struct SharedBody {
    slot: Arc<Mutex<Option<Body>>>,
    seen: Arc<AtomicU64>,
}

impl SharedBody {
    /// Shares `body` with an empty counter.
    fn shared(body: Body) -> Self {
        Self {
            slot: Arc::new(Mutex::new(Some(body))),
            seen: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The next chunk of the body, counted. `None` when the body has ended.
    ///
    /// The body is taken out of the slot for the read and put back after it,
    /// so no lock is held across the read. Another reader between the two
    /// (the drain taking the body over) leaves the slot empty and the body
    /// with the drain, never dropped.
    async fn next_chunk(&self) -> Option<Result<Bytes, axum::Error>> {
        let mut body = {
            let mut guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            guard.take()?
        };
        let chunk = next_data(Pin::new(&mut body)).await;
        {
            let mut guard = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
            if guard.is_none() {
                *guard = Some(body);
            }
        }
        if let Some(Ok(bytes)) = &chunk {
            self.seen.fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
        chunk
    }
}

/// The chunks of a shared body as a stream, so the inner service can read
/// the body as a body again.
fn shared_chunks(
    shared: SharedBody,
) -> impl Stream<Item = Result<Bytes, axum::BoxError>> + Send + 'static {
    unfold(shared, |shared| async move {
        match shared.next_chunk().await {
            None => None,
            Some(Ok(bytes)) => Some((Ok(bytes), shared)),
            Some(Err(err)) => Some((Err(err.into_inner()), shared)),
        }
    })
}

/// Takes the body out of a shared slot and leaves the slot empty: the
/// caller now owns the body and the inner service's stream ends.
fn take_body(shared: &SharedBody) -> Option<Body> {
    let mut guard = shared.slot.lock().unwrap_or_else(PoisonError::into_inner);
    guard.take()
}

/// The next chunk of `body`, as bytes. A frame that is not a data frame
/// (a trailer) contributes nothing, and is never buffered.
async fn next_data(mut body: Pin<&mut Body>) -> Option<Result<Bytes, axum::Error>> {
    std::future::poll_fn(|cx| body.as_mut().poll_frame(cx))
        .await
        .map(|result| result.map(|frame| frame.data_ref().cloned().unwrap_or_default()))
}

/// Whether `body` has ended: its size hint says nothing is left. Only a body
/// whose length its sender declared can say this exactly; a body whose end
/// cannot be told reads as not ended.
fn body_ended(body: &Body) -> bool {
    body.size_hint().upper() == Some(0)
}

/// The answer for a body over the cap: the size sentence of the kind, in the
/// same JSON envelope every other error uses. `ApiError` has no 413 variant,
/// so the response is built here rather than mapped through it.
fn over_cap(kind: ImageKind) -> Response {
    (
        StatusCode::PAYLOAD_TOO_LARGE,
        ApiJson(FixedErrorBody {
            error: picture_size_sentence(kind).to_owned(),
        }),
    )
        .into_response()
}

/// Whether this request is one the drain acts on: a `PUT` on the picture
/// route, whatever kind of picture it names. Every other request is left
/// alone.
fn is_picture_put(req: &Request) -> bool {
    if req.method() != Method::PUT {
        return false;
    }
    req.extensions()
        .get::<MatchedPath>()
        .is_some_and(|matched| matched.as_str() == PICTURE_ROUTE_PATTERN)
}

/// The kind of the picture this request is about, when the path names one in
/// the list: `logo`, `logo-dark` or `icon`.
///
/// `None` for a kind outside the list: the route answers its own `404` for
/// such a kind, and there is no size sentence to give.
fn picture_kind_of(req: &Request) -> Option<ImageKind> {
    let kind = req.uri().path().rsplit('/').next()?;
    picture_kind(kind).ok()
}

/// The drain: reads what is left of the request body after the inner
/// service answered, up to [`PICTURE_DRAIN_MAX_BYTES`] plus one frame, and
/// answers `413` when the body turns out to be bigger than that. A kind
/// outside the list has no size sentence, so its own answer stands. See the
/// module doc comment for what broke and why the layer sits where it sits.
pub(crate) async fn drain(req: Request, next: Next) -> Response {
    if !is_picture_put(&req) {
        return next.run(req).await;
    }
    // The kind is resolved apart from the scope check: a kind outside the
    // list is drained all the same, and its own `404` stands.
    let kind = picture_kind_of(&req);

    let (parts, body) = req.into_parts();
    let shared = SharedBody::shared(body);
    let answer = next
        .run(Request::from_parts(
            parts,
            Body::from_stream(shared_chunks(shared.clone())),
        ))
        .await;

    // The inner service answered. Whatever is left of the body is read and
    // thrown away, so the client can finish sending it and read the answer
    // on a connection that is still good.
    let Some(mut body) = take_body(&shared) else {
        return answer;
    };
    let mut seen = shared.seen.load(Ordering::Relaxed);
    loop {
        if seen >= PICTURE_DRAIN_MAX_BYTES as u64 {
            // The cap is reached. The body may have ended exactly on it —
            // the size hint says so when the sender declared its length —
            // and that body is not over the cap.
            return if body_ended(&body) {
                answer
            } else {
                // A kind outside the list has no size sentence, so the
                // inner answer stands: the `404`, never a `413`.
                kind.map_or_else(|| answer, over_cap)
            };
        }
        match next_data(Pin::new(&mut body)).await {
            // A body that ended, and a body that cannot be read to its end,
            // cannot make the answer better or worse: the inner answer
            // stands.
            None | Some(Err(_)) => return answer,
            Some(Ok(bytes)) => seen += bytes.len() as u64,
        }
    }
}
