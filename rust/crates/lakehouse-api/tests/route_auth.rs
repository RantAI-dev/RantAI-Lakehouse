//! HTTP-level authorization tests, driven entirely by
//! `lakehouse_api::policy::POLICY_TABLE` — Phase 3a.
//!
//! # Why driven from the table, not hand-written per route
//!
//! `lakehouse_api::routes::route_policy_tests` (in-crate, `src/routes/mod.rs`)
//! already proves, against the real router, that every table entry is
//! mounted and that presenting NO credentials at all yields exactly the
//! policy-appropriate refusal (`Policy::Public` never 401/403,
//! `Policy::RequiresAuth`/`Policy::RequiresPermission` always 401). What is
//! missing — and what this file adds — is the other two-thirds of the
//! authz contract the task brief calls out as "the most valuable set":
//! a real, authenticated-but-under-permissioned principal must be refused
//! (403) on every `Policy::RequiresPermission` route, and a real,
//! correctly-permissioned principal must never be refused (401/403) on
//! ANY route.
//!
//! Looping `POLICY_TABLE` itself (rather than writing ~120 individual test
//! functions) is deliberate: a route added to `routes::router` with a
//! matching `POLICY_TABLE` entry is automatically covered by both loops
//! below the moment it exists — nothing here needs updating. A route added
//! WITHOUT a table entry is caught by the deny-by-default regression in
//! `src/routes/mod.rs` instead (a 500, not a 401/403, so it would not
//! silently blend into either loop's counts here).
//!
//! # What "not 401/403" does and doesn't prove
//!
//! For a correctly-permissioned request, this file only ever asserts the
//! response is NOT 401/403 — never a specific 2xx body. Almost every
//! handler beyond the auth gate itself calls `ClickHouse`, `Dagster`, or
//! the LLM, which `tests/common::spin_up` deliberately points at
//! `127.0.0.1:1` (an instantly-refused connection, never a real or even a
//! reachable host — see that module's doc comment). Such a handler
//! legitimately answers 500/502/503, and that is fine: what this file
//! exists to catch is authorization being wrong, not an unrelated
//! downstream outage. `tests/parity.rs` (out of scope for this phase) is
//! what proves response bodies against real backends.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_api::policy::{POLICY_TABLE, Policy};
use tower::ServiceExt;

use common::{
    TestApp, create_zero_permission_principal, session_cookie_for_seeded_user,
    session_cookie_for_user, spin_up,
};
use sqlx::PgPool;
use uuid::Uuid;

/// Look up the seeded `fajar@meridian.example` (Platform Admin) user id
/// once, so each loop iteration below can mint a FRESH session for it.
async fn platform_admin_user_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM app_user WHERE email = 'fajar@meridian.example'")
        .fetch_one(pool)
        .await
        .expect("seeded fajar@meridian.example (0002_seed_identity.sql)")
}

/// `{id}`/`{token}`/`{kind}`/`{runId}` — any `{...}` capture segment —
/// substituted with a fixed placeholder so the concrete request resolves
/// to the exact route pattern the table names. Mirrors
/// `routes::route_policy_tests::concretize` exactly (kept as a separate
/// copy: that one is `crate`-private to the binary's own test module and
/// cannot be reused from here).
fn concretize(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len());
    let mut in_capture = false;
    for ch in pattern.chars() {
        match ch {
            '{' => in_capture = true,
            '}' => {
                in_capture = false;
                out.push('x');
            }
            _ if in_capture => {}
            _ => out.push(ch),
        }
    }
    out
}

