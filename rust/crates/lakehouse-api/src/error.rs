//! Bridges [`ApiError`] to an axum HTTP response.
//!
//! Every route handler returns [`ApiResult<T>`], and the `?` operator
//! converts any `Into<ApiError>` failure (a `ClickHouse` `ChError`, for
//! instance) into an [`ApiRejection`] automatically. The wire contract is
//! `{"error": "<message>"}` at the error's status code — verified against
//! `src/app/api/query/run/route.ts:78` and
//! `src/app/api/embed/data/route.ts:43`.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use lakehouse_core::ApiError;
use serde::Serialize;

use crate::json::ApiJson;

/// Wraps an [`ApiError`] so it can be returned directly from an axum
/// handler and rendered as `{"error": "<message>"}` at the right status
/// code.
///
/// Not yet constructed outside tests: route handlers that produce it land
/// in later tasks. `#[allow(dead_code)]` (not `#[expect(dead_code)]`, which
/// would itself warn once a real handler starts using this) documents that
/// this is intentional scaffolding, not an oversight.
#[derive(Debug)]
#[allow(dead_code)]
pub struct ApiRejection(pub ApiError);

impl<E: Into<ApiError>> From<E> for ApiRejection {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

/// The JSON body shape every error response takes, matching the
/// TypeScript route handlers' `NextResponse.json({ error: msg }, { status })`.
///
/// Constructed only from [`ApiRejection::into_response`], which itself is
/// unreachable from `main` until later tasks add real handlers — see the
/// note on [`ApiRejection`].
#[derive(Debug, Serialize)]
#[allow(dead_code)]
struct ErrorBody {
    error: String,
}

impl IntoResponse for ApiRejection {
    fn into_response(self) -> Response {
        // `ApiError::status()` returns a `u16` known to be a valid HTTP
        // status code (400/401/403/404/422/500); `StatusCode::from_u16`
        // cannot fail for these constants, but `unwrap`/`expect` are denied
        // outside tests, so fall back to 500 on the theoretical error path
        // rather than panicking.
        let status =
            StatusCode::from_u16(self.0.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = ErrorBody {
            error: self.0.to_string(),
        };
        (status, ApiJson(body)).into_response()
    }
}

/// The result type every route handler returns.
#[allow(dead_code)]
pub type ApiResult<T> = Result<T, ApiRejection>;

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use axum::body::to_bytes;
    use lakehouse_clickhouse::ChError;
    use serde_json::{Value, json};

    use super::*;

    async fn body_json(resp: Response) -> Value {
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Asserts the exact `content-type` byte string recorded across the
    /// parity corpus (`application/json;charset=utf-8`, no space before
    /// `charset`) is what error responses actually send — see `json.rs`.
    #[tokio::test]
    async fn error_response_has_exact_content_type_header() {
        let resp = ApiRejection(ApiError::NotFound("not_found".to_owned())).into_response();
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .unwrap(),
            "application/json;charset=utf-8"
        );
    }

    #[tokio::test]
    async fn bad_request_renders_400_with_message() {
        let resp =
            ApiRejection(ApiError::BadRequest("Body harus JSON {sql}".to_owned())).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(resp).await,
            json!({"error": "Body harus JSON {sql}"})
        );
    }

    #[tokio::test]
    async fn unauthorized_renders_401_with_message() {
        let resp = ApiRejection(ApiError::invalid_or_expired()).into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            body_json(resp).await,
            json!({"error": "invalid_or_expired"})
        );
    }

    #[tokio::test]
    async fn forbidden_renders_403_with_embedding_disabled() {
        let resp = ApiRejection(ApiError::Forbidden).into_response();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            body_json(resp).await,
            json!({"error": "embedding_disabled"})
        );
    }

    #[tokio::test]
    async fn not_found_renders_404_with_message() {
        let resp = ApiRejection(ApiError::NotFound("not_found".to_owned())).into_response();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        assert_eq!(body_json(resp).await, json!({"error": "not_found"}));
    }

    #[tokio::test]
    async fn unprocessable_renders_422_with_message() {
        let resp = ApiRejection(ApiError::Unprocessable("bad sql".to_owned())).into_response();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body_json(resp).await, json!({"error": "bad sql"}));
    }

    #[tokio::test]
    async fn internal_renders_500_with_message() {
        let resp = ApiRejection(ApiError::Internal("boom".to_owned())).into_response();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body_json(resp).await, json!({"error": "boom"}));
    }

    /// End-to-end path every data route depends on: a `ClickHouse` failure
    /// converts through `?` into a 422 rejection with the `ClickHouse`
    /// message intact.
    #[tokio::test]
    async fn ch_error_converts_through_question_mark_to_422_rejection() {
        fn handler() -> Result<(), ApiRejection> {
            Err(ChError::Server(
                "Code: 47. Unknown identifier: nope".to_owned(),
            ))?;
            Ok(())
        }

        let rejection = handler().unwrap_err();
        let resp = rejection.into_response();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            body_json(resp).await,
            json!({"error": "Code: 47. Unknown identifier: nope"})
        );
    }
}
