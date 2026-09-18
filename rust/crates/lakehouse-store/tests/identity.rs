//! Integration tests for `lakehouse_store::identity` against a real
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
//!
//! Every test below provisions a database with BOTH migrations applied, so
//! the `0002_seed_identity` fixtures are present — which is the point: the
//! seed is what the console shows on a fresh deployment, so it is worth
//! asserting against rather than hand-rolling fixtures per test.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::StoreError;
use lakehouse_store::identity::{
    CreateRoleInput, CreateServiceIdentityInput, CreateTenantInput, InviteUserInput,
    ServiceIdentityFilter, TenantFilter, UserFilter, create_role, create_service_identity,
    create_tenant, create_user, delete_user, get_service_identity, get_tenant, get_user,
    list_roles, list_service_identities, list_tenants, list_users, rotate_service_identity,
};
use sqlx::PgPool;

/// The seed lands the full `mock/identity.ts` fixture set, and list queries
/// return it in the fixture's own order (newest `created_at` first, which
/// the staggered seed timestamps reproduce).
#[sqlx::test(migrations = "../../migrations")]
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
    // J18: nothing writes `app_user.last_activity_at`, so a freshly created
    // user must not be served its insert-time default as "active now".
    assert_eq!(
        user.last_activity, None,
        "last_activity is never served: nothing writes the column"
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
    // J18: nothing writes `service_identity.last_used_at`, so a freshly
    // created identity must not be served its insert-time default as "used
    // just now".
    assert_eq!(
        identity.last_used_at, None,
        "last_used_at is never served: nothing writes the column"
    );
    Ok(())
}

/// `rotation_status` is stored, but authentication
/// (`lakehouse-auth::service_token`) already refuses a credential once
/// `expires_at <= now()`. An identity that is still marked `'current'` in
/// the table but has aged past its `expires_at` must read `"expired"` from
/// both `list_service_identities` and `get_service_identity`, so a filter
/// and the list can never disagree with each other or with what
/// authentication actually does.
#[sqlx::test(migrations = "../../migrations")]
async fn an_identity_past_its_expiry_reads_expired_even_if_stored_current(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO service_identity (name, scopes, environment, rotation_status, expires_at) \
         VALUES ($1, $2, $3, 'current', now() - interval '1 minute') RETURNING id",
    )
    .bind("stale-credential")
    .bind(vec!["query:read".to_owned()])
    .bind("staging")
    .fetch_one(&pool)
    .await?;

    let fetched = get_service_identity(&pool, &id.to_string()).await.unwrap();
    assert_eq!(fetched.rotation_status, "expired");

    let listed = list_service_identities(&pool, &ServiceIdentityFilter::default())
        .await
        .unwrap();
    let listed = listed
        .iter()
        .find(|s| s.id == id.to_string())
        .expect("just-inserted identity");
    assert_eq!(listed.rotation_status, "expired");
    Ok(())
}

/// An identity that has not yet reached `expires_at` reports whatever
/// `rotation_status` is stored, unchanged — no `"due"` threshold is derived,
/// because none is specified (`0001_init.sql`'s header comment for the
/// column says so explicitly).
#[sqlx::test(migrations = "../../migrations")]
async fn an_unexpired_identity_keeps_its_stored_status(pool: PgPool) -> sqlx::Result<()> {
    let (id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO service_identity (name, scopes, environment, rotation_status, expires_at) \
         VALUES ($1, $2, $3, 'due', now() + interval '1 day') RETURNING id",
    )
    .bind("aging-credential")
    .bind(vec!["query:read".to_owned()])
    .bind("staging")
    .fetch_one(&pool)
    .await?;

    let fetched = get_service_identity(&pool, &id.to_string()).await.unwrap();
    assert_eq!(fetched.rotation_status, "due");

    let listed = list_service_identities(&pool, &ServiceIdentityFilter::default())
        .await
        .unwrap();
    let listed = listed
        .iter()
        .find(|s| s.id == id.to_string())
        .expect("just-inserted identity");
    assert_eq!(listed.rotation_status, "due");
    Ok(())
}

