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

/// Every `supported = false` row other than Google Sheets has
/// `adapter = NULL` — there is no `Dial::parse` shape yet for a type this
/// build cannot dial. Google Sheets (`SRC-6` F5, `0062`) keeps its adapter
/// so existing connectors still open; the assertion that used to cover it
/// was narrowed by name, not loosened for other rows.
#[sqlx::test(migrations = "../../migrations")]
async fn unsupported_rows_have_no_adapter(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for connector_type in &types {
        if !connector_type.supported && connector_type.name != "Google Sheets" {
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

/// `0062` (`SRC-6` F5): Google Sheets is listed, unsupported, with a
/// reason, and keeps its `sheets` adapter.
#[sqlx::test(migrations = "../../migrations")]
async fn google_sheets_is_listed_as_unsupported_with_a_reason(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    let sheets = types
        .iter()
        .find(|t| t.name == "Google Sheets")
        .expect("Google Sheets must stay listed");
    assert!(!sheets.supported, "{sheets:?}");
    assert_eq!(sheets.adapter.as_deref(), Some("sheets"));
    assert!(
        sheets
            .unsupported_reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty()),
        "{sheets:?}"
    );
    Ok(())
}

/// `0062`: only Google Sheets carries a reason; the other roadmap rows keep
/// the generic "Not available yet" in the console, and supported rows have
/// none.
#[sqlx::test(migrations = "../../migrations")]
async fn only_google_sheets_has_an_unsupported_reason(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for connector_type in types.iter().filter(|t| t.name != "Google Sheets") {
        assert!(
            connector_type.unsupported_reason.is_none(),
            "{connector_type:?} must have no unsupported_reason"
        );
    }
    Ok(())
}
