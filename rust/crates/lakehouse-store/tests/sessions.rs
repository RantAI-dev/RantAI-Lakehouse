//! Integration tests for `lakehouse_store::sessions` against a real
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
//! Every test below provisions a database with the full migration set
//! applied, so the `app_user` and `session` tables exist for direct
//! `INSERT`s — sessions tests don't need the seeded identity fixture, just
//! an `app_user_id` to hang a session on. The plain
//! `INSERT INTO app_user (name, email) VALUES (...) RETURNING id` shape
//! mirrors `tests/identity.rs`'s style and avoids the role/tenant-membership
//! transaction `lakehouse_store::identity::create_user` runs (which complicates
//! the test setup without adding anything these tests need).
//!
//! # Why direct SQL, not `lakehouse_auth::session`
//!
//! These tests write `session` rows with raw `INSERT`/`UPDATE` rather than
//! going through `lakehouse_auth::session::create_session` /
//! `revoke_session`. The behavior under test is the *listing* — the
//! listing's own `WHERE revoked_at IS NULL AND expires_at > now()` clause
//! is the spec, and that clause has no dependency on `lakehouse-auth`'s
//! hash function or session minting path. Keeping the writes direct also
//! keeps this test file's dependency surface to `lakehouse-store` and
//! `sqlx` only, with no `lakehouse-auth` dev-dep required (which would
//! mean a seventh changed file outside the six this workstream owns — see
//! the commit body for the deviation note).

#![allow(clippy::unwrap_used, clippy::expect_used)]

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped
// by the linker before its ctor section is ever considered).
use lakehouse_test_support as _;

use lakehouse_store::sessions::{SessionRow, list_sessions_for_caller};
use sqlx::PgPool;
use uuid::Uuid;

/// Insert a bare `app_user` row with no roles, no tenants, no
/// memberships — just the `app_user_id` a session hangs off of. Mirrors
/// the direct-`INSERT` shape `tests/identity.rs` uses for service-identity
/// rows rather than going through `lakehouse_store::identity::create_user`
/// (which runs a transaction with role/tenant joins that complicates the
/// test setup, see the module doc comment).
async fn seed_user(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar::<_, Uuid>("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
        .bind(email.split('@').next().unwrap_or(email))
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("insert app_user")
}

/// Insert a live (`revoked_at = NULL`, `expires_at > now()`) `session`
/// row for `app_user_id`. `created_ip`/`user_agent` are deliberately left
/// `NULL` — the honest-`null` shape [`lakehouse_store::sessions::SessionRow`]
/// renders on the wire (see `created_ip_and_user_agent_round_trip_as_null_on_the_wire`
/// below for the wire-shape pin). `token_hash` is an arbitrary 64-character
/// hex string; the listing's own `WHERE` clause never reads it, so any
/// unique hex string suffices.
async fn insert_live_session(pool: &PgPool, app_user_id: Uuid, hash: &str) {
    assert_eq!(
        hash.len(),
        64,
        "token_hash placeholder must be 64 hex chars"
    );
    sqlx::query(
        "INSERT INTO session (app_user_id, token_hash, expires_at) \
         VALUES ($1, $2, now() + interval '1 hour')",
    )
    .bind(app_user_id)
    .bind(hash)
    .execute(pool)
    .await
    .expect("insert live session");
}

/// The default branch: a caller authenticated as `caller_a` sees their
/// own sessions and no-one else's, even though `caller_b` has a session
/// too. The `WHERE s.app_user_id = $1` branch — `$2 = false` — is what
/// makes this safe: a row owned by anyone else never enters the result set.
#[sqlx::test(migrations = "../../migrations")]
async fn sessions_list_shows_only_the_callers_own_sessions_by_default(
    pool: PgPool,
) -> sqlx::Result<()> {
    let caller_a = seed_user(&pool, "a@x.invalid").await;
    let caller_b = seed_user(&pool, "b@x.invalid").await;

    // Both sessions live; the row we care about is caller_a's. The
    // `caller_b` session exists so a future regression that stopped
    // binding `caller_id` (and just returned every session) would be
    // caught by the row-count assertion below — at least one foreign
    // session has to be present for a leak to be observable.
    insert_live_session(&pool, caller_b, &"b".repeat(64)).await;
    insert_live_session(&pool, caller_a, &"a".repeat(64)).await;

    let rows = list_sessions_for_caller(&pool, caller_a, false)
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "caller_a sees exactly their own session, not caller_b's"
    );
    assert_eq!(rows[0].user_id, caller_a.to_string());
    assert!(
        rows.iter().all(|r| r.user_id == caller_a.to_string()),
        "every returned row's user_id must equal caller_a"
    );
    assert!(
        rows.iter().all(|r| r.user_id != caller_b.to_string()),
        "caller_b's session must not be visible to caller_a"
    );
    Ok(())
}

