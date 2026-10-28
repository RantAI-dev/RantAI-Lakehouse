//! Column-level lineage (`DATA-13`, migration `0079`): which source column
//! feeds which output column, recorded by the producers that know it
//! (an authored pipeline run, a Silver or Gold build read from the query log,
//! a saved dashboard SQL source) and read back up or down the mappings.
//!
//! # Replace a producer as a set
//!
//! [`replace_producer`] deletes every row of one `(producer_kind,
//! producer_ref)` and inserts the new set in one transaction, so a changed
//! statement never leaves its old mapping behind and a reader never sees a
//! half-written set.
//!
//! # Reading follows mappings, never guesses
//!
//! [`walk`] follows `from -> to` rows up to [`MAX_HOPS`] hops. A partial row
//! with no source column ends the walk at that row; it is returned (with its
//! reason) rather than dropped, because the reader must be told what could
//! not be mapped.
//!
//! # Who may see a row
//!
//! A row is either shared (`tenant_id` NULL: builds read from the one
//! per-deployment query log) or a tenant's. [`Scope`] says which the caller
//! may read; the handler fills it from the same rules as the table-level
//! lineage route.

use sqlx::{FromRow, PgConnection};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{PgPool, StoreError};

/// Most hops a walk follows from the starting column.
pub const MAX_HOPS: i32 = 10;
/// Most rows one walk returns, so a hub column cannot return the warehouse.
pub const MAX_WALK_ROWS: i64 = 2000;
// PR #106 review R1 SHOULD-FIX DATA-13 4
/// Most rows one producer may hold; a statement with more output columns
/// than this is refused by the store before it writes any row.
pub const MAX_ROWS_PER_PRODUCER: usize = 5000;
/// How long a row is kept, in days (the spec's retention).
pub const RETENTION_DAYS: i32 = 365;

/// One mapping to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMapping {
    /// Source table key, or `None` when none could be resolved.
    pub from_table: Option<String>,
    /// Source column, or `None` when none could be resolved.
    pub from_column: Option<String>,
    /// Output table key.
    pub to_table: String,
    /// Output column.
    pub to_column: String,
    /// `copy`, `cast`, `expression` or `unknown`.
    pub transform: String,
    /// `true` when the mapping is incomplete; `reason` then says why.
    pub partial: bool,
    /// Why the mapping is partial.
    pub reason: Option<String>,
}

/// One stored mapping.
#[derive(Debug, Clone, FromRow)]
pub struct MappingRow {
    /// Row id.
    pub id: i64,
    /// `pipeline`, `build` or `sql_source`.
    pub producer_kind: String,
    /// Pipeline id, target table or source id.
    pub producer_ref: String,
    /// Source table key.
    pub from_table: Option<String>,
    /// Source column.
    pub from_column: Option<String>,
    /// Output table key.
    pub to_table: String,
    /// Output column.
    pub to_column: String,
    /// `copy`, `cast`, `expression` or `unknown`.
    pub transform: String,
    /// `true` when incomplete.
    pub partial: bool,
    /// Why it is partial.
    pub reason: Option<String>,
    /// When the producer last wrote it.
    pub recorded_at: OffsetDateTime,
    /// Distance from the starting column (1 = directly adjacent).
    pub hop: i32,
}

/// Which rows a caller may read.
#[derive(Debug, Clone, Copy)]
pub struct Scope {
    /// The caller's tenant; `None` with `all_tenants` for a platform admin.
    pub tenant: Option<Uuid>,
    /// Read every tenant's rows.
    pub all_tenants: bool,
    /// Read the shared rows (`tenant_id` NULL).
    pub shared: bool,
    /// Read the rows of dashboard SQL sources (`producer_kind` `sql_source`).
    /// A source is a dashboard object, listed to `dashboard:read` holders
    /// only, so its mappings are hidden from everyone else; a walk neither
    /// returns nor follows a hidden row.
    pub dashboards: bool,
}

/// Direction of a [`walk`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From an output column back to the columns it comes from.
    Up,
    /// From a source column to the columns built from it.
    Down,
}

