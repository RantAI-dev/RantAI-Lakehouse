//! Integration tests for `lakehouse_store::identity` against a real
//! Postgres.
//!
//! # Why every test here is `#[ignore]`d
//!
//! Same reason as `tests/schema.rs`: `#[sqlx::test]` needs a live Postgres
//! reachable via `DATABASE_URL` (it provisions and tears down an isolated
//! database per test), and `cargo test --workspace --locked` must stay
//! green on a machine that has none. Run them explicitly with:
//!
//! ```sh
//! docker compose up -d postgres
//! DATABASE_URL=postgres://lakehouse:lakehouse@localhost:5432/lakehouse \
//!   cargo test -p lakehouse-store -- --ignored
//! ```
//!
//! Every test below provisions a database with BOTH migrations applied, so
//! the `0002_seed_identity` fixtures are present — which is the point: the
//! seed is what the console shows on a fresh deployment, so it is worth
//! asserting against rather than hand-rolling fixtures per test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use lakehouse_store::StoreError;
use lakehouse_store::identity::{
    CreateRoleInput, CreateServiceIdentityInput, CreateTenantInput, InviteUserInput,
    ServiceIdentityFilter, TenantFilter, UserFilter, create_role, create_service_identity,
    create_tenant, create_user, delete_user, get_user, list_roles, list_service_identities,
    list_tenants, list_users,
};
use sqlx::PgPool;

/// The seed lands the full `mock/identity.ts` fixture set, and list queries
/// return it in the fixture's own order (newest `created_at` first, which
/// the staggered seed timestamps reproduce).
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_populates_every_identity_list(pool: PgPool) -> sqlx::Result<()> {
    let users = list_users(&pool, &UserFilter::default()).await.unwrap();
    assert_eq!(users.len(), 12);
    assert_eq!(users[0].name, "Rina Wijaya", "fixture order is preserved");

    let roles = list_roles(&pool).await.unwrap();
    assert_eq!(roles.len(), 7);

    let tenants = list_tenants(&pool, &TenantFilter::default()).await.unwrap();
    assert_eq!(tenants.len(), 4);

    let identities = list_service_identities(&pool, &ServiceIdentityFilter::default())
        .await
        .unwrap();
    assert_eq!(identities.len(), 6);
    Ok(())
}

/// `Tenant.users` and `Role.members` are `COUNT(*)`s over the join tables,
/// not stored columns — so they must reflect the membership rows exactly,
/// and must move when a membership does. This is the test that would fail
/// if someone "optimized" either into a denormalized column.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn counts_are_derived_from_the_join_tables(pool: PgPool) -> sqlx::Result<()> {
    let tenants = list_tenants(&pool, &TenantFilter::default()).await.unwrap();
    let group = tenants
        .iter()
        .find(|t| t.slug == "meridian-group")
        .expect("seeded tenant");
    assert_eq!(group.users, 7, "seeded memberships for meridian-group");
    assert_eq!(
        group.agents, 0,
        "no agents table exists yet; the count is an honest zero"
    );

    let roles = list_roles(&pool).await.unwrap();
    let analyst = roles
        .iter()
        .find(|r| r.name == "Analyst")
        .expect("seeded role");
    assert_eq!(analyst.members, 6);

    // Add a user holding `Analyst` in `meridian-group`; both counts move.
    create_user(
        &pool,
        &InviteUserInput {
            name: "Sinta Dewi".to_owned(),
            email: "sinta@meridian.example".to_owned(),
            roles: vec!["Analyst".to_owned()],
            tenants: vec!["Meridian Group".to_owned()],
        },
    )
    .await
    .unwrap();

    let tenants = list_tenants(&pool, &TenantFilter::default()).await.unwrap();
    assert_eq!(
        tenants
            .iter()
            .find(|t| t.slug == "meridian-group")
            .unwrap()
            .users,
        8
    );
    let roles = list_roles(&pool).await.unwrap();
    assert_eq!(
        roles.iter().find(|r| r.name == "Analyst").unwrap().members,
        7
    );
    Ok(())
}

