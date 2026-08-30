//! Shared harness for the HTTP-level (`tower::ServiceExt::oneshot`)
//! integration tests in this directory.
//!
//! Every test gets its own freshly created, freshly migrated Postgres
//! database (on the single `testcontainers` Postgres server
//! `lakehouse-test-support` starts once per test binary — see that
//! crate), and a real [`lakehouse_api::routes::router`] built from a real
//! [`lakehouse_api::state::AppState`]. `ClickHouse`/`Dagster`/the LLM are
//! never real: [`spin_up`] points every one of those URLs at
//! `http://127.0.0.1:1` (port 0/1 refuses a TCP connection immediately on
//! Linux, no DNS lookup, no timeout to wait out), so any handler whose
//! authorization passes and which then tries to reach one of those
//! upstreams fails fast and deterministically with a 502/503/500 rather
//! than hanging or reaching a real host — see the module docs on
//! `tests/route_auth.rs` for why that is exactly the behavior the authz
//! matrix needs (it only ever asserts "not 401/403", never a 2xx).

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use std::collections::HashMap;

use lakehouse_api::config::Config;
use lakehouse_api::state::AppState;
use lakehouse_auth::Secret;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

// Force-links `lakehouse-test-support` so its `#[ctor]` Postgres
// testcontainer bootstrap actually runs for this test binary (an
// unreferenced dev-dependency's rlib member can otherwise be dropped by
// the linker before its ctor section is ever considered — see that
// crate's doc comment).
use lakehouse_test_support as _;

/// A running [`AppState`]/[`axum::Router`] pair over an isolated,
/// migrated-and-seeded test database, plus the raw pool for direct setup
/// (minting sessions, inserting throwaway fixtures) that goes around the
/// HTTP surface entirely.
pub struct TestApp {
    pub router: axum::Router,
    pub pool: PgPool,
}

/// An unreachable-but-instantly-refusing HTTP origin — see the module doc
/// comment for why this, rather than a real mock server, is the right
/// default for every upstream this crate does not itself own.
const DEAD_UPSTREAM: &str = "http://127.0.0.1:1";

/// Creates a brand-new, randomly named database on the shared test
/// Postgres container, migrates it (`0002_seed_identity.sql` and friends
/// run as part of that), and returns a [`TestApp`] wired to it.
///
/// Each call is fully isolated from every other: two tests calling this
/// concurrently never see each other's rows, matching the isolation
/// `#[sqlx::test]` gives `lakehouse-store`/`lakehouse-auth`'s tests (see
/// `lakehouse-test-support`), just done by hand here because
/// `AppState::new` — not a `PgPool` extractor — is what this crate's tests
/// need.
pub async fn spin_up() -> TestApp {
    let base_url = lakehouse_test_support::database_url();
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&base_url)
        .await
        .expect("connect to the test-support admin database");

    let db_name = format!("lakehouse_api_test_{}", Uuid::new_v4().simple());
    sqlx::query(&format!(r#"CREATE DATABASE "{db_name}""#))
        .execute(&admin_pool)
        .await
        .expect("create a fresh per-test database");
    admin_pool.close().await;

    let db_url = swap_database_name(&base_url, &db_name);

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect to the fresh per-test database");
    lakehouse_store::migrate(&pool)
        .await
        .expect("apply migrations to the fresh per-test database");

    let mut env = HashMap::new();
    env.insert("DATABASE_URL".to_owned(), db_url);
    env.insert("CH_URL".to_owned(), DEAD_UPSTREAM.to_owned());
    env.insert("DAGSTER_URL".to_owned(), DEAD_UPSTREAM.to_owned());
    env.insert("LLM_URL".to_owned(), DEAD_UPSTREAM.to_owned());
    // Deliberately NOT `APP_ENV=development`: keeps the session cookie's
    // `Secure` attribute on, matching the fail-closed production default —
    // these tests never rely on the cookie flowing over plaintext HTTP,
    // since `tower::ServiceExt::oneshot` never touches a real socket.
    let config = Config::from_map(&env).expect("a valid test Config");
    let state = AppState::new(config);
    let router = lakehouse_api::routes::router(state);

    TestApp { router, pool }
}

/// Swaps the database name in `postgres://user:pass@host:port/dbname` for
/// `new_db`, without pulling in a URL-parsing crate for one substitution.
fn swap_database_name(url: &str, new_db: &str) -> String {
    let (base, _old_db) = url.rsplit_once('/').expect("a postgres:// URL with a path");
    format!("{base}/{new_db}")
}

/// Mints a live session for the seeded `email` (from
/// `0002_seed_identity.sql` — see that migration for the full roster) and
/// returns the `Cookie` header value to send it as.
pub async fn session_cookie_for_seeded_user(pool: &PgPool, email: &str) -> String {
    let (user_id,): (Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .unwrap_or_else(|e| panic!("seeded user {email} must exist (0002_seed_identity.sql): {e}"));
    session_cookie_for_user(pool, user_id).await
}

/// Mints a live session for an arbitrary `app_user.id` and returns the
/// `Cookie` header value to send it as.
pub async fn session_cookie_for_user(pool: &PgPool, user_id: Uuid) -> String {
    let token: Secret = lakehouse_auth::session::create_session(
        pool,
        user_id,
        lakehouse_auth::session::DEFAULT_SESSION_TTL,
        None,
        None,
    )
    .await
    .expect("mint a session for a test principal");
    format!("lh_session={}", token.expose())
}

/// Creates a fresh `app_user` holding a role that grants NO permissions at
/// all (`role.permissions = ''`), and returns its `app_user.id`.
///
/// This is the "authenticated, but must be denied by every
/// `Policy::RequiresPermission` entry" principal the authz matrix needs —
/// no seeded role is a clean stand-in for "holds zero permissions" (even
/// `Dashboard Viewer` holds `dashboard:read`), so the test fixture creates
/// one explicitly rather than repurposing a seeded role that happens, for
/// now, not to satisfy whichever permission a given test checks.
///
/// Returns the user id, not a session cookie: callers that send more than
/// one request (e.g. looping every `POLICY_TABLE` entry, which includes
/// `POST /api/auth/logout` — a request that revokes whatever session
/// cookie it is sent with) must mint a FRESH session per request via
/// [`session_cookie_for_user`] rather than reuse one cookie across many
/// requests.
pub async fn create_zero_permission_principal(pool: &PgPool) -> Uuid {
    let role_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO role (id, name, permissions, description) VALUES ($1, $2, '', 'test: grants nothing')",
    )
    .bind(role_id)
    .bind(format!("Zero Perm {}", Uuid::new_v4()))
    .execute(pool)
    .await
    .expect("insert a zero-permission role");

    let user_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, email, status) VALUES ($1, 'Zero Perm Test User', $2, 'active')",
    )
    .bind(user_id)
    .bind(format!("zero-perm-{user_id}@test.invalid"))
    .execute(pool)
    .await
    .expect("insert a zero-permission test user");

    sqlx::query("INSERT INTO app_user_role (user_id, role_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(role_id)
        .execute(pool)
        .await
        .expect("attach the zero-permission role to the test user");

    user_id
}
