//! Integration tests for `lakehouse_auth::repository` against a real
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
//! Every test below provisions a database with all migrations applied
//! (including `0002_seed_identity`), so the `mock/identity.ts` fixture set
//! is present.

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_auth::AuthError;
use lakehouse_auth::repository::load_principal_for_user;
use sqlx::PgPool;
use uuid::Uuid;

/// Rina Wijaya (seeded) holds `Analyst` + `Approver` and belongs to
/// `meridian-group`. `load_principal_for_user` must merge both roles'
/// permissions and collect the tenant membership.
#[sqlx::test(migrations = "../../migrations")]
async fn loads_a_seeded_users_merged_permissions_and_tenants(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str("33333333-3333-4333-8333-000000000001").unwrap();
    let meridian_group = Uuid::parse_str("11111111-1111-4111-8111-000000000001").unwrap();

    let principal = load_principal_for_user(&pool, rina_id, "local".to_owned(), false)
        .await
        .unwrap();

    assert_eq!(principal.display_name, "Rina Wijaya");
    assert_eq!(principal.provider, "local");
    // Analyst -> query:read, catalog:read, lineage:read
    assert!(principal.has("query:read"));
    assert!(principal.has("catalog:read"));
    // Approver -> agent:approve, policy:review
    assert!(principal.has("agent:approve"));
    assert!(principal.has("policy:review"));
    // Never granted to either role.
    assert!(!principal.has("pipeline:run"));
    assert!(principal.in_tenant(meridian_group));
    Ok(())
}