/// The admin branch: a principal holding `identity:sessions:manage`
/// (the only caller the route treats as `is_admin = true`) sees every
/// live session. Same fixture as the default-branch test — `caller_a`'s
/// permission set is "what would let `is_admin` flip true" — so the
/// single change is the third argument to [`list_sessions_for_caller`].
#[sqlx::test(migrations = "../../migrations")]
async fn sessions_list_shows_every_session_only_with_the_admin_permission(
    pool: PgPool,
) -> sqlx::Result<()> {
    let caller_a = seed_user(&pool, "a@x.invalid").await;
    let caller_b = seed_user(&pool, "b@x.invalid").await;

    insert_live_session(&pool, caller_a, &"1".repeat(64)).await;
    insert_live_session(&pool, caller_b, &"2".repeat(64)).await;

    let rows = list_sessions_for_caller(&pool, caller_a, true)
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "an admin caller sees every live session, across owners"
    );
    let user_ids: std::collections::HashSet<String> =
        rows.iter().map(|r| r.user_id.clone()).collect();
    assert!(user_ids.contains(&caller_a.to_string()));
    assert!(user_ids.contains(&caller_b.to_string()));
    Ok(())
}

/// A revoked session must never appear in the listing. The listing's
/// `WHERE s.revoked_at IS NULL AND expires_at > now()` matches the live
/// predicate [`lakehouse_auth::session::validate_session`] itself uses
/// on the same table — so a row that would refuse to authenticate never
/// enters the list either.
///
/// One row is seeded for one caller; the row is revoked via the same SQL
/// UPDATE [`lakehouse_auth::session::revoke_session`] issues; the listing
/// goes from 1 → 0.
#[sqlx::test(migrations = "../../migrations")]
async fn a_revoked_session_is_never_returned(pool: PgPool) -> sqlx::Result<()> {
    let caller = seed_user(&pool, "a@x.invalid").await;
    let hash = "r".repeat(64);
    insert_live_session(&pool, caller, &hash).await;

    // Baseline: the row is live.
    let rows = list_sessions_for_caller(&pool, caller, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "baseline before revocation");

    // Same UPDATE `lakehouse_auth::session::revoke_session` runs: the
    // listing view of "live" must agree with what that function would
    // say if the same cookie were re-presented.
    sqlx::query(
        "UPDATE session SET revoked_at = now() \
         WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&hash)
    .execute(&pool)
    .await?;

    let rows = list_sessions_for_caller(&pool, caller, false)
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        0,
        "a revoked session must never be returned to its owner"
    );
    Ok(())
}