/// An invite resolves role and tenant *names* into membership rows, and the
/// created user comes back with those names populated — the round trip the
/// console's invite dialog depends on.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_user_links_roles_and_tenants_by_name(pool: PgPool) -> sqlx::Result<()> {
    let user = create_user(
        &pool,
        &InviteUserInput {
            name: "Sinta Dewi".to_owned(),
            email: "sinta@meridian.example".to_owned(),
            roles: vec!["Analyst".to_owned(), "Approver".to_owned()],
            tenants: vec!["Meridian Group".to_owned(), "Meridian Retail".to_owned()],
        },
    )
    .await
    .unwrap();

    assert_eq!(user.status, "active", "column default");
    assert_eq!(user.roles, vec!["Analyst", "Approver"]);
    assert_eq!(user.tenants, vec!["Meridian Group", "Meridian Retail"]);
    assert!(
        user.last_activity.ends_with('Z'),
        "timestamps render like Date.toISOString(), got {}",
        user.last_activity
    );

    let refetched = get_user(&pool, &user.id).await.unwrap();
    assert_eq!(refetched, user, "list and get agree");
    Ok(())
}

/// An unknown role name inserts zero membership rows rather than raising a
/// database FK error, so it has to be caught explicitly — and it must be a
/// 400-mapped [`StoreError::ForeignKeyViolation`], with the whole invite
/// rolled back rather than a half-created user left behind.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn create_user_rejects_an_unknown_role_and_rolls_back(pool: PgPool) -> sqlx::Result<()> {
    let err = create_user(
        &pool,
        &InviteUserInput {
            name: "Ghost".to_owned(),
            email: "ghost@meridian.example".to_owned(),
            roles: vec!["No Such Role".to_owned()],
            tenants: vec!["Meridian Group".to_owned()],
        },
    )
    .await
    .expect_err("an unknown role must not create a user");
    assert!(
        matches!(err, StoreError::ForeignKeyViolation),
        "got {err:?}"
    );

    let users = list_users(&pool, &UserFilter::default()).await.unwrap();
    assert!(
        !users.iter().any(|u| u.email == "ghost@meridian.example"),
        "the failed invite must be rolled back entirely"
    );
    Ok(())
}

/// Every natural key collides as a 409, not a 500: this is the case the
/// `StoreError` mapping exists for, exercised through the repository rather
/// than through raw SQL.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn duplicate_natural_keys_are_conflicts(pool: PgPool) -> sqlx::Result<()> {
    let dup_email = create_user(
        &pool,
        &InviteUserInput {
            name: "Rina Again".to_owned(),
            email: "rina@meridian.example".to_owned(),
            roles: vec![],
            tenants: vec![],
        },
    )
    .await
    .expect_err("email is unique");
    assert!(matches!(dup_email, StoreError::Conflict), "{dup_email:?}");

    let dup_slug = create_tenant(
        &pool,
        &CreateTenantInput {
            name: "Another Meridian".to_owned(),
            slug: "meridian-group".to_owned(),
            plan: "Trial".to_owned(),
            residency: "Jakarta (ID)".to_owned(),
        },
    )
    .await
    .expect_err("slug is unique");
    assert!(matches!(dup_slug, StoreError::Conflict), "{dup_slug:?}");

    let dup_role = create_role(
        &pool,
        &CreateRoleInput {
            name: "Analyst".to_owned(),
            permissions: String::new(),
            description: String::new(),
        },
    )
    .await
    .expect_err("role name is unique");
    assert!(matches!(dup_role, StoreError::Conflict), "{dup_role:?}");

    let dup_identity = create_service_identity(
        &pool,
        &CreateServiceIdentityInput {
            name: "bi-dashboard-reader".to_owned(),
            scopes: vec![],
            environment: "staging".to_owned(),
        },
    )
    .await
    .expect_err("identity name is unique");
    assert!(
        matches!(dup_identity, StoreError::Conflict),
        "{dup_identity:?}"
    );
    Ok(())
}

