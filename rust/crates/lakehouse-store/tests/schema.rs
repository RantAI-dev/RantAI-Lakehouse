//! Integration tests against a real Postgres, exercising the `0001_init`
//! migration and the [`StoreError`] classification it feeds.
//!
//! # Why every test here is `#[ignore]`d
//!
//! `#[sqlx::test]` needs a live Postgres reachable via `DATABASE_URL` (it
//! provisions and tears down an isolated database per test). CI, and any
//! contributor's machine without `docker compose up -d postgres` running,
//! has no such thing — and `cargo test --workspace --locked` must stay
//! green in that environment, exactly as it did before this crate existed.
//! So every test below is `#[ignore]`d: `cargo test --workspace` skips them
//! by default, and they are run explicitly, with Postgres up, via:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-store -- --ignored
//! ```
//!
//! This is a deliberate gap in default `cargo test` coverage, not an
//! oversight — see the crate's root doc comment for the boot-behavior
//! reasoning this mirrors.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_core::ApiError;
use lakehouse_store::StoreError;
use sqlx::PgPool;

/// Migrations applying cleanly from empty is implicit in every other test
/// here (`#[sqlx::test]` runs them as part of provisioning the test
/// database and fails the test if they don't apply), but this test pins it
/// down explicitly and independently of any table-specific behavior.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn migrations_apply_cleanly_from_empty(pool: PgPool) -> sqlx::Result<()> {
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = 'public' ORDER BY table_name",
    )
    .fetch_all(&pool)
    .await?;
    let names: Vec<&str> = tables.iter().map(|(n,)| n.as_str()).collect();
    for expected in [
        "tenant",
        "role",
        "app_user",
        "app_user_role",
        "app_user_tenant",
        "service_identity",
    ] {
        assert!(names.contains(&expected), "missing table: {expected}");
    }
    Ok(())
}

/// A foreign key genuinely rejects an orphan row: `app_user_tenant`
/// references `app_user`/`tenant` by id, and inserting a row that points at
/// ids nothing else in the database has ever inserted must fail.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn foreign_key_rejects_orphan_row(pool: PgPool) -> sqlx::Result<()> {
    let result = sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(uuid::Uuid::new_v4())
        .bind(uuid::Uuid::new_v4())
        .execute(&pool)
        .await;

    let err = result.expect_err("orphan FK insert must fail");
    let store_err: StoreError = err.into();
    assert!(
        matches!(store_err, StoreError::ForeignKeyViolation),
        "expected ForeignKeyViolation, got {store_err:?}"
    );
    let api_err: ApiError = store_err.into();
    assert_eq!(api_err.status(), 400);
    Ok(())
}

/// A unique constraint genuinely rejects a duplicate: `tenant.slug` is
/// unique, so inserting the same slug twice must fail on the second
/// attempt, classified as [`StoreError::Conflict`] and rendered as HTTP 409
/// — this is the exact case the module doc comment on
/// `lakehouse_store::error` argues should NOT blanket-map to 500/422.
///
/// Uses a synthetic `constraint-probe` slug rather than a seeded one: as of
/// the `0002_seed_identity` migration the test database is not empty, and
/// the point here is to observe the SECOND insert failing, not the first.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn unique_constraint_rejects_duplicate(pool: PgPool) -> sqlx::Result<()> {
    let insert = "INSERT INTO tenant (name, slug, plan, residency) VALUES ($1, $2, $3, $4)";
    sqlx::query(insert)
        .bind("Constraint Probe")
        .bind("constraint-probe")
        .bind("Enterprise")
        .bind("Jakarta (ID)")
        .execute(&pool)
        .await?;

    let dup = sqlx::query(insert)
        .bind("Constraint Probe (duplicate slug)")
        .bind("constraint-probe")
        .bind("Enterprise")
        .bind("Jakarta (ID)")
        .execute(&pool)
        .await;

    let err = dup.expect_err("duplicate slug insert must fail");
    let store_err: StoreError = err.into();
    assert!(
        matches!(store_err, StoreError::Conflict),
        "expected Conflict, got {store_err:?}"
    );
    let api_err: ApiError = store_err.into();
    assert_eq!(api_err.status(), 409);
    Ok(())
}

