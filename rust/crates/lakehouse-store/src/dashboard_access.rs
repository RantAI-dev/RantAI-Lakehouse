//! Who may see which dashboard or folder: the rules and the "only the people
//! listed" marks (`BI-19` part B, migration `0103`, plan decision D6; feature
//! page `docs/core/features/sharing-and-delivery.md`, decisions B1 to B9).
//!
//! This module stores and answers; it decides nothing. Which level a person
//! ends up with (inheritance down the folders, the restriction dropping the
//! starting point, the higher level winning) is computed by
//! `lakehouse_bi::access::AccessMap`, and who may change a rule is checked by
//! the routes, so the console, the assistant and a test cannot differ.
//! `item_kind` and `level` are plain strings here; the table's CHECKs are the
//! closed sets (`board` / `folder`, `view` / `edit` / `manage`), so a value
//! outside them cannot be stored whoever calls.
//!
//! # One write replaces an item's whole state
//!
//! [`replace_item`] deletes the item's rules and mark and inserts the new ones
//! in one transaction. Nobody ever reads a half-replaced list, and a second
//! manager saving at the same time waits for the first and then replaces it
//! (last writer wins, as for every other settings form here).

use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::StoreError;

/// One stored rule: `level` on one item for one person or one role.
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct AccessRule {
    /// `"board"` or `"folder"`.
    pub item_kind: String,
    /// The dashboard's or folder's id (a `ClickHouse` row, so no foreign key).
    pub item_id: String,
    /// The person, when the rule names one.
    pub user_id: Option<Uuid>,
    /// The role, when the rule names one. Exactly one of the two is set.
    pub role_id: Option<Uuid>,
    /// `"view"`, `"edit"` or `"manage"`.
    pub level: String,
    /// Who gave it, while their account exists.
    pub granted_by: Option<Uuid>,
    /// When.
    pub granted_at: OffsetDateTime,
}

/// A rule to store (the caller has checked the level against its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRule {
    /// The person, or `None` when `role_id` is set.
    pub user_id: Option<Uuid>,
    /// The role, or `None` when `user_id` is set.
    pub role_id: Option<Uuid>,
    /// `"view"`, `"edit"` or `"manage"`.
    pub level: String,
}

/// An item marked "only the people listed".
#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
pub struct Restriction {
    /// `"board"` or `"folder"`.
    pub item_kind: String,
    /// The item's id.
    pub item_id: String,
    /// Who marked it, while their account exists.
    pub restricted_by: Option<Uuid>,
    /// When.
    pub restricted_at: OffsetDateTime,
}

const RULE_COLUMNS: &str = "item_kind, item_id, user_id, role_id, level, granted_by, granted_at";

/// The rules stored directly on one item.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn rules_for_item(
    pool: &PgPool,
    item_kind: &str,
    item_id: &str,
) -> Result<Vec<AccessRule>, StoreError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {RULE_COLUMNS} FROM dashboard_access_rule \
         WHERE item_kind = $1 AND item_id = $2 ORDER BY granted_at, user_id, role_id"
    ))
    .bind(item_kind)
    .bind(item_id)
    .fetch_all(pool)
    .await?)
}

/// The rules that name this person or a role they hold (`role_names`, as the
/// principal carries them; role names are unique), on any item: what every
/// dashboard request needs to know about the caller.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn rules_for_grantee(
    pool: &PgPool,
    user_id: Uuid,
    role_names: &[String],
) -> Result<Vec<AccessRule>, StoreError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {RULE_COLUMNS} FROM dashboard_access_rule \
         WHERE user_id = $1 \
            OR role_id IN (SELECT id FROM role WHERE name = ANY($2))"
    ))
    .bind(user_id)
    .bind(role_names)
    .fetch_all(pool)
    .await?)
}

/// The rules on any of these items (a folder's ancestors, say), for the
/// "inherited from" list.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn rules_for_items(
    pool: &PgPool,
    item_kind: &str,
    item_ids: &[String],
) -> Result<Vec<AccessRule>, StoreError> {
    Ok(sqlx::query_as(&format!(
        "SELECT {RULE_COLUMNS} FROM dashboard_access_rule \
         WHERE item_kind = $1 AND item_id = ANY($2) ORDER BY granted_at, user_id, role_id"
    ))
    .bind(item_kind)
    .bind(item_ids)
    .fetch_all(pool)
    .await?)
}