/// Replace every row of one producer with `rows`, in one transaction.
///
/// An empty `rows` deletes the producer's rows.
///
/// # Errors
///
/// [`StoreError::Validation`] if `rows` is longer than
/// [`MAX_ROWS_PER_PRODUCER`]; [`StoreError::Database`] on any other failure
/// (including a row the table's `CHECK`s refuse).
pub async fn replace_producer(
    pool: &PgPool,
    producer_kind: &str,
    producer_ref: &str,
    tenant_id: Option<Uuid>,
    rows: &[NewMapping],
) -> Result<(), StoreError> {
    if rows.len() > MAX_ROWS_PER_PRODUCER {
        return Err(StoreError::Validation(format!(
            "a statement may map at most {MAX_ROWS_PER_PRODUCER} columns"
        )));
    }
    let mut tx = pool.begin().await?;
    // Two writers of one producer (a retried success event, a re-save) must
    // not both insert after each other's delete: serialize them per producer.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
        .bind(format!("column_lineage:{producer_kind}:{producer_ref}"))
        .execute(&mut *tx)
        .await?;
    delete_in(&mut tx, producer_kind, producer_ref).await?;
    if !rows.is_empty() {
        let from_table: Vec<Option<String>> = rows.iter().map(|r| r.from_table.clone()).collect();
        let from_column: Vec<Option<String>> = rows.iter().map(|r| r.from_column.clone()).collect();
        let to_table: Vec<String> = rows.iter().map(|r| r.to_table.clone()).collect();
        let to_column: Vec<String> = rows.iter().map(|r| r.to_column.clone()).collect();
        let transform: Vec<String> = rows.iter().map(|r| r.transform.clone()).collect();
        let partial: Vec<bool> = rows.iter().map(|r| r.partial).collect();
        let reason: Vec<Option<String>> = rows.iter().map(|r| r.reason.clone()).collect();
        sqlx::query(
            "INSERT INTO column_lineage (producer_kind, producer_ref, tenant_id, from_table, \
             from_column, to_table, to_column, transform, partial, reason) \
             SELECT $1::text, $2::text, $3::uuid, ft, fc, tt, tc, tr, pa, re \
             FROM UNNEST($4::text[], $5::text[], $6::text[], $7::text[], $8::text[], \
                         $9::bool[], $10::text[]) AS t(ft, fc, tt, tc, tr, pa, re)",
        )
        .bind(producer_kind)
        .bind(producer_ref)
        .bind(tenant_id)
        .bind(&from_table)
        .bind(&from_column)
        .bind(&to_table)
        .bind(&to_column)
        .bind(&transform)
        .bind(&partial)
        .bind(&reason)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

async fn delete_in(
    conn: &mut PgConnection,
    producer_kind: &str,
    producer_ref: &str,
) -> Result<u64, StoreError> {
    let done =
        sqlx::query("DELETE FROM column_lineage WHERE producer_kind = $1 AND producer_ref = $2")
            .bind(producer_kind)
            .bind(producer_ref)
            .execute(conn)
            .await?;
    Ok(done.rows_affected())
}

/// Delete every row of one producer (a deleted SQL source), returning the
/// number removed.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn delete_producer(
    pool: &PgPool,
    producer_kind: &str,
    producer_ref: &str,
) -> Result<u64, StoreError> {
    let mut conn = pool.acquire().await?;
    delete_in(&mut conn, producer_kind, producer_ref).await
}

/// Follow mappings from `(table, column)` up or down, at most [`MAX_HOPS`]
/// hops and [`MAX_WALK_ROWS`] rows, nearest first. Each row appears once, at
/// its smallest hop.
///
/// Rows outside `scope` are neither returned nor followed.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn walk(
    pool: &PgPool,
    table: &str,
    column: &str,
    direction: Direction,
    scope: Scope,
) -> Result<Vec<MappingRow>, StoreError> {
    // The two directions differ only in which end is matched and which end
    // the walk continues from; both are constants, never caller text.
    let (match_table, match_column, next_table, next_column) = match direction {
        Direction::Up => ("to_table", "to_column", "from_table", "from_column"),
        Direction::Down => ("from_table", "from_column", "to_table", "to_column"),
    };
    let sql = format!(
        "WITH RECURSIVE visible AS ( \
             SELECT * FROM column_lineage \
             WHERE ((tenant_id IS NULL AND $3) \
                OR (tenant_id IS NOT NULL AND ($4 OR tenant_id = $5))) \
               AND (producer_kind <> 'sql_source' OR $6) \
         ), walk AS ( \
             SELECT v.id, 1 AS hop, v.{next_table} AS nt, v.{next_column} AS nc \
             FROM visible v WHERE v.{match_table} = $1 AND v.{match_column} = $2 \
           UNION \
             SELECT v.id, w.hop + 1, v.{next_table}, v.{next_column} \
             FROM walk w JOIN visible v \
               ON v.{match_table} = w.nt AND v.{match_column} = w.nc \
             WHERE w.hop < {MAX_HOPS} \
         ) \
         SELECT l.id, l.producer_kind, l.producer_ref, l.from_table, l.from_column, \
                l.to_table, l.to_column, l.transform, l.partial, l.reason, l.recorded_at, \
                MIN(w.hop) AS hop \
         FROM walk w JOIN column_lineage l ON l.id = w.id \
         GROUP BY l.id ORDER BY MIN(w.hop), l.id LIMIT {MAX_WALK_ROWS}"
    );
    // safe: the table/column names come from `match direction` over constants, plus
    // `MAX_HOPS`/`MAX_WALK_ROWS`; values are bound.
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(table)
        .bind(column)
        .bind(scope.shared)
        .bind(scope.all_tenants)
        .bind(scope.tenant)
        .bind(scope.dashboards)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

/// Every row that touches `table` as source or output, nearest-first
/// ordering not implied; for the table-level route's `columnMappings`.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn touching_table(
    pool: &PgPool,
    table: &str,
    scope: Scope,
) -> Result<Vec<MappingRow>, StoreError> {
    // safe: only the constant `MAX_WALK_ROWS` is formatted in; values are bound.
    let rows = sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT id, producer_kind, producer_ref, from_table, from_column, to_table, to_column, \
                transform, partial, reason, recorded_at, 1 AS hop \
         FROM column_lineage \
         WHERE (from_table = $1 OR to_table = $1) \
           AND ((tenant_id IS NULL AND $2) OR (tenant_id IS NOT NULL AND ($3 OR tenant_id = $4))) \
           AND (producer_kind <> 'sql_source' OR $5) \
         ORDER BY to_table, to_column, id LIMIT {MAX_WALK_ROWS}"
    )))
    .bind(table)
    .bind(scope.shared)
    .bind(scope.all_tenants)
    .bind(scope.tenant)
    .bind(scope.dashboards)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// The oldest `recorded_at` among the rows `scope` may read, or `None` when
