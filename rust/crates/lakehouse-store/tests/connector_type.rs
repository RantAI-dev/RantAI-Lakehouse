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

/// `0035` seeds sixteen connector types: ten `supported = true` rows with a
/// named `adapter`, and six `supported = false` roadmap rows. `0095` (`SRC-3`
/// T1.1) adds the ten business-app types, so there are twenty-six.
#[sqlx::test(migrations = "../../migrations")]
async fn seeds_sixteen_connector_types(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    assert_eq!(
        types.len(),
        26,
        "expected 26 seeded connector types, got {types:?}"
    );
    Ok(())
}

/// The ten business-app types of `0095` (`SRC-3` T1.1, AD10): unsupported
/// until the last slice, with a reason, and with `adapter = 'saas'` from the
/// start, for the reason `0062` gives for Google Sheets: `Dial::parse` already
/// has their shape, and a connector created with one must open in the edit
/// page, which resolves the dial form from the adapter. Named, like the rest.
const SAAS_TYPES: [&str; 10] = [
    "Salesforce",
    "ServiceNow",
    "Workday Reports",
    "Google Analytics 4",
    "HubSpot",
    "Zendesk",
    "Confluence",
    "Dynamics 365",
    "NetSuite",
    "SharePoint",
];

/// The unsupported types that carry a reason and keep an adapter.
const TYPES_WITH_REASON_AND_ADAPTER: [&str; 13] = [
    "Google Sheets",
    "MySQL CDC",
    "SQL Server CDC",
    "Salesforce",
    "ServiceNow",
    "Workday Reports",
    "Google Analytics 4",
    "HubSpot",
    "Zendesk",
    "Confluence",
    "Dynamics 365",
    "NetSuite",
    "SharePoint",
];

/// Every `supported = false` row other than the thirteen that keep an adapter
/// has `adapter = NULL` — there is no `Dial::parse` shape yet for a type this
/// build cannot dial. Google Sheets (`SRC-6` F5, `0062`), `MySQL` CDC / SQL
/// Server CDC (`SRC-9` decision 8, `0066`) and the ten `saas` types (`0095`)
/// keep their adapter so existing connectors still open. The list is named,
/// not loosened for other rows.
#[sqlx::test(migrations = "../../migrations")]
async fn unsupported_rows_have_no_adapter(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for connector_type in &types {
        if !connector_type.supported
            && !TYPES_WITH_REASON_AND_ADAPTER.contains(&connector_type.name.as_str())
        {
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

/// `0066` (`SRC-9` decision 8): `MySQL` CDC and `SQL Server` CDC are listed,
/// unsupported, with a reason, and keep their `cdc` adapter.
#[sqlx::test(migrations = "../../migrations")]
async fn mysql_and_sql_server_cdc_are_listed_as_unsupported_with_a_reason(
    pool: PgPool,
) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for (name, reason) in [
        (
            "MySQL CDC",
            "Change capture for MySQL is not built yet: this build can only prepare it for PostgreSQL.",
        ),
        (
            "SQL Server CDC",
            "Change capture for SQL Server is not built yet: this build can only prepare it for PostgreSQL.",
        ),
    ] {
        let found = types
            .iter()
            .find(|t| t.name == name)
            .expect("the type must stay listed");
        assert!(!found.supported, "{found:?}");
        assert_eq!(found.adapter.as_deref(), Some("cdc"), "{found:?}");
        assert_eq!(
            found.unsupported_reason.as_deref(),
            Some(reason),
            "{found:?}"
        );
    }
    Ok(())
}

/// `0062`, `0066` and `0095`: only Google Sheets, `MySQL` CDC, `SQL Server` CDC
/// and the ten `saas` types carry a reason; the other roadmap rows keep the
/// generic "Not available yet" in the console, and supported rows have none.
#[sqlx::test(migrations = "../../migrations")]
async fn only_the_named_types_have_an_unsupported_reason(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for connector_type in types
        .iter()
        .filter(|t| !TYPES_WITH_REASON_AND_ADAPTER.contains(&t.name.as_str()))
    {
        assert!(
            connector_type.unsupported_reason.is_none(),
            "{connector_type:?} must have no unsupported_reason"
        );
    }
    Ok(())
}

/// `0095` (`SRC-3` T1.1, AD10): the ten business-app types are listed,
/// unsupported, with the reason "Being built (SRC-3)." and the `saas` adapter.
#[sqlx::test(migrations = "../../migrations")]
async fn the_ten_business_app_types_are_listed_unsupported_with_the_saas_adapter(
    pool: PgPool,
) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for name in SAAS_TYPES {
        let found = types
            .iter()
            .find(|t| t.name == name)
            .expect("the type must be listed");
        assert!(!found.supported, "{found:?}");
        assert_eq!(found.adapter.as_deref(), Some("saas"), "{found:?}");
        assert_eq!(
            found.unsupported_reason.as_deref(),
            Some("Being built (SRC-3)."),
            "{found:?}"
        );
    }
    Ok(())
}
