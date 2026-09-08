//! Integration tests for `lakehouse_store::connectors` against a real
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

use lakehouse_store::StoreError;
use lakehouse_store::connectors::{
    CreateConnectorInput, create_connector, delete_connector, get_connector, list_connectors,
    test_connection,
};
use lakehouse_store::pipelines::{CreatePipelineInput, create_pipeline};
use sqlx::PgPool;

/// The seed lands the full `mock/connectors.ts` fixture set.
#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_connector_list(pool: PgPool) -> sqlx::Result<()> {
    let connectors = list_connectors(&pool).await.unwrap();
    assert_eq!(connectors.len(), 28);
    assert!(connectors.iter().any(|c| c.id == "conn-pg-oms"));
    Ok(())
}

/// The whole point of this domain: no read path can round-trip a `host` or
/// `secretRef` back to a caller, no matter how the row was created.
#[sqlx::test(migrations = "../../migrations")]
async fn created_connector_never_carries_host_or_secret_ref_on_the_wire(
    pool: PgPool,
) -> sqlx::Result<()> {
    let input = CreateConnectorInput {
        name: "leak test connector".to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "super-secret-internal-host.example".to_owned(),
        secret_ref: "env:LEAK_TEST_TOKEN".to_owned(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: "in-region".to_owned(),
        capabilities: vec![],
        owner: None,
    };
    let created = create_connector(&pool, &input).await.unwrap();

    let as_json = serde_json::to_value(&created).unwrap();
    assert!(as_json.get("host").is_none());
    assert!(as_json.get("secretRef").is_none());
    let raw = serde_json::to_string(&as_json).unwrap();
    assert!(!raw.contains("super-secret-internal-host"));
    assert!(!raw.contains("LEAK_TEST_TOKEN"));

    // Also true of the list/detail reads, not just the create response.
    let list = list_connectors(&pool).await.unwrap();
    let list_json = serde_json::to_string(&list).unwrap();
    assert!(!list_json.contains("super-secret-internal-host"));
    assert!(!list_json.contains("LEAK_TEST_TOKEN"));

    let detail = get_connector(&pool, &created.id).await.unwrap().unwrap();
    let detail_json = serde_json::to_string(&detail).unwrap();
    assert!(!detail_json.contains("super-secret-internal-host"));
    assert!(!detail_json.contains("LEAK_TEST_TOKEN"));

    Ok(())
}

/// A duplicate name is a 409, matching `pipeline_definition_name_unique`'s
/// treatment.
#[sqlx::test(migrations = "../../migrations")]
async fn create_connector_rejects_duplicate_name(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateConnectorInput {
        name: "order management system (CDC)".to_owned(), // seeded name
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "h".to_owned(),
        secret_ref: "env:X".to_owned(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    };
    let err = create_connector(&pool, &input).await.unwrap_err();
    assert!(matches!(err, StoreError::Conflict));
    Ok(())
}

/// `get_connector` derives `dependentPipelines` from
/// `pipeline_definition.connector_id`, live — not from a stored/denormalized
/// column.
#[sqlx::test(migrations = "../../migrations")]
async fn dependent_pipelines_are_derived_from_pipeline_definition(
    pool: PgPool,
) -> sqlx::Result<()> {
    let connector = create_connector(
        &pool,
        &CreateConnectorInput {
            name: "dependents test connector".to_owned(),
            kind: "REST API".to_owned(),
            direction: "source".to_owned(),
            host: "h".to_owned(),
            secret_ref: "env:X".to_owned(),
            environment: "staging".to_owned(),
            tenant: "Meridian Group".to_owned(),
            residency: String::new(),
            capabilities: vec![],
            owner: None,
        },
    )
    .await
    .unwrap();

    let before = get_connector(&pool, &connector.id).await.unwrap().unwrap();
    assert!(before.dependent_pipelines.is_empty());

    let pipeline = create_pipeline(
        &pool,
        &CreatePipelineInput {
            name: "dependents test pipeline".to_owned(),
            kind: "batch".to_owned(),
            source_zone: "bronze".to_owned(),
            source_table: "t".to_owned(),
            target_zone: "silver".to_owned(),
            target_table: "t".to_owned(),
            schedule: "manual".to_owned(),
            owner: None,
        },
    )
    .await
    .unwrap();
    sqlx::query("UPDATE pipeline_definition SET connector_id = $1 WHERE id = $2")
        .bind(&connector.id)
        .bind(&pipeline.id)
        .execute(&pool)
        .await
        .unwrap();

    let after = get_connector(&pool, &connector.id).await.unwrap().unwrap();
    assert_eq!(after.dependent_pipelines.len(), 1);
    assert_eq!(after.dependent_pipelines[0].id, pipeline.id);
    assert_eq!(after.dependent_pipelines[0].kind, "pipeline");
    Ok(())
}

/// `test_connection` reports `ok` from the connector's stored `health`, and
/// stamps `lastTestAt` forward.
#[sqlx::test(migrations = "../../migrations")]
async fn test_connection_reflects_stored_health_and_stamps_last_test_at(
    pool: PgPool,
) -> sqlx::Result<()> {
    let before = get_connector(&pool, "conn-pg-oms").await.unwrap().unwrap();
    let result = test_connection(&pool, "conn-pg-oms").await.unwrap();
    assert!(result.ok, "conn-pg-oms is seeded healthy");
    let after = get_connector(&pool, "conn-pg-oms").await.unwrap().unwrap();
    assert_ne!(
        before.connector.last_test_at, after.connector.last_test_at,
        "testConnection must stamp lastTestAt forward"
    );

    let unhealthy = test_connection(&pool, "conn-gsheets-ops").await.unwrap();
    assert!(!unhealthy.ok, "conn-gsheets-ops is seeded unhealthy");
    Ok(())
}

/// Testing a connector that doesn't exist is a `NotFound`, not a panic or a
/// silently-empty success.
#[sqlx::test(migrations = "../../migrations")]
async fn test_connection_not_found_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let err = test_connection(&pool, "conn-does-not-exist")
        .await
        .unwrap_err();
    assert!(matches!(err, StoreError::NotFound));
    Ok(())
}

/// Fetching an unknown connector's detail returns `None`, not an error.
#[sqlx::test(migrations = "../../migrations")]
async fn get_connector_none_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let detail = get_connector(&pool, "conn-does-not-exist").await.unwrap();
    assert!(detail.is_none());
    Ok(())
}

/// `delete_connector` actually removes the row, and reports `true`.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_connector_removes_the_row(pool: PgPool) -> sqlx::Result<()> {
    let input = CreateConnectorInput {
        name: "delete me".to_owned(),
        kind: "REST API".to_owned(),
        direction: "source".to_owned(),
        host: "h".to_owned(),
        secret_ref: "env:DELETE_ME".to_owned(),
        environment: "staging".to_owned(),
        tenant: "Meridian Group".to_owned(),
        residency: String::new(),
        capabilities: vec![],
        owner: None,
    };
    let created = create_connector(&pool, &input).await.unwrap();

    let deleted = delete_connector(&pool, &created.id).await.unwrap();
    assert!(deleted);
    assert!(get_connector(&pool, &created.id).await.unwrap().is_none());
    Ok(())
}

/// Deleting an unknown id is `Ok(false)`, not an error — idempotent-delete
/// convention.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_connector_unknown_id_is_false_not_an_error(pool: PgPool) -> sqlx::Result<()> {
    let deleted = delete_connector(&pool, "conn-does-not-exist")
        .await
        .unwrap();
    assert!(!deleted);
    Ok(())
}
