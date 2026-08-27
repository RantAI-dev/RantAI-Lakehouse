//! [`StoreError`] and its mapping onto [`lakehouse_core::ApiError`].
//!
//! # On this mapping vs `ChError`'s
//!
//! `lakehouse-clickhouse`'s `ChError` blanket-maps every failure to HTTP
//! 422, because that is what the TypeScript it ports (`query/run/route.ts`)
//! actually does — parity with an existing behavior, not a considered
//! design choice, and the doc comment on `ChError` says so explicitly.
//!
//! There is no TypeScript to be faithful to here: `lakehouse-store` is new
//! Phase 2 code with no prior implementation, so that precedent does not
//! carry over. Postgres failures get a real, considered mapping instead:
//!
//! | [`StoreError`]            | [`ApiError`]         | status | why                                                          |
//! |----------------------------|-----------------------|--------|---------------------------------------------------------------|
//! | [`StoreError::Conflict`]   | `Conflict`            | 409    | caller-supplied data collides with a unique constraint         |
//! | [`StoreError::ForeignKeyViolation`] | `BadRequest`  | 400    | caller referenced a row that doesn't exist                     |
//! | [`StoreError::NotFound`]   | `NotFound`            | 404    | caller asked for a row that doesn't exist                      |
//! | [`StoreError::Unavailable`]| `Internal`            | 500    | no pool configured (see [`crate::connect_lazy`])                |
//! | [`StoreError::Database`]   | `Internal`            | 500    | anything else — connection refused, timeout, syntax error, ...  |
//! | [`StoreError::Migration`]  | `Internal`            | 500    | `sqlx::migrate!` failed to apply                                |
//!
//! The first three are genuinely the caller's fault (4xx); the rest are
//! this service's problem (5xx) — the split a blanket 422/500 mapping would
//! erase. [`ApiError::Internal`]'s message is deliberately generic (`"{0}"`
//! renders [`StoreError`]'s own `Display`, which never includes the
//! `#[source]` error text) so a raw `sqlx::Error` — which can embed the
//! connection string, host, or driver-internal detail — never reaches an
//! HTTP response body, mirroring the same leak concern `ChError`'s doc
//! comment raises for `reqwest::Error`.

use lakehouse_core::ApiError;
use thiserror::Error;

/// Errors produced by [`crate`]'s pool construction, migrations, and (in
/// later Phase 2 tasks) query helpers.
#[derive(Debug, Error)]
pub enum StoreError {
    /// A unique-constraint violation (e.g. a duplicate email or slug).
    #[error("a record with that value already exists")]
    Conflict,
    /// A foreign-key violation: the caller referenced a row that does not
    /// exist (or tried to delete a row something else still references,
    /// under `ON DELETE RESTRICT`).
    #[error("referenced record does not exist")]
    ForeignKeyViolation,
    /// A query expected to find a row (e.g. `fetch_one`) found none.
    #[error("record not found")]
    NotFound,
    /// No Postgres pool is configured/available for this request. See the
    /// boot-behavior note on [`crate::connect_lazy`]: an unreachable
    /// Postgres is not fatal at startup, so this is the error a Phase 2
    /// handler gets instead of a panic when the database truly cannot be
    /// reached.
    #[error("database is not available")]
    Unavailable,
    /// Any other `sqlx` failure: connection refused, pool timeout, a SQL
    /// syntax error, a type-decode failure, and so on. Intentionally does
    /// NOT interpolate the source error into `Display` — see the module
    /// doc comment.
    #[error("database error")]
    Database(#[source] sqlx::Error),
    /// `sqlx::migrate!` failed to apply the migration set.
    #[error("migration failed")]
    Migration(#[source] sqlx::migrate::MigrateError),
}

impl From<sqlx::Error> for StoreError {
    /// Classifies a raw `sqlx::Error` into the specific, HTTP-status-bearing
    /// variant it corresponds to, falling back to the catch-all
    /// [`Self::Database`] for everything that isn't a row-not-found or a
    /// constraint violation this crate knows how to name.
    fn from(err: sqlx::Error) -> Self {
        if matches!(err, sqlx::Error::RowNotFound) {
            return Self::NotFound;
        }
        if let sqlx::Error::Database(db_err) = &err {
            if db_err.is_unique_violation() {
                return Self::Conflict;
            }
            if db_err.is_foreign_key_violation() {
                return Self::ForeignKeyViolation;
            }
        }
        Self::Database(err)
    }
}

impl From<sqlx::migrate::MigrateError> for StoreError {
    fn from(err: sqlx::migrate::MigrateError) -> Self {
        Self::Migration(err)
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        let message = err.to_string();
        match err {
            StoreError::Conflict => Self::Conflict(message),
            StoreError::ForeignKeyViolation => Self::BadRequest(message),
            StoreError::NotFound => Self::NotFound(message),
            StoreError::Unavailable | StoreError::Database(_) | StoreError::Migration(_) => {
                Self::Internal(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn conflict_maps_to_409() {
        let api: ApiError = StoreError::Conflict.into();
        assert_eq!(api.status(), 409);
    }

    #[test]
    fn foreign_key_violation_maps_to_400() {
        let api: ApiError = StoreError::ForeignKeyViolation.into();
        assert_eq!(api.status(), 400);
    }

    #[test]
    fn not_found_maps_to_404() {
        let api: ApiError = StoreError::NotFound.into();
        assert_eq!(api.status(), 404);
    }

    #[test]
    fn unavailable_maps_to_500() {
        let api: ApiError = StoreError::Unavailable.into();
        assert_eq!(api.status(), 500);
    }

    #[test]
    fn row_not_found_classifies_as_not_found() {
        let err: StoreError = sqlx::Error::RowNotFound.into();
        assert!(matches!(err, StoreError::NotFound));
    }

    /// `Internal`'s message must never embed a raw `sqlx::Error`'s
    /// `Display`, which can carry connection details — this is the
    /// regression test for the leak concern the module doc comment raises.
    #[test]
    fn internal_message_never_leaks_source_error_text() {
        let io_err = std::io::Error::other("connect to db.internal:5432 failed");
        let sqlx_err = sqlx::Error::Io(io_err);
        let store_err: StoreError = sqlx_err.into();
        let api: ApiError = store_err.into();
        assert_eq!(api.status(), 500);
        assert!(!api.to_string().contains("db.internal"));
        assert_eq!(api.to_string(), "database error");
    }
}
