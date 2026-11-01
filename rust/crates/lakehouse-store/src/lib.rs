//! Postgres-backed OLTP storage for Phase 2 (`console`/mutation state that
//! `ClickHouse` — an analytical, transaction-less store — is a poor fit
//! for). `ClickHouse` stays the analytics store; this crate is additive,
//! not a replacement.
//!
//! # Boot behavior: Postgres down must not stop the process
//!
//! [`connect_lazy`] never performs network I/O and therefore never fails
//! because Postgres happens to be unreachable — it only fails if the URL
//! itself doesn't parse. This is deliberate: `lakehouse-api` is a
//! big-bang-cutover service that already serves the full Phase 1 route
//! surface (`ClickHouse`/`Dagster`/LLM — no Postgres involved) before any
//! Phase 2 domain exists. If constructing the pool blocked on, or failed
//! because of, Postgres being down at startup, bringing up a fresh
//! `docker compose` stack (or losing the Postgres container in production
//! while `ClickHouse`-backed traffic is otherwise healthy) would take down
//! routes that have nothing to do with Postgres — a regression relative to
//! today. So: the pool is always constructed (lazily) at startup, and
//! actual connectivity is only ever discovered — and only ever fails — at
//! the point a Phase 2 handler issues its first query, where it surfaces as
//! an ordinary [`error::StoreError::Database`] -> `ApiError::Internal(500)`
//! response instead of a boot-time panic or a refused-to-start process.
//!
//! `lakehouse-api::state::AppState` additionally wraps the pool in
//! `Option`, for the one case [`connect_lazy`] itself can fail: a
//! malformed `DATABASE_URL`. In that case there is no pool to hand a
//! handler at all, and it must reply with
//! [`error::StoreError::Unavailable`] rather than reach for an `Option`
//! that was never `Some`.

// `sqlx::test` embeds the migration set at compile time; touching this file
// (adding `0033_connector_ingest_spec.sql`, WS3 item 1; `0064_connector_schema_changes.sql`, SRC-8; `0083_governed_tags.sql`, DATA-12; `0079_column_lineage.sql`, DATA-13; `0081_quality_incidents.sql`, DATA-14) forces the macro to
// re-expand and pick up the new migration for every `#[sqlx::test]` in this
// crate's `tests/` binaries.
pub mod agents;
pub mod alert_state;
pub mod annotation;
pub mod audit;
pub mod cdc;
pub mod chat_term;
pub mod column_lineage;
pub mod connector_discovery;
pub mod connector_oauth;
pub mod connector_probe_result;
pub mod connector_run;
pub mod connector_type;
pub mod connectors;
pub mod dashboard_access;
pub mod dashboard_comments;
pub mod error;
pub mod geo_map;
pub mod gold_publication;
pub mod governance;
pub mod governed_tag;
pub mod home_layout;
pub mod identity;
pub mod ingest_spec;
pub mod knowledge;
pub mod maintenance_policy;
pub mod outside_reads;
pub mod overview;
pub mod pipelines;
pub mod public_links;
pub mod quality;
pub mod queries;
pub mod query_limits;
pub mod report_schedule;
pub mod report_subscriptions;
pub mod reporting_settings;
pub mod saas_vendors;
pub mod schema_change;
pub mod schema_diff;
pub mod semantic;
pub mod sessions;
pub mod storage;
pub mod upload_folders;
pub mod uploads;

pub use error::StoreError;

/// A connection pool to the OLTP Postgres database.
///
/// Re-exported so callers never need to depend on `sqlx` directly just to
/// name this type.
pub type PgPool = sqlx::PgPool;

/// Construct a [`PgPool`] without performing any network I/O.
///
/// `sqlx`'s "lazy" pool parses and validates `database_url` synchronously
/// but defers actually opening a connection until the pool is first used
/// (the first query, or an explicit `.acquire()`). See the module doc
/// comment for why this — rather than an eager, awaited `connect` — is the
/// right constructor for this service's boot behavior.
///
/// Synchronous, but still needs to be called from inside a Tokio runtime
/// (e.g. `#[tokio::main]`/`#[tokio::test]`): the pool registers an
/// idle-connection reaper against the ambient runtime as part of
/// construction, even though it opens no sockets. `lakehouse-api::main`
/// only ever calls this from inside `#[tokio::main]`, so this is never a
/// concern in production; it only matters for tests calling this directly.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if `database_url` cannot be parsed as a
/// Postgres connection string. Never fails due to Postgres being
/// unreachable.
pub fn connect_lazy(database_url: &str) -> Result<PgPool, StoreError> {
    sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(database_url)
        .map_err(StoreError::from)
}

