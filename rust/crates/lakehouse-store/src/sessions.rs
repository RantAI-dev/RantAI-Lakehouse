//! Read-side repository for browser sessions.
//!
//! `lakehouse-auth` owns the write-side CRUD for the `session` table
//! (`create_session`, `revoke_session`, `validate_session`,
//! `revoke_all_sessions_for_user`, all in `lakehouse_auth::session`) and
//! nothing else. This module is the matching read-side: a single listing
//! used by `GET /api/auth/sessions`, kept here in the repository layer
//! rather than the route so the SQL stays out of the handler and the
//! handler stays a thin principal-handling shell.
//!
//! # Honest about what's known
//!
//! [`SessionRow`] exposes the columns `session.id` / `app_user_id` /
//! `app_user.name` / `created_at` / `expires_at` / `created_ip` /
//! `user_agent` only — never `token_hash`. The hash is what makes
//! `lakehouse_auth::session::validate_session` work, and a hash is enough
//! to authenticate, so once it leaves the database it has the same threat
//! shape as the raw token. The handler that serves [`SessionRow`] is
//! allowed to be on a public route (the only check it enforces itself is
//! "is the caller authenticated and is the row theirs, or do they hold the
//! admin permission"), so a hash in the wire shape would be a path to
//! session takeover.
//!
//! `created_ip` / `user_agent` round-trip as honest `null` for every
//! session minted today: `routes::auth::login` and `routes::auth::oidc_callback`
//! both mint sessions through `session::create_session(pool, user_id, ttl,
//! None, None)`, and the response shape reflects that — `Option<String>`
//! serializing to JSON `null` rather than a fabricated placeholder.

use serde::Serialize;
use sqlx::FromRow;
use thiserror::Error;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// One row of `GET /api/auth/sessions`. Deliberately carries no token or
/// token hash — session tokens are never returned to a caller; only ids
/// and metadata are — `id`/`user_id`/`user_name`/`created_at`/
/// `expires_at`/`created_ip`/`user_agent` only, every field already
/// non-secret in `session` (`0019_auth.sql`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, FromRow)]
#[serde(rename_all = "camelCase")]
pub struct SessionRow {
    /// `session.id` (a UUID), rendered as a string.
    pub id: String,
    /// `session.app_user_id`, rendered as a string. Joins to `app_user.id`
    /// — `PrincipalId::User(user_id).to_string()` matches this byte for
    /// byte.
    pub user_id: String,
    /// `app_user.name` for the session's owner — display only, never an
    /// authorization input.
    pub user_name: String,
    /// `session.created_at` as the text Postgres renders it for
    /// `TIMESTAMPTZ`. Newest-first ordering is what [`list_sessions_for_caller`]
    /// sorts by; callers that need a parsed timestamp parse this themselves
    /// rather than relying on a particular wire format.
    pub created_at: String,
    /// `session.expires_at` as the text Postgres renders it for
    /// `TIMESTAMPTZ`. Distinct from `revoked_at` (deliberately not exposed
    /// — a session that appears in this list is by definition live, so the
    /// column is constant `NULL` for every row and would only add noise).
    pub expires_at: String,
    /// `session.created_ip` — the IP address the browser presented at
    /// login. Every session minted today passes `None` here, so this is
    /// `null` on the response; see the module doc comment on why the
    /// absence is preserved rather than fabricated.
    pub created_ip: Option<String>,
    /// `session.user_agent` — the browser's `User-Agent` header at login.
    /// Same honest-`null` posture as [`SessionRow::created_ip`].
    pub user_agent: Option<String>,
}