/// The honest-`null` shape for `created_ip`/`user_agent`: every session
/// minted today passes `None` for both columns, and the JSON the handler
/// returns carries an explicit `null` (not absent, not a guessed default).
///
/// This test runs against the JSON wire shape — the bytes that go to the
/// browser — rather than the in-memory struct, so a future
/// `#[serde(skip_serializing_if = "Option::is_none")]`-style slip would
/// fail it.
#[sqlx::test(migrations = "../../migrations")]
async fn created_ip_and_user_agent_round_trip_as_null_on_the_wire(
    pool: PgPool,
) -> sqlx::Result<()> {
    let caller = seed_user(&pool, "a@x.invalid").await;
    insert_live_session(&pool, caller, &"i".repeat(64)).await;

    let rows: Vec<SessionRow> = list_sessions_for_caller(&pool, caller, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1);

    let json = serde_json::to_value(&rows[0]).unwrap();
    assert_eq!(
        json.get("createdIp"),
        Some(&serde_json::Value::Null),
        "createdIp must be explicit null, not omitted"
    );
    assert_eq!(
        json.get("userAgent"),
        Some(&serde_json::Value::Null),
        "userAgent must be explicit null, not omitted"
    );
    Ok(())
}

/// The list is ordered newest-first. Two rows are inserted a few
/// milliseconds apart; the row from the second `INSERT` comes back
/// first.
///
/// `ORDER BY s.created_at DESC, s.id` (rather than the bare
/// `created_at DESC` the plan sketch shows) gives a deterministic order
/// even when the timestamps collide — two sessions in the same
/// microsecond-second window sort by `id` rather than by Postgres's
/// unspecified tie-break. A regression that lost the `DESC` would put
/// the older session first; one that lost the `, s.id` would leave the
/// order flaky under high-rate login.
///
/// The two `INSERT`s set `created_at` explicitly (rather than relying on
/// the column default of `now()`) so the older-vs-newer ordering is
/// deterministic from the test's perspective, independent of the clock
/// resolution Postgres happens to land on.
#[sqlx::test(migrations = "../../migrations")]
async fn sessions_list_orders_newest_first(pool: PgPool) -> sqlx::Result<()> {
    let caller = seed_user(&pool, "a@x.invalid").await;

    sqlx::query(
        "INSERT INTO session (app_user_id, token_hash, created_at, expires_at) \
         VALUES ($1, $2, now() - interval '2 seconds', now() + interval '1 hour')",
    )
    .bind(caller)
    .bind("1".repeat(64))
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO session (app_user_id, token_hash, created_at, expires_at) \
         VALUES ($1, $2, now(), now() + interval '1 hour')",
    )
    .bind(caller)
    .bind("2".repeat(64))
    .execute(&pool)
    .await?;

    let rows = list_sessions_for_caller(&pool, caller, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows[0].created_at >= rows[1].created_at,
        "the newer row (now()) must sort before the older row (now()-2s): {} vs {}",
        rows[0].created_at,
        rows[1].created_at,
    );
    Ok(())
}

/// `lakehouse_auth::session::validate_session` uses the same live
/// predicate — `revoked_at IS NULL AND expires_at > now()` — so the
/// listing's view of "live" agrees with what authentication itself
/// accepts. This test seeds one already-expired session and one live
/// session for the same caller and checks that the listing returns only
/// the live one.
///
/// The expired row is inserted directly with `expires_at` already in
/// the past (the testcontainer's `now() - interval '1 hour'`). The
/// `token_hash` value here is an arbitrary 64-character hex string
/// (matching the column's CHECK-free `TEXT NOT NULL` shape) — the listing's
/// own `WHERE` clause never reads `token_hash`, so the value only has to be
/// present and unique.
#[sqlx::test(migrations = "../../migrations")]
async fn an_already_expired_session_is_not_returned(pool: PgPool) -> sqlx::Result<()> {
    let caller = seed_user(&pool, "a@x.invalid").await;

    // Live session, normal TTL.
    insert_live_session(&pool, caller, &"k".repeat(64)).await;

    // Direct INSERT of a row whose `expires_at` is already in the past.
    sqlx::query(
        "INSERT INTO session (app_user_id, token_hash, expires_at) \
         VALUES ($1, $2, now() - interval '1 hour')",
    )
    .bind(caller)
    .bind("e".repeat(64))
    .execute(&pool)
    .await?;

    let rows = list_sessions_for_caller(&pool, caller, false)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "the expired row must not be returned");
    Ok(())
}