async fn request_with_cookie(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// # Authz: authenticated-but-missing-permission -> 403
///
/// Every `Policy::RequiresPermission` entry, hit by a principal
/// authenticated as a real session but holding a role that grants no
/// permissions at all, must come back 403 — never 401 (the principal IS
/// valid) and never anything in the 2xx/4xx-other range (the auth gate
/// runs before the handler, so a permission failure can never be
/// shadowed by, say, a downstream 503).
///
/// Every `Policy::RequiresAuth` entry, hit by that same principal, must
/// NOT be 401: `RequiresAuth` checks only that SOME principal is present,
/// never a specific permission, so a permission-less-but-authenticated
/// caller must be let through the gate (whatever the handler itself then
/// does with an unreachable ClickHouse/Dagster is out of scope here — see
/// the module doc comment).
#[tokio::test]
async fn zero_permission_principal_is_denied_every_gated_route_and_let_past_auth_only_routes() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_zero_permission_principal(&pool).await;

    let mut failures = Vec::new();
    for (method, pattern, policy) in POLICY_TABLE {
        // A FRESH session per request: `POST /api/auth/logout` is itself a
        // `POLICY_TABLE` entry, and revokes whatever cookie it is sent
        // with — reusing one cookie across every entry would make that one
        // request poison every entry walked after it. See
        // `common::create_zero_permission_principal`'s doc comment.
        let cookie = session_cookie_for_user(&pool, user_id).await;
        let path = concretize(pattern);
        let resp = request_with_cookie(&router, method, &path, &cookie).await;
        let status = resp.status();
        match policy {
            Policy::RequiresPermission(perm) => {
                if status != StatusCode::FORBIDDEN {
                    failures.push(format!(
                        "{method} {pattern}: a zero-permission principal must get 403 \
                         (missing {perm}), got {status}"
                    ));
                }
            }
            Policy::RequiresAuth => {
                if status == StatusCode::UNAUTHORIZED {
                    failures.push(format!(
                        "{method} {pattern}: RequiresAuth must accept ANY authenticated \
                         principal (no permission check), got 401"
                    ));
                }
            }
            Policy::Public => {}
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// # Authz: correctly-permissioned -> not 401/403
///
/// `fajar@meridian.example` (seeded by `0002_seed_identity.sql` with the
/// `Platform Admin` role, whose `permissions = "*:*"`) satisfies every
/// `Policy::RequiresPermission` string in the table by the resource/action
/// wildcard rule (`lakehouse_auth::permissions`) and is obviously a valid
/// `Policy::RequiresAuth` principal — so a session for it must never be
/// refused by the auth gate on ANY of the 122 entries.
#[tokio::test]
async fn platform_admin_principal_is_never_refused_by_the_auth_gate() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = platform_admin_user_id(&pool).await;

    let mut failures = Vec::new();
    for (method, pattern, policy) in POLICY_TABLE {
        // Fresh session per request — see the identical comment in
        // `zero_permission_principal_is_denied_every_gated_route_and_let_past_auth_only_routes`.
        let cookie = session_cookie_for_user(&pool, user_id).await;
        let path = concretize(pattern);
        let resp = request_with_cookie(&router, method, &path, &cookie).await;
        let status = resp.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            failures.push(format!(
                "{method} {pattern} ({policy:?}): a Platform Admin (*:*) must never be \
                 refused by the auth gate, got {status}"
            ));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

/// A seeded Analyst (`sari@meridian.example` — `query:read, catalog:read,
/// lineage:read`, no other grants) is a second, independently-seeded
/// "real, under-permissioned" principal, spot-checked against one route
/// from each of the four privilege-escalation-hardened permission
/// families the module doc comment on `policy.rs` calls out by name
/// (`identity:write`, `agent:manage`, `storage:restore`, `alert:write`):
/// every one of them must refuse an Analyst exactly the same way the
/// zero-permission principal is refused above, proving the 403 is about
/// the SPECIFIC permission, not merely "this principal happens to hold
/// nothing".
#[tokio::test]
async fn a_seeded_analyst_is_denied_the_four_hardened_permission_families() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    for (method, path) in [
        ("POST", "/api/identity/roles"),
        ("POST", "/api/agents/employees/x/suspend"),
        ("POST", "/api/storage/restore"),
        ("POST", "/api/alerts"),
    ] {
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {path}: a seeded Analyst must be denied"
        );
    }
}

/// # Input validation: malformed body -> 400 with the `{"error": "..."}`
/// envelope
///
/// `POST /api/auth/login` is `Policy::Public`, so this is the one route in
/// the table where a malformed-body 400 is reachable with zero setup
/// (every other `POST`/`PUT` route is behind the auth gate, and its body
/// is never even read on an unauthenticated/under-permissioned request —
/// see the module doc comment on why the authz loops above never send a
/// body at all).
#[tokio::test]
async fn malformed_login_body_is_a_400_with_the_error_envelope() {
    let TestApp { router, .. } = spin_up().await;

    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .body(Body::from("not json at all"))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .expect("content-type header"),
        "application/json;charset=utf-8"
    );
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    assert!(
        body.get("error")
            .and_then(serde_json::Value::as_str)
            .is_some(),
        "expected an {{\"error\": \"...\"}} envelope, got {body}"
    );
}

/// A well-formed-JSON-but-wrong-shape login body (missing `password`) is
/// still a 400 with the same envelope — `serde`'s `#[derive(Deserialize)]`
/// rejection path, not just the raw-JSON-parse-failure path above.
#[tokio::test]
async fn wrong_shaped_login_body_is_also_a_400_with_the_error_envelope() {
    let TestApp { router, .. } = spin_up().await;

    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .body(Body::from(r#"{"email": "someone@example.com"}"#))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    assert!(body.get("error").is_some());
}

/// The two authz loops above iterate `POLICY_TABLE` directly, so their
/// coverage IS the table's coverage by construction — there is no
/// separate list to keep in sync, and no route can be added to
/// `routes::router` with a matching table entry without automatically
/// gaining both a 403-when-underpermissioned and a
/// not-401/403-when-correctly-permissioned assertion the next time this
/// file runs. This test only pins that the table is non-trivially sized
/// (catches, e.g., an accidental `POLICY_TABLE = &[]`), not an exact
/// count, which would just be a second copy of the number to keep updated
/// by hand for no safety benefit over the loops themselves.
#[test]
fn policy_table_is_non_trivial_and_every_entry_is_walked_by_construction() {
    let public = POLICY_TABLE
        .iter()
        .filter(|(_, _, p)| *p == Policy::Public)
        .count();
    let auth_only = POLICY_TABLE
        .iter()
        .filter(|(_, _, p)| *p == Policy::RequiresAuth)
        .count();
    let permissioned = POLICY_TABLE
        .iter()
        .filter(|(_, _, p)| matches!(p, Policy::RequiresPermission(_)))
        .count();
    assert_eq!(public + auth_only + permissioned, POLICY_TABLE.len());
    assert!(
        POLICY_TABLE.len() > 50,
        "POLICY_TABLE looks suspiciously small ({}) for a ~30-route-module service",
        POLICY_TABLE.len()
    );
}

/// Every route the router actually registers must have a `POLICY_TABLE`
/// entry — the direction the two loops above do NOT cover.
///
/// They iterate `POLICY_TABLE` and prove each entry behaves correctly, which
/// says nothing about a route that was registered and then never classified.
/// `auth_gate` denies an unclassified route with a hard 500 rather than a
/// silent allow, so the failure mode is loud at runtime but invisible to the
/// test suite — `DELETE /api/connectors/{id}` shipped exactly that way, 500ing
/// on every call.
///
/// `axum::Router` exposes no way to enumerate its routes, so this reads
/// `routes/mod.rs` at compile time via `include_str!` and extracts the
/// `.route("<pattern>", get(..).post(..).delete(..))` registrations. Parsing
/// source is unlovely, but the alternative is a hand-maintained list that
/// drifts in exactly the same way the bug it is catching did.
#[test]
fn every_registered_route_has_a_policy_entry() {
    const ROUTES_SRC: &str = include_str!("../src/routes/mod.rs");

    // Strip comment lines first. `routes/mod.rs`'s own doc comments contain a
    // worked example of a deliberately-unclassified route
    // (`/api/__throwaway`), which a naive scan reports as a real registration.
    let code: String = ROUTES_SRC
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");

    let mut registered: Vec<(String, String)> = Vec::new();
    let mut rest = code.as_str();
    while let Some(at) = rest.find(".route(") {
        rest = &rest[at + ".route(".len()..];
        // The pattern is the first string literal in the call.
        let Some(open) = rest.find('"') else { break };
        let Some(close) = rest[open + 1..].find('"') else { break };
        let pattern = &rest[open + 1..open + 1 + close];
        if !pattern.starts_with("/api/") && pattern != "/health" {
            continue;
        }
        // Method handlers appear between the pattern and the closing paren of
        // this `.route(` call; a following `.route(` bounds the search.
        let tail = &rest[open + 1 + close..];
        let bound = tail.find(".route(").unwrap_or(tail.len());
        let handlers = &tail[..bound];
        for (needle, method) in [
            ("get(", "GET"),
            ("post(", "POST"),
            ("put(", "PUT"),
            ("patch(", "PATCH"),
            ("delete(", "DELETE"),
        ] {
            if handlers.contains(needle) {
                registered.push((method.to_owned(), pattern.to_owned()));
            }
        }
    }

    assert!(
        registered.len() > 50,
        "parsed only {} routes from routes/mod.rs — the extractor is probably \
         broken rather than the router being tiny",
        registered.len()
    );

    let missing: Vec<_> = registered
        .iter()
        .filter(|(m, p)| {
            !POLICY_TABLE
                .iter()
                .any(|(pm, pp, _)| pm == m && pp == p)
        })
        .collect();

    assert!(
        missing.is_empty(),
        "these routes are registered but absent from POLICY_TABLE, so auth_gate \
         will 500 on every call to them: {missing:?}"
    );
}