/// List live (unrevoked, unexpired) sessions: every one if `is_admin`,
/// else only `caller_id`'s own. The `WHERE` branch is chosen in Rust, not
/// string-built, so there is no path where a caller-controlled value
/// decides the SQL shape — `$2` is a Rust `bool` bound as a Postgres
/// boolean, never interpolated.
///
/// "Live" means `revoked_at IS NULL AND expires_at > now()`, matching
/// [`lakehouse_auth::session::validate_session`]'s own predicate on the
/// `session` table — a row that would refuse to authenticate never
/// appears in this list, so the handler's "what sessions does this
/// caller have right now" answer agrees with what
/// `session::validate_session` would say if those cookies were
/// re-presented.
///
/// # Errors
///
/// Returns [`StoreError::Database`] on any query failure.
pub async fn list_sessions_for_caller(
    pool: &PgPool,
    caller_id: Uuid,
    is_admin: bool,
) -> Result<Vec<SessionRow>, StoreError> {
    let rows: Vec<SessionRow> = sqlx::query_as(
        "SELECT s.id::text AS id, s.app_user_id::text AS user_id, u.name AS user_name, \
                s.created_at::text AS created_at, s.expires_at::text AS expires_at, \
                s.created_ip, s.user_agent \
         FROM session s JOIN app_user u ON u.id = s.app_user_id \
         WHERE s.revoked_at IS NULL AND s.expires_at > now() \
           AND ($2 OR s.app_user_id = $1) \
         ORDER BY s.created_at DESC, s.id",
    )
    .bind(caller_id)
    .bind(is_admin)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// What can go wrong revoking a session by id — distinct from
/// [`StoreError`] because the non-enumeration rule on this route is
/// load-bearing: a session that exists but belongs to someone else, and a
/// session that does not exist at all, must return the **exact same**
/// variant, not just the same status code ([`Self::NotFound`] in both
/// cases). Mapping them onto `StoreError::NotFound` directly would not
/// express that intent — they're deliberately *one* error path, not two
/// that happen to render identically, so they share a single variant.
///
/// `Database` carries the underlying `sqlx::Error` as a `#[source]` for
/// server-side `tracing`; its `Display` is the fixed `"database error"`,
/// never the upstream driver's text (AGENTS.md: "upstream error text
/// never reaches a response"). The route's `?` turns both variants into
/// `ApiError` via the existing `From<StoreError> for ApiError` impl —
/// `NotFound` → `ApiError::NotFound("record not found")` (404),
/// `Database` → `ApiError::Internal("database error")` (500) — so the
/// route never has to know about this enum's variants directly.
#[derive(Debug, Error)]
pub enum SessionOwnershipError {
    /// The session id does not exist, OR it exists but belongs to a
    /// different caller (and `is_admin` is `false`). Same variant on
    /// purpose — see the type doc comment.
    #[error("session not found")]
    NotFound,
    /// Any storage failure. Never renders the upstream error text — see
    /// the type doc comment.
    #[error("database error")]
    Database(#[source] sqlx::Error),
}

impl From<sqlx::Error> for SessionOwnershipError {
    fn from(err: sqlx::Error) -> Self {
        // No `RowNotFound`-to-`NotFound` mapping here: this is an UPDATE,
        // which never produces `RowNotFound`. Every sqlx error here is a
        // real storage failure, so they all collapse to `Database`.
        Self::Database(err)
    }
}

impl From<SessionOwnershipError> for StoreError {
    fn from(err: SessionOwnershipError) -> Self {
        match err {
            SessionOwnershipError::NotFound => Self::NotFound,
            SessionOwnershipError::Database(e) => Self::Database(e),
        }
    }
}

/// Revoke `session_id` if it belongs to `caller_id`, or unconditionally
/// if `is_admin`. A session that exists but belongs to someone else, and
/// a session that does not exist at all, return the exact same
/// [`SessionOwnershipError::NotFound`] — this route must never reveal
/// whether another user's session id exists. The route's
/// single error-mapping path then renders both as the same 404 body.
///
/// The `WHERE ... AND revoked_at IS NULL` predicate is what makes a
/// second revoke of the same id a [`SessionOwnershipError::NotFound`]:
/// the row's `revoked_at` is now non-`NULL`, so the predicate matches
/// zero rows and `rows_affected() == 0` — the same "no matching row"
/// signal as a foreign id, by construction. This is also what makes a
/// stale `DELETE` from a UI that already cleared its own cookie an
/// `NotFound` rather than a silent no-op success, so the route cannot
/// be used to confirm "yes, the row I knew about is still gone."
///
/// # Errors
///
/// Returns [`SessionOwnershipError::NotFound`] for both a missing id and
/// a foreign id (non-enumeration, see the type doc comment), or
/// [`SessionOwnershipError::Database`] on any storage failure.
pub async fn revoke_session_as_caller(
    pool: &PgPool,
    session_id: Uuid,
    caller_id: Uuid,
    is_admin: bool,
) -> Result<(), SessionOwnershipError> {
    let result = sqlx::query(
        "UPDATE session SET revoked_at = now() \
         WHERE id = $1 AND revoked_at IS NULL AND ($3 OR app_user_id = $2)",
    )
    .bind(session_id)
    .bind(caller_id)
    .bind(is_admin)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(SessionOwnershipError::NotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// The wire shape of [`SessionRow`] never carries
    /// a token or token hash. This is a structural test (no DB), pinning
    /// down what `#[derive(Serialize)]` actually emits rather than what the
    /// struct declaration says: a future field added by name "token" or
    /// "tokenHash" would have to consciously break this test to slip
    /// through, rather than pass it accidentally.
    #[test]
    fn session_row_never_serializes_a_token_field() {
        let row = SessionRow {
            id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            user_name: "Rina".to_owned(),
            created_at: "2026-01-01 00:00:00+00".to_owned(),
            expires_at: "2026-01-02 00:00:00+00".to_owned(),
            created_ip: None,
            user_agent: None,
        };
        let json = serde_json::to_value(&row).unwrap();
        assert!(
            json.get("token").is_none(),
            "SessionRow must never serialize a `token` key"
        );
        assert!(
            json.get("tokenHash").is_none(),
            "SessionRow must never serialize a `tokenHash` key"
        );
        assert!(
            json.get("token_hash").is_none(),
            "SessionRow must never serialize a `token_hash` key"
        );
    }

    /// The honest-`null` shape for `created_ip`/`user_agent`: the keys
    /// are present in the JSON output (matching the camelCase contract)
    /// and carry an explicit JSON `null`, not omitted — a frontend
    /// rendering a placeholder for an absent value can branch on the
    /// key's presence, never on whether the field is "missing or null".
    #[test]
    fn session_row_serializes_absent_created_ip_and_user_agent_as_explicit_null() {
        let row = SessionRow {
            id: "s1".to_owned(),
            user_id: "u1".to_owned(),
            user_name: "Rina".to_owned(),
            created_at: "2026-01-01 00:00:00+00".to_owned(),
            expires_at: "2026-01-02 00:00:00+00".to_owned(),
            created_ip: None,
            user_agent: None,
        };
        let json = serde_json::to_value(&row).unwrap();
        assert!(
            json.get("createdIp").is_some(),
            "`createdIp` key must be present even when null"
        );
        assert!(
            json.get("userAgent").is_some(),
            "`userAgent` key must be present even when null"
        );
        assert_eq!(json.get("createdIp"), Some(&serde_json::Value::Null));
        assert_eq!(json.get("userAgent"), Some(&serde_json::Value::Null));
    }

    /// The non-enumeration guarantee lives in two layers at once: the SQL
    /// (`UPDATE ... AND revoked_at IS NULL AND ($3 OR app_user_id = $2)`)
    /// and the error mapping. The latter is a `Display` test, not a DB
    /// test — `SessionOwnershipError::NotFound`'s message is what
    /// `From<SessionOwnershipError> for StoreError` → `From<StoreError>
    /// for ApiError` chains together to render the route's 404 body,
    /// and any future reader touching that string has to consciously
    /// re-confirm this test (which deliberately pins the message,
    /// because the integration test's byte-equal assertion depends on
    /// both the foreign-UUID arm and the missing-UUID arm rendering the
    /// exact same `Display`).
    #[test]
    fn session_ownership_error_not_found_display_is_a_fixed_string() {
        assert_eq!(
            SessionOwnershipError::NotFound.to_string(),
            "session not found"
        );
    }

    /// The database-error variant must never embed upstream `sqlx` text
    /// in its `Display` — same leak concern
    /// `lakehouse_core::error::ApiError::Internal` and
    /// `lakehouse_auth::AuthError::Database` carry. A raw `sqlx::Error`
    /// can name the connection string / host, which must never reach an
    /// HTTP response body.
    #[test]
    fn session_ownership_error_database_display_never_contains_source_text() {
        let io_err = std::io::Error::other("connect to db.internal:5432 failed");
        let err = SessionOwnershipError::Database(sqlx::Error::Io(io_err));
        assert_eq!(err.to_string(), "database error");
        assert!(!err.to_string().contains("db.internal"));
    }
}
