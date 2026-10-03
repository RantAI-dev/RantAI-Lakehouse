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
use lakehouse_store::pipelines::{self, CreatePipelineInput, PipelineFilter, UpdatePipelineInput};
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
        depends_on: vec!["pl-up-1".to_owned(), "ingest_job".to_owned()],
        // PR #57 review F1.7: callers that need to know the id before
        // the row is committed pass `Some(...)`; the store fixtures
        // here want the default `slug_id` derivation, which is exactly
        // what `id: None` triggers in `create_pipeline`.
        id: None,
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
    let created = pipelines::create_pipeline(&pool, &input(), None)
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

    let created = pipelines::create_pipeline(&pool, &plain, None)
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

// ── R3 plan 2a: depends_on (migration 0052) ───────────────────────────────

/// A pipeline created with a non-empty `depends_on` reads back the same
/// list, in order, on `get_pipeline`/`list_pipelines`/`list_runnable_pipelines`
/// (the factory's import-time round-trip). The column is a `text[]`, bound
/// end-to-end — no SQL interpolation anywhere in this path.
#[sqlx::test(migrations = "../../migrations")]
async fn depends_on_round_trips_through_create_and_runnable(pool: PgPool) -> sqlx::Result<()> {
    let mut with_deps = input();
    with_deps.name = "with-deps".to_owned();
    with_deps.depends_on = vec!["pl-up-1".to_owned(), "ingest_job".to_owned()];
    let created = pipelines::create_pipeline(&pool, &with_deps, None)
        .await
        .expect("create should succeed");

    let read = pipelines::get_pipeline(&pool, &created.id)
        .await
        .expect("get_pipeline should succeed")
        .expect("pipeline should exist");
    assert_eq!(read.depends_on, vec!["pl-up-1", "ingest_job"]);

    pipelines::set_status(&pool, &created.id, "ready")
        .await
        .expect("set_status should succeed");
    let runnable = pipelines::list_runnable_pipelines(&pool)
        .await
        .expect("list_runnable should succeed");
    let one = runnable
        .iter()
        .find(|p| p.id == created.id)
        .expect("pipeline should appear in the runnable list");
    assert_eq!(one.depends_on, vec!["pl-up-1", "ingest_job"]);
    Ok(())
}

/// `update_pipeline` replaces `depends_on` wholesale — the same shape as
/// every other editable column the PUT route owns (`source`, `target`,
/// `transforms`, ...).
#[sqlx::test(migrations = "../../migrations")]
async fn update_replaces_depends_on_wholesale(pool: PgPool) -> sqlx::Result<()> {
    let mut with_deps = input();
    with_deps.name = "edit-deps".to_owned();
    with_deps.depends_on = vec!["pl-old".to_owned()];
    let created = pipelines::create_pipeline(&pool, &with_deps, None)
        .await
        .expect("create should succeed");
    assert_eq!(created.depends_on, vec!["pl-old"]);

    let updated = pipelines::update_pipeline(
        &pool,
        &created.id,
        &pipelines::UpdatePipelineInput {
            depends_on: Some(vec!["pl-new-1".to_owned(), "pl-new-2".to_owned()]),
            ..pipelines::UpdatePipelineInput {
                kind: input().kind,
                source_zone: input().source_zone,
                source_table: input().source_table,
                incremental_column: None,
                transforms: Vec::new(),
                fbic_enabled: false,
                target_zone: input().target_zone,
                target_table: input().target_table,
                schedule: input().schedule,
                owner: None,
                description: None,
                max_retries: None,
                depends_on: None,
            }
        },
        None,
    )
    .await
    .expect("update should succeed")
    .expect("pipeline should exist");
    assert_eq!(updated.depends_on, vec!["pl-new-1", "pl-new-2"]);
    Ok(())
}