/// Both never-written activity timestamps come back `None`, and — because
/// this is the wire-format-facing check — the *serialized* JSON keeps the
/// key present with an explicit `null` rather than dropping it, matching
/// what `src/services/contracts/identity.ts` declares (`string | null`, not
/// an optional field).
#[sqlx::test(migrations = "../../migrations")]
async fn identity_activity_timestamps_are_not_served(pool: PgPool) -> sqlx::Result<()> {
    let users = list_users(&pool, &UserFilter::default()).await.unwrap();
    assert!(
        users.iter().all(|u| u.last_activity.is_none()),
        "no seeded user's last_activity may be served"
    );
    let user_json = serde_json::to_value(&users[0]).unwrap();
    assert_eq!(
        user_json.get("lastActivity"),
        Some(&serde_json::Value::Null)
    );

    let identities = list_service_identities(&pool, &ServiceIdentityFilter::default())
        .await
        .unwrap();
    assert!(
        identities.iter().all(|s| s.last_used_at.is_none()),
        "no seeded identity's last_used_at may be served"
    );
    let identity_json = serde_json::to_value(&identities[0]).unwrap();
    assert_eq!(
        identity_json.get("lastUsedAt"),
        Some(&serde_json::Value::Null)
    );
    Ok(())
}

/// The list filters narrow on the server rather than shipping everything to
/// the caller.
#[sqlx::test(migrations = "../../migrations")]
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

/// WS8 plan Phase B, Task B1 (migration `0042_tenant_provisioning.sql`,
/// renumbered from the plan's `0040` — see that file's why-header): a
/// tenant seeded by `0002_seed_identity.sql` predates provisioning
/// entirely, so it must land on `not_applicable`, never `complete` — the
/// P2 fix's whole point is that nothing was actually provisioned for it.
#[sqlx::test(migrations = "../../migrations")]
async fn seeded_tenant_is_grandfathered_not_applicable_not_complete(
    pool: PgPool,
) -> sqlx::Result<()> {
    let tenant = get_tenant(&pool, "11111111-1111-4111-8111-000000000001")
        .await
        .unwrap();
    assert_eq!(tenant.warehouse_id, None);
    assert_eq!(tenant.provisioning_status, "not_applicable");
    Ok(())
}

/// The P2 fix backfills exactly the two connector rows
/// `0022_prune_connector_seed.sql` seeds, and no others, to the seed
/// tenant id.
#[sqlx::test(migrations = "../../migrations")]
async fn seeded_connectors_are_backfilled_to_the_seed_tenant(pool: PgPool) -> sqlx::Result<()> {
    let rows: Vec<(String, Option<sqlx::types::Uuid>)> = sqlx::query_as(
        "SELECT id, tenant_id FROM connector WHERE id IN ('conn-pg-lakehouse', 'conn-s3-warehouse') ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    for (_, tenant_id) in &rows {
        assert_eq!(
            tenant_id.map(|u| u.to_string()),
            Some("11111111-1111-4111-8111-000000000001".to_owned())
        );
    }
    Ok(())
}

/// `0027_prune_seeded_activity.sql` already deletes every
/// `pipeline_definition` row `0008_seed_pipelines.sql` seeded, so
/// `0042_tenant_provisioning.sql` has nothing to backfill in that table —
/// asserting that stays zero guards against a future migration silently
/// reintroducing a backfill target this one deliberately does not claim.
#[sqlx::test(migrations = "../../migrations")]
async fn no_pipeline_definition_row_is_backfilled_because_none_are_seeded(
    pool: PgPool,
) -> sqlx::Result<()> {
    let (count,): (i64,) = sqlx::query_as("SELECT count(*) FROM pipeline_definition")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "0027 already deleted every seeded pipeline_definition row — 0042 must not invent a backfill target here"
    );
    Ok(())
}

// ── WS8 plan §Phase E (Hard Requirement 5): rotate_service_identity ─────

/// Seed a `service_identity` row exactly the shape the plan's `seed_*`
/// helpers produce (name + scopes + environment + future expiry +
/// `rotation_status = 'current'`), so the rotate tests' input matches what
/// production seeding would create. The `name` is suffixed with a fresh
/// `Uuid` because `#[sqlx::test]` runs every test in the same binary in
/// the SAME database — only the test function gets a fresh connection per
/// run, not a fresh schema (the `service_identity_name_unique` constraint
/// would otherwise block a second test reusing a literal name).
///
/// Returned `id` is the freshly-inserted `service_identity.id`. The
/// identity is created without a `service_credential.token_hash` row — the
/// production `create_service_identity` doesn't insert one either (it
/// only registers metadata, per its own doc comment), and the rotate
/// tests that need a pre-existing credential insert one explicitly.
async fn seed_service_identity(pool: &PgPool, name: &str) -> sqlx::Result<uuid::Uuid> {
    let unique_name = format!("{name}-{}", uuid::Uuid::new_v4().simple());
    let (id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO service_identity (name, scopes, environment, rotation_status, expires_at) \
         VALUES ($1, $2, $3, 'current', now() + interval '90 days') RETURNING id",
    )
    .bind(&unique_name)
    .bind(vec!["query:read".to_owned()])
    .bind("production")
    .fetch_one(pool)
    .await?;
    Ok(id)
}