/// Platform Admin's `*:*` must round-trip through the real column, not
/// just through unit-tested parsing logic.
#[sqlx::test(migrations = "../../migrations")]
async fn platform_admin_wildcard_grants_everything_end_to_end(pool: PgPool) -> sqlx::Result<()> {
    let fajar_id = Uuid::parse_str("33333333-3333-4333-8333-000000000006").unwrap();
    let principal = load_principal_for_user(&pool, fajar_id, "local".to_owned(), false)
        .await
        .unwrap();
    assert!(principal.has("anything:whatsoever"));
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn an_unknown_user_id_is_not_found(pool: PgPool) -> sqlx::Result<()> {
    let err = load_principal_for_user(&pool, Uuid::nil(), "local".to_owned(), false)
        .await
        .unwrap_err();
    assert!(matches!(err, AuthError::NotFound));
    Ok(())
}

/// `0020_extend_role_grants.sql` end to end: Bayu Pratama (seeded Data
/// Engineer) must hold the two tokens that migration adds to that role
/// (`workload:cancel`, `alert:write` — gating `POST /api/ops/workloads/{id}/
/// cancel` and `/api/alerts` CRUD in `lakehouse_api::policy`), but neither
/// of the two deliberately Platform-Admin-only tokens from the same
/// change (`agent:manage`, `storage:restore`) — this is a real column
/// read, not just a parsed literal, so it catches the migration failing to
/// apply as much as it catches a typo in the token string.
#[sqlx::test(migrations = "../../migrations")]
async fn data_engineer_gains_workload_cancel_and_alert_write_from_0020(
    pool: PgPool,
) -> sqlx::Result<()> {
    let bayu_id = Uuid::parse_str("33333333-3333-4333-8333-000000000002").unwrap();
    let principal = load_principal_for_user(&pool, bayu_id, "local".to_owned(), false)
        .await
        .unwrap();

    assert!(principal.has("pipeline:read"), "still holds pipeline:*");
    assert!(principal.has("workload:cancel"));
    assert!(principal.has("alert:write"));
    assert!(!principal.has("agent:manage"));
    assert!(!principal.has("storage:restore"));
    Ok(())
}

/// The same migration, from the other side: Platform Admin's `*:*` still
/// satisfies every one of the four new tokens, including the two
/// deliberately admin-only ones with no explicit grant row.
#[sqlx::test(migrations = "../../migrations")]
async fn platform_admin_satisfies_all_four_new_tokens_from_0020(pool: PgPool) -> sqlx::Result<()> {
    let fajar_id = Uuid::parse_str("33333333-3333-4333-8333-000000000006").unwrap();
    let principal = load_principal_for_user(&pool, fajar_id, "local".to_owned(), false)
        .await
        .unwrap();

    assert!(principal.has("agent:manage"));
    assert!(principal.has("workload:cancel"));
    assert!(principal.has("storage:restore"));
    assert!(principal.has("alert:write"));
    Ok(())
}

// ── WS7 item E4: an active, unexpired access_grant widens `permissions`
//    (never `role_names`) ─────────────────────────────────────────────────

/// Inserts one `access_grant` row directly (never through
/// `lakehouse_store::agents::decide_access_request` — this test exercises
/// ONLY `load_principal_for_user`'s read side, WS7 item E4, independent of
/// how the row got there), against a synthetic `approval_item` row it also
/// creates (both `access_grant.approval_id` and `.user_id` are `NOT NULL`
/// foreign keys, so a real referenced row is required either way).
async fn insert_active_grant(
    pool: &PgPool,
    user_id: Uuid,
    permission: &str,
    expires_at: time::OffsetDateTime,
) {
    let approval_id = format!("appr-test-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO approval_item (id, kind, requested_by_user_id, action, status) \
         VALUES ($1, 'access', $2, $3, 'approved')",
    )
    .bind(&approval_id)
    .bind(user_id)
    .bind(format!("access:{permission}"))
    .execute(pool)
    .await
    .expect("seed a synthetic approved access_item");

    let grant_id = format!("grant-test-{}", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO access_grant (id, approval_id, user_id, permission, expires_at) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&grant_id)
    .bind(&approval_id)
    .bind(user_id)
    .bind(permission)
    .bind(expires_at)
    .execute(pool)
    .await
    .expect("seed an access_grant row");
}

/// Failing-test-first for WS7 item E4: before this task,
/// `load_principal_for_user` never read `access_grant` at all, so a
/// principal holding a real, active, unexpired grant for a permission
/// their ROLES do not carry would still fail `principal.has(...)` for it.
/// Quoted failure text (`cargo test -p lakehouse-auth
/// repository::load_principal 2>&1 | tail -30` against the pre-E4 state —
/// run as `cargo test -p lakehouse-auth --test repository`, since this
/// crate's own convention keeps every DB-backed `load_principal_for_user`
/// test in this file, not `src/repository.rs`): `assertion failed:
/// principal.has("catalog:write")` (Rina/Analyst holds `query:read,
/// catalog:read, lineage:read` — never `catalog:write`).
#[sqlx::test(migrations = "../../migrations")]
async fn load_principal_for_user_includes_an_active_access_grant(pool: PgPool) -> sqlx::Result<()> {
    // Rina Wijaya (seeded Analyst): query:read, catalog:read, lineage:read
    // — never catalog:write on her own.
    let rina_id = Uuid::parse_str("33333333-3333-4333-8333-000000000001").unwrap();
    insert_active_grant(
        &pool,
        rina_id,
        "catalog:write",
        time::OffsetDateTime::now_utc() + time::Duration::days(7),
    )
    .await;

    let principal = load_principal_for_user(&pool, rina_id, "local".to_owned(), false)
        .await
        .unwrap();
    assert!(principal.has("catalog:write"));
    // The grant never fabricates a role — Rina's role_names stays exactly
    // her real memberships.
    assert!(!principal.role_names.iter().any(|r| r == "catalog:write"));
    Ok(())
}

/// The before/after half of the same acceptance criterion, from the other
/// side: an EXPIRED grant must never widen access — `expires_at > now()`
/// is checked at read time, not merely by the (separate, cleanup-only)
/// revocation sweep.
#[sqlx::test(migrations = "../../migrations")]
async fn load_principal_for_user_ignores_an_expired_grant(pool: PgPool) -> sqlx::Result<()> {
    let rina_id = Uuid::parse_str("33333333-3333-4333-8333-000000000001").unwrap();
    insert_active_grant(
        &pool,
        rina_id,
        "catalog:write",
        time::OffsetDateTime::now_utc() - time::Duration::days(1),
    )
    .await;

    let principal = load_principal_for_user(&pool, rina_id, "local".to_owned(), false)
        .await
        .unwrap();
    assert!(!principal.has("catalog:write"));
    Ok(())
}