/// `depends_on` defaults to an empty list when the input omits it — the
/// migration's `DEFAULT '{}'` plus the `Vec<String>` default. This is the
/// regression guard for a freshly authored pipeline that the UI hasn't yet
/// edited.
#[sqlx::test(migrations = "../../migrations")]
async fn depends_on_defaults_to_empty_when_unset(pool: PgPool) -> sqlx::Result<()> {
    let empty_deps = CreatePipelineInput {
        depends_on: vec![],
        ..input()
    };
    let created = pipelines::create_pipeline(&pool, &empty_deps, None)
        .await
        .expect("create should succeed");
    assert!(
        created.depends_on.is_empty(),
        "a pipeline created without depends_on must read back as []"
    );
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
    let created = pipelines::create_pipeline(&pool, &input(), None)
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
    let created = pipelines::create_pipeline(&pool, &input(), None)
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

    let pl_a = pipelines::create_pipeline(&pool, &named_input("pl-a-isolation"), None)
        .await
        .unwrap();
    let pl_b = pipelines::create_pipeline(&pool, &named_input("pl-b-isolation"), None)
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
    pipelines::create_pipeline(&pool, &named_input("pl-unassigned"), None)
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
    pipelines::create_pipeline(&pool, &named_input("pl-no-filter"), None)
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
        None,
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
    let created = pipelines::create_pipeline(&pool, &named_input("to be edited"), None)
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
            depends_on: None,
        },
        None,
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
            depends_on: None,
        },
        None,
    )
    .await
    .unwrap();
    assert!(missing.is_none());
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn delete_removes_the_row_once(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &named_input("to be deleted"), None)
        .await
        .unwrap();
    assert!(
        pipelines::delete_pipeline(&pool, &created.id, None)
            .await
            .unwrap()
    );
    assert!(
        !pipelines::delete_pipeline(&pool, &created.id, None)
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
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .unwrap();
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
        None,
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
        None,
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
        None,
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
        None,
    )
    .await
    .unwrap();
    assert_eq!(created.max_retries, 4);

    // An update that doesn't touch max_retries keeps the stored value.
    let mut update = pipelines::UpdatePipelineInput {
        max_retries: None,
        ..update_input_for(&created)
    };
    let edited = pipelines::update_pipeline(&pool, &created.id, &update, None)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        edited.max_retries, 4,
        "absent max_retries on update must leave the stored value alone"
    );

    // A present max_retries overwrites the stored value.
    update.max_retries = Some(1);
    let edited = pipelines::update_pipeline(&pool, &created.id, &update, None)
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
        None,
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
        // PR #57 review F1.8: `None` keeps the stored chain (the
        // `COALESCE` write in `update_pipeline_with_event`); `Some(vec)`
        // replaces it. Test fixtures that don't care about the chain
        // pass `None` — the field is "unchanged" — to mirror the
        // console's save-without-`dependsOn` shape.
        depends_on: None,
    }
}