/// `rotate_service_identity` writes the new credential with whatever hash
/// the mint callback returns, and the response's `secret` is whatever
/// string the mint callback puts in the first tuple slot. This test
/// exercises that wiring end to end through the store, without depending
/// on `lakehouse-auth` (which `lakehouse-store`'s tests cannot import —
/// `lakehouse-auth` depends on `lakehouse-store`, and Cargo rejects the
/// resulting cycle as either a regular or dev-dep; see the rotation
/// function's doc comment for the full reasoning). The "hash produced by
/// the store equals the hash `lakehouse_auth::token::hash_token` would
/// compute for the same secret" property is asserted in
/// `lakehouse-auth`'s test suite (end-to-end, via
/// `verify_service_token`).
#[sqlx::test(migrations = "../../migrations")]
async fn rotate_returns_the_new_secret_and_persists_its_hash(pool: PgPool) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;

    let response = rotate_service_identity(&pool, identity_id, || {
        (
            "a-freshly-minted-raw-token".to_owned(),
            "stored-hash-for-it".to_owned(),
        )
    })
    .await
    .unwrap();

    assert_eq!(response.secret, "a-freshly-minted-raw-token");
    let (stored_hash,): (String,) = sqlx::query_as(
        "SELECT token_hash FROM service_credential WHERE service_identity_id = $1 \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(identity_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        stored_hash, "stored-hash-for-it",
        "the hash persisted in service_credential.token_hash is the one the mint callback provided"
    );
    Ok(())
}

/// Hard Requirement 5 — "old credential's fate": every previously-unrevoked
/// credential for the identity must be `revoked_at` non-NULL once rotation
/// succeeds, and the freshly inserted credential must be the only
/// `revoked_at IS NULL` row for this identity.
///
/// We can't import `lakehouse_auth::service_token::verify_service_token`
/// here (same cycle), so we exercise the property structurally: the SQL
/// `service_token.rs::verify_service_token` filters on (`token_hash` matches,
/// `revoked_at` IS NULL, identity not expired), and the only row that
/// satisfies that after a rotation is the one the mint callback just
/// inserted. The "leaked token can no longer authenticate" assertion is
/// made end-to-end in `lakehouse-auth`'s own test suite, where both
/// `lakehouse-auth` and the rest of the stack are reachable.
#[sqlx::test(migrations = "../../migrations")]
async fn rotate_revokes_old_credentials_and_leaves_only_the_new_one_active(
    pool: PgPool,
) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;

    // Seed two pre-existing credentials — one already revoked, one still
    // active. After rotation, the active one must be revoked, the previously
    // revoked one stays revoked, and the new one is active.
    let (old_active_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO service_credential (service_identity_id, token_hash) VALUES ($1, $2) RETURNING id",
    )
    .bind(identity_id)
    .bind("old-active-hash")
    .fetch_one(&pool)
    .await?;
    let (already_revoked_id,): (uuid::Uuid,) = sqlx::query_as(
        "INSERT INTO service_credential (service_identity_id, token_hash, revoked_at) \
         VALUES ($1, $2, now() - interval '1 hour') RETURNING id",
    )
    .bind(identity_id)
    .bind("already-revoked-hash")
    .fetch_one(&pool)
    .await?;

    rotate_service_identity(&pool, identity_id, || {
        ("new-raw-token".to_owned(), "new-stored-hash".to_owned())
    })
    .await
    .unwrap();

    let rows: Vec<(String, Option<time::OffsetDateTime>)> = sqlx::query_as(
        "SELECT token_hash, revoked_at FROM service_credential WHERE service_identity_id = $1 \
         ORDER BY created_at ASC",
    )
    .bind(identity_id)
    .fetch_all(&pool)
    .await?;
    let by_hash: std::collections::HashMap<&str, Option<time::OffsetDateTime>> =
        rows.iter().map(|(h, r)| (h.as_str(), *r)).collect();

    assert!(
        by_hash
            .get("already-revoked-hash")
            .and_then(|r| r.as_ref())
            .is_some(),
        "a previously-revoked credential must stay revoked"
    );
    assert!(
        by_hash
            .get("old-active-hash")
            .and_then(|r| r.as_ref())
            .is_some(),
        "the previously-active credential must be revoked by rotation"
    );
    assert!(
        by_hash
            .get("new-stored-hash")
            .and_then(|r| r.as_ref())
            .is_none(),
        "the freshly inserted credential must be the only unrevoked one"
    );

    // And there is exactly one unrevoked credential for this identity.
    let (active_count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM service_credential WHERE service_identity_id = $1 AND revoked_at IS NULL",
    )
    .bind(identity_id)
    .fetch_one(&pool)
    .await?;
    assert_eq!(
        active_count, 1,
        "exactly one active credential after rotation"
    );

    // The two pre-existing ids are still around (revoke is not delete) —
    // a future "log every credential ever issued" report still needs them.
    assert!(
        (by_hash.len() == 3) || (by_hash.len() == 2 && old_active_id != already_revoked_id),
        "rotate must not delete old credential rows"
    );
    Ok(())
}