/// The migration set this binary carries — the one [`migrate`] applies.
///
/// A `static`, not a call: `sqlx::migrate!` expands to the whole set
/// (every `.sql` file's text, embedded at compile time), and expanding it
/// a second time would embed every migration's SQL twice in the binary.
/// One owner, one copy: [`embedded_schema_version`] reads the same set
/// [`migrate`] applies, so the two can never disagree about what this
/// build expects the schema to be.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations");

/// Apply every migration in `rust/migrations/` that hasn't already been
/// applied to `pool`, in order. Safe to call on every boot: `sqlx::migrate!`
/// tracks applied versions in a `_sqlx_migrations` table and is a no-op once
/// a migration has been recorded.
///
/// # Errors
///
/// Returns [`StoreError::Database`] if `pool` cannot be reached (this is the
/// one place in this crate that legitimately needs Postgres to be up — a
/// caller invokes this explicitly, at a point of its choosing, rather than
/// it happening implicitly at pool construction) or
/// [`StoreError::Migration`] if a migration file fails to apply (e.g. it
/// was edited after being applied, or the SQL itself is invalid).
pub async fn migrate(pool: &PgPool) -> Result<(), StoreError> {
    MIGRATOR.run(pool).await.map_err(StoreError::from)
}

/// The highest migration version EMBEDDED in this binary — what this build
/// expects the database's schema to be.
///
/// The counterpart of [`applied_schema_version`]: this crate owns the
/// migration set, so it is the only place that can answer both questions.
/// `lakehouse-api`'s readiness route (`GET /health/ready`, REL-2 decision
/// D7) compares the two to answer "is the database at this build's schema
/// version?".
///
/// Never fails: an empty migration set is a build mistake, and `0` is the
/// honest answer for it (no migration has ever been applied to anything).
#[must_use]
pub fn embedded_schema_version() -> i64 {
    MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or_default()
}

/// The highest migration version APPLIED to `pool` — what the database's
/// schema actually is.
///
/// Reads sqlx's own bookkeeping table (`_sqlx_migrations`), so the answer
/// comes from the same records [`migrate`] writes. Only rows with
/// `success = true` count: sqlx keeps a failed migration's row as a
/// "dirty" marker, and a dirty database is not at any version.
///
/// A database that has never been migrated has no such table at all; the
/// query then fails and the caller sees [`StoreError::Database`] — which
/// is the honest answer ("this database is not at this build's schema
/// version") rather than a `0` that could be mistaken for "version 0 is
/// fine".
///
/// # Errors
///
/// Returns [`StoreError::Database`] if `pool` cannot be reached or the
/// bookkeeping table cannot be read.
pub async fn applied_schema_version(pool: &PgPool) -> Result<i64, StoreError> {
    let version: Option<i64> =
        sqlx::query_scalar("SELECT MAX(version) FROM _sqlx_migrations WHERE success = true")
            .fetch_one(pool)
            .await
            .map_err(StoreError::from)?;
    Ok(version.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    /// [`connect_lazy`] must not require a live Postgres: this is the test
    /// that pins down "constructing the pool never blocks on, or fails
    /// because of, connectivity" without needing `#[sqlx::test]` (and
    /// therefore runs in every `cargo test --workspace`, with or without a
    /// database available).
    ///
    /// `#[tokio::test]`, not plain `#[test]`: `connect_lazy` performs no
    /// network I/O, but `sqlx`'s lazy pool still needs an ambient Tokio
    /// runtime to register its idle-connection reaper against — exactly
    /// the situation it runs in for real, since `lakehouse-api` only ever
    /// calls this from inside `#[tokio::main]`.
    #[tokio::test]
    async fn connect_lazy_succeeds_without_a_reachable_database() {
        let pool = connect_lazy("postgres://user:pass@127.0.0.1:1/nonexistent_db_xyz");
        assert!(pool.is_ok());
    }

    #[test]
    fn connect_lazy_rejects_a_malformed_url() {
        let pool = connect_lazy("not a postgres url");
        assert!(pool.is_err());
    }

    // Migration-application correctness (`migrate` against a real,
    // freshly-provisioned database) is covered by the `#[sqlx::test]`s in
    // `tests/schema.rs`, which run against the `lakehouse-test-support`
    // testcontainers Postgres — see that file's module doc comment.
}