/// Every item marked "only the people listed". The list is short (a person
/// marks an item on purpose) and the walk up the folders needs all of it.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn all_restrictions(pool: &PgPool) -> Result<Vec<Restriction>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT item_kind, item_id, restricted_by, restricted_at FROM dashboard_restriction",
    )
    .fetch_all(pool)
    .await?)
}

/// The mark on one item, if any.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn restriction_for_item(
    pool: &PgPool,
    item_kind: &str,
    item_id: &str,
) -> Result<Option<Restriction>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT item_kind, item_id, restricted_by, restricted_at FROM dashboard_restriction \
         WHERE item_kind = $1 AND item_id = $2",
    )
    .bind(item_kind)
    .bind(item_id)
    .fetch_optional(pool)
    .await?)
}

/// Replace everything stored for one item: its rules and its mark. `restricted`
/// marks it "only the people listed" (by `granted_by`); `false` clears the
/// mark. One transaction.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a write fails (a rule naming neither or
/// both of a user and a role, or a level outside the closed set, is refused by
/// the table's CHECKs; nothing is then changed), [`StoreError::Conflict`] if
/// two rules name the same grantee, and [`StoreError::ForeignKeyViolation`]
/// if a rule names a user or role that does not exist.
pub async fn replace_item(
    pool: &PgPool,
    item_kind: &str,
    item_id: &str,
    rules: &[NewRule],
    restricted: bool,
    granted_by: Option<Uuid>,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM dashboard_access_rule WHERE item_kind = $1 AND item_id = $2")
        .bind(item_kind)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM dashboard_restriction WHERE item_kind = $1 AND item_id = $2")
        .bind(item_kind)
        .bind(item_id)
        .execute(&mut *tx)
        .await?;
    for rule in rules {
        sqlx::query(
            "INSERT INTO dashboard_access_rule \
                 (item_kind, item_id, user_id, role_id, level, granted_by) \
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(item_kind)
        .bind(item_id)
        .bind(rule.user_id)
        .bind(rule.role_id)
        .bind(&rule.level)
        .bind(granted_by)
        .execute(&mut *tx)
        .await?;
    }
    if restricted {
        sqlx::query(
            "INSERT INTO dashboard_restriction (item_kind, item_id, restricted_by) \
             VALUES ($1, $2, $3)",
        )
        .bind(item_kind)
        .bind(item_id)
        .bind(granted_by)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// Drop everything stored for one item, when the item is purged for good.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if a write fails.
pub async fn delete_item(pool: &PgPool, item_kind: &str, item_id: &str) -> Result<(), StoreError> {
    replace_item(pool, item_kind, item_id, &[], false, None).await
}

/// Names for the grantees of a list of rules, for the People tab: `(id, name,
/// email)` for each person that still exists.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn user_names(
    pool: &PgPool,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, String, String)>, StoreError> {
    Ok(
        sqlx::query_as("SELECT id, name, email FROM app_user WHERE id = ANY($1)")
            .bind(ids)
            .fetch_all(pool)
            .await?,
    )
}

/// `(id, name)` for each of these roles that still exists.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn role_names(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<(Uuid, String)>, StoreError> {
    Ok(
        sqlx::query_as("SELECT id, name FROM role WHERE id = ANY($1)")
            .bind(ids)
            .fetch_all(pool)
            .await?,
    )
}

/// Roles whose name contains `needle`, for the picker, at most `limit`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn search_roles(
    pool: &PgPool,
    needle: &str,
    limit: i64,
) -> Result<Vec<(Uuid, String)>, StoreError> {
    let escaped = needle
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    Ok(
        sqlx::query_as("SELECT id, name FROM role WHERE name ILIKE $1 ORDER BY name LIMIT $2")
            .bind(format!("%{escaped}%"))
            .bind(limit)
            .fetch_all(pool)
            .await?,
    )
}