/// Reset `expires_at` to a fresh `NEW_IDENTITY_VALIDITY_DAYS` window and
/// `rotation_status` to `'current'` — matches the shape of a freshly
/// created identity (`create_service_identity`'s doc comment), so a
/// rotation looks exactly like a brand-new identity for downstream
/// filtering and authentication purposes.
#[sqlx::test(migrations = "../../migrations")]
async fn rotate_resets_expires_at_to_a_fresh_window_and_status_to_current(
    pool: PgPool,
) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;

    // Drive the seeded identity into the "about to expire" shape the plan's
    // `seed_service_identity_expiring_soon` would — already past its
    // expiry. The post-rotation row must come back with `expires_at > now()
    // + 29 days` regardless of where it was.
    sqlx::query(
        "UPDATE service_identity SET expires_at = now() + interval '1 day', \
         rotation_status = 'due' WHERE id = $1",
    )
    .bind(identity_id)
    .execute(&pool)
    .await?;

    rotate_service_identity(&pool, identity_id, || {
        ("new-raw-token".to_owned(), "new-stored-hash".to_owned())
    })
    .await
    .unwrap();

    let (expires_at, rotation_status): (time::OffsetDateTime, String) =
        sqlx::query_as("SELECT expires_at, rotation_status FROM service_identity WHERE id = $1")
            .bind(identity_id)
            .fetch_one(&pool)
            .await?;
    assert!(
        expires_at > time::OffsetDateTime::now_utc() + time::Duration::days(29),
        "fresh expires_at must be > now + 29 days, got {expires_at}"
    );
    assert_eq!(
        rotation_status, "current",
        "rotation must reset rotation_status to 'current'"
    );
    Ok(())
}

/// Rotating an identity that does not exist is `StoreError::NotFound` —
/// the route surfaces this as 404. The function does not try to first
/// read and then write (a check-then-act that would race a concurrent
/// delete); the final `get_service_identity` re-read is what raises the
/// `NotFound` if the `UPDATE`s touched zero rows.
#[sqlx::test(migrations = "../../migrations")]
async fn rotate_an_unknown_identity_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let bogus = uuid::Uuid::from_u128(0xDEAD_BEEF);
    let err = rotate_service_identity(&pool, bogus, || ("unused".to_owned(), "unused".to_owned()))
        .await
        .expect_err("rotating an unknown identity must surface NotFound");
    assert!(matches!(err, StoreError::NotFound), "got {err:?}");
    Ok(())
}

/// The mint callback's exception short-circuits the whole transaction —
/// if it returns a (token, hash) pair that the SQL layer can use, every
/// other invariant (revocation, expiry reset, row insert) holds. This
/// test pins down the response field carrying the exact secret the
/// callback returned, which is the load-bearing property: the caller must
/// see the same string that was hashed.
#[sqlx::test(migrations = "../../migrations")]
async fn rotate_response_secret_matches_the_mint_callback_output(pool: PgPool) -> sqlx::Result<()> {
    let identity_id = seed_service_identity(&pool, "ingestion-worker").await?;

    let expected_secret = "callers-raw-secret-string";
    let response = rotate_service_identity(&pool, identity_id, || {
        (expected_secret.to_owned(), "expected-hash".to_owned())
    })
    .await
    .unwrap();

    assert_eq!(response.secret, expected_secret);
    assert_eq!(
        response.identity.id,
        identity_id.to_string(),
        "the response must carry the freshly re-read identity, including the post-rotation expires_at/rotation_status"
    );
    assert_eq!(response.identity.rotation_status, "current");
    Ok(())
}
