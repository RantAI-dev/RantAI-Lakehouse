//! Uploaded maps (`BI-16` part C2b, migration `0100`).
//!
//! A row is a checked, simplified `GeoJSON` file (`lakehouse_bi::geo`) with the
//! name an administrator gave it. This module stores and reads rows and knows
//! nothing about `GeoJSON`: the route validates and simplifies, the table's
//! `CHECK`s are the backstop for shape. A name already taken (case ignored)
//! is [`StoreError::Conflict`].

use time::OffsetDateTime;

use crate::{PgPool, StoreError};

/// A map as the list shows it (no file).
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct GeoMap {
    /// `m_` and 12 hex digits.
    pub id: String,
    /// The label in the map picker.
    pub name: String,
    /// The property of the uploaded file that held the region names.
    pub name_property: String,
    /// How many regions the map has.
    pub region_count: i32,
    /// Display name of the uploader.
    pub created_by: String,
    /// When it was uploaded.
    pub created_at: OffsetDateTime,
    /// Whether the data engine holds the map's region lookup (its polygon
    /// dictionary was created and loaded): what `RegionOf` needs (migration `0102`).
    pub region_lookup: bool,
}

/// A map to store.
#[derive(Debug, Clone)]
pub struct NewGeoMap<'a> {
    /// `m_` and 12 hex digits, made by the caller.
    pub id: &'a str,
    /// The label.
    pub name: &'a str,
    /// The property that held the region names.
    pub name_property: &'a str,
    /// The number of regions.
    pub region_count: i32,
    /// The simplified file, at most 5 MB.
    pub geojson: &'a [u8],
    /// Display name of the uploader.
    pub created_by: &'a str,
}

const COLS: &str = "id, name, name_property, region_count, created_by, created_at, region_lookup";

/// Store a map and return it.
///
/// # Errors
///
/// [`StoreError::Conflict`] when the name is taken, [`StoreError::Database`]
/// otherwise (including a `CHECK` refusing the values).
pub async fn insert(pool: &PgPool, map: &NewGeoMap<'_>) -> Result<GeoMap, StoreError> {
    let sql = format!(
        "INSERT INTO geo_map (id, name, name_property, region_count, geojson, created_by) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {COLS}"
    );
    Ok(sqlx::query_as(&sql)
        .bind(map.id)
        .bind(map.name)
        .bind(map.name_property)
        .bind(map.region_count)
        .bind(map.geojson)
        .bind(map.created_by)
        .fetch_one(pool)
        .await?)
}

/// Every map, oldest first (the order the console lists them).
///
/// # Errors
///
/// [`StoreError::Database`] on a database failure.
pub async fn list(pool: &PgPool) -> Result<Vec<GeoMap>, StoreError> {
    let sql = format!("SELECT {COLS} FROM geo_map ORDER BY created_at, id");
    Ok(sqlx::query_as(&sql).fetch_all(pool).await?)
}

/// One map without its file, or `None`.
///
/// # Errors
///
/// [`StoreError::Database`] on a database failure.
pub async fn get(pool: &PgPool, id: &str) -> Result<Option<GeoMap>, StoreError> {
    let sql = format!("SELECT {COLS} FROM geo_map WHERE id = $1");
    Ok(sqlx::query_as(&sql).bind(id).fetch_optional(pool).await?)
}

/// A map's simplified file and a tag that changes when the file does.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct GeoFile {
    /// The `GeoJSON` text.
    pub geojson: Vec<u8>,
    /// `md5` of the file as Postgres computes it, for an `ETag`.
    pub etag: String,
}

/// The file of one map, or `None`.
///
/// # Errors
///
/// [`StoreError::Database`] on a database failure.
pub async fn file(pool: &PgPool, id: &str) -> Result<Option<GeoFile>, StoreError> {
    Ok(
        sqlx::query_as("SELECT geojson, md5(geojson) AS etag FROM geo_map WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Rename a map and return it, or `None` for an unknown id.
///
/// # Errors
///
/// [`StoreError::Conflict`] when the name is taken, [`StoreError::Database`]
/// otherwise.
pub async fn rename(pool: &PgPool, id: &str, name: &str) -> Result<Option<GeoMap>, StoreError> {
    let sql = format!("UPDATE geo_map SET name = $2 WHERE id = $1 RETURNING {COLS}");
    Ok(sqlx::query_as(&sql)
        .bind(id)
        .bind(name)
        .fetch_optional(pool)
        .await?)
}

/// Record whether the map's region lookup exists on the engine.
///
/// # Errors
///
/// [`StoreError::Database`] on a database failure.
pub async fn set_region_lookup(pool: &PgPool, id: &str, ready: bool) -> Result<(), StoreError> {
    sqlx::query("UPDATE geo_map SET region_lookup = $2 WHERE id = $1")
        .bind(id)
        .bind(ready)
        .execute(pool)
        .await?;
    Ok(())
}

/// Delete a map; whether a row was removed.
///
/// # Errors
///
/// [`StoreError::Database`] on a database failure.
pub async fn delete(pool: &PgPool, id: &str) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM geo_map WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}
