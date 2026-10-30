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
/// T1.1) adds the ten business-app types, so there are twenty-six; `0096`
/// (T7.1) flips eight of them and adds none; `0119` (`SRC-4` slice 5) adds
/// `MongoDB CDC`, so there are twenty-seven.
#[sqlx::test(migrations = "../../migrations")]
async fn seeds_sixteen_connector_types(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    assert_eq!(
        types.len(),
        27,
        "expected 27 seeded connector types, got {types:?}"
    );
    Ok(())
}

/// The ten business-app types of `0095` (`SRC-3` T1.1, AD10): unsupported
/// until the last slice (`0096` flips eight, see `SAAS_STILL_UNSUPPORTED`),
/// with a reason, and with `adapter = 'saas'` from the
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

/// The two `saas` types `0096` (`SRC-3` T7.1, decision 10) leaves
/// unsupported: built and gate-tested against mocks, waiting for a test on a
/// real account.
const SAAS_STILL_UNSUPPORTED: [&str; 2] = ["Workday Reports", "NetSuite"];

/// The unsupported types that carry a reason and keep an adapter: Google Sheets
/// and the two `saas` types above. The other eight `saas` types are supported
/// after `0096` and carry no reason; `MySQL CDC` and `SQL Server CDC` are
/// supported again after `0117` (`SRC-4` slice 3) and carry none either.
const TYPES_WITH_REASON_AND_ADAPTER: [&str; 3] = ["Google Sheets", "Workday Reports", "NetSuite"];

/// Every `supported = false` row other than the three that keep an adapter
/// has `adapter = NULL` — there is no `Dial::parse` shape yet for a type this
/// build cannot dial. Google Sheets (`SRC-6` F5, `0062`) and the two
/// still-unsupported `saas` types (`0095`, `0096`)
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

/// `0117` (`SRC-4` slice 3): `MySQL` CDC and `SQL Server` CDC are supported
/// again, keep their `cdc` adapter and carry no reason.
#[sqlx::test(migrations = "../../migrations")]
async fn mysql_and_sql_server_cdc_are_supported_again_after_0117(pool: PgPool) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    for name in ["MySQL CDC", "SQL Server CDC"] {
        let found = types
            .iter()
            .find(|t| t.name == name)
            .expect("the type must stay listed");
        assert!(found.supported, "{found:?}");
        assert_eq!(found.adapter.as_deref(), Some("cdc"), "{found:?}");
        assert_eq!(found.unsupported_reason, None, "{found:?}");
    }
    Ok(())
}

/// `0119` (`SRC-4` slice 5): `MongoDB CDC` is a supported `cdc` type of its own, beside
/// the batch `MongoDB` type, which keeps its `mongodb` adapter.
#[sqlx::test(migrations = "../../migrations")]
async fn mongodb_cdc_is_a_supported_cdc_type_beside_the_batch_mongodb_type(
    pool: PgPool,
) -> sqlx::Result<()> {
    let types = list_connector_types(&pool)
        .await
        .expect("list_connector_types should succeed");
    let cdc = types
        .iter()
        .find(|t| t.name == "MongoDB CDC")
        .expect("MongoDB CDC must be listed");
    assert!(cdc.supported, "{cdc:?}");
    assert_eq!(cdc.adapter.as_deref(), Some("cdc"), "{cdc:?}");
    assert_eq!(cdc.unsupported_reason, None, "{cdc:?}");
    let batch = types
        .iter()
        .find(|t| t.name == "MongoDB")
        .expect("the batch MongoDB type must stay listed");
    assert_eq!(batch.adapter.as_deref(), Some("mongodb"), "{batch:?}");
    Ok(())
}

/// `0062` and `0096`: only Google Sheets and the two still-unsupported `saas`
/// types carry a reason; the other roadmap rows keep the
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

/// `0095` and `0096` (`SRC-3` T1.1, T7.1, AD10): the ten business-app types are
/// listed with the `saas` adapter; the eight with a passing gate are supported
/// and carry no reason, Workday Reports and `NetSuite` (decision 10) stay
/// unsupported with a reason that says they wait for a real account.
#[sqlx::test(migrations = "../../migrations")]
async fn the_ten_business_app_types_are_listed_and_eight_are_supported(
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
        assert_eq!(found.adapter.as_deref(), Some("saas"), "{found:?}");
        if SAAS_STILL_UNSUPPORTED.contains(&name) {
            assert!(!found.supported, "{found:?}");
            let reason = found.unsupported_reason.as_deref().unwrap_or_default();
            assert!(reason.contains("real account"), "{found:?}");
        } else {
            assert!(found.supported, "{found:?}");
            assert!(found.unsupported_reason.is_none(), "{found:?}");
        }
    }
    Ok(())
}
