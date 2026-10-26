//! Governed tags (`DATA-12` part 2, migration `0083`): tag keys an admin
//! defines with the values each may take, and the `key = value` pairs a table
//! carries from them. Free tags (`asset_annotation.tags`) are a separate
//! column and are never read or written here.
//!
//! # Why the rules live here and not in the handlers
//!
//! A Postgres `CHECK` sees one row, so "the value is one the key allows",
//! "at most 20 governed tags per table" and "no value is removed while a
//! table still holds it" cannot be constraints. They are checked in the
//! transaction that writes, under a row lock on the key, so an admin editing
//! a key's values at the same moment as an editor tagging a table cannot
//! leave a table holding a value its key no longer allows. The handlers
//! forward the [`StoreError::Validation`] sentence as a 400; every sentence
//! here names the key and, where useful, the allowed values, and never
//! embeds database text.
//!
//! The shape bounds (1-64 characters of `^[a-z0-9][a-z0-9_-]*$`, at most 50
//! values per key) are checked here first for the same reason and repeated
//! as `CHECK`s in the migration as defense in depth.

use sqlx::{FromRow, PgConnection};
use time::OffsetDateTime;

use crate::{PgPool, StoreError};

/// Longest key or value, in characters (the free-tag bound, `0032`).
pub const WORD_MAX_LEN: usize = 64;
/// Most values one key may allow.
pub const MAX_VALUES_PER_KEY: usize = 50;
/// Most governed tags one table may carry.
pub const MAX_TAGS_PER_ASSET: usize = 20;

/// One tag key and the values it allows. Keyed by `key`; never renamed.
#[derive(Debug, Clone, FromRow)]
pub struct TagKeyRow {
    /// The key, e.g. `data_domain`.
    pub key: String,
    /// The values a table may be given for this key, in the order the admin
    /// wrote them.
    pub allowed_values: Vec<String>,
    /// Display name of whoever defined the key.
    pub created_by: String,
    /// When the key was defined.
    pub created_at: OffsetDateTime,
    /// When the key was last defined or its values last changed.
    pub updated_at: OffsetDateTime,
}

/// One governed tag on one table.
#[derive(Debug, Clone, FromRow)]
pub struct AssetTagRow {
    /// The full catalog id, the same string `asset_annotation` is keyed by.
    pub asset_id: String,
    /// The key this value belongs to.
    pub tag_key: String,
    /// The value, one the key allowed when it was set.
    pub value: String,
    /// Display name of whoever set (or last changed) the value.
    pub set_by: String,
    /// When the value was set or last changed.
    pub set_at: OffsetDateTime,
}

/// `true` if `word` is 1-64 characters matching `^[a-z0-9][a-z0-9_-]*$`:
/// the shape of a free tag, a governed key and a governed value.
#[must_use]
pub fn is_valid_word(word: &str) -> bool {
    if word.is_empty() || word.chars().count() > WORD_MAX_LEN {
        return false;
    }
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_lowercase() || first.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::Validation(message.into())
}

fn validate_key(key: &str) -> Result<(), StoreError> {
    if is_valid_word(key) {
        return Ok(());
    }
    Err(invalid(format!(
        "key must be 1-{WORD_MAX_LEN} characters matching ^[a-z0-9][a-z0-9_-]*$"
    )))
}

fn validate_allowed_values(values: &[String]) -> Result<(), StoreError> {
    if values.is_empty() {
        return Err(invalid("allowedValues must have at least one value"));
    }
    if values.len() > MAX_VALUES_PER_KEY {
        return Err(invalid(format!(
            "allowedValues must have at most {MAX_VALUES_PER_KEY} values"
        )));
    }
    for (i, value) in values.iter().enumerate() {
        if !is_valid_word(value) {
            return Err(invalid(format!(
                "allowedValues: \"{value}\" must be 1-{WORD_MAX_LEN} characters matching ^[a-z0-9][a-z0-9_-]*$"
            )));
        }
        if values[..i].contains(value) {
            return Err(invalid(format!(
                "allowedValues: \"{value}\" is listed more than once"
            )));
        }
    }
    Ok(())
}

const KEY_COLUMNS: &str = "key, allowed_values, created_by, created_at, updated_at";

/// Define a tag key with the values it may take.
///
/// # Errors
///
/// [`StoreError::Validation`] if the key or a value is not 1-64 characters
/// matching `^[a-z0-9][a-z0-9_-]*$`, the list is empty, longer than 50 or
/// repeats a value; [`StoreError::Conflict`] if the key already exists;
/// [`StoreError::Database`] on any other failure.
pub async fn create_key(
    pool: &PgPool,
    key: &str,
    allowed_values: &[String],
    created_by: &str,
) -> Result<TagKeyRow, StoreError> {
    validate_key(key)?;
    validate_allowed_values(allowed_values)?;
    // safe: only the constant `KEY_COLUMNS` is formatted in; values are bound.
    let row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "INSERT INTO governed_tag_key (key, allowed_values, created_by) \
         VALUES ($1, $2, $3) RETURNING {KEY_COLUMNS}"
    )))
    .bind(key)
    .bind(allowed_values)
    .bind(created_by)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

