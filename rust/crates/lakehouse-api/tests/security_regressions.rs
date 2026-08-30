//! Named regression tests for specific security properties — Phase 3a,
//! task 3: "explicit, named tests for the four issues that were fixed...
//! these must fail loudly if anyone regresses them."
//!
//! `tests/route_auth.rs` already proves the general authz CONTRACT
//! (401/403/pass) across all ~100 `POLICY_TABLE` entries; this file proves
//! specific, named properties the task brief calls out individually, each
//! as its own test so a regression names exactly what broke instead of
//! showing up as one row in a big table-driven failure list.
//!
//! # The cross-tenant / `IDOR` question — read before assuming a green
//! test here means tenant isolation exists
//!
//! It does not, and this file does not claim otherwise. See
//! [`tenant_membership_is_not_used_to_scope_any_domain_query`] below: it is
//! a real finding, not a vacuous pass, and it is asserted directly rather
//! than hidden behind an "IDOR is prevented" test that would have nothing
//! to fail against. See that test's doc comment for the full explanation.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_auth::Secret;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

async fn get(app: &axum::Router, path: &str) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn get_with_header(
    app: &axum::Router,
    path: &str,
    header_name: &str,
    header_value: &str,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header(header_name, header_value)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// The password `main::bootstrap_admin`-shaped test fixtures below log in
/// with, before ever rotating it.
const BOOTSTRAP_PASSWORD: &str = "Boots7rapPassw0rd!";
/// What they rotate `BOOTSTRAP_PASSWORD` to via `POST
/// /api/auth/change-password`.
const ROTATED_PASSWORD: &str = "R0tatedPassw0rd!";

/// Create a `provider = 'local'`, `must_change_password = true` identity
/// for the seeded Platform Admin (`fajar@meridian.example`), exactly like
/// `main::bootstrap_admin`'s — shared setup for every test in this file
/// exercising the `must_change_password` gate on a Platform Admin account
/// (`*:*`), so the ONLY variable under test is the flag itself, never a
/// permission difference.
async fn create_bootstrap_admin(pool: &sqlx::PgPool) {
    let (user_id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM app_user WHERE email = 'fajar@meridian.example'")
            .fetch_one(pool)
            .await
            .expect("seeded fajar@meridian.example");
    lakehouse_auth::password::create_local_identity(
        pool,
        user_id,
        &Secret::new(BOOTSTRAP_PASSWORD),
        true,
    )
    .await
    .expect("create a bootstrap-shaped local identity");
}

/// `POST /api/auth/login` as `fajar@meridian.example` with `password`.
async fn login(router: &axum::Router, password: &str) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .body(Body::from(format!(
                    r#"{{"email": "fajar@meridian.example", "password": "{password}"}}"#
                )))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// Pull just the `name=value` pair out of a login response's `Set-Cookie`
/// header, without consuming the response body — callers that also need
/// the body (e.g. to check `mustChangePassword`) can still read it
/// afterward.
fn session_cookie_from(resp: &axum::http::Response<Body>) -> String {
    resp.headers()
        .get(axum::http::header::SET_COOKIE)
        .expect("Set-Cookie header on a successful login")
        .to_str()
        .expect("ASCII header value")
        .split(';')
        .next()
        .expect("at least the name=value pair")
        .to_owned()
}

/// # 1. Unauthenticated access to a protected route is refused
#[tokio::test]
async fn unauthenticated_request_to_a_protected_route_is_refused() {
    let TestApp { router, .. } = spin_up().await;
    // No cookie, no Authorization header at all.
    let resp = get(&router, "/api/catalog").await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// # 3a. A missing credential is refused
///
/// Same request as above, named separately (per the task brief's "missing
/// token, malformed token, and expired session are each refused
/// (separately)") on a `Policy::RequiresAuth` route rather than a
/// `Policy::RequiresPermission` one, so this and the two tests below cover
/// three DIFFERENT failure shapes hitting the SAME kind of route.
#[tokio::test]
async fn missing_credential_is_refused() {
    let TestApp { router, .. } = spin_up().await;
    let resp = get(&router, "/api/auth/me").await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// # 3b. A malformed/garbage bearer token is refused
///
/// `crate::auth::bearer_credential_kind` classifies this specific value as
/// "opaque" (not `.`-shaped like a JWT — see that module's doc comment),
/// so it is tried against `ServiceTokenAuthenticator`, which rejects it as
/// an unknown token hash. Neither branch of the bearer dispatch ever
/// treats an unrecognized token as anonymous-but-allowed.
#[tokio::test]
async fn malformed_bearer_token_is_refused() {
    let TestApp { router, .. } = spin_up().await;
    let resp = get_with_header(
        &router,
        "/api/auth/me",
        "authorization",
        "Bearer not-a-real-token-at-all",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// # 3c. An expired session is refused
///
/// Mints a session with a NEGATIVE ttl — `session::create_session` computes
/// `expires_at = now() + ttl`, so this is a session that was already
/// expired the instant it was created, deterministically (no
/// `sleep`/wall-clock dependence — see the task's determinism
/// requirement) rather than a real session raced against the clock.
#[tokio::test]
async fn expired_session_is_refused() {
    let TestApp { router, pool } = spin_up().await;
    let (user_id,): (Uuid,) =
        sqlx::query_as("SELECT id FROM app_user WHERE email = 'sari@meridian.example'")
            .fetch_one(&pool)
            .await
            .expect("seeded sari@meridian.example");

    let token: Secret = lakehouse_auth::session::create_session(
        &pool,
        user_id,
        time::Duration::seconds(-60),
        None,
        None,
    )
    .await
    .expect("mint an already-expired session");

    let cookie = format!("lh_session={}", token.expose());
    let resp = get_with_header(&router, "/api/auth/me", "cookie", &cookie).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// # 4. `/api/identity/*` privilege escalation is refused
///
/// The exact scenario `policy.rs`'s own module doc comment names as the
/// vulnerability `identity:read`/`identity:write` were introduced to
/// close: a low-permission principal (a seeded Analyst — `query:read,
/// catalog:read, lineage:read`, none of which is `identity:*`) attempting
/// to mint a NEW role, including one that would grant itself `*:*`. This
/// must be refused by the auth gate BEFORE the handler (and therefore
/// before the request body — the crafted `permissions: "*:*"` payload — is
/// ever read), which is exactly what a 403 (not a 400/422 from body
/// validation) proves.
#[tokio::test]
async fn analyst_cannot_mint_a_role_granting_itself_platform_admin_permissions() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/identity/roles")
                .header("cookie", cookie)
                .body(Body::from(
                    r#"{"name": "Self-Granted Admin", "permissions": "*:*", "description": "escalation attempt"}"#,
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "an Analyst must never be able to mint a *:* role for itself"
    );

    // And the row must genuinely not exist — the 403 isn't just an HTTP
    // decoration on top of a handler that ran anyway.
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM role WHERE name = 'Self-Granted Admin'")
            .fetch_one(&pool)
            .await
            .expect("query role table");
    assert_eq!(
        count, 0,
        "the escalation attempt must not have created a row"
    );
}

/// # 5. The bootstrap-admin path: `must_change_password` — NOW ENFORCED
///
/// This test previously documented a real gap (`Principal` carried no
/// `must_change_password` field, so nothing server-side gated on it — a
/// bootstrapped session could call any permission-satisfying route,
/// including `POST /api/identity/roles`, without ever rotating its
/// credential). That gap is now fixed:
///
/// * `lakehouse_auth::Principal::must_change_password` is populated by
///   every authenticator that can know it (`local` password verification
///   and session validation join `auth_identity.must_change_password` in
///   their existing query, at no extra round trip; a service token or
///   OIDC login always sets it `false` — see that field's doc comment).
/// * `crate::policy::auth_gate` refuses (403) any route for a principal
///   with the flag set, EXCEPT the three in
///   `crate::policy::ALLOWED_WHILE_MUST_CHANGE_PASSWORD`:
///   `POST /api/auth/change-password`, `POST /api/auth/logout`, and
///   `GET /api/auth/me`.
///
/// `GET /api/auth/me` is deliberately allowed, not refused — this is a
/// design decision, not an oversight. The frontend `AuthProvider` calls it
/// on every page load to learn who is signed in; refusing it would break
/// a plain page refresh mid-rotation, and it returns nothing more
/// sensitive than the caller's own identity plus this same flag. So this
/// test now asserts the OTHER half of the property against a route the
/// design actually protects — `GET /api/identity/roles`, an ordinary
/// Postgres-only, permission-gated route with no special-case reason to
/// stay reachable —
/// rather than against `/me`, which the design intentionally exempts.
/// [`bootstrap_admin_can_still_reach_the_three_allowed_routes`] and
/// [`must_change_password_session_works_normally_after_rotation`] below
/// cover the two halves this test doesn't: that the three exempted routes
/// stay reachable, and that a rotated session goes back to normal.
#[tokio::test]
async fn bootstrap_admin_with_must_change_password_is_blocked_from_other_routes() {
    let TestApp { router, pool } = spin_up().await;
    create_bootstrap_admin(&pool).await;

    let login_resp = login(&router, BOOTSTRAP_PASSWORD).await;
    assert_eq!(login_resp.status(), StatusCode::OK);
    let cookie = session_cookie_from(&login_resp);

    // The flag IS surfaced in the login response...
    let bytes = axum::body::to_bytes(login_resp.into_body(), usize::MAX)
        .await
        .expect("read login response body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["mustChangePassword"], serde_json::json!(true));

    // ...and now a genuinely protected route (NOT one of the three
    // exempted routes) refuses this session until the password is
    // rotated.
    let resp = get_with_header(&router, "/api/identity/roles", "cookie", &cookie).await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a must_change_password session must be refused on routes other than \
         change-password/logout/me until the password is rotated"
    );
}

/// # 5b. The three exempted routes stay reachable pre-rotation
///
/// The positive half of the property above: `change-password`, `logout`,
/// and `me` must all still work for a `must_change_password` session, or
/// that session has no way to ever get out of the state (and `AuthProvider`
/// would break on refresh). Same bootstrap-shaped identity as the test
/// above.
#[tokio::test]
async fn bootstrap_admin_can_still_reach_the_three_allowed_routes() {
    let TestApp { router, pool } = spin_up().await;
    create_bootstrap_admin(&pool).await;

    let login_resp = login(&router, BOOTSTRAP_PASSWORD).await;
    let cookie = session_cookie_from(&login_resp);

    // GET /api/auth/me stays reachable.
    let me_resp = get_with_header(&router, "/api/auth/me", "cookie", &cookie).await;
    assert_eq!(
        me_resp.status(),
        StatusCode::OK,
        "/api/auth/me must stay reachable pre-rotation"
    );

    // POST /api/auth/change-password stays reachable (and actually
    // rotates the credential — proven by the next test, which reuses this
    // exact flow).
    let change_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/change-password")
                .header("cookie", cookie.clone())
                .body(Body::from(format!(
                    r#"{{"oldPassword": "{BOOTSTRAP_PASSWORD}", "newPassword": "{ROTATED_PASSWORD}"}}"#
                )))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(
        change_resp.status(),
        StatusCode::NO_CONTENT,
        "/api/auth/change-password must stay reachable pre-rotation"
    );

    // Re-login (change-password revokes sessions) to get a fresh cookie
    // for the logout check, so this assertion isn't riding on the
    // just-rotated session possibly already being revoked.
    let relogin_resp = login(&router, ROTATED_PASSWORD).await;
    assert_eq!(relogin_resp.status(), StatusCode::OK);
    let fresh_cookie = session_cookie_from(&relogin_resp);

    let logout_resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", fresh_cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(
        logout_resp.status(),
        StatusCode::NO_CONTENT,
        "/api/auth/logout must stay reachable regardless of must_change_password"
    );
}

/// # 5c. Once the password is rotated, the session behaves normally
///
/// Proves the gate is actually keyed on the live `must_change_password`
/// state, not just "this session was ever bootstrap-shaped": after a
/// successful `change-password`, a fresh login must be able to reach an
/// ordinary permission-gated route the earlier test proved was refused
/// pre-rotation.
#[tokio::test]
async fn must_change_password_session_works_normally_after_rotation() {
    let TestApp { router, pool } = spin_up().await;
    create_bootstrap_admin(&pool).await;

    let login_resp = login(&router, BOOTSTRAP_PASSWORD).await;
    let cookie = session_cookie_from(&login_resp);

    let change_resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/change-password")
                .header("cookie", cookie)
                .body(Body::from(format!(
                    r#"{{"oldPassword": "{BOOTSTRAP_PASSWORD}", "newPassword": "{ROTATED_PASSWORD}"}}"#
                )))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(change_resp.status(), StatusCode::NO_CONTENT);

    // A fresh login after rotation must have must_change_password: false,
    // and must be able to reach /api/identity/roles — the exact route the
    // pre-rotation test proved was refused.
    let relogin_resp = login(&router, ROTATED_PASSWORD).await;
    assert_eq!(relogin_resp.status(), StatusCode::OK);
    let fresh_cookie = session_cookie_from(&relogin_resp);
    let bytes = axum::body::to_bytes(relogin_resp.into_body(), usize::MAX)
        .await
        .expect("read login response body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
    assert_eq!(body["mustChangePassword"], serde_json::json!(false));

    let resp = get_with_header(&router, "/api/identity/roles", "cookie", &fresh_cookie).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a rotated session must be able to reach an ordinary permission-gated route"
    );
}

/// # The cross-tenant / `IDOR` finding
///
/// Checked whether ANY Phase 2 domain table (agents, pipelines,
/// connectors, knowledge sources, governance policies, saved queries,
/// dashboards — everything under `crate::routes` besides `identity`
/// itself) carries a `tenant_id` column at all: none of them do (grep
/// `migrations/*.sql` for `tenant_id` — it exists only in `0001_init.sql`,
/// on `tenant` and `app_user_tenant` themselves, and nowhere else).
/// [`lakehouse_auth::Principal::tenant_ids`] is populated from
/// `app_user_tenant` (`lakehouse_auth::repository`) and surfaced verbatim
/// by `GET /api/auth/me`, but grepping every `SELECT`/`UPDATE`/`DELETE` in
/// `lakehouse-store` for a `tenant_id` predicate driven by the CALLER
/// (as opposed to the seed data's own foreign keys) turns up nothing:
/// `crate::policy` gates access by RESOURCE PERMISSION
/// (`pipeline:read`, `dashboard:write`, ...), never by WHICH tenant's rows
/// a query returns.
///
/// This test demonstrates the concrete consequence rather than asserting
/// an "IDOR is prevented" property that would have nothing to fail
/// against. `identity:read`/`identity:write` are held ONLY via Platform
/// Admin's `*:*` wildcard (no other seeded role grants either token — see
/// `policy.rs`'s own module doc comment), so `fajar@meridian.example`
/// (Platform Admin) is the only seeded principal that can reach
/// `GET /api/identity/tenants` at all; per `0002_seed_identity.sql` it is
/// a member of `meridian-group`/`meridian-retail`/`meridian-logistics` but
/// explicitly NOT `meridian-labs` (`Meridian Labs (sandbox)`) — yet the
/// response includes it anyway. There is no narrower endpoint to
/// demonstrate this against for the other domains (agents/pipelines/etc.
/// have no tenant concept in the schema at all, so there is nothing there
/// for a caller's tenant membership to even filter by) — `tenant` itself
/// is the one resource in this schema that unambiguously SHOULD be scoped
/// by membership and demonstrably isn't.
///
/// # Verdict: repositories do NOT enforce tenant scoping
///
/// See the task's final report for the full writeup. This is a real
/// finding, reported here rather than hidden behind a vacuous test.
#[tokio::test]
async fn tenant_membership_is_not_used_to_scope_any_domain_query() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let resp = router
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/identity/tenants")
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON");
    let names: Vec<&str> = body
        .as_array()
        .expect("a JSON array of tenants")
        .iter()
        .filter_map(|t| t.get("name").and_then(serde_json::Value::as_str))
        .collect();

    // `fajar@meridian.example` is NOT a member of `Meridian Labs
    // (sandbox)` (0002_seed_identity.sql), yet it is returned anyway:
    // today's `identity:read` gate is global, not scoped to the caller's
    // own `tenant_ids`.
    assert!(
        names.contains(&"Meridian Labs (sandbox)"),
        "expected the (undesirable, but currently accurate) result that a caller \
         sees every tenant regardless of membership; got {names:?} — if this now \
         fails, tenant scoping has been added and this test should be rewritten \
         to assert the new, correct behavior instead of this documented gap"
    );
}
