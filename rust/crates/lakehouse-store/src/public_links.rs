//! Public links to dashboards: one per dashboard, with an expiry, an optional
//! password and the tickets a visitor holds after typing it (`BI-19` part A,
//! migration `0098`, plan decisions D1 to D5; feature page
//! `docs/core/features/sharing-and-delivery.md`, decisions A1 to A8).
//!
//! This module stores and answers; it decides nothing about who may make a
//! link, what an expiry may be, or how a password is hashed. The routes
//! (`lakehouse-api`) own those rules, so the console, the assistant and a
//! test cannot differ. The password arrives here already hashed, and nothing
//! in this module can read one back.
//!
//! # No read-modify-write
//!
//! Every change is one statement on the link's own row. That is the reason the
//! links left the dashboard row (migration `0098`): a link is never carried
//! from an earlier read into a later write, so a dashboard save, a second
//! administrator and an expiry that passes in between cannot bring back a link
//! somebody just ended. [`replace`] and [`update`] both say what they changed
//! and return the row as stored.
//!
//! # Expiry is the database's clock
//!
//! "Live" is `expires_at > now()` in Postgres, in every query, so the API
//! processes cannot disagree about it.

use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::StoreError;

/// How long a visitor's ticket lives, at most. It also never outlives the
/// link (plan D4).
pub const TICKET_HOURS: i32 = 12;

/// Days a link that existed before migration `0098` is given (feature A2).
pub const LEGACY_DAYS: i32 = 30;

/// A stored link.
#[derive(Clone, PartialEq, Eq, FromRow)]
pub struct PublicLink {
    /// The dashboard's id.
    pub board_id: String,
    /// What the URL carries.
    pub token: String,
    /// The link stops working at this time.
    pub expires_at: OffsetDateTime,
    /// The `Argon2id` hash of the password, `None` for a link without one.
    pub password_hash: Option<String>,
    /// Bumped whenever the password changes or is removed.
    pub password_version: i32,
    /// The person who made it, while their account exists.
    pub created_by: Option<Uuid>,
    /// When the row was made (a replacement is a new row).
    pub created_at: OffsetDateTime,
    /// When it last changed.
    pub updated_at: OffsetDateTime,
}

impl std::fmt::Debug for PublicLink {
    /// Neither the token nor the hash is printed: a log line that held this
    /// value would hold a credential.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicLink")
            .field("board_id", &self.board_id)
            .field("expires_at", &self.expires_at)
            .field("has_password", &self.password_hash.is_some())
            .finish_non_exhaustive()
    }
}

impl PublicLink {
    /// Whether the link has a password.
    #[must_use]
    pub fn has_password(&self) -> bool {
        self.password_hash.is_some()
    }
}

/// What a change does to the password.
#[derive(Clone, PartialEq, Eq)]
pub enum PasswordChange {
    /// Leave the password as it is.
    Keep,
    /// Remove it.
    Remove,
    /// Replace it with this `Argon2id` hash.
    Set(String),
}

impl std::fmt::Debug for PasswordChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Keep => f.write_str("Keep"),
            Self::Remove => f.write_str("Remove"),
            Self::Set(_) => f.write_str("Set(..)"),
        }
    }
}

const COLUMNS: &str = "board_id, token, expires_at, password_hash, password_version, created_by, \
     created_at, updated_at";

/// The link of a dashboard, live or not, for the Share dialog.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_for_board(
    pool: &PgPool,
    board_id: &str,
) -> Result<Option<PublicLink>, StoreError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM dashboard_public_link WHERE board_id = $1"
    ))
    .bind(board_id)
    .fetch_optional(pool)
    .await?)
}

/// The link a token names, only while it has not expired. An unknown and an
/// expired token are both `None`: the caller cannot tell them apart (feature
/// A5).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_live_by_token(
    pool: &PgPool,
    token: &str,
) -> Result<Option<PublicLink>, StoreError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {COLUMNS} FROM dashboard_public_link WHERE token = $1 AND expires_at > now()"
    ))
    .bind(token)
    .fetch_optional(pool)
    .await?)
}

/// Every link that has not expired, as `(board_id, token)`, for the
/// dashboard list's "public" mark.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn live_tokens(pool: &PgPool) -> Result<Vec<(String, String)>, StoreError> {
    Ok(
        sqlx::query_as(
            "SELECT board_id, token FROM dashboard_public_link WHERE expires_at > now()",
        )
        .fetch_all(pool)
        .await?,
    )
}

/// Make a new link for a dashboard, ending the one it had (feature A6): the
/// row is replaced, so the old token stops at once and its tickets go with it.
/// `password` is the hash, or `None` for a link without a password.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails (a token that collides
/// with another link's included; the caller draws a new one).
pub async fn replace(
    pool: &PgPool,
    board_id: &str,
    token: &str,
    expires_at: OffsetDateTime,
    password: Option<&str>,
    created_by: Option<Uuid>,
) -> Result<PublicLink, StoreError> {
    // One transaction. BI-19 part A review (NOTE): a DELETE then an INSERT let
    // two simultaneous "new link" calls both pass the delete and the second
    // INSERT fail on the primary key (a 500). The tickets are deleted first
    // (a new link ends them), then the row is written with ON CONFLICT DO
    // UPDATE, so the second caller waits for the first and replaces it. The
    // password version is bumped on replacement, so a ticket issued in the
    // gap names a password that no longer exists.
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM dashboard_public_link_ticket WHERE board_id = $1")
        .bind(board_id)
        .execute(&mut *tx)
        .await?;
    let link = sqlx::query_as(&format!(
        "INSERT INTO dashboard_public_link (board_id, token, expires_at, password_hash, created_by) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (board_id) DO UPDATE SET token = EXCLUDED.token, \
             expires_at = EXCLUDED.expires_at, password_hash = EXCLUDED.password_hash, \
             password_version = dashboard_public_link.password_version + 1, \
             created_by = EXCLUDED.created_by, created_at = now(), updated_at = now() \
         RETURNING {COLUMNS}"
    ))
    .bind(board_id)
    .bind(token)
    .bind(expires_at)
    .bind(password)
    .bind(created_by)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(link)
}

