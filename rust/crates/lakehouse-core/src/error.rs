//! The single error type every route handler funnels into.
//!
//! Mirrors the status-code and message contract that the TypeScript route
//! handlers currently return, e.g. `{"error": "invalid_or_expired"}` with
//! HTTP 401 (see `src/app/api/embed/data/route.ts`).

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
    /// The message is fixed to match the TypeScript contract exactly.
    #[error("invalid_or_expired")]
    Unauthorized,
    /// The caller is authenticated but not allowed to perform this action.
    /// Maps to HTTP 403.
    ///
    /// The message is fixed to match the TypeScript contract exactly.
    #[error("embedding_disabled")]
    Forbidden,
    /// The requested resource does not exist. Maps to HTTP 404.
    #[error("{0}")]
    NotFound(String),
    /// The request was well-formed but semantically invalid (e.g. a query
    /// that fails validation or execution). Maps to HTTP 422.
    #[error("{0}")]
    Unprocessable(String),
    /// An unexpected server-side failure. Maps to HTTP 500.
    #[error("{0}")]
    Internal(String),
}

impl ApiError {
    /// The HTTP status code this error should be rendered with.
    #[must_use]
    pub const fn status(&self) -> u16 {
        match self {
            Self::BadRequest(_) => 400,
            Self::Unauthorized => 401,
            Self::Forbidden => 403,
            Self::NotFound(_) => 404,
            Self::Unprocessable(_) => 422,
            Self::Internal(_) => 500,
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
    fn unauthorized_message_matches_ts_contract() {
        assert_eq!(ApiError::Unauthorized.to_string(), "invalid_or_expired");
    }

    #[test]
    fn forbidden_message_matches_ts_contract() {
        assert_eq!(ApiError::Forbidden.to_string(), "embedding_disabled");
    }

    #[test]
    fn not_found_maps_to_404() {
        assert_eq!(ApiError::NotFound("not_found".to_owned()).status(), 404);
    }
}