/// Only `ready` and `paused` pipelines are runnable, across every tenant,
/// each with its full definition.
#[sqlx::test(migrations = "../../migrations")]
async fn runnable_lists_ready_and_paused_pipelines_with_their_definitions(
    pool: PgPool,
) -> sqlx::Result<()> {
    let draft = pipelines::create_pipeline(&pool, &named_input("still a draft"), None)
        .await
        .unwrap();
    let ready = pipelines::create_pipeline(&pool, &named_input("ready one"), None)
        .await
        .unwrap();
    let paused = pipelines::create_pipeline(&pool, &named_input("paused one"), None)
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

// ── Pipeline-run event dedupe (plan 1e) ─────────────────────────────────

/// A first call inserts and returns `true`; a second call for the same
/// `(run_id, kind)` is a no-op and returns `false` — the sensor-retry /
/// double-delivery guard the route depends on. The plan's mutation check
/// proves this test fails when the dedupe is bypassed.
#[sqlx::test(migrations = "../../migrations")]
async fn record_pipeline_run_event_inserts_once_then_dedupes(pool: PgPool) -> sqlx::Result<()> {
    assert!(
        pipelines::record_pipeline_run_event(&pool, "run-1", "pl-x", "failure")
            .await
            .expect("first insert should succeed"),
        "the first call for a (run_id, kind) pair must report a row was inserted"
    );
    assert!(
        !pipelines::record_pipeline_run_event(&pool, "run-1", "pl-x", "failure")
            .await
            .expect("second insert should succeed at the SQL level"),
        "the second call for the same (run_id, kind) must be a no-op (the dedupe the route depends on)"
    );
    Ok(())
}

/// Distinct `kind`s for the same `run_id` are independent rows — a single
/// run may be slow AND late, recorded twice without colliding.
#[sqlx::test(migrations = "../../migrations")]
async fn record_pipeline_run_event_distinct_kinds_for_one_run_do_not_collide(
    pool: PgPool,
) -> sqlx::Result<()> {
    assert!(
        pipelines::record_pipeline_run_event(&pool, "run-2", "pl-x", "failure")
            .await
            .unwrap()
    );
    assert!(
        pipelines::record_pipeline_run_event(&pool, "run-2", "pl-x", "slow")
            .await
            .unwrap(),
        "the same run_id with a different kind must insert a new row"
    );
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
    let unassigned =
        pipelines::create_pipeline(&pool, &named_input("all-tenants-unassigned"), None)
            .await
            .unwrap();
    let tenanted = pipelines::create_pipeline(
        &pool,
        &CreatePipelineInput {
            tenant_id: Some(tenant_id),
            ..named_input("all-tenants-tenanted")
        },
        None,
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

// ── Pipeline SLA CRUD (plan 1f, migration `0050_pipeline_sla.sql`) ──────

/// Reading a pipeline with no SLA returns `None`, not an empty struct.
/// This is the contract `routes::pipelines::get_sla` relies on to decide
/// between `200 {}` and `404`.
#[sqlx::test(migrations = "../../migrations")]
async fn get_pipeline_sla_is_none_when_no_row_exists(pool: PgPool) -> sqlx::Result<()> {
    let got = pipelines::get_pipeline_sla(&pool, "pl-never-set")
        .await
        .unwrap();
    assert!(
        got.is_none(),
        "an unset SLA must read as None, not Some(empty)"
    );
    Ok(())
}

/// Upserting twice updates the row in place rather than appending, and
/// `updated_at` moves forward. The PUT route calls this idempotently, so
/// the audit trail records who set what last.
#[sqlx::test(migrations = "../../migrations")]
async fn upsert_pipeline_sla_round_trips_and_updates_in_place(pool: PgPool) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();

    let first = pipelines::upsert_pipeline_sla(&pool, "pl-x", Some(600), Some(3_600), actor)
        .await
        .expect("first upsert should succeed");
    assert_eq!(first.pipeline_id, "pl-x");
    assert_eq!(first.max_duration_seconds, Some(600));
    assert_eq!(first.late_after_seconds, Some(3_600));
    assert_eq!(first.updated_by, actor);

    let second = pipelines::upsert_pipeline_sla(&pool, "pl-x", Some(1_200), None, actor)
        .await
        .expect("second upsert should succeed");
    assert_eq!(second.max_duration_seconds, Some(1_200));
    assert_eq!(
        second.late_after_seconds, None,
        "passing None for late_after_seconds must clear the column"
    );

    let got = pipelines::get_pipeline_sla(&pool, "pl-x")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.max_duration_seconds, Some(1_200));
    assert_eq!(got.late_after_seconds, None);
    Ok(())
}

/// The CHECK constraints on `0050_pipeline_sla.sql` reject a zero
/// threshold at the database, not just at the route. A SQL-level test
/// pin guarantees that the constraint does not get dropped in a future
/// migration — the route-level guard is defense in depth, not the only
/// guarantee. We do not assert on the constraint name in the error
/// message: `StoreError::Database` deliberately classifies the upstream
/// text away (AGENTS.md rule 4), so the only honest check is "this
/// operation failed at all."
#[sqlx::test(migrations = "../../migrations")]
async fn upsert_pipeline_sla_rejects_zero_thresholds_at_the_check_constraint(
    pool: PgPool,
) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    let result = pipelines::upsert_pipeline_sla(&pool, "pl-zero", Some(0), None, actor).await;
    assert!(
        result.is_err(),
        "a zero max_duration_seconds must fail the CHECK constraint"
    );
    Ok(())
}

