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

use lakehouse_store::identity::{CreateTenantInput, create_tenant};
use lakehouse_store::pipelines::{self, CreatePipelineInput, PipelineFilter};
use sqlx::PgPool;
use uuid::Uuid;

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

// ── WS4 item D4: `set_status`, now explicitly defense-in-depth only ────

/// `set_status`'s own validation is entirely the `pipeline_definition_
/// status_check` CHECK constraint — a status outside the fixed vocabulary
/// still fails here, exactly as before this task. `POST
/// /api/pipelines/{id}/status`'s REAL validation is
/// `routes::pipelines::ALLOWED_TRANSITIONS`, checked before this function
/// is ever called (see `set_status`'s own doc comment).
#[sqlx::test(migrations = "../../migrations")]
async fn set_status_rejects_a_status_outside_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input())
        .await
        .expect("create_pipeline should succeed");

    let result = pipelines::set_status(&pool, &created.id, "not_a_real_status").await;
    assert!(
        matches!(result, Err(lakehouse_store::StoreError::Database(_))),
        "expected StoreError::Database from the CHECK constraint, got {result:?}"
    );
    Ok(())
}

/// `set_status` itself still moves a `"draft"` pipeline to `"ready"` when
/// asked — the function's own behavior is unchanged by this task, only its
/// role (defense in depth, not the route's real gate) is.
#[sqlx::test(migrations = "../../migrations")]
async fn set_status_moves_a_draft_pipeline_to_ready(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input())
        .await
        .expect("create_pipeline should succeed");
    assert_eq!(created.status, "draft");

    let updated = pipelines::set_status(&pool, &created.id, "ready")
        .await
        .expect("set_status should succeed")
        .expect("pipeline should exist");
    assert_eq!(updated.status, "ready");
    Ok(())
}

// ── WS8 plan Task C3: tenant-scoped list, Hard Requirement 2 ───────────

/// Minimal valid input for [`create_tenant`], varying only `slug` (unique).
fn tenant_input(slug: &str) -> CreateTenantInput {
    CreateTenantInput {
        name: slug.to_owned(),
        slug: slug.to_owned(),
        plan: "starter".to_owned(),
        residency: "in-region".to_owned(),
    }
}

/// A named [`CreatePipelineInput`], varying only `name` (the table's
/// unique natural key) so each test can create more than one pipeline
/// without a name collision.
fn named_input(name: &str) -> CreatePipelineInput {
    CreatePipelineInput {
        name: name.to_owned(),
        ..input()
    }
}

/// Assign a pipeline to a tenant. No store function does this yet
/// (`assign_pipeline_tenant` is WS8 plan Task C7, not this task) — a
/// direct, bound `UPDATE` is the only way this test can set up a
/// tenant-scoped fixture today, matching `0042_tenant_provisioning.sql`'s
/// own column exactly (nullable `pipeline_definition.tenant_id`).
async fn set_pipeline_tenant(pool: &PgPool, pipeline_id: &str, tenant_id: Uuid) {
    sqlx::query("UPDATE pipeline_definition SET tenant_id = $1 WHERE id = $2")
        .bind(tenant_id)
        .bind(pipeline_id)
        .execute(pool)
        .await
        .unwrap();
}

/// WS8 plan Task C3, Step 1 (TDD): written and run BEFORE `PipelineFilter`
/// existed. The real Step 1 failure this produced (`PipelineFilter`
/// stripped, `list_pipelines` reverted to its pre-Task-C3 one-arg
/// signature):
///
/// ```text
/// error[E0433]: failed to resolve: could not find `PipelineFilter` in `pipelines`
/// error[E0061]: this function takes 1 argument but 2 arguments were supplied
/// ```
///
/// Hard Requirement 2's own wording: a specific second tenant's row must
/// be asserted ABSENT, never merely "the list is shorter" or "non-empty."
#[sqlx::test(migrations = "../../migrations")]
async fn list_pipelines_filtered_by_tenant_excludes_another_tenants_row(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = create_tenant(&pool, &tenant_input("tenant-a-pipelines"))
        .await
        .unwrap();
    let tenant_b = create_tenant(&pool, &tenant_input("tenant-b-pipelines"))
        .await
        .unwrap();
    let tenant_a_id: Uuid = tenant_a.id.parse().unwrap();
    let tenant_b_id: Uuid = tenant_b.id.parse().unwrap();

    let pl_a = pipelines::create_pipeline(&pool, &named_input("pl-a-isolation"))
        .await
        .unwrap();
    let pl_b = pipelines::create_pipeline(&pool, &named_input("pl-b-isolation"))
        .await
        .unwrap();
    set_pipeline_tenant(&pool, &pl_a.id, tenant_a_id).await;
    set_pipeline_tenant(&pool, &pl_b.id, tenant_b_id).await;

    let rows = pipelines::list_pipelines(
        &pool,
        &PipelineFilter {
            tenant_id: Some(tenant_a_id),
        },
    )
    .await
    .unwrap();
    assert!(
        rows.iter().any(|r| r.id == pl_a.id),
        "tenant-a's own pipeline must be present"
    );
    assert!(
        !rows.iter().any(|r| r.id == pl_b.id),
        "tenant-b's pipeline must be absent, not merely unlisted-first"
    );
    Ok(())
}

/// Fail closed: a pipeline whose `tenant_id` column is `NULL` (unassigned
/// — every pipeline created before Task C7 lands, per
/// `0042_tenant_provisioning.sql`'s own header comment) must never appear
/// in ANY tenant-scoped list.
#[sqlx::test(migrations = "../../migrations")]
async fn list_pipelines_with_a_null_tenant_id_is_invisible_once_scoped(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant_a = create_tenant(&pool, &tenant_input("tenant-a-null-pipeline"))
        .await
        .unwrap();
    let tenant_a_id: Uuid = tenant_a.id.parse().unwrap();

    // Freshly created via `create_pipeline`, never assigned a tenant —
    // `tenant_id` stays the column's default, `NULL`.
    pipelines::create_pipeline(&pool, &named_input("pl-unassigned"))
        .await
        .unwrap();

    let rows = pipelines::list_pipelines(
        &pool,
        &PipelineFilter {
            tenant_id: Some(tenant_a_id),
        },
    )
    .await
    .unwrap();
    assert!(
        rows.is_empty(),
        "a pipeline with tenant_id NULL must not leak into any tenant's scoped list"
    );
    Ok(())
}

/// A `PipelineFilter { tenant_id: None }` matches nothing — the
/// `IS NOT NULL AND` guard's whole reason for existing (see
/// `PipelineFilter`'s doc comment): the store function is fail-closed on
/// its own terms, independent of whether its one HTTP caller always
/// resolves a real tenant first.
#[sqlx::test(migrations = "../../migrations")]
async fn list_pipelines_with_no_tenant_filter_matches_nothing(pool: PgPool) -> sqlx::Result<()> {
    pipelines::create_pipeline(&pool, &named_input("pl-no-filter"))
        .await
        .unwrap();

    let rows = pipelines::list_pipelines(&pool, &PipelineFilter::default())
        .await
        .unwrap();
    assert!(
        rows.is_empty(),
        "PipelineFilter::default() (tenant_id: None) must match zero rows, not every row"
    );
    Ok(())
}
