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
use lakehouse_auth::openfga::OpenfgaError;
use lakehouse_core::ApiError;
use serde::Serialize;

use crate::json::ApiJson;

/// Wraps an [`ApiError`] so it can be returned directly from an axum
/// handler and rendered as `{"error": "<message>"}` at the right status
/// code.
#[derive(Debug)]
pub struct ApiRejection(pub ApiError);

impl<E: Into<ApiError>> From<E> for ApiRejection {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

/// The JSON body shape every error response takes, matching the
/// TypeScript route handlers' `NextResponse.json({ error: msg }, { status })`.
///
/// Constructed only from [`ApiRejection::into_response`].
#[derive(Debug, Serialize)]
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
pub type ApiResult<T> = Result<T, ApiRejection>;

/// A fixed 503 for `POST /api/identity/tenants` when a
/// call into Lakekeeper's management API fails.
///
/// Matches this crate's existing `ApiError::unauthorized()`/
/// `invalid_or_expired()` constructor-function convention
/// (`lakehouse-core/src/error.rs:100-108`): a small named `fn` rather than
/// building the string at each `routes::identity::provision_tenant` call
/// site, so every provisioning failure of the same category renders
/// identically. Deliberately never carries `OpenfgaError`'s own message or
/// any upstream Lakekeeper response text (AGENTS.md: "upstream error text
/// never reaches a response") — `OpenfgaError` itself already strips that
/// (see its doc comment), but this constructor keeps the guarantee
/// visible at the call site rather than relying on the callee alone.
///
/// Takes `cause` to pick one of two fixed messages — `OpenfgaError::Rejected`
/// (Lakekeeper reached, our request refused — a `4xx`) reads differently
/// from `Transport`/`Server` (Lakekeeper unreachable, or a `5xx`/unparseable
/// response) — without ever repeating what Lakekeeper actually said.
#[must_use]
pub fn provisioning_unavailable(cause: &OpenfgaError) -> ApiError {
    let message = match cause {
        OpenfgaError::Rejected => {
            "tenant provisioning was rejected by Lakekeeper (a Lakekeeper request was reachable \
             but refused)"
        }
        OpenfgaError::Transport | OpenfgaError::Server => {
            "tenant provisioning could not reach Lakekeeper"
        }
    };
    ApiError::Unavailable(message.to_owned())
}

/// A fixed 503 for `POST /api/identity/tenants` when this deployment has
/// not set the dedicated `TENANT_WAREHOUSE_S3_*` settings tenant-warehouse
/// provisioning requires — see `lakehouse_api::config::TenantWarehouseStorageConfig`'s
/// doc comment for the exact fields. Named after the settings themselves
/// (not just "unavailable") so an operator reading this response knows
/// what to set, the same honest-and-specific posture the "no
/// `lakekeeper_admin` client" branch in `routes::identity::create_tenant`
/// already takes — this is that same branch's sibling for a Lakekeeper
/// admin token that IS configured but whose tenant-warehouse storage
/// settings are not.
#[must_use]
pub fn tenant_warehouse_storage_not_configured() -> ApiError {
    ApiError::Unavailable(
        "tenant warehouse storage is not configured: set TENANT_WAREHOUSE_S3_ENDPOINT, \
         TENANT_WAREHOUSE_S3_ACCESS_KEY, and TENANT_WAREHOUSE_S3_SECRET_KEY"
            .to_owned(),
    )
}

/// A fixed 503 for `POST /api/identity/tenants` when
/// `TENANT_WAREHOUSE_S3_BUCKET` is set to the SAME bucket as
/// `LAKEHOUSE_WAREHOUSE_BUCKET` — the sibling of
/// [`tenant_warehouse_storage_not_configured`] for the case the settings
/// ARE all set, but to a shape Lakekeeper can never accept. Every
/// deployment's `lakekeeper-warehouse-init` (`docker-compose.yml`) creates
/// the shared `default` warehouse at `LAKEHOUSE_WAREHOUSE_BUCKET`'s
/// storage-profile ROOT (no `key-prefix`) — a warehouse whose profile
/// claims a bucket's root makes `Lakekeeper` refuse EVERY other warehouse
/// in that same bucket with `CreateWarehouseStorageProfileOverlap`,
/// regardless of `key-prefix`. Caught here, before ever attempting the
/// Lakekeeper call, rather than surfacing as a confusing `Rejected`
/// (`provisioning_unavailable`) on the very first tenant provisioned.
#[must_use]
pub fn tenant_warehouse_bucket_overlaps_default() -> ApiError {
    ApiError::Unavailable(
        "tenant warehouses need their own bucket: TENANT_WAREHOUSE_S3_BUCKET must not equal \
         LAKEHOUSE_WAREHOUSE_BUCKET, because the default warehouse (lakekeeper-warehouse-init) \
         owns the root of that bucket"
            .to_owned(),
    )
}

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
        let resp = ApiRejection(ApiError::BadRequest("Body must be JSON {sql}".to_owned()))
            .into_response();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(resp).await,
            json!({"error": "Body must be JSON {sql}"})
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
    async fn conflict_renders_409_with_message() {
        let resp = ApiRejection(ApiError::Conflict("duplicate".to_owned())).into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
        assert_eq!(body_json(resp).await, json!({"error": "duplicate"}));
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

    /// `OpenfgaError::Rejected` (Lakekeeper reached, our request refused)
    /// must not render the same message `Transport`/`Server` do — the
    /// whole point of splitting the cause categories.
    #[test]
    fn provisioning_unavailable_names_a_rejected_request_distinctly() {
        let err = provisioning_unavailable(&OpenfgaError::Rejected);
        assert_eq!(err.status(), 503);
        assert!(err.to_string().contains("rejected"));
        assert!(!err.to_string().contains("could not reach"));
    }

    /// `Transport`/`Server` share the "could not reach Lakekeeper" message
    /// category — both mean the deployment cannot get a usable response
    /// out of Lakekeeper at all, unlike a `Rejected` 4xx.
    #[test]
    fn provisioning_unavailable_groups_transport_and_server_together() {
        let transport = provisioning_unavailable(&OpenfgaError::Transport);
        let server = provisioning_unavailable(&OpenfgaError::Server);
        assert_eq!(transport.to_string(), server.to_string());
        assert!(transport.to_string().contains("could not reach"));
    }

    /// The honest refusal for unset tenant-warehouse storage settings
    /// names the settings themselves, not a generic "unavailable".
    #[test]
    fn tenant_warehouse_storage_not_configured_names_the_settings() {
        let err = tenant_warehouse_storage_not_configured();
        assert_eq!(err.status(), 503);
        assert!(err.to_string().contains("TENANT_WAREHOUSE_S3_ENDPOINT"));
        assert!(err.to_string().contains("TENANT_WAREHOUSE_S3_ACCESS_KEY"));
        assert!(err.to_string().contains("TENANT_WAREHOUSE_S3_SECRET_KEY"));
    }

    /// The overlap refusal names both settings and says why, not just
    /// "unavailable" — an operator reading it must be able to fix it
    /// without reading this file's source.
    #[test]
    fn tenant_warehouse_bucket_overlaps_default_names_both_settings() {
        let err = tenant_warehouse_bucket_overlaps_default();
        assert_eq!(err.status(), 503);
        assert!(err.to_string().contains("TENANT_WAREHOUSE_S3_BUCKET"));
        assert!(err.to_string().contains("LAKEHOUSE_WAREHOUSE_BUCKET"));
    }
}