/// `app_user.email` is unique, `role.name` is unique, and
/// `service_identity.name` is unique — each domain's own natural key.
///
/// Every value inserted here is a deliberately synthetic `constraint-probe`
/// rather than a realistic name: since Task 2.2 added the
/// `0002_seed_identity` migration, `#[sqlx::test]` provisions a database
/// that already contains the demo fixtures, so probing a constraint with a
/// name the seed also uses would collide on the FIRST insert and fail the
/// test for the wrong reason.
/// Covers the other two `UNIQUE` constraints the migration adds beyond
/// `tenant.slug`, so a future edit narrowing any one of them to a
/// non-unique column is caught here.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn unique_constraints_cover_every_natural_key(pool: PgPool) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO app_user (name, email) VALUES ('Probe', 'probe@example.com')")
        .execute(&pool)
        .await?;
    let dup_email =
        sqlx::query("INSERT INTO app_user (name, email) VALUES ('Probe 2', 'probe@example.com')")
            .execute(&pool)
            .await;
    assert!(dup_email.is_err());

    sqlx::query("INSERT INTO role (name) VALUES ('Constraint Probe')")
        .execute(&pool)
        .await?;
    let dup_role = sqlx::query("INSERT INTO role (name) VALUES ('Constraint Probe')")
        .execute(&pool)
        .await;
    assert!(dup_role.is_err());

    sqlx::query(
        "INSERT INTO service_identity (name, environment, expires_at) \
         VALUES ('constraint-probe', 'production', now())",
    )
    .execute(&pool)
    .await?;
    let dup_service = sqlx::query(
        "INSERT INTO service_identity (name, environment, expires_at) \
         VALUES ('constraint-probe', 'staging', now())",
    )
    .execute(&pool)
    .await;
    assert!(dup_service.is_err());

    Ok(())
}

/// `app_user.status` is constrained to the contract's closed
/// `"active" | "inactive"` union; anything else must be rejected at the
/// database boundary, not just by whichever service-layer code happens to
/// validate it first.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn app_user_status_check_rejects_unknown_value(pool: PgPool) -> sqlx::Result<()> {
    let result = sqlx::query(
        "INSERT INTO app_user (name, email, status) VALUES ('X', 'x@example.com', 'pending')",
    )
    .execute(&pool)
    .await;
    assert!(result.is_err(), "CHECK constraint must reject 'pending'");
    Ok(())
}

/// `ON DELETE RESTRICT` on `app_user_tenant.tenant_id`: a tenant that a user
/// still belongs to cannot be deleted out from under them — deliberately
/// the opposite of `app_user_role`/`app_user_tenant`'s `ON DELETE CASCADE`
/// on the `app_user` side, where deleting the user is what should clean up
/// its membership rows.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn tenant_delete_is_restricted_while_a_user_belongs_to_it(pool: PgPool) -> sqlx::Result<()> {
    let (tenant_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO tenant (name, slug, plan, residency) \
         VALUES ('T', 't-slug', 'Enterprise', 'Jakarta (ID)') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    let (user_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO app_user (name, email) VALUES ('U', 'u@example.com') RETURNING id",
    )
    .fetch_one(&pool)
    .await?;
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(tenant_id)
        .execute(&pool)
        .await?;

    let delete = sqlx::query("DELETE FROM tenant WHERE id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await;
    assert!(
        delete.is_err(),
        "RESTRICT must block deleting a tenant a user still belongs to"
    );

    // Deleting the user first cascades away the membership row, after which
    // the tenant delete succeeds — proves RESTRICT is scoped to the
    // membership row, not permanently blocking the tenant.
    sqlx::query("DELETE FROM app_user WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM tenant WHERE id = $1")
        .bind(tenant_id)
        .execute(&pool)
        .await?;
    Ok(())
}