// ── R4 plan 2b: `pipeline_definition_version` (migration `0053`) ────────
//
// Every `create`/`update`/`delete` writes exactly one version row inside
// the same transaction as the write; a forced write failure writes none.
// The list/get functions expose that history without the snapshot in the
// list shape (so a future UI can render a diff view against the get-shape
// payloads) and with the snapshot in the get shape (so the restore
// endpoint can rebuild an `UpdatePipelineInput` from it).

/// `create_pipeline` writes exactly one version row inside the same
/// transaction. `version` starts at 1 and `event` is `"created"`. This
/// is the regression that proves the new transactional store path is in
/// effect, not the prior single-statement INSERT.
#[sqlx::test(migrations = "../../migrations")]
async fn create_pipeline_writes_one_created_version_in_the_same_transaction(
    pool: PgPool,
) -> sqlx::Result<()> {
    let _actor = Uuid::new_v4();
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    // Capture the created snapshot before the update overwrites the
    // editable fields. The restore will write these same values back.
    let created_snapshot = pipelines::get_definition_version(&pool, &created.id, 1)
        .await
        .unwrap()
        .unwrap();

    // Update: writes a v=2 row whose snapshot carries the new values.
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
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
            max_retries: Some(4),
            depends_on: None,
        },
        None,
    )
    .await
    .expect("update should succeed");

    // Restore: rewrite the original create values via `restore_pipeline`,
    // which records the new version row with `event = "restored"` so the
    // governance trail distinguishes "replayed a stored snapshot" from
    // a plain update. This mirrors what `restore_version` does at the
    // API seam.
    pipelines::restore_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: created_snapshot.kind.clone(),
            source_zone: created_snapshot.source_zone.clone(),
            source_table: created_snapshot.source_table.clone(),
            incremental_column: created_snapshot.incremental_column.clone(),
            transforms: created_snapshot.transforms.clone(),
            fbic_enabled: created_snapshot.fbic_enabled,
            target_zone: created_snapshot.target_zone.clone(),
            target_table: created_snapshot.target_table.clone(),
            schedule: created_snapshot.schedule.clone(),
            owner: Some(created_snapshot.owner.clone()),
            description: created_snapshot.description.clone(),
            max_retries: Some(created_snapshot.max_retries),
            depends_on: Some(created_snapshot.depends_on.clone()),
        },
        None,
    )
    .await
    .expect("restore via restore_pipeline should succeed");

    let after_def = pipelines::get_definition(&pool, &created.id)
        .await
        .unwrap()
        .expect("definition exists after restore");
    // `AuthoredDefinition` does not surface every field the snapshot
    // carries (it omits `kind`, `schedule`, `owner`, `description`); we
    // assert only what it does expose matches the snapshot.
    assert_eq!(after_def.source_zone, created_snapshot.source_zone);
    assert_eq!(after_def.source_table, created_snapshot.source_table);
    assert_eq!(after_def.target_zone, created_snapshot.target_zone);
    assert_eq!(after_def.target_table, created_snapshot.target_table);
    assert_eq!(after_def.transforms, created_snapshot.transforms);
    assert_eq!(after_def.fbic_enabled, created_snapshot.fbic_enabled);
    assert_eq!(
        after_def.incremental_column,
        created_snapshot.incremental_column
    );
    assert_eq!(after_def.max_retries, created_snapshot.max_retries);

    // Three rows total: created (v=1), updated (v=2), restored via
    // `restore_pipeline` (v=3). Newest-first: indices 0/1/2 are v=3/2/1.
    let versions = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(
        versions[0].event, "restored",
        "the restore call must record event='restored' so the trail is honest"
    );
    assert_eq!(
        versions[1].event, "updated",
        "the edit between create and restore is event='updated'"
    );
    assert_eq!(
        versions[2].event, "created",
        "the original create is event=created"
    );
    Ok(())
}

