//! Integration tests for `0035_connector_type.sql` against a real
//! Postgres.
//!
//! # Postgres backing
//!
//! These are `#[sqlx::test(migrations = "../../migrations")]` tests: each
//! one gets a freshly migrated, isolated database. The Postgres *server*
//! itself is started once per test binary by the `lakehouse-test-support`
//! dev-dependency, which spins up a `testcontainers`-managed Postgres and
//! points `DATABASE_URL` at it before any test runs — no manual
//! `docker compose up`, no external database required. Docker must be
//! reachable from the environment running `cargo test`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::connector_type::list_connector_types;
use sqlx::PgPool;

/// `0035` seeds exactly sixteen connector types: ten `supported = true`
/// rows with a named `adapter`, and six `supported = false` roadmap rows.
#[sqlx::test(migrations = "../../migrations")]
async fn seeds_sixteen_connector_types(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    assert_eq!(
        types.len(),
        16,
        "expected 16 seeded connector types, got {types:?}"
    );
    Ok(())
}

/// Every `supported = false` row has `adapter = NULL` — there is no
/// `Dial::parse` shape yet for a type this build cannot dial.
#[sqlx::test(migrations = "../../migrations")]
async fn unsupported_rows_have_no_adapter(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for connector_type in &types {
        if !connector_type.supported {
            assert!(
                connector_type.adapter.is_none(),
                "unsupported type {connector_type:?} must have adapter = NULL"
            );
        }
    }
    Ok(())
}

/// Every `supported = true` row does name an `adapter`.
#[sqlx::test(migrations = "../../migrations")]
async fn supported_rows_name_an_adapter(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for connector_type in &types {
        if connector_type.supported {
            assert!(
                connector_type.adapter.is_some(),
                "supported type {connector_type:?} must name an adapter"
            );
        }
    }
    Ok(())
}