/// there are none; the answer's "kept since" line.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn oldest_recorded_at(
    pool: &PgPool,
    scope: Scope,
) -> Result<Option<OffsetDateTime>, StoreError> {
    let oldest: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT MIN(recorded_at) FROM column_lineage \
         WHERE ((tenant_id IS NULL AND $1) OR (tenant_id IS NOT NULL AND ($2 OR tenant_id = $3))) \
           AND (producer_kind <> 'sql_source' OR $4)",
    )
    .bind(scope.shared)
    .bind(scope.all_tenants)
    .bind(scope.tenant)
    .bind(scope.dashboards)
    .fetch_one(pool)
    .await?;
    Ok(oldest)
}

/// Delete rows older than [`RETENTION_DAYS`], returning how many.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn purge_expired(pool: &PgPool) -> Result<u64, StoreError> {
    // safe: only the constant `RETENTION_DAYS` is formatted in; values are bound.
    let done = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DELETE FROM column_lineage WHERE recorded_at < now() - interval '{RETENTION_DAYS} days'"
    )))
    .execute(pool)
    .await?;
    Ok(done.rows_affected())
}

/// Remember the SQL the API approved for a pipeline run, replacing any
/// earlier one for the same pipeline.
///
/// # Errors
///
/// [`StoreError::Database`] if the write fails.
pub async fn put_pending(pool: &PgPool, pipeline_id: &str, sql: &str) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO column_lineage_pending (pipeline_id, sql) VALUES ($1, $2) \
         ON CONFLICT (pipeline_id) DO UPDATE SET sql = EXCLUDED.sql, enforced_at = now()",
    )
    .bind(pipeline_id)
    .bind(sql)
    .execute(pool)
    .await?;
    Ok(())
}

/// The SQL waiting for a pipeline's run to finish, if any.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn get_pending(pool: &PgPool, pipeline_id: &str) -> Result<Option<String>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT sql FROM column_lineage_pending WHERE pipeline_id = $1")
            .bind(pipeline_id)
            .fetch_optional(pool)
            .await?,
    )
}

/// Forget a pipeline's waiting SQL.
///
/// # Errors
///
/// [`StoreError::Database`] if the delete fails.
pub async fn clear_pending(pool: &PgPool, pipeline_id: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM column_lineage_pending WHERE pipeline_id = $1")
        .bind(pipeline_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// The newest query-log time the build pass has mapped, or `None` if it has
/// never run.
///
/// # Errors
///
/// [`StoreError::Database`] if the query fails.
pub async fn builds_cursor(pool: &PgPool) -> Result<Option<OffsetDateTime>, StoreError> {
    Ok(
        sqlx::query_scalar("SELECT builds_processed_until FROM column_lineage_state")
            .fetch_optional(pool)
            .await?,
    )
}

/// Move the build pass's cursor.
///
/// # Errors
///
/// [`StoreError::Database`] if the write fails.
pub async fn set_builds_cursor(pool: &PgPool, until: OffsetDateTime) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO column_lineage_state (singleton, builds_processed_until) VALUES (TRUE, $1) \
         ON CONFLICT (singleton) DO UPDATE SET builds_processed_until = EXCLUDED.builds_processed_until",
    )
    .bind(until)
    .execute(pool)
    .await?;
    Ok(())
}