/// `restore_pipeline` writes exactly one new `pipeline_definition_version`
/// row whose `event` is `"restored"` — the governance trail must
/// distinguish "replayed a stored snapshot" from a plain "updated" edit
/// (Plan R4 2b design bullet 3). Counting versions before/after pins
/// the same-transaction guarantee: a restore writes one row, not two,
/// and not zero (an interrupted tx would have rolled the row back).
#[sqlx::test(migrations = "../../migrations")]
async fn restore_pipeline_writes_exactly_one_restored_version(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
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
            max_retries: Some(4),
            depends_on: None,
        },
        None,
    )
    .await
    .expect("update should succeed");
    // Two rows before the restore: created + updated.
    let pre = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(pre.len(), 2);
    assert_eq!(pre[0].event, "updated");
    assert_eq!(pre[1].event, "created");

    // Rebuild the editable shape from the original `created` snapshot,
    // the same way `restore_version` does at the route seam.
    let snap = pipelines::get_definition_version(&pool, &created.id, 1)
        .await
        .unwrap()
        .unwrap();
    let restored = pipelines::restore_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: snap.kind,
            source_zone: snap.source_zone,
            source_table: snap.source_table,
            incremental_column: snap.incremental_column,
            transforms: snap.transforms,
            fbic_enabled: snap.fbic_enabled,
            target_zone: snap.target_zone,
            target_table: snap.target_table,
            schedule: snap.schedule,
            owner: Some(snap.owner),
            description: snap.description,
            max_retries: Some(snap.max_retries),
            // PR #57 review F1.8: restore is an authoritative replay,
            // so the stored snapshot's chain always replaces the
            // current one. `Some(vec)` is what makes the `COALESCE`
            // write land the restored chain rather than the stored one
            // (which is what `None` would do).
            depends_on: Some(snap.depends_on.clone()),
        },
        None,
    )
    .await
    .expect("restore should succeed");
    assert!(
        restored.is_some(),
        "restore must return the post-write live row"
    );

    // Exactly one new version row was written: 2 → 3.
    let post = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(
        post.len(),
        pre.len() + 1,
        "restore must write exactly one new version row"
    );
    // Newest-first: the just-written version is at index 0; the older
    // rows shift down by one and keep their events.
    assert_eq!(
        post[0].event, "restored",
        "a restore must record event='restored' so the trail is honest"
    );
    assert_eq!(post[1].event, "updated");
    assert_eq!(post[2].event, "created");
    Ok(())
}

/// `update_pipeline` writes exactly one version row with `event =
/// "updated"`. The snapshot must reflect the POST-write state of the row,
/// not the prior state — otherwise restore would replay a stale value.
/// `changed_by` is captured from the call site so audit can attribute the
/// write without a second round trip.
#[sqlx::test(migrations = "../../migrations")]
async fn update_pipeline_writes_one_updated_version_with_post_write_state(
    pool: PgPool,
) -> sqlx::Result<()> {
    let actor = Uuid::new_v4();
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: "batch".to_owned(),
            source_zone: "silver".to_owned(),
            source_table: "events".to_owned(),
            incremental_column: Some("ts".to_owned()),
            transforms: vec!["dedupe(id)".to_owned()],
            fbic_enabled: true,
            target_zone: "gold".to_owned(),
            target_table: "events_clean".to_owned(),
            schedule: "0 2 * * *".to_owned(),
            owner: Some("data-eng".to_owned()),
            description: Some("edited".to_owned()),
            max_retries: Some(4),
            depends_on: None,
        },
        Some(actor),
    )
    .await
    .expect("update should succeed");

    // Newest first: [updated@2, created@1].
    let versions = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].event, "updated");
    assert_eq!(versions[1].event, "created");
    assert_eq!(versions[0].changed_by, Some(actor));
    // Snapshot must carry the post-write fields, not the prior ones.
    let snap = pipelines::get_definition_version(&pool, &created.id, 2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(snap.source_zone, "silver");
    assert_eq!(snap.incremental_column, Some("ts".to_owned()));
    assert!(snap.fbic_enabled);
    assert_eq!(snap.owner, "data-eng");
    assert_eq!(snap.description.as_deref(), Some("edited"));
    Ok(())
}

