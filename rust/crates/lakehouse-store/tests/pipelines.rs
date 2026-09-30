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
        description: None,
        // None here means "use the migration's DEFAULT 2" — not "set 0".
        // The per-row tests that need a specific count set it explicitly.
        max_retries: None,
        tenant_id: None,
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

// ── Tenant-scoped list: tenant isolation ────────────────────────────────

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

/// Assign a pipeline to a tenant. Written before
/// `assign_tenant` (this module's own store function for that write)
/// existed — a
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

/// Written and run BEFORE `PipelineFilter`
/// existed, as a failing test first. The real failure this produced
/// (`PipelineFilter` stripped, `list_pipelines` reverted to its
/// one-arg signature):
///
/// ```text
/// error[E0433]: failed to resolve: could not find `PipelineFilter` in `pipelines`
/// error[E0061]: this function takes 1 argument but 2 arguments were supplied
/// ```
///
/// Tenant isolation requires asserting a specific second tenant's row
/// ABSENT, never merely "the list is shorter" or "non-empty."
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
            all_tenants: false,
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
/// — every pipeline created before it is assigned a tenant, per
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
            all_tenants: false,
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

/// A pipeline created with the creator's tenant is on that tenant's list
/// straight away: before `tenant_id` was set at creation every
/// console-created pipeline was unassigned and invisible to every list.
#[sqlx::test(migrations = "../../migrations")]
async fn a_pipeline_created_with_a_tenant_is_on_that_tenants_list(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant = create_tenant(&pool, &tenant_input("tenant-create-assign"))
        .await
        .unwrap();
    let tenant_id: Uuid = tenant.id.parse().unwrap();
    let created = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            tenant_id: Some(tenant_id),
            ..named_input("assigned at creation")
        },
    )
    .await
    .unwrap();
    let listed = pipelines::list_pipelines(
        &pool,
        &PipelineFilter {
            tenant_id: Some(tenant_id),
            all_tenants: false,
        },
    )
    .await
    .unwrap();
    assert!(listed.iter().any(|p| p.id == created.id));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn update_replaces_the_definition_and_keeps_the_status(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &named_input("to be edited"))
        .await
        .unwrap();
    pipelines::set_status(&pool, &created.id, "ready")
        .await
        .unwrap();
    let updated = pipelines::update_pipeline(
        &pool,
        &created.id,
        &pipelines::UpdatePipelineInput {
            kind: "batch".to_owned(),
            source_zone: "silver".to_owned(),
            source_table: "events".to_owned(),
            incremental_column: None,
            transforms: vec!["dedupe(id)".to_owned()],
            fbic_enabled: false,
            target_zone: "gold".to_owned(),
            target_table: "events_clean".to_owned(),
            schedule: "0 2 * * *".to_owned(),
            owner: None,
            description: Some("edited".to_owned()),
            max_retries: None,
        },
    )
    .await
    .unwrap()
    .expect("the pipeline exists");
    assert_eq!(updated.status, "ready");
    assert_eq!(updated.source, "silver.events");
    assert_eq!(
        updated.owner, created.owner,
        "an absent owner leaves it unchanged"
    );
    let def = pipelines::get_definition(&pool, &created.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(def.transforms, vec!["dedupe(id)".to_owned()]);
    assert_eq!(def.incremental_column, None);

    let missing = pipelines::update_pipeline(
        &pool,
        "pl-missing",
        &pipelines::UpdatePipelineInput {
            kind: "batch".to_owned(),
            source_zone: "a".to_owned(),
            source_table: "b".to_owned(),
            incremental_column: None,
            transforms: Vec::new(),
            fbic_enabled: false,
            target_zone: "c".to_owned(),
            target_table: "d".to_owned(),
            schedule: "manual".to_owned(),
            owner: None,
            description: None,
            max_retries: None,
        },
    )
    .await
    .unwrap();
    assert!(missing.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_removes_the_row_once(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &named_input("to be deleted"))
        .await
        .unwrap();
    assert!(
        pipelines::delete_pipeline(&pool, &created.id)
            .await
            .unwrap()
    );
    assert!(
        !pipelines::delete_pipeline(&pool, &created.id)
            .await
            .unwrap()
    );
    assert!(
        pipelines::get_pipeline(&pool, &created.id)
            .await
            .unwrap()
            .is_none()
    );
    Ok(())
}

// ── Plan 1c (R2, day-1): `pipeline_definition.max_retries` ─────────────
//
// Migration `0051_pipeline_max_retries.sql` adds the column with default
// 2 and a CHECK (max_retries BETWEEN 0 AND 5). Every authored pipeline
// carries one; the route layer rejects out-of-range values with 400
// before they reach the store, and `RunnablePipeline` exposes it for
// `authored_factory._op_for_pipeline` to consume.

/// A freshly authored pipeline reads `max_retries = 2` (the migration's
/// `DEFAULT 2`), matching the single source of truth in
/// `dagster/dispar_orchestrate/op_metadata.py::DEFAULT_RETRY_POLICY`.
/// Without `max_retries` on `CreatePipelineInput` the route's `None`
/// resolves to this default on insert.
#[sqlx::test(migrations = "../../migrations")]
async fn create_pipeline_defaults_max_retries_to_two(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input()).await.unwrap();
    assert_eq!(
        created.max_retries, 2,
        "absent max_retries on insert must fall back to the column default"
    );
    Ok(())
}

