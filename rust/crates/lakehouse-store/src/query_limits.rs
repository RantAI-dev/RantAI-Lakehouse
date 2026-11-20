//! The time and row limits of charts and Query Studio (`BI-5`, migration
//! `0101`): one install-wide row and, for the time limit only, a row per role.
//!
//! This module stores numbers and knows nothing about what a sensible limit
//! is; the route checks the ranges and says so in a sentence, and the table's
//! `CHECK`s are the backstop for shape. The install row is seeded by the
//! migration, so [`get`] always has an answer; [`QueryLimits::default`] is
//! the same numbers for a database that somehow lacks the row.

use uuid::Uuid;

use crate::{PgPool, StoreError};

/// The install-wide limits, as stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::FromRow)]
pub struct QueryLimits {
    /// How long a chart or Query Studio query may run, in seconds.
    pub timeout_seconds: i32,
    /// Rows a chart reads.
    pub chart_rows: i32,
    /// Points a map shows.
    pub map_points: i32,
    /// Rows Query Studio returns.
    pub studio_rows: i32,
}

impl Default for QueryLimits {
    /// The feature page's defaults (decisions 1 and 2): the column defaults
    /// of migration `0101`, with the Query Studio rows from migration `0116`.
    fn default() -> Self {
        Self {
            timeout_seconds: 60,
            chart_rows: 2000,
            map_points: 5000,
            studio_rows: 64000,
        }
    }
}

/// A role's own time limit.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct RoleLimit {
    /// The role's id.
    pub role_id: Uuid,
    /// The role's name (joined, so a caller can match a principal's roles).
    pub role_name: String,
    /// The role's time limit in seconds.
    pub timeout_seconds: i32,
}

/// The install-wide limits.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn get(pool: &PgPool) -> Result<QueryLimits, StoreError> {
    let row: Option<QueryLimits> = sqlx::query_as(
        "SELECT timeout_seconds, chart_rows, map_points, studio_rows FROM query_limits",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.unwrap_or_default())
}

/// Every role that has a time limit of its own, ordered by role name.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if the query fails.
pub async fn role_limits(pool: &PgPool) -> Result<Vec<RoleLimit>, StoreError> {
    Ok(sqlx::query_as(
        "SELECT q.role_id, r.name AS role_name, q.timeout_seconds \
         FROM query_limits_role q JOIN role r ON r.id = q.role_id ORDER BY r.name",
    )
    .fetch_all(pool)
    .await?)
}

/// What a save replaced: the install-wide limits stored before it and the
/// role limits that had one before it, with the roles' names. The route that
/// saves names the settings that changed by comparing these against the
/// values it saved (plan D3b), so they come out of the save's own
/// transaction, never from a read taken before it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplacedLimits {
    /// The install-wide limits the save replaced. `None` when the row did
    /// not exist; readers treat that shape as [`QueryLimits::default`]
    /// (`get` does), and so does the diff.
    pub install: Option<QueryLimits>,
    /// The role limits the save replaced, ordered by role name.
    pub roles: Vec<RoleLimit>,
}

/// What a save stored: the install-wide limits as they now stand and the
/// role limits as they now stand, with the roles' names. The route that
/// saved answers from these and names what changed by comparing them with
/// the values the save replaced (plan D3b), so they come out of the save's
/// own transaction, never from a read taken after the commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLimits {
    /// The install-wide limits now stored.
    pub install: QueryLimits,
    /// The role limits now stored, ordered by role name.
    pub roles: Vec<RoleLimit>,
}

/// Save the install-wide limits and replace the set of role limits, in one
/// transaction (a role not listed has no limit of its own afterwards), and
/// return what the save replaced together with what it stored. `updated_by`
/// is the saving principal's id.
///
/// Two concurrent saves serialise on the install row: the second waits for
/// the first to commit, so each reports the values the other committed as
/// the ones it replaced (plan D3b), never values read before they changed.
///
/// The stored values are read inside that same transaction, so the caller
/// answers and records from them without reading again after the commit: a
/// read taken later could name another save's values, or fail and leave a
/// committed save with no record of it.
///
/// # Errors
///
/// Returns [`StoreError::ForeignKeyViolation`] when a role id does not exist,
/// and [`StoreError::Database`] if a write fails, including when the table's
/// `CHECK`s refuse a value.
pub async fn save(
    pool: &PgPool,
    install: &QueryLimits,
    roles: &[(Uuid, i32)],
    updated_by: &str,
) -> Result<(ReplacedLimits, StoredLimits), StoreError> {
    let mut tx = pool.begin().await?;
    // Plan D3b: the replaced values are read under `FOR UPDATE`, so two
    // concurrent saves queue here, and a save can never read the row while
    // an uncommitted save beside it holds it.
    let replaced_install: Option<QueryLimits> = sqlx::query_as(
        "SELECT timeout_seconds, chart_rows, map_points, studio_rows \
         FROM query_limits WHERE singleton FOR UPDATE",
    )
    .fetch_optional(&mut *tx)
    .await?;
    // The names come from this join, so a diff reads `roles.<name>` the way
    // the console names a role limit.
    let replaced_roles: Vec<RoleLimit> = sqlx::query_as(
        "SELECT q.role_id, r.name AS role_name, q.timeout_seconds \
         FROM query_limits_role q JOIN role r ON r.id = q.role_id ORDER BY r.name",
    )
    .fetch_all(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO query_limits \
             (singleton, timeout_seconds, chart_rows, map_points, studio_rows, updated_at, updated_by) \
         VALUES (TRUE, $1, $2, $3, $4, now(), $5) \
         ON CONFLICT (singleton) DO UPDATE SET timeout_seconds = EXCLUDED.timeout_seconds, \
         chart_rows = EXCLUDED.chart_rows, map_points = EXCLUDED.map_points, \
         studio_rows = EXCLUDED.studio_rows, updated_at = now(), updated_by = EXCLUDED.updated_by",
    )
    .bind(install.timeout_seconds)
    .bind(install.chart_rows)
    .bind(install.map_points)
    .bind(install.studio_rows)
    .bind(updated_by)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM query_limits_role")
        .execute(&mut *tx)
        .await?;
    for (role_id, seconds) in roles {
        sqlx::query("INSERT INTO query_limits_role (role_id, timeout_seconds) VALUES ($1, $2)")
            .bind(role_id)
            .bind(seconds)
            .execute(&mut *tx)
            .await?;
    }
    // The values the save stored, read here, inside its own transaction
    // (plan D3b): the caller answers from these and names what changed by
    // comparing them with the values the save replaced, so no read sits
    // between the write and the audit row, and a read taken after the commit
    // can never name another save's values as this one's.
    let stored_install: QueryLimits = sqlx::query_as(
        "SELECT timeout_seconds, chart_rows, map_points, studio_rows \
         FROM query_limits WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?;
    let stored_roles: Vec<RoleLimit> = sqlx::query_as(
        "SELECT q.role_id, r.name AS role_name, q.timeout_seconds \
         FROM query_limits_role q JOIN role r ON r.id = q.role_id ORDER BY r.name",
    )
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok((
        ReplacedLimits {
            install: replaced_install,
            roles: replaced_roles,
        },
        StoredLimits {
            install: stored_install,
            roles: stored_roles,
        },
    ))
}