/// New rows land with the defaults `mock/identity.ts` used, so a create
/// through the real backend looks like a create always did.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn creates_use_the_mock_fixtures_defaults(pool: PgPool) -> sqlx::Result<()> {
    let tenant = create_tenant(
        &pool,
        &CreateTenantInput {
            name: "Meridian Freight".to_owned(),
            slug: "meridian-freight".to_owned(),
            plan: "Trial".to_owned(),
            residency: "Jakarta (ID)".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(tenant.users, 0);
    assert_eq!(tenant.agents, 0);
    assert_eq!(tenant.storage_bytes, 0);
    assert_eq!(tenant.quota_compute, 5000, "mock's createTenant default");
    assert_eq!(tenant.used_compute, 0);

    let role = create_role(
        &pool,
        &CreateRoleInput {
            name: "Auditor".to_owned(),
            permissions: "audit:read".to_owned(),
            description: "Read the audit log.".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(role.members, 0, "a new role has no members");

    let identity = create_service_identity(
        &pool,
        &CreateServiceIdentityInput {
            name: "report-mailer".to_owned(),
            scopes: vec!["query:read".to_owned()],
            environment: "staging".to_owned(),
        },
    )
    .await
    .unwrap();
    assert_eq!(identity.rotation_status, "current");
    assert_eq!(identity.scopes, vec!["query:read"]);
    assert!(
        identity.expires_at > identity.last_used_at,
        "a new credential expires in the future"
    );
    Ok(())
}

/// The list filters narrow on the server rather than shipping everything to
/// the caller.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn list_filters_narrow_results(pool: PgPool) -> sqlx::Result<()> {
    let inactive = list_users(
        &pool,
        &UserFilter {
            status: Some("inactive".to_owned()),
            tenant_slug: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(inactive.len(), 2);
    assert!(inactive.iter().all(|u| u.status == "inactive"));

    let retail = list_users(
        &pool,
        &UserFilter {
            status: None,
            tenant_slug: Some("meridian-retail".to_owned()),
        },
    )
    .await
    .unwrap();
    assert_eq!(retail.len(), 6);
    assert!(
        retail
            .iter()
            .all(|u| u.tenants.contains(&"Meridian Retail".to_owned()))
    );

    let enterprise = list_tenants(
        &pool,
        &TenantFilter {
            plan: Some("Enterprise".to_owned()),
        },
    )
    .await
    .unwrap();
    assert_eq!(enterprise.len(), 2);

    let staging = list_service_identities(
        &pool,
        &ServiceIdentityFilter {
            environment: Some("staging".to_owned()),
        },
    )
    .await
    .unwrap();
    assert_eq!(staging.len(), 1);
    assert_eq!(staging[0].name, "price-crawler-agent");
    Ok(())
}

/// Deleting a user cascades its memberships away (so the derived tenant
/// count drops) and a subsequent read is a 404-mapped `NotFound`, not a
/// 500. A junk id is the same `NotFound`, never a decode error.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn delete_user_cascades_memberships(pool: PgPool) -> sqlx::Result<()> {
    let users = list_users(
        &pool,
        &UserFilter {
            status: None,
            tenant_slug: Some("meridian-logistics".to_owned()),
        },
    )
    .await
    .unwrap();
    let victim = users
        .iter()
        .find(|u| u.email == "maya@meridian.example")
        .expect("seeded user");

    delete_user(&pool, &victim.id).await.unwrap();

    let err = get_user(&pool, &victim.id)
        .await
        .expect_err("deleted user must be gone");
    assert!(matches!(err, StoreError::NotFound), "{err:?}");

    let tenants = list_tenants(&pool, &TenantFilter::default()).await.unwrap();
    assert_eq!(
        tenants
            .iter()
            .find(|t| t.slug == "meridian-logistics")
            .unwrap()
            .users,
        5,
        "the membership row went with the user"
    );

    let junk = get_user(&pool, "definitely-not-a-uuid")
        .await
        .expect_err("a junk id is not found, not a database error");
    assert!(matches!(junk, StoreError::NotFound), "{junk:?}");
    Ok(())
}

/// The seed is safe to apply twice.
///
/// `sqlx::migrate!` would never re-run it, so this executes the migration's
/// SQL directly against an already-seeded database — the situation an
/// operator creates by hand when re-seeding an environment. Nothing may
/// fail, and nothing may double up.
#[sqlx::test(migrations = "../../migrations")]
#[ignore = "requires a live Postgres; see module doc comment"]
async fn seed_is_idempotent_when_applied_twice(pool: PgPool) -> sqlx::Result<()> {
    let seed = include_str!("../../../migrations/0002_seed_identity.sql");
    sqlx::raw_sql(seed).execute(&pool).await?;

    assert_eq!(
        list_users(&pool, &UserFilter::default())
            .await
            .unwrap()
            .len(),
        12
    );
    assert_eq!(list_roles(&pool).await.unwrap().len(), 7);
    assert_eq!(
        list_tenants(&pool, &TenantFilter::default())
            .await
            .unwrap()
            .len(),
        4
    );
    assert_eq!(
        list_service_identities(&pool, &ServiceIdentityFilter::default())
            .await
            .unwrap()
            .len(),
        6
    );
    let tenants = list_tenants(&pool, &TenantFilter::default()).await.unwrap();
    assert_eq!(
        tenants
            .iter()
            .find(|t| t.slug == "meridian-group")
            .unwrap()
            .users,
        7,
        "membership rows must not double up either"
    );
    Ok(())
}