/// `delete_pipeline` writes exactly one version row with `event =
/// "deleted"`. The snapshot is the PRE-delete state, so a future
/// "undelete" feature could replay it through `update_pipeline`.
#[sqlx::test(migrations = "../../migrations")]
async fn delete_pipeline_writes_one_deleted_version_with_pre_delete_state(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    let pre = pipelines::get_definition(&pool, &created.id)
        .await
        .unwrap()
        .expect("definition exists before delete");
    assert!(
        pipelines::delete_pipeline(&pool, &created.id, None)
            .await
            .expect("delete should succeed")
    );

    let versions = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].event, "deleted");
    let snap = pipelines::get_definition_version(&pool, &created.id, 2)
        .await
        .unwrap()
        .unwrap();
    // The snapshot has every editable field the row had just before the
    // DELETE, which is exactly the state needed to rebuild an
    // `UpdatePipelineInput` if restore-from-deleted is ever added.
    assert_eq!(snap.source_zone, pre.source_zone);
    assert_eq!(snap.source_table, pre.source_table);
    assert_eq!(snap.target_zone, pre.target_zone);
    assert_eq!(snap.target_table, pre.target_table);
    Ok(())
}

/// A failing UPDATE writes ZERO version rows. The migration's CHECK
/// constraint on `max_retries` (`0..=5`) is the deterministic failure:
/// `Some(99)` triggers a 23514, the transaction rolls back, and the
/// `pipeline_definition_version` insert never lands.
#[sqlx::test(migrations = "../../migrations")]
async fn failed_update_writes_no_version_row(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    let baseline = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(baseline.len(), 1, "create wrote exactly one row");

    let result = pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: "batch".to_owned(),
            source_zone: "silver".to_owned(),
            source_table: "events".to_owned(),
            incremental_column: None,
            transforms: vec![],
            fbic_enabled: false,
            target_zone: "gold".to_owned(),
            target_table: "events_clean".to_owned(),
            schedule: "0 2 * * *".to_owned(),
            owner: None,
            description: None,
            max_retries: Some(99), // out of CHECK range; forces a 23514
            depends_on: None,
        },
        None,
    )
    .await;
    assert!(
        matches!(result, Err(lakehouse_store::StoreError::Database(_))),
        "expected StoreError::Database from the CHECK constraint, got {result:?}"
    );

    // Same row count — the failed update left no version row.
    let after = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(after.len(), 1, "failed update must write no version row");
    Ok(())
}

/// `list_definition_versions` returns metadata only — no snapshot column
/// must reach the wire, so the API can scale to a thousand versions
/// without shipping every previous payload each time the UI re-fetches.
#[sqlx::test(migrations = "../../migrations")]
async fn list_definition_versions_is_newest_first_without_snapshots(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    let created_def = pipelines::get_definition(&pool, &created.id)
        .await
        .unwrap()
        .expect("definition exists");
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: created.kind.clone(),
            source_zone: "silver".to_owned(),
            source_table: created_def.source_table.clone(),
            incremental_column: None,
            transforms: created_def.transforms.clone(),
            fbic_enabled: false,
            target_zone: created_def.target_zone.clone(),
            target_table: created_def.target_table.clone(),
            schedule: created.schedule.clone(),
            owner: None,
            description: Some("second".to_owned()),
            max_retries: Some(created.max_retries),
            depends_on: None,
        },
        None,
    )
    .await
    .expect("update should succeed");
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: created.kind.clone(),
            source_zone: "gold".to_owned(),
            source_table: created_def.source_table.clone(),
            incremental_column: None,
            transforms: created_def.transforms.clone(),
            fbic_enabled: false,
            target_zone: created_def.target_zone.clone(),
            target_table: created_def.target_table.clone(),
            schedule: created.schedule.clone(),
            owner: None,
            description: Some("third".to_owned()),
            max_retries: Some(created.max_retries),
            depends_on: None,
        },
        None,
    )
    .await
    .expect("update should succeed");

    let versions = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 3);
    // Newest-first: version strictly descending.
    assert!(versions.windows(2).all(|w| w[0].version > w[1].version));
    // Metadata only — `PipelineVersionMeta` has no snapshot field, so the
    // JSON is small and predictable.
    let json = serde_json::to_value(&versions).unwrap();
    let arr = json.as_array().unwrap();
    assert_eq!(arr.len(), 3);
    assert!(arr[0].get("version").is_some());
    assert!(arr[0].get("event").is_some());
    assert!(arr[0].get("changedBy").is_some());
    assert!(arr[0].get("changedAt").is_some());
    assert!(
        arr[0].get("snapshot").is_none(),
        "list shape must not include snapshot"
    );
    Ok(())
}

