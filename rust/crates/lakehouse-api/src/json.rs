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
use serde_json::Value;

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
        // Route through `serde_json::Value` and renumber whole-valued
        // floats before handing off to `axum::Json`, rather than serializing
        // `T` directly. `serde_json`'s default `f64` `Serialize` always
        // keeps a decimal point (`0.0`, `3000000.0`); every JS number in the
        // TS backend's responses is un-decorated when whole (`JSON.stringify
        // (0.0) === "0"`). Parity caught this on `queryErrorRate`,
        // `failureRate`, and chart `target` — three unrelated f64 fields
        // across three unrelated modules — so it is fixed once, here, at
        // the single point every JSON response in this crate passes
        // through, instead of annotating every f64 field individually.
        //
        // Falls back to serializing `T` directly if `to_value` fails (e.g.
        // a `NaN`/`Infinity` float, or a map with non-string keys) so a
        // handler that returns a genuinely non-JSON-representable value
        // fails exactly the way `axum::Json` would, rather than silently
        // swallowing the error here.
        let mut response = match serde_json::to_value(&self.0) {
            Ok(value) => axum::Json(normalize_whole_floats(value)).into_response(),
            Err(_) => axum::Json(self.0).into_response(),
        };
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_VALUE));
        response
    }
}

/// Recursively rewrites any `Value::Number` that was serialized as an `f64`
/// (`serde_json::Number::is_f64`) but holds a whole value into a plain
/// integer, matching how `JSON.stringify` renders a whole-valued JS
/// `number`. Numbers that were already integers (`is_i64`/`is_u64`) are
/// untouched, as are genuinely fractional floats.
fn normalize_whole_floats(value: Value) -> Value {
    match value {
        Value::Number(n) => {
            if let Some(f) = n.as_f64().filter(|_| n.is_f64()) {
                #[allow(clippy::cast_possible_truncation)]
                if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
                    return Value::Number((f as i64).into());
                }
            }
            Value::Number(n)
        }
        Value::Array(items) => {
            Value::Array(items.into_iter().map(normalize_whole_floats).collect())
        }
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, normalize_whole_floats(v)))
                .collect(),
        ),
        other => other,
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