/// Every defined key, ordered by key.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_keys(pool: &PgPool) -> Result<Vec<TagKeyRow>, StoreError> {
    // safe: only the constant `KEY_COLUMNS` is formatted in; values are bound.
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {KEY_COLUMNS} FROM governed_tag_key ORDER BY key"
    )))
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Replace the values `key` allows.
///
/// A value that a table still holds cannot be removed: the refusal names each
/// such value with the number of tables holding it, and nothing changes. The
/// key row is locked for the check, so a table cannot be given a value
/// between the count and the write.
///
/// # Errors
///
/// [`StoreError::Validation`] if a value is malformed, the list is empty,
/// longer than 50 or repeats a value, or if a removed value is still in use;
/// [`StoreError::NotFound`] if the key does not exist;
/// [`StoreError::Database`] on any other failure.
pub async fn update_key_values(
    pool: &PgPool,
    key: &str,
    allowed_values: &[String],
) -> Result<TagKeyRow, StoreError> {
    validate_allowed_values(allowed_values)?;
    let mut tx = pool.begin().await?;
    lock_key(&mut tx, key).await?;
    let in_use: Vec<(String, i64)> = sqlx::query_as(
        "SELECT value, count(*) FROM asset_governed_tag \
         WHERE tag_key = $1 AND NOT (value = ANY($2)) \
         GROUP BY value ORDER BY value",
    )
    .bind(key)
    .bind(allowed_values)
    .fetch_all(&mut *tx)
    .await?;
    if !in_use.is_empty() {
        let parts: Vec<String> = in_use
            .iter()
            .map(|(value, n)| format!("\"{value}\" ({n})"))
            .collect();
        return Err(invalid(format!(
            "values still given to tables cannot be removed from \"{key}\": {}",
            parts.join(", ")
        )));
    }
    // safe: only the constant `KEY_COLUMNS` is formatted in; values are bound.
    let row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "UPDATE governed_tag_key SET allowed_values = $2, updated_at = now() \
         WHERE key = $1 RETURNING {KEY_COLUMNS}"
    )))
    .bind(key)
    .bind(allowed_values)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(row)
}

/// Delete a key nobody uses.
///
/// Refused while any table still has a value for it, with the number of
/// tables in the sentence: a tag is never lost silently. The key row is
/// locked for the count, as in [`update_key_values`].
///
/// # Errors
///
/// [`StoreError::Validation`] if a table still has a value for the key;
/// [`StoreError::NotFound`] if the key does not exist;
/// [`StoreError::Database`] on any other failure.
pub async fn delete_key(pool: &PgPool, key: &str) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    lock_key(&mut tx, key).await?;
    let in_use: i64 =
        sqlx::query_scalar("SELECT count(*) FROM asset_governed_tag WHERE tag_key = $1")
            .bind(key)
            .fetch_one(&mut *tx)
            .await?;
    if in_use > 0 {
        return Err(invalid(format!(
            "\"{key}\" is still given to {in_use} {}; remove it from {} first",
            if in_use == 1 { "table" } else { "tables" },
            if in_use == 1 {
                "that table"
            } else {
                "those tables"
            },
        )));
    }
    sqlx::query("DELETE FROM governed_tag_key WHERE key = $1")
        .bind(key)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// Lock one key row against a concurrent edit; `NotFound` if there is none.
async fn lock_key(conn: &mut PgConnection, key: &str) -> Result<(), StoreError> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT key FROM governed_tag_key WHERE key = $1 FOR UPDATE")
            .bind(key)
            .fetch_optional(conn)
            .await?;
    found.map(|_| ()).ok_or(StoreError::NotFound)
}

/// Replace one table's whole governed set with `tags` (`key`, `value` pairs).
///
/// Same semantics as the free `tags` list: pairs absent from `tags` are
/// removed, an empty `tags` clears the set. Runs in its own transaction; a
/// caller that must write something else atomically with it (the annotation
/// route) uses [`set_asset_tags_in`] on its own.
///
/// # Errors
///
/// As [`set_asset_tags_in`], plus [`StoreError::Database`] if the
/// transaction cannot begin or commit.
pub async fn set_asset_tags(
    pool: &PgPool,
    asset_id: &str,
    tags: &[(String, String)],
    set_by: &str,
) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    set_asset_tags_in(&mut tx, asset_id, tags, set_by).await?;
    tx.commit().await?;
    Ok(())
}

