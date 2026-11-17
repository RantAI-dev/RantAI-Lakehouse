//! `SEC-41` decision 6 / AC2: a database `sqlx` 0.8 has already migrated
//! must start under 0.9 — no version error, no checksum error, and not one
//! row rewritten.
//!
//! # Why the table is written by hand
//!
//! 0.8 is no longer in the dependency graph (that is what `SEC-41` removed),
//! so "a database 0.8 migrated" has to be reproduced by writing what 0.8
//! writes. Both halves below are copied from the 0.8.6 sources, not
//! remembered:
//!
//! - the DDL is `ensure_migrations_table` in
//!   `sqlx-postgres-0.8.6/src/migrate.rs`;
//! - a row is `execute_migration` in the same file: version, description,
//!   `success = TRUE`, `execution_time`, and `checksum` = SHA-384 over the
//!   file's bytes as `sqlx-core-0.8.6/src/migrate/migration.rs`
//!   (`Sha384::digest(sql.as_bytes())`, `VersionMismatch` on any change)
//!   computes it;
//! - `description` is the file name after `NNNN_` with `.sql` trimmed and
//!   `_` replaced by a space (`sqlx-core-0.8.6/src/migrate/source.rs`).
//!
//! If 0.9 had changed the table shape, the version ordering or the checksum
//! algorithm, [`MIGRATOR`] would reject these rows and this test would fail.
//! That is the assertion: the upgrade is additive, nothing is "fixed" by
//! rewriting history.
//!
//! # Postgres backing
//!
//! `#[sqlx::test(migrations = false)]`: a fresh, empty database per test (no
//! migration applied, so the 0.8 rows are the only history), backed by the
//! `lakehouse-test-support` testcontainer bootstrap like every other test in
//! this crate.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_test_support as _;

use sha2::{Digest, Sha384};
use sqlx::PgPool;
use sqlx::migrate::Migrator;

/// The migration set the API runs (`main.rs`'s `MIGRATOR` equivalent), now
/// compiled against `sqlx` 0.9.
static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// `sqlx-postgres` 0.8.6's `ensure_migrations_table`, verbatim.
const SQLX_0_8_MIGRATIONS_TABLE: &str = "
CREATE TABLE IF NOT EXISTS _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMPTZ NOT NULL DEFAULT now(),
    success BOOLEAN NOT NULL,
    checksum BYTEA NOT NULL,
    execution_time BIGINT NOT NULL
);";

/// One migration file as 0.8 reads it: numeric prefix, description and the
/// SQL text whose bytes are the checksum's input.
struct Migration0508 {
    version: i64,
    description: String,
    sql: String,
}

/// Every `migrations/NNNN_*.sql` this repository ships, in version order —
/// the same set `sqlx::migrate!` embeds.
fn migrations_as_0_8_read_them() -> Vec<Migration0508> {
    let mut paths: Vec<std::path::PathBuf> = std::fs::read_dir("../../migrations")
        .expect("the migrations directory is next to the crate")
        .map(|entry| entry.expect("a readable migrations entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .collect();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .expect("a migration file has a name")
                .to_str()
                .expect("a migration file name is UTF-8")
                .to_owned();
            let (version, rest) = name
                .split_once('_')
                .expect("a migration file is named NNNN_description.sql");
            Migration0508 {
                version: version.parse().expect("a numeric migration version"),
                // `sqlx-core-0.8.6/src/migrate/source.rs`: trim the suffix,
                // turn `_` into a space.
                description: rest.trim_end_matches(".sql").replace('_', " "),
                sql: std::fs::read_to_string(&path).expect("a readable migration file"),
            }
        })
        .collect()
}

#[sqlx::test(migrations = false)]
async fn a_database_migrated_by_sqlx_0_8_starts_under_0_9(pool: PgPool) -> sqlx::Result<()> {
    let migrations = migrations_as_0_8_read_them();
    assert!(
        !migrations.is_empty(),
        "the migration set must not be empty, or this test proves nothing"
    );

    sqlx::query(SQLX_0_8_MIGRATIONS_TABLE)
        .execute(&pool)
        .await?;

    // Apply each migration and record it exactly as 0.8's migrator does: one
    // transaction per file (none of ours carries `-- no-transaction`), the
    // file's SQL, then its row.
    for migration in &migrations {
        let mut tx = pool.begin().await?;
        // safe: the text is one of this repository's own `migrations/*.sql`
        // files, read from disk in this test; no caller or upstream data
        // reaches it.
        sqlx::raw_sql(sqlx::AssertSqlSafe(migration.sql.as_str()))
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, success, checksum, execution_time) \
             VALUES ($1, $2, TRUE, $3, 0)",
        )
        .bind(migration.version)
        .bind(&migration.description)
        .bind(Sha384::digest(migration.sql.as_bytes()).to_vec())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
    }

    let recorded: Vec<(i64,)> =
        sqlx::query_as("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        recorded
            .iter()
            .map(|(version,)| *version)
            .collect::<Vec<_>>(),
        migrations.iter().map(|m| m.version).collect::<Vec<_>>(),
        "the fixture must look exactly like a 0.8-migrated database"
    );

    // The upgrade itself: the 0.9 migrator runs over that history. `Ok`
    // means every applied version matched (same version, same SHA-384), and
    // there was nothing left to apply.
    MIGRATOR
        .run(&pool)
        .await
        .expect("sqlx 0.9 must accept a database sqlx 0.8 migrated");

    let after: Vec<(i64,)> =
        sqlx::query_as("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        after, recorded,
        "0.9 must add no row to a fully migrated database"
    );

    Ok(())
}
