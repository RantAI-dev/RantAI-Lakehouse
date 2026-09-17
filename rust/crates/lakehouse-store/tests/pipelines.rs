//! Integration tests for `pipelines.rs` against a real Postgres — WS4 item
//! D3: `CreatePipelineBody`'s `incremental_column`/`transforms`/
//! `fbic_enabled` used to be accepted and dropped on the floor
//! (`#[allow(dead_code, ...)]` at `routes/pipelines.rs`); this proves they
//! now round-trip through `create_pipeline`/`get_definition`.
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

use lakehouse_store::pipelines::{self, CreatePipelineInput};
use sqlx::PgPool;

fn input() -> CreatePipelineInput {
    CreatePipelineInput {
        name: "orders_hourly_rollup".to_owned(),
        kind: "incremental".to_owned(),
        source_zone: "bronze".to_owned(),
        source_table: "orders".to_owned(),
        incremental_column: Some("updated_at".to_owned()),
        transforms: vec![
            "dedupe(order_id)".to_owned(),
            "select(order_id,total)".to_owned(),
        ],
        fbic_enabled: true,
        target_zone: "silver".to_owned(),
        target_table: "orders_clean".to_owned(),
        schedule: "manual".to_owned(),
        owner: None,
    }
}

/// A round-tripped `AuthoredDefinition` carries the real
/// `incremental_column`/`transforms`/`fbic_enabled` values this task
/// persists, not the pre-Phase-D dropped ones (which would report `None`/
/// `[]`/`false` regardless of what was submitted).
#[sqlx::test(migrations = "../../migrations")]
async fn create_pipeline_persists_incremental_column_fbic_enabled_and_transforms(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input())
        .await
        .expect("create_pipeline should succeed");

    let definition = pipelines::get_definition(&pool, &created.id)
        .await
        .expect("get_definition should succeed")
        .expect("definition should exist for a pipeline just created");

    assert_eq!(definition.source_zone, "bronze");
    assert_eq!(definition.source_table, "orders");
    assert_eq!(definition.target_zone, "silver");
    assert_eq!(definition.target_table, "orders_clean");
    assert_eq!(definition.incremental_column, Some("updated_at".to_owned()));
    assert_eq!(
        definition.transforms,
        vec![
            "dedupe(order_id)".to_owned(),
            "select(order_id,total)".to_owned()
        ]
    );
    assert!(definition.fbic_enabled);
    assert_eq!(definition.connector_id, None);
    Ok(())
}

/// A pipeline created with none of the three optional fields set reports
/// the honest "not configured" shape — `None`/`[]`/`false` — rather than a
/// row `get_definition` cannot read at all.
#[sqlx::test(migrations = "../../migrations")]
async fn get_definition_reports_absent_optional_fields_honestly(pool: PgPool) -> sqlx::Result<()> {
    let mut plain = input();
    plain.incremental_column = None;
    plain.transforms = Vec::new();
    plain.fbic_enabled = false;

    let created = pipelines::create_pipeline(&pool, &plain)
        .await
        .expect("create_pipeline should succeed");
    let definition = pipelines::get_definition(&pool, &created.id)
        .await
        .expect("get_definition should succeed")
        .expect("definition should exist");

    assert_eq!(definition.incremental_column, None);
    assert!(definition.transforms.is_empty());
    assert!(!definition.fbic_enabled);
    Ok(())
}

/// `get_definition` on an id with no matching row returns `Ok(None)`, not
/// an error — same "unknown id" shape `get_pipeline` already uses.
#[sqlx::test(migrations = "../../migrations")]
async fn get_definition_none_for_unknown_id(pool: PgPool) -> sqlx::Result<()> {
    let definition = pipelines::get_definition(&pool, "pl-does-not-exist")
        .await
        .expect("get_definition should succeed even for an unknown id");
    assert!(definition.is_none());
    Ok(())
}