/// [`set_asset_tags`] on a connection the caller has already begun a
/// transaction on; the caller commits.
///
/// Each key row is read under `FOR SHARE`, which a concurrent
/// [`update_key_values`] or [`delete_key`] (`FOR UPDATE`) waits for, so the
/// allowed list checked here is the list the write is made against. A pair
/// whose value is unchanged keeps its `set_by` and `set_at`: saving the form
/// again does not re-attribute someone else's tag to the saver.
///
/// # Errors
///
/// [`StoreError::Validation`] naming the offending key if there are more than
/// 20 pairs, a key appears twice, a key is not defined, or a value is not one
/// the key allows (the sentence lists the allowed values);
/// [`StoreError::Database`] on any other failure.
pub async fn set_asset_tags_in(
    conn: &mut PgConnection,
    asset_id: &str,
    tags: &[(String, String)],
    set_by: &str,
) -> Result<(), StoreError> {
    if tags.len() > MAX_TAGS_PER_ASSET {
        return Err(invalid(format!(
            "governedTags must have at most {MAX_TAGS_PER_ASSET} entries"
        )));
    }
    for (i, (key, _)) in tags.iter().enumerate() {
        if tags[..i].iter().any(|(earlier, _)| earlier == key) {
            return Err(invalid(format!("governedTags: \"{key}\" is given twice")));
        }
    }
    let keys: Vec<String> = tags.iter().map(|(k, _)| k.clone()).collect();
    let values: Vec<String> = tags.iter().map(|(_, v)| v.clone()).collect();

    let defined: Vec<(String, Vec<String>)> = sqlx::query_as(
        "SELECT key, allowed_values FROM governed_tag_key \
         WHERE key = ANY($1) FOR SHARE",
    )
    .bind(&keys)
    .fetch_all(&mut *conn)
    .await?;
    for (key, value) in tags {
        let Some((_, allowed)) = defined.iter().find(|(k, _)| k == key) else {
            return Err(invalid(format!(
                "governedTags: \"{key}\" is not a defined tag key"
            )));
        };
        if !allowed.contains(value) {
            return Err(invalid(format!(
                "governedTags: \"{value}\" is not an allowed value for \"{key}\"; allowed values: {}",
                allowed.join(", ")
            )));
        }
    }

    sqlx::query("DELETE FROM asset_governed_tag WHERE asset_id = $1 AND NOT (tag_key = ANY($2))")
        .bind(asset_id)
        .bind(&keys)
        .execute(&mut *conn)
        .await?;
    sqlx::query(
        "INSERT INTO asset_governed_tag (asset_id, tag_key, value, set_by, set_at) \
         SELECT $1, t.key, t.value, $4, now() FROM unnest($2::text[], $3::text[]) AS t(key, value) \
         ON CONFLICT (asset_id, tag_key) DO UPDATE SET \
           value = EXCLUDED.value, set_by = EXCLUDED.set_by, set_at = EXCLUDED.set_at \
         WHERE asset_governed_tag.value IS DISTINCT FROM EXCLUDED.value",
    )
    .bind(asset_id)
    .bind(&keys)
    .bind(&values)
    .bind(set_by)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

const TAG_COLUMNS: &str = "asset_id, tag_key, value, set_by, set_at";

/// One table's governed tags, ordered by key.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_for_asset(pool: &PgPool, asset_id: &str) -> Result<Vec<AssetTagRow>, StoreError> {
    // safe: only the constant `TAG_COLUMNS` is formatted in; values are bound.
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {TAG_COLUMNS} FROM asset_governed_tag \
         WHERE asset_id = $1 ORDER BY tag_key"
    )))
    .bind(asset_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The governed tags of every table in `asset_ids`, in one query, ordered by
/// table then key: the batch the catalog overlays use so a list of tables is
/// never one query per table.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn list_for_assets(
    pool: &PgPool,
    asset_ids: &[String],
) -> Result<Vec<AssetTagRow>, StoreError> {
    // safe: only the constant `TAG_COLUMNS` is formatted in; values are bound.
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT {TAG_COLUMNS} FROM asset_governed_tag \
         WHERE asset_id = ANY($1) ORDER BY asset_id, tag_key"
    )))
    .bind(asset_ids)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_is_lowercase_alphanumeric_with_inner_underscore_or_hyphen() {
        assert!(is_valid_word("data_domain"));
        assert!(is_valid_word("tier-1"));
        assert!(is_valid_word("0"));
        assert!(!is_valid_word(""));
        assert!(!is_valid_word("Finance"));
        assert!(!is_valid_word("-finance"));
        assert!(!is_valid_word("a b"));
        assert!(!is_valid_word(&"a".repeat(65)));
        assert!(is_valid_word(&"a".repeat(64)));
    }
}
