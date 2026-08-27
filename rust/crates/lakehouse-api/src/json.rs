//! [`ApiJson`] — the only JSON responder allowed in route handlers.
//!
//! `axum::Json<T>` sets `content-type: application/json` (no charset). Every
//! one of the 72 JSON entries in the golden parity corpus
//! (`rust/tests/parity/corpus/*.json`) records
//! `content-type: application/json;charset=utf-8` — that is what Next.js's
//! `NextResponse.json` actually sends, and it is the captured contract this
//! port is held to. `axum::Json` therefore must never be used directly in a
//! handler; wrap the body in [`ApiJson`] instead, everywhere, including
//! error paths.

use axum::http::header::{CONTENT_TYPE, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The exact `content-type` value recorded across the parity corpus, byte
/// for byte (no space before `charset`, matching `NextResponse.json`).
const CONTENT_TYPE_VALUE: &str = "application/json;charset=utf-8";

/// A JSON responder that serializes `T` like `axum::Json` but always emits
/// `content-type: application/json;charset=utf-8`, matching Next.js's
/// `NextResponse.json` byte for byte.
///
/// Use this in place of `axum::Json` in every route handler and in
/// [`crate::error::ApiRejection`]'s `IntoResponse` impl. Do not use bare
/// `axum::Json` anywhere in this crate's handlers — it would silently drop
/// the `;charset=utf-8` suffix that the parity corpus requires.
#[derive(Debug, Clone, Copy)]
pub struct ApiJson<T>(pub T);

impl<T: Serialize> IntoResponse for ApiJson<T> {
    fn into_response(self) -> Response {
        let mut response = axum::Json(self.0).into_response();
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_VALUE));
        response
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::body::to_bytes;
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn sets_exact_content_type_header() {
        let resp = ApiJson(json!({"ok": true})).into_response();
        assert_eq!(
            resp.headers().get(CONTENT_TYPE).unwrap(),
            "application/json;charset=utf-8"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert_eq!(bytes.as_ref(), br#"{"ok":true}"#);
    }
}
