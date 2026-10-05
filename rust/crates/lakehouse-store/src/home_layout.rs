//! Per-user Home page layout (migration `0056`).
//!
//! One row per `owner`, the same key Copilot sessions are scoped to (the
//! signed-in principal's id as a UUID string). Every function takes the
//! owner and binds it, so there is no query that reads or writes across
//! owners: that is the whole isolation story, and `tests/home_layout.rs`
//! pins it.
//!
//! The layout is an opaque JSON document here. Its shape is validated by
//! the route (`routes::home` in `lakehouse-api`), which is also where the
//! reason the server does not know the card catalogue is written down; a
//! row with no entry means "the default layout".

use serde_json::Value;

use crate::{PgPool, StoreError};

/// Fetch `owner`'s saved layout, or `None` when they never saved one (or
/// reset to default).
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool, owner: &str) -> Result<Option<Value>, StoreError> {
    let row: Option<(Value,)> = sqlx::query_as("SELECT layout FROM home_layout WHERE owner = $1")
        .bind(owner)
        .fetch_optional(pool)
        .await?;
    Ok(row.map(|(layout,)| layout))
}

/// Save `layout` as `owner`'s layout, replacing any earlier one, and return
/// what is now stored.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert/update fails.
pub async fn upsert(pool: &PgPool, owner: &str, layout: &Value) -> Result<Value, StoreError> {
    let (stored,): (Value,) = sqlx::query_as(
        "INSERT INTO home_layout (owner, layout, updated_at) VALUES ($1, $2, now()) \
         ON CONFLICT (owner) DO UPDATE SET layout = EXCLUDED.layout, updated_at = now() \
         RETURNING layout",
    )
    .bind(owner)
    .bind(layout)
    .fetch_one(pool)
    .await?;
    Ok(stored)
}

/// Remove `owner`'s saved layout so Home falls back to the default. Returns
/// whether a row existed; deleting nothing is not an error, so the route is
/// idempotent.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the delete fails.
pub async fn delete(pool: &PgPool, owner: &str) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM home_layout WHERE owner = $1")
        .bind(owner)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
