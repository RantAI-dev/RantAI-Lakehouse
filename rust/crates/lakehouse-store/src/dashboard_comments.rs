//! Comments on dashboards and the mentions in them (`BI-19` part E, migration
//! `0114`, plan decision D15; feature page
//! `docs/core/features/sharing-and-delivery.md`, E1 to E3).
//!
//! This module stores and answers; it decides nothing about who may read,
//! write, edit or delete. The routes (`lakehouse-api`) own those rules, because
//! they depend on the dashboard's access levels, which live elsewhere. The one
//! rule kept here is the one the database can keep: the length of the text
//! (`CHECK` in the migration, and [`MAX_CHARS`] for the route's sentence).
//!
//! A comment is plain text. Nothing here parses or renders it; the console
//! shows it as text and the notification email never contains it.
//!
//! # Errors
//!
//! Every function returns [`StoreError::Database`] when its query fails.

use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::StoreError;

/// The longest comment, in characters (the migration's `CHECK`).
pub const MAX_CHARS: usize = 2000;

/// How many comments one dashboard's panel reads (the newest).
pub const PANEL_LIMIT: i64 = 200;

/// How many unread mentions the bell lists.
pub const BELL_LIMIT: i64 = 50;

/// A live comment with its author's current name.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Comment {
    /// The comment's id.
    pub id: Uuid,
    /// The dashboard it is on.
    pub board_id: String,
    /// The author, while their account exists.
    pub author_id: Option<Uuid>,
    /// The author's name, while their account exists.
    pub author_name: Option<String>,
    /// The text.
    pub body: String,
    /// When it was posted.
    pub created_at: OffsetDateTime,
    /// When its text last changed.
    pub edited_at: Option<OffsetDateTime>,
}

/// An unread mention, for the bell.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Mention {
    /// The comment that mentions the reader.
    pub comment_id: Uuid,
    /// The dashboard it is on.
    pub board_id: String,
    /// The author's name, while their account exists.
    pub author_name: Option<String>,
    /// When the comment was posted.
    pub created_at: OffsetDateTime,
}

const COMMENT_COLUMNS: &str = "c.id, c.board_id, c.author_id, u.name AS author_name, c.body, \
     c.created_at, c.edited_at";

/// The newest [`PANEL_LIMIT`] live comments of a dashboard, oldest first.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list(pool: &PgPool, board_id: &str) -> Result<Vec<Comment>, StoreError> {
    // safe: only the constant `COMMENT_COLUMNS` is formatted in; values are bound (SEC-41).
    let mut rows: Vec<Comment> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COMMENT_COLUMNS} FROM dashboard_comment c \
         LEFT JOIN app_user u ON u.id = c.author_id \
         WHERE c.board_id = $1 AND c.deleted_at IS NULL \
         ORDER BY c.created_at DESC, c.id DESC LIMIT $2"
    )))
    .bind(board_id)
    .bind(PANEL_LIMIT)
    .fetch_all(pool)
    .await?;
    rows.reverse();
    Ok(rows)
}

/// One live comment; a deleted or unknown id is `None`.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool, id: Uuid) -> Result<Option<Comment>, StoreError> {
    // safe: only the constant `COMMENT_COLUMNS` is formatted in; values are bound (SEC-41).
    Ok(sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {COMMENT_COLUMNS} FROM dashboard_comment c \
         LEFT JOIN app_user u ON u.id = c.author_id \
         WHERE c.id = $1 AND c.deleted_at IS NULL"
    )))
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

/// The people mentioned in each of `comment_ids`, as `(comment_id, user_id,
/// name)`.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn mentions_of(
    pool: &PgPool,
    comment_ids: &[Uuid],
) -> Result<Vec<(Uuid, Uuid, String)>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT m.comment_id, m.user_id, u.name FROM dashboard_comment_mention m \
         JOIN app_user u ON u.id = m.user_id WHERE m.comment_id = ANY($1) ORDER BY u.name",
    )
    .bind(comment_ids)
    .fetch_all(pool)
    .await?)
}