/// The store round-trips every boundary value the route layer accepts:
/// 0 (no retries) and 5 (the max the CHECK constraint allows).
#[sqlx::test(migrations = "../../migrations")]
async fn create_pipeline_persists_max_retries_zero_and_five(pool: PgPool) -> sqlx::Result<()> {
    let zero = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            max_retries: Some(0),
            ..named_input("zero retries")
        },
    )
    .await
    .unwrap();
    assert_eq!(zero.max_retries, 0);
    let five = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            max_retries: Some(5),
            ..named_input("five retries")
        },
    )
    .await
    .unwrap();
    assert_eq!(five.max_retries, 5);
    let reread = pipelines::get_pipeline(&pool, &five.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reread.max_retries, 5);
    Ok(())
}

/// Out-of-range values still hit the CHECK constraint, exactly as
/// `set_status`'s CHECK does for `status`. The route layer catches the
/// common case with a 400 first; this proves the database is defense in
/// depth, not the primary safety boundary (same posture as
/// [`set_status_rejects_a_status_outside_the_check_constraint`]).
#[sqlx::test(migrations = "../../migrations")]
async fn create_pipeline_rejects_max_retries_outside_zero_to_five(
    pool: PgPool,
) -> sqlx::Result<()> {
    let result = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            max_retries: Some(6),
            ..named_input("out of range")
        },
    )
    .await;
    assert!(
        matches!(result, Err(lakehouse_store::StoreError::Database(_))),
        "expected StoreError::Database from the CHECK constraint, got {result:?}"
    );
    Ok(())
}

/// `update_pipeline` without `max_retries` in `UpdatePipelineInput`
/// leaves the stored value alone — an absent field is "unchanged", not
/// "set to the default". `Some(n)` overwrites.
#[sqlx::test(migrations = "../../migrations")]
async fn update_pipeline_keeps_max_retries_when_absent_and_overwrites_when_present(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            max_retries: Some(4),
            ..named_input("editable retries")
        },
    )
    .await
    .unwrap();
    assert_eq!(created.max_retries, 4);

    // An update that doesn't touch max_retries keeps the stored value.
    let mut update = pipelines::UpdatePipelineInput {
        max_retries: None,
        ..update_input_for(&created)
    };
    let edited = pipelines::update_pipeline(&pool, &created.id, &update)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        edited.max_retries, 4,
        "absent max_retries on update must leave the stored value alone"
    );

    // A present max_retries overwrites the stored value.
    update.max_retries = Some(1);
    let edited = pipelines::update_pipeline(&pool, &created.id, &update)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(edited.max_retries, 1);
    Ok(())
}