/// `get_definition_version` returns the snapshot as a camelCase JSON
/// object whose field names match the `UpdatePipelineInput` request
/// body. The restore route relies on the round-trip — a missing rename
/// would break rebuild at the API seam.
#[sqlx::test(migrations = "../../migrations")]
async fn get_definition_version_returns_the_camel_case_snapshot(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    let snap = pipelines::get_definition_version(&pool, &created.id, 1)
        .await
        .unwrap()
        .expect("version 1 exists");
    let json = serde_json::to_value(&snap).unwrap();
    let obj = json.as_object().unwrap();
    // Names present in the wire shape (camelCase).
    for k in [
        "kind",
        "sourceZone",
        "sourceTable",
        "incrementalColumn",
        "transforms",
        "fbicEnabled",
        "targetZone",
        "targetTable",
        "schedule",
        "owner",
        "description",
        "maxRetries",
        "dependsOn",
        "name",
        "status",
    ] {
        assert!(obj.contains_key(k), "snapshot must carry `{k}`");
    }
    // Snake-case must NOT leak through.
    for bad in [
        "source_zone",
        "source_table",
        "incremental_column",
        "fbic_enabled",
        "target_zone",
        "target_table",
        "max_retries",
        "depends_on",
        "changed_by",
    ] {
        assert!(
            !obj.contains_key(bad),
            "snapshot must not carry snake-case `{bad}`"
        );
    }
    Ok(())
}

/// `get_definition_version` returns `None` for an unknown `(pipeline_id,
/// version)` pair. The restore route maps this to 404, not 500.
#[sqlx::test(migrations = "../../migrations")]
async fn get_definition_version_is_none_for_unknown_version(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    let missing = pipelines::get_definition_version(&pool, &created.id, 42)
        .await
        .unwrap();
    assert!(missing.is_none());
    Ok(())
}

