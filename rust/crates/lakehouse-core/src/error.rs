//! The single error type every route handler funnels into.
//!
//! Mirrors the status-code and message contract that the TypeScript route
//! handlers currently return, e.g. `{"error": "invalid_or_expired"}` with
//! HTTP 401 (see `src/app/api/embed/data/route.ts:43`).
//!
//! # On 422 vs 500
//!
//! The TypeScript is deliberately over-broad here and we reproduce it. At
//! `src/app/api/query/run/route.ts:72-74` the `catch` is unconditional
//! `status: 422`, so a `ClickHouse` *connection* failure — genuinely our outage,
//! not the user's SQL — surfaces as 422 alongside real syntax errors. Parity
//! is the cutover gate, so [`Self::Unprocessable`] keeps that behavior and
//! [`Self::Internal`] is reserved for Rust-side failures with no TypeScript
//! counterpart. Revisit only after cutover, as a deliberate divergence.

use thiserror::Error;

/// The unified API error, carrying enough information to render the exact
/// `{"error": "<message>"}` JSON body and status code the TypeScript
/// handlers produce today.
#[derive(Debug, Error)]
pub enum ApiError {
    /// Malformed or missing request body/fields. Maps to HTTP 400.
    #[error("{0}")]
    BadRequest(String),
    /// Missing, invalid, or expired credentials. Maps to HTTP 401.
    ///
    /// Carries its message because the TypeScript returns two different 401
    /// bodies: `invalid_or_expired` from `src/app/api/embed/data/route.ts:43`
    /// (bad or expired embed JWT) and `unauthorized` from
    /// `src/app/api/alerts/run/route.ts:19` (bad `ALERTS_RUN_TOKEN`). Prefer
    /// the [`Self::invalid_or_expired`] and [`Self::unauthorized`]
    /// constructors over building the string at each call site.
    #[error("{0}")]
    Unauthorized(String),
    /// The caller is authenticated but not allowed to perform this action.
    /// Maps to HTTP 403.
    ///
    /// A unit variant because `embedding_disabled` is the only 403 body the
    /// TypeScript ever produced — verified by sweeping every handler under
    /// `src/app/api`. [`Self::PermissionDenied`] (Task 3.2, Rust/Phase
    /// 3-only) is the other 403, kept as a separate variant rather than
    /// reused here specifically so this one keeps its fixed, ported body.
    #[error("embedding_disabled")]
    Forbidden,
    /// The caller is authenticated but lacks a permission the route policy
    /// (`lakehouse-api::policy`) requires. Maps to HTTP 403.
    ///
    /// Phase-3-only, like [`Self::Unavailable`]/[`Self::Conflict`]: no
    /// TypeScript route ever checked a permission, so there is no ported
    /// body to match. Carries the missing permission string so a client
    /// can tell what to request, rather than folding into
    /// [`Self::Forbidden`]'s fixed `embedding_disabled` body, which would
    /// make the two unrelated 403 causes indistinguishable.
    #[error("permission_denied: {0}")]
    PermissionDenied(String),
    /// The requested resource does not exist. Maps to HTTP 404.
    #[error("{0}")]
    NotFound(String),
    /// The request was well-formed but semantically invalid (e.g. a query
    /// that fails validation or execution). Maps to HTTP 422.
    #[error("{0}")]
    Unprocessable(String),
    /// The request conflicts with existing state (e.g. a unique-constraint
    /// violation on a Postgres write). Maps to HTTP 409.
    ///
    /// Phase-2-only: unlike every other variant here, this has no
    /// TypeScript precedent to reproduce — Phase 1 never wrote to
    /// persistent storage, so no route ever needed a 409. It exists purely
    /// for `lakehouse-store`'s `StoreError -> ApiError` mapping and is a
    /// deliberate, REST-idiomatic addition rather than a ported behavior.
    #[error("{0}")]
    Conflict(String),
    /// A dependency this request needs is not configured or not reachable,
    /// and the request may well succeed if retried once it is. Maps to HTTP
    /// 503.
    ///
    /// Phase-2-only, like [`Self::Conflict`], and for the same reason: no
    /// Phase 1 route has a dependency it can *know* is absent before trying
    /// (a `ClickHouse` outage surfaces as a failed query, not as a missing
    /// client). The Postgres pool is different — `AppState::pg` is
    /// `Option`, so a handler can tell "there is no pool at all" apart from
    /// "the pool failed", and the two deserve different answers: the former
    /// is a deployment/configuration problem the caller should retry
    /// against a fixed deployment (503), not an unexpected server bug
    /// (500).
    #[error("{0}")]
    Unavailable(String),
    /// An unexpected server-side failure. Maps to HTTP 500.
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    /// A 401 for a bad or expired embed JWT, matching
    /// `src/app/api/embed/data/route.ts:43`.
    #[must_use]
    pub fn invalid_or_expired() -> Self {
        Self::Unauthorized("invalid_or_expired".to_owned())
    }

    /// A 401 for a missing or wrong shared token, matching
    /// `src/app/api/alerts/run/route.ts:19`.
    #[must_use]
    pub fn unauthorized() -> Self {
        Self::Unauthorized("unauthorized".to_owned())
    }

    /// The HTTP status code this error should be rendered with.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Unauthorized(_) => 401,
            Self::Forbidden | Self::PermissionDenied(_) => 403,
            Self::NotFound(_) => 404,
            Self::Unprocessable(_) => 422,
            Self::Conflict(_) => 409,
            Self::Internal(_) => 500,
            Self::Unavailable(_) => 503,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bad_request_maps_to_400() {
        assert_eq!(
            ApiError::BadRequest("Body harus JSON {sql}".to_owned()).status(),
            400
        );
    }

    #[test]
    fn unprocessable_maps_to_422() {
        assert_eq!(ApiError::Unprocessable("bad sql".to_owned()).status(), 422);
    }

    #[test]
    fn invalid_or_expired_matches_embed_route_contract() {
        assert_eq!(
            ApiError::invalid_or_expired().to_string(),
            "invalid_or_expired"
        );
    }

    #[test]
    fn unauthorized_matches_alerts_run_route_contract() {
        assert_eq!(ApiError::unauthorized().to_string(), "unauthorized");
    }

    #[test]
    fn both_unauthorized_flavours_map_to_401() {
        assert_eq!(ApiError::invalid_or_expired().status(), 401);
        assert_eq!(ApiError::unauthorized().status(), 401);
    }

    #[test]
    fn forbidden_message_matches_ts_contract() {
        assert_eq!(ApiError::Forbidden.to_string(), "embedding_disabled");
    }

    #[test]
    fn not_found_maps_to_404() {
        assert_eq!(ApiError::NotFound("not_found".to_owned()).status(), 404);
    }

    #[test]
    fn conflict_maps_to_409() {
        assert_eq!(ApiError::Conflict("duplicate".to_owned()).status(), 409);
    }

    #[test]
    fn unavailable_maps_to_503() {
        assert_eq!(
            ApiError::Unavailable("database is not available".to_owned()).status(),
            503
        );
    }
}
