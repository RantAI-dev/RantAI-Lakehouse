//! Console-only catalog metadata (migration `0032`): owner, steward, tags,
//! and description for one catalog asset, keyed by the asset's own id.
//!
//! `asset_id` holds the FULL id `GET /api/catalog/{id}` is called with
//! verbatim — not a re-derived table name. See `0032_asset_annotation.sql`'s
//! why-header for the real id shapes `routes/catalog.rs` emits per layer
//! (a bare slug for Bronze, `silver.<name>`, `serving.<name>`).
//!
//! `asset_annotation`'s own CHECK constraints (`asset_id` <= 200 chars,
//! `owner`/`steward` <= 128, `description` <= 4,000, `tags` <= 20 entries
//! each 1-64 chars matching `^[a-z0-9][a-z0-9_-]*$`) are the last line of
//! defense against unbounded input from a `catalog:write` holder; the
//! `PUT /api/catalog/{id}/annotation` handler is expected to validate the
//! same bounds first and return `400` naming the offending field, so a
//! CHECK violation here surfaces as the generic [`StoreError::Database`]
//! rather than a dedicated variant.

use sqlx::FromRow;

use crate::{PgPool, StoreError};

/// Everything [`upsert_annotation`] needs to create or replace one asset's
/// annotation. Mirrors [`AnnotationRow`], minus `updated_at`
/// (server-assigned).
#[derive(Debug, Clone)]
pub struct AnnotationInput {
    /// The full catalog id, e.g. a bare Bronze slug or `"silver.<name>"`/
    /// `"serving.<name>"`. Must be at most 200 characters.
    pub asset_id: String,
    /// Free-text owner, at most 128 characters.
    pub owner: Option<String>,
    /// Free-text steward, at most 128 characters.
    pub steward: Option<String>,
    /// At most 20 tags, each 1-64 characters matching
    /// `^[a-z0-9][a-z0-9_-]*$`.
    pub tags: Vec<String>,
    /// Free-text description, at most 4,000 characters.
    pub description: Option<String>,
}

/// One row of `asset_annotation`. Uniquely keyed by `asset_id`.
#[derive(Debug, Clone, FromRow)]
pub struct AnnotationRow {
    /// The full catalog id this annotation belongs to.
    pub asset_id: String,
    /// Free-text owner, or `None` if not set.
    pub owner: Option<String>,
    /// Free-text steward, or `None` if not set.
    pub steward: Option<String>,
    /// This asset's tags, possibly empty.
    pub tags: Vec<String>,
    /// Free-text description, or `None` if not set.
    pub description: Option<String>,
}

/// Create this asset's annotation, or replace it if one already exists for
/// `input.asset_id`.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the insert/update fails — including
/// when `asset_annotation`'s CHECK constraints refuse the row (an
/// oversized field, too many tags, or a tag that doesn't match
/// `^[a-z0-9][a-z0-9_-]*$`).
pub async fn upsert_annotation(pool: &PgPool, input: &AnnotationInput) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO asset_annotation \
         (asset_id, owner, steward, tags, description, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT (asset_id) DO UPDATE SET \
           owner = EXCLUDED.owner, \
           steward = EXCLUDED.steward, \
           tags = EXCLUDED.tags, \
           description = EXCLUDED.description, \
           updated_at = now()",
    )
    .bind(&input.asset_id)
    .bind(&input.owner)
    .bind(&input.steward)
    .bind(&input.tags)
    .bind(&input.description)
    .execute(pool)
    .await?;
    Ok(())
}

/// Fetch one asset's annotation, if one has been set.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get_annotation(
    pool: &PgPool,
    asset_id: &str,
) -> Result<Option<AnnotationRow>, StoreError> {
    let row = sqlx::query_as(
        "SELECT asset_id, owner, steward, tags, description \
         FROM asset_annotation WHERE asset_id = $1",
    )
    .bind(asset_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

/// Fetch every asset's annotation — used by `GET /api/catalog?q=` (WS2 §13)
/// to widen the free-text search to annotation `description`/`tags`
/// without one round trip per asset.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn list_all(pool: &PgPool) -> Result<Vec<AnnotationRow>, StoreError> {
    let rows =
        sqlx::query_as("SELECT asset_id, owner, steward, tags, description FROM asset_annotation")
            .fetch_all(pool)
            .await?;
    Ok(rows)
}