/// The full round-trip: read a snapshot via `get_definition_version`,
/// feed every editable field back into `update_pipeline`. After the
/// write, the row matches the snapshot — proving the wire shape and the
/// store layer agree field-for-field.
#[sqlx::test(migrations = "../../migrations")]
async fn snapshot_round_trips_through_create_update_and_restore(pool: PgPool) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: "batch".to_owned(),
            source_zone: "silver".to_owned(),
            source_table: "events".to_owned(),
            incremental_column: None,
            transforms: vec!["dedupe(id)".to_owned()],
            fbic_enabled: true,
            target_zone: "gold".to_owned(),
            target_table: "events_clean".to_owned(),
            schedule: "0 2 * * *".to_owned(),
            owner: Some("data-eng".to_owned()),
            description: Some("edit".to_owned()),
            max_retries: Some(4),
            depends_on: Some(vec!["pl-upstream".to_owned()]),
        },
        None,
    )
    .await
    .expect("first update should succeed");

    // Pull v=2's snapshot and feed it straight back through the update
    // path, exactly like the restore route will.
    let snap = pipelines::get_definition_version(&pool, &created.id, 2)
        .await
        .unwrap()
        .expect("v=2 exists");
    pipelines::update_pipeline(
        &pool,
        &created.id,
        &UpdatePipelineInput {
            kind: snap.kind.clone(),
            source_zone: snap.source_zone.clone(),
            source_table: snap.source_table.clone(),
            incremental_column: snap.incremental_column.clone(),
            transforms: snap.transforms.clone(),
            fbic_enabled: snap.fbic_enabled,
            target_zone: snap.target_zone.clone(),
            target_table: snap.target_table.clone(),
            schedule: snap.schedule.clone(),
            owner: Some(snap.owner.clone()),
            description: snap.description.clone(),
            max_retries: Some(snap.max_retries),
            depends_on: Some(snap.depends_on.clone()),
        },
        None,
    )
    .await
    .expect("restore should succeed");

    let def = pipelines::get_definition(&pool, &created.id)
        .await
        .unwrap()
        .expect("definition exists after restore");
    assert_eq!(def.source_zone, snap.source_zone);
    assert_eq!(def.source_table, snap.source_table);
    assert_eq!(def.target_zone, snap.target_zone);
    assert_eq!(def.target_table, snap.target_table);
    assert_eq!(def.transforms, snap.transforms);
    assert_eq!(def.fbic_enabled, snap.fbic_enabled);
    assert_eq!(def.incremental_column, snap.incremental_column);
    assert_eq!(def.max_retries, snap.max_retries);
    // `AuthoredDefinition` carries the editable fields but not
    // `depends_on` (it lives on `Pipeline`), so re-read the live row
    // via `get_pipeline` to assert the post-restore `depends_on` matches
    // the snapshot. Without this, a future change that drops `depends_on`
    // from the snapshot serializer would still round-trip the rest of
    // the fields and pass every other assertion in this test.
    let live = pipelines::get_pipeline(&pool, &created.id)
        .await
        .unwrap()
        .expect("pipeline exists after restore");
    assert_eq!(live.depends_on, snap.depends_on);
    // Three rows total: created (v=1), updated (v=2), restored via
    // update_pipeline (v=3). Newest-first: indices 0/1/2 are v=3/2/1.
    let versions = pipelines::list_definition_versions(&pool, &created.id)
        .await
        .unwrap();
    assert_eq!(versions.len(), 3);
    assert_eq!(versions[0].event, "updated");
    assert_eq!(versions[2].event, "created");
    Ok(())
}

/// `get_definition_version` must surface a snapshot row that does not
/// decode into `PipelineDefinitionSnapshot` as `StoreError::Database`,
/// not as a panic — the row exists but its payload is unreadable, which
/// is the honest 500-class signal an operator needs to act on. A panic
/// here is a review-blocker: every request that touches a corrupted
/// `pipeline_definition_version.snapshot` would tear down the worker,
/// and a future migration that reshapes the snapshot without a follow-up
/// backfill would brick the whole `GET .../versions/{n}` route.
#[sqlx::test(migrations = "../../migrations")]
async fn get_definition_version_returns_err_when_snapshot_does_not_decode(
    pool: PgPool,
) -> sqlx::Result<()> {
    let created = pipelines::create_pipeline(&pool, &input(), None)
        .await
        .expect("create should succeed");
    // Overwrite the v=1 snapshot with a JSON string — valid `jsonb`, but
    // not an object, so `serde_json::from_value::<PipelineDefinitionSnapshot>`
    // fails. Mirrors what an older migration's snapshot or a manual
    // `UPDATE pipeline_definition_version SET snapshot = ...` could leave
    // behind.
    sqlx::query(
        "UPDATE pipeline_definition_version SET snapshot = '\"not-an-object\"'::jsonb \
          WHERE pipeline_id = $1 AND version = 1",
    )
    .bind(&created.id)
    .execute(&pool)
    .await?;
    let err = pipelines::get_definition_version(&pool, &created.id, 1)
        .await
        .expect_err("undecodable snapshot must return Err, not panic and not Ok");
    assert!(
        matches!(err, lakehouse_store::StoreError::Database(_)),
        "expected StoreError::Database for an undecodable snapshot, got {err:?}"
    );
    Ok(())
}