/// `list_runnable_pipelines` exposes `max_retries` on every row it
/// returns — the orchestrator factory reads it from this list, so a
/// missing field would force the orchestrator to invent one.
#[sqlx::test(migrations = "../../migrations")]
async fn runnable_pipelines_carry_their_max_retries(pool: PgPool) -> sqlx::Result<()> {
    let ready = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            max_retries: Some(3),
            ..named_input("ready with 3")
        },
    )
    .await
    .unwrap();
    pipelines::set_status(&pool, &ready.id, "ready")
        .await
        .unwrap();

    let runnable = pipelines::list_runnable_pipelines(&pool).await.unwrap();
    let one = runnable.iter().find(|p| p.id == ready.id).unwrap();
    assert_eq!(one.definition.max_retries, 3);
    Ok(())
}

fn update_input_for(p: &pipelines::Pipeline) -> pipelines::UpdatePipelineInput {
    // The minimal legal UpdatePipelineInput for an existing pipeline.
    // This is a fixture helper, not a route: the test feeds its own
    // values into `max_retries`, not derived from `p` (the UpdateInput
    // has no name field, and the other defaults below match the
    // `update_replaces_the_definition_and_keeps_the_status` test's own
    // choices, so an absent `max_retries` over the same row stays a
    // single-field change).
    let _ = p; // helper shape only
    pipelines::UpdatePipelineInput {
        kind: "batch".to_owned(),
        source_zone: "a".to_owned(),
        source_table: "b".to_owned(),
        incremental_column: None,
        transforms: Vec::new(),
        fbic_enabled: false,
        target_zone: "c".to_owned(),
        target_table: "d".to_owned(),
        schedule: "manual".to_owned(),
        owner: None,
        description: None,
        max_retries: None,
    }
}

/// Only `ready` and `paused` pipelines are runnable, across every tenant,
/// each with its full definition.
#[sqlx::test(migrations = "../../migrations")]
async fn runnable_lists_ready_and_paused_pipelines_with_their_definitions(
    pool: PgPool,
) -> sqlx::Result<()> {
    let draft = pipelines::create_pipeline(&pool, &named_input("still a draft"))
        .await
        .unwrap();
    let ready = pipelines::create_pipeline(&pool, &named_input("ready one"))
        .await
        .unwrap();
    let paused = pipelines::create_pipeline(&pool, &named_input("paused one"))
        .await
        .unwrap();
    pipelines::set_status(&pool, &ready.id, "ready")
        .await
        .unwrap();
    pipelines::set_status(&pool, &paused.id, "paused")
        .await
        .unwrap();

    let runnable = pipelines::list_runnable_pipelines(&pool).await.unwrap();
    let ids: Vec<&str> = runnable.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&ready.id.as_str()));
    assert!(ids.contains(&paused.id.as_str()));
    assert!(!ids.contains(&draft.id.as_str()));
    let one = runnable.iter().find(|p| p.id == ready.id).unwrap();
    assert_eq!(one.definition.source_table, "orders");
    assert_eq!(one.definition.transforms.len(), 2);
    Ok(())
}

/// `all_tenants` (a tenantless Platform Admin) lists every row, including
/// an unassigned one and one in some tenant; without it the admin saw no
/// authored pipeline on the list at all.
#[sqlx::test(migrations = "../../migrations")]
async fn list_pipelines_for_all_tenants_includes_unassigned_and_tenanted_rows(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant = create_tenant(&pool, &tenant_input("tenant-all-tenants"))
        .await
        .unwrap();
    let tenant_id: Uuid = tenant.id.parse().unwrap();
    let unassigned = pipelines::create_pipeline(&pool, &named_input("all-tenants-unassigned"))
        .await
        .unwrap();
    let tenanted = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            tenant_id: Some(tenant_id),
            ..named_input("all-tenants-tenanted")
        },
    )
    .await
    .unwrap();

    let rows = pipelines::list_pipelines(
        &pool,
        &PipelineFilter {
            tenant_id: None,
            all_tenants: true,
        },
    )
    .await
    .unwrap();
    let ids: Vec<&str> = rows.iter().map(|p| p.id.as_str()).collect();
    assert!(ids.contains(&unassigned.id.as_str()));
    assert!(ids.contains(&tenanted.id.as_str()));
    Ok(())
}
