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
//! | [`StoreError::NotAllowed`] | `PermissionDenied`    | 403    | the store's own rule refuses the action on a row that does exist (`QS-3` review of #382: a move needs manage) |
//! | [`StoreError::Unavailable`]| `Unavailable`         | 503    | no pool configured at all (see [`crate::connect_lazy`])         |
//! | [`StoreError::Validation`] | `BadRequest`          | 400    | this crate's own validator rejected caller input (never a raw `sqlx` error) |
//! | [`StoreError::Database`]   | `Internal`            | 500    | anything else — connection refused, timeout, syntax error, ...  |
//! | [`StoreError::Migration`]  | `Internal`            | 500    | `sqlx::migrate!` failed to apply                                |
//!
//! The first three are genuinely the caller's fault (4xx); the rest are
//! this service's problem (5xx) — the split a blanket 422/500 mapping would
//! erase. Within the 5xx group, [`StoreError::Unavailable`] is further
//! separated out as a 503: it means `AppState::pg` is `None` — the pool was
//! never constructed, because `DATABASE_URL` did not parse — which is a
//! deployment/configuration problem that a caller can sensibly retry once
//! the deployment is fixed, not an unexpected bug in a code path that was
//! supposed to work. A genuine connection failure (Postgres down while the
//! pool exists) stays [`StoreError::Database`] -> 500, matching the
//! `ClickHouse` side's treatment of an outage. [`ApiError::Internal`]'s message is deliberately generic (`"{0}"`
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
    /// A field the caller tried to set is already taken by ANOTHER user's
    /// username or email, compared case-insensitively (ADM-11 D5: one
    /// sign-in box must never match two people). Unlike [`Self::Conflict`],
    /// which says only "some value collides", this names the field so the
    /// route can answer 409 with the field-specific sentence the invite and
    /// rename forms show. The message is this crate's own fixed sentence —
    /// never an upstream error's text — the same discipline
    /// [`Self::Validation`] and [`Self::NotAllowed`] follow.
    #[error("{0}")]
    FieldConflict(String),
    /// A foreign-key violation: the caller referenced a row that does not
    /// exist (or tried to delete a row something else still references,
    /// under `ON DELETE RESTRICT`).
    #[error("referenced record does not exist")]
    ForeignKeyViolation,
    /// A query expected to find a row (e.g. `fetch_one`) found none.
    #[error("record not found")]
    NotFound,
    /// The caller may not do this to a row that does exist and may even
    /// be visible to them: the store's own rule refuses the action, not
    /// the row's existence — unlike [`Self::NotFound`], which says the
    /// row is not there at all. The one rule that raises it today is the
    /// move rule (`QS-3` review of #382): a query move needs manage on the query.
    /// The message is this crate's own
    /// fixed sentence — the one the fix ships to callers — never an
    /// upstream error's text, so it is safe to surface the way
    /// [`Self::Validation`]'s is.
    #[error("Only someone who manages this can move it.")]
    NotAllowed,
    /// No Postgres pool is configured/available for this request. See the
    /// boot-behavior note on [`crate::connect_lazy`]: an unreachable
    /// Postgres is not fatal at startup, so this is the error a Phase 2
    /// handler gets instead of a panic when the database truly cannot be
    /// reached.
    #[error("database is not available")]
    Unavailable,
    /// A caller-supplied value failed application-level validation this
    /// crate performs itself (never a raw `sqlx`/Postgres error) -- e.g. a
    /// `dial` that fails `ingest_spec::Dial::parse` (`set_ingest_spec`,
    /// `connectors.rs`). Carries the validator's own message, which is
    /// already safe to surface (it never echoes raw upstream driver/
    /// database text, nor the raw `dial` JSON a smuggled credential could
    /// hide in -- see `ingest_spec`'s module doc comment -- unlike
    /// [`Self::Database`], whose message is deliberately generic; see the
    /// module doc comment's rule for why).
    #[error("{0}")]
    Validation(String),
    /// Any other `sqlx` failure: connection refused, pool timeout, a SQL
    /// syntax error, a type-decode failure, and so on. Intentionally does
    /// NOT interpolate the source error into `Display` — see the module
    /// doc comment.
    #[error("database error")]
    Database(#[source] sqlx::Error),
    /// `sqlx::migrate!` failed to apply the migration set.
    #[error("migration failed")]
    Migration(#[source] sqlx::migrate::MigrateError),
    /// A stored pipeline definition whose `inputs`, `steps` or `transforms`
    /// column does not decode into the shape the store wrote there.
    ///
    /// BLD-1 part 1 review R1-B5: this used to be `unwrap_or_default()`,
    /// which made an unreadable `steps` column compile to a plain
    /// `SELECT * FROM <first input>` and overwrite the pipeline's output with
    /// unfiltered rows (AGENTS.md principle 3). A definition this crate
    /// cannot read is refused instead — 422, the status the orchestrator
    /// treats as final, with this fixed sentence and never a serde message.
    #[error("this pipeline's stored definition cannot be read")]
    CorruptDefinition,
    /// `serde_json` failed to encode or decode a value this crate owns
    /// (a `pipeline_definition_version.snapshot`, the row that every
    /// `created`/`updated`/`restored`/`deleted` event captures). A
    /// serialization failure is a programmer error — every type that
    /// reaches `to_value` is one this crate can deserialize — so the
    /// mapping is 500 (`Internal`). The `Display` is deliberately generic
    /// for the same reason `Self::Database`'s is: the serde message can
    /// embed the offending field name, which we treat as upstream text
    /// and never forward.
    #[error("database error")]
    Serialization(#[source] serde_json::Error),
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

impl From<serde_json::Error> for StoreError {
    /// Every `serde_json::Error` produced inside this crate is either a
    /// snapshot encode/decode against a known Rust type (a programmer
    /// error if it fires at all — see [`StoreError::Serialization`]) or
    /// a row-decode for a column this crate's own INSERTs write (same
    /// argument, same status mapping).
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err)
    }
}

impl From<StoreError> for ApiError {
    fn from(err: StoreError) -> Self {
        let message = err.to_string();
        match err {
            // ADM-11 D5: a username/email that would make one sign-in box
            // match two people is a 409 that names the field ("Username
            // already in use"), so the invite and rename forms can point at
            // the field that must change. Same refusal shape as
            // `Conflict`, with the field-specific sentence instead of the
            // generic one — hence one arm for both variants.
            StoreError::Conflict | StoreError::FieldConflict(_) => Self::Conflict(message),
            // `ForeignKeyViolation` and `Validation` both map to `BadRequest`
            // for different reasons (see the module doc comment's mapping
            // table): the former because the caller referenced a row that
            // doesn't exist, the latter because this crate's own validator
            // (e.g. `ingest_spec::Dial::parse`'s field-path error) rejected
            // caller input -- `Validation`'s message is safe to surface,
            // never a raw `sqlx`/database error, so it never needs the
            // generic `Internal` treatment `Database`/`Migration` get below.
            StoreError::ForeignKeyViolation | StoreError::Validation(_) => {
                Self::BadRequest(message)
            }
            // `NotAllowed` is the store's own refusal of an action on a
            // row that exists: 403, the caller's fault, carrying the
            // fixed sentence the store wrote — never 404, which would
            // tell the caller the row is not there at all.
            StoreError::NotAllowed => Self::PermissionDenied(message),
            StoreError::NotFound => Self::NotFound(message),
            // A stored definition the store cannot read is a refusal no retry
            // changes: 422, with the fixed sentence above (R1-B5).
            StoreError::CorruptDefinition => Self::Unprocessable(message),
            StoreError::Unavailable => Self::Unavailable(message),
            StoreError::Database(_) | StoreError::Migration(_) | StoreError::Serialization(_) => {
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

    /// ADM-11 D5: a field-specific username/email clash maps to 409 and
    /// carries the store's own fixed sentence, so an invite or rename form
    /// can show "Username already in use" at the field that must change.
    #[test]
    fn field_conflict_maps_to_409_and_carries_the_field_sentence() {
        let api: ApiError = StoreError::FieldConflict("Username already in use".to_owned()).into();
        assert_eq!(api.status(), 409);
        assert_eq!(api.to_string(), "Username already in use");
    }

    /// `Validation` must map to 400, never `Internal` (500) -- an invalid
    /// `dial` is the caller's fault, and the exhaustive `match` in
    /// `From<StoreError> for ApiError` above forces every future variant
    /// to name its status explicitly rather than fall through to a `_`
    /// arm.
    #[test]
    fn validation_maps_to_400_and_carries_the_validator_message() {
        let api: ApiError =
            StoreError::Validation("dial for adapter \"sql\" is invalid".to_owned()).into();
        assert_eq!(api.status(), 400);
        assert_eq!(api.to_string(), "dial for adapter \"sql\" is invalid");
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

    /// `NotAllowed` must map to 403 and never to 404: the row exists and
    /// is even visible to the caller, and what is refused is the action —
    /// a refusal that looked like a missing row would hide the rule
    /// (`QS-3` review finding 1). The asserted body is the full
    /// `Display` the caller receives: [`ApiError::PermissionDenied`]'s
    /// own `permission_denied:` prefix — kept by lakehouse-core so its
    /// 403s stay distinguishable from [`ApiError::Forbidden`]'s fixed
    /// body — followed by this store's fixed sentence, unchanged.
    #[test]
    fn not_allowed_maps_to_403_and_carries_the_fixed_sentence() {
        let api: ApiError = StoreError::NotAllowed.into();
        assert_eq!(api.status(), 403);
        assert_eq!(
            api.to_string(),
            "permission_denied: Only someone who manages this can move it."
        );
    }

    /// `Unavailable` is the one 5xx that is NOT a 500: "there is no pool
    /// at all" is a configuration problem worth retrying, and the routes
    /// that hit it must say 503 rather than claim an internal bug.
    #[test]
    fn unavailable_maps_to_503() {
        let api: ApiError = StoreError::Unavailable.into();
        assert_eq!(api.status(), 503);
        assert_eq!(api.to_string(), "database is not available");
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