/// Change the expiry and the password of a link that has not expired, keeping
/// its token. `None` when the dashboard has no live link (nothing is written:
/// an expired link is not revived, the caller makes a new one with
/// [`replace`]). A new or removed password bumps `password_version`, which
/// ends every ticket issued before.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn update(
    pool: &PgPool,
    board_id: &str,
    expires_at: OffsetDateTime,
    password: &PasswordChange,
) -> Result<Option<PublicLink>, StoreError> {
    let (change, hash) = match password {
        PasswordChange::Keep => (false, None),
        PasswordChange::Remove => (true, None),
        PasswordChange::Set(hash) => (true, Some(hash.as_str())),
    };
    Ok(sqlx::query_as(&format!(
        "UPDATE dashboard_public_link SET expires_at = $2, \
             password_hash = CASE WHEN $3 THEN $4 ELSE password_hash END, \
             password_version = password_version + CASE WHEN $3 THEN 1 ELSE 0 END, \
             updated_at = now() \
         WHERE board_id = $1 AND expires_at > now() RETURNING {COLUMNS}"
    ))
    .bind(board_id)
    .bind(expires_at)
    .bind(change)
    .bind(hash)
    .fetch_optional(pool)
    .await?)
}

/// End a dashboard's link, tickets included (cascade). `true` when there was
/// one.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn delete(pool: &PgPool, board_id: &str) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM dashboard_public_link WHERE board_id = $1")
        .bind(board_id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

/// Record a visitor's ticket (the SHA-256 of it, never the ticket) for the
/// link of `board_id` as it is now. The ticket lives [`TICKET_HOURS`] hours or
/// until the link expires, whichever is first. `None` when the link has
/// expired, was ended, or its password changed since `password_version` was
/// read: no ticket is issued then.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails.
pub async fn issue_ticket(
    pool: &PgPool,
    board_id: &str,
    password_version: i32,
    ticket_hash: &str,
) -> Result<Option<OffsetDateTime>, StoreError> {
    // Old tickets are removed on the way, so the table never needs a sweep of
    // its own.
    sqlx::query("DELETE FROM dashboard_public_link_ticket WHERE expires_at <= now()")
        .execute(pool)
        .await?;
    Ok(sqlx::query_scalar(
        "INSERT INTO dashboard_public_link_ticket (ticket_hash, board_id, password_version, expires_at) \
         SELECT $3, board_id, password_version, \
                LEAST(now() + make_interval(hours => $4), expires_at) \
         FROM dashboard_public_link \
         WHERE board_id = $1 AND password_version = $2 AND expires_at > now() \
         RETURNING expires_at",
    )
    .bind(board_id)
    .bind(password_version)
    .bind(ticket_hash)
    .bind(TICKET_HOURS)
    .fetch_optional(pool)
    .await?)
}

/// Whether `ticket_hash` is a live ticket for the link of `board_id` as it is
/// now: not expired, and issued for the current password.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn ticket_is_live(
    pool: &PgPool,
    board_id: &str,
    ticket_hash: &str,
) -> Result<bool, StoreError> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM dashboard_public_link_ticket t \
             JOIN dashboard_public_link l ON l.board_id = t.board_id \
             WHERE t.ticket_hash = $1 AND t.board_id = $2 AND t.expires_at > now() \
               AND t.password_version = l.password_version AND l.expires_at > now())",
    )
    .bind(ticket_hash)
    .bind(board_id)
    .fetch_one(pool)
    .await?)
}

/// Copy the links that existed before migration `0098` (`(board_id, token)`
/// pairs read from the dashboard rows) into the table, each expiring
/// [`LEGACY_DAYS`] days from now (feature A2). Returns how many were copied.
///
/// It copies once per database: the first call writes the marker row of
/// `dashboard_public_link_copy` in the same transaction as the links, and every
/// later call, from this process or another, copies nothing. A link that a
/// board already has stays as it is (`ON CONFLICT DO NOTHING`).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the write fails; nothing is then
/// copied and the marker is not written, so the next start tries again.
pub async fn copy_legacy(pool: &PgPool, links: &[(String, String)]) -> Result<usize, StoreError> {
    let mut tx = pool.begin().await?;
    let first: Option<bool> = sqlx::query_scalar(
        "INSERT INTO dashboard_public_link_copy (singleton, copied) VALUES (TRUE, 0) \
         ON CONFLICT DO NOTHING RETURNING singleton",
    )
    .fetch_optional(&mut *tx)
    .await?;
    if first.is_none() {
        return Ok(0);
    }
    let mut copied = 0_usize;
    for (board_id, token) in links {
        let done = sqlx::query(
            "INSERT INTO dashboard_public_link (board_id, token, expires_at) \
             VALUES ($1, $2, now() + make_interval(days => $3)) ON CONFLICT DO NOTHING",
        )
        .bind(board_id)
        .bind(token)
        .bind(LEGACY_DAYS)
        .execute(&mut *tx)
        .await?;
        copied += usize::try_from(done.rows_affected()).unwrap_or(0);
    }
    sqlx::query("UPDATE dashboard_public_link_copy SET copied = $1")
        .bind(i32::try_from(copied).unwrap_or(i32::MAX))
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(copied)
}