/// Post a comment with its mentions, in one transaction.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn insert(
    pool: &PgPool,
    board_id: &str,
    author: Uuid,
    body: &str,
    mentions: &[Uuid],
) -> Result<Uuid, StoreError> {
    let mut tx = pool.begin().await?;
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO dashboard_comment (board_id, author_id, body) VALUES ($1, $2, $3) \
         RETURNING id",
    )
    .bind(board_id)
    .bind(author)
    .bind(body)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO dashboard_comment_mention (comment_id, user_id) \
         SELECT $1, u FROM unnest($2::uuid[]) AS u ON CONFLICT DO NOTHING",
    )
    .bind(id)
    .bind(mentions)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Replace a live comment's text and mentions, in one transaction. Returns the
/// people newly mentioned (so only they are notified), or `None` when the
/// comment is deleted or unknown. A person who stays mentioned keeps their
/// `read_at`.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn update(
    pool: &PgPool,
    id: Uuid,
    body: &str,
    mentions: &[Uuid],
) -> Result<Option<Vec<Uuid>>, StoreError> {
    let mut tx = pool.begin().await?;
    let changed = sqlx::query(
        "UPDATE dashboard_comment SET body = $2, edited_at = now() \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(body)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed == 0 {
        return Ok(None);
    }
    sqlx::query(
        "DELETE FROM dashboard_comment_mention \
         WHERE comment_id = $1 AND NOT (user_id = ANY($2))",
    )
    .bind(id)
    .bind(mentions)
    .execute(&mut *tx)
    .await?;
    let added: Vec<(Uuid,)> = sqlx::query_as(
        "INSERT INTO dashboard_comment_mention (comment_id, user_id) \
         SELECT $1, u FROM unnest($2::uuid[]) AS u ON CONFLICT DO NOTHING RETURNING user_id",
    )
    .bind(id)
    .bind(mentions)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(added.into_iter().map(|(u,)| u).collect()))
}

/// Delete a live comment (the row stays, with who deleted it and when).
/// Returns whether one was deleted.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn soft_delete(pool: &PgPool, id: Uuid, by: Uuid) -> Result<bool, StoreError> {
    Ok(sqlx::query(
        "UPDATE dashboard_comment SET deleted_at = now(), deleted_by = $2 \
         WHERE id = $1 AND deleted_at IS NULL",
    )
    .bind(id)
    .bind(by)
    .execute(pool)
    .await?
    .rows_affected()
        > 0)
}

/// Remove every comment of a dashboard that was purged for good (the trash
/// sweep). Returns how many rows went.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn delete_for_board(pool: &PgPool, board_id: &str) -> Result<u64, StoreError> {
    Ok(
        sqlx::query("DELETE FROM dashboard_comment WHERE board_id = $1")
            .bind(board_id)
            .execute(pool)
            .await?
            .rows_affected(),
    )
}

/// The unread mentions of a person, newest first, at most [`BELL_LIMIT`], for
/// comments that are still live. The caller re-checks the reader's access to
/// each dashboard before showing a line.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn unread_mentions(pool: &PgPool, user: Uuid) -> Result<Vec<Mention>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT c.id AS comment_id, c.board_id, u.name AS author_name, c.created_at \
         FROM dashboard_comment_mention m \
         JOIN dashboard_comment c ON c.id = m.comment_id \
         LEFT JOIN app_user u ON u.id = c.author_id \
         WHERE m.user_id = $1 AND m.read_at IS NULL AND c.deleted_at IS NULL \
         ORDER BY c.created_at DESC LIMIT $2",
    )
    .bind(user)
    .bind(BELL_LIMIT)
    .fetch_all(pool)
    .await?)
}

/// Mark every unread mention of a person as read. Returns how many.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn mark_mentions_read(pool: &PgPool, user: Uuid) -> Result<u64, StoreError> {
    Ok(sqlx::query(
        "UPDATE dashboard_comment_mention SET read_at = now() \
         WHERE user_id = $1 AND read_at IS NULL",
    )
    .bind(user)
    .execute(pool)
    .await?
    .rows_affected())
}
