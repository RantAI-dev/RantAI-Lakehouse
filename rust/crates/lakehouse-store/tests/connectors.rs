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
    CreateConnectorInput, create_connector, delete_connector, get_connector,
    get_connector_dial_info, list_connectors, record_test_result,
};
use lakehouse_store::pipelines::{CreatePipelineInput, create_pipeline};
use sqlx::PgPool;

/// P6 shrank the seed to the two connector types this build can actually
/// dial (`0022_prune_connector_seed.sql`) — see that migration's header
/// comment for why the 28-row `mock/connectors.ts` fixture was removed.
#[sqlx::test(migrations = "../../migrations")]
async fn seed_populates_connector_list(pool: PgPool) -> sqlx::Result<()> {
    let connectors = list_connectors(&pool).await.unwrap();
    assert_eq!(connectors.len(), 2);
    assert!(connectors.iter().any(|c| c.id == "conn-pg-lakehouse"));
    assert!(connectors.iter().any(|c| c.id == "conn-s3-warehouse"));
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
        name: "Lakehouse OLTP (Postgres)".to_owned(), // seeded name
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

/// `get_connector_dial_info` hands back exactly the fields a real probe
/// needs — including `secret_ref_secondary` for the S3 connector, which
/// `get_connector`/`list_connectors` never expose at all.
#[sqlx::test(migrations = "../../migrations")]
async fn dial_info_returns_type_host_and_secret_refs(pool: PgPool) -> sqlx::Result<()> {
    let pg = get_connector_dial_info(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pg.kind, "PostgreSQL");
    assert_eq!(pg.host, "lakehouse@postgres:5432/lakehouse");
    assert_eq!(pg.secret_ref, "env:POSTGRES_PASSWORD");
    assert_eq!(pg.secret_ref_secondary, None);

    let s3 = get_connector_dial_info(&pool, "conn-s3-warehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(s3.kind, "Object storage");
    assert_eq!(
        s3.secret_ref_secondary.as_deref(),
        Some("env:RUSTFS_SECRET_KEY")
    );

    assert!(
        get_connector_dial_info(&pool, "conn-does-not-exist")
            .await
            .unwrap()
            .is_none()
    );
    Ok(())
}

/// `record_test_result` persists exactly what the caller measured — never
/// derives `ok`/`latency_ms` itself — and stamps `lastTestAt` forward.
/// `health` follows `ok` only when `supported` is `true`.
#[sqlx::test(migrations = "../../migrations")]
async fn record_test_result_persists_outcome_and_stamps_last_test_at(
    pool: PgPool,
) -> sqlx::Result<()> {
    let before = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();

    let ok_result = record_test_result(&pool, "conn-pg-lakehouse", true, true, Some(12), "ok")
        .await
        .unwrap();
    assert!(ok_result.ok);
    assert!(ok_result.supported);
    assert_eq!(ok_result.latency_ms, Some(12));
    let after_ok = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_ok.connector.health, "healthy");
    assert_ne!(
        before.connector.last_test_at, after_ok.connector.last_test_at,
        "record_test_result must stamp lastTestAt forward"
    );

    let fail_result = record_test_result(
        &pool,
        "conn-pg-lakehouse",
        false,
        true,
        Some(4999),
        "refused",
    )
    .await
    .unwrap();
    assert!(!fail_result.ok);
    let after_fail = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_fail.connector.health, "unhealthy");

    // Unsupported: health is left untouched, and no latency is recorded.
    let before_unsupported = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    let unsupported_result = record_test_result(
        &pool,
        "conn-pg-lakehouse",
        false,
        false,
        None,
        "unsupported",
    )
    .await
    .unwrap();
    assert!(!unsupported_result.ok);
    assert!(!unsupported_result.supported);
    assert_eq!(unsupported_result.latency_ms, None);
    let after_unsupported = get_connector(&pool, "conn-pg-lakehouse")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_unsupported.connector.health, before_unsupported.connector.health,
        "an unsupported test must not change stored health"
    );
    Ok(())
}

/// Testing a connector that doesn't exist is a `NotFound`, not a panic or a
/// silently-empty success.
#[sqlx::test(migrations = "../../migrations")]
async fn record_test_result_not_found_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let err = record_test_result(&pool, "conn-does-not-exist", true, true, Some(1), "x")
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
