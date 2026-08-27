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

pub mod error;
pub mod identity;

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
    sqlx::migrate!("../../migrations")
        .run(pool)
        .await
        .map_err(StoreError::from)
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
    // `tests/schema.rs`, which are `#[ignore]`d by default — see that
    // file's module doc comment for why, and how to run them.
}
