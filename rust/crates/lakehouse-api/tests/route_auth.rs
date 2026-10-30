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
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_seeded_user, session_cookie_for_user, spin_up,
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

/// [`request_with_cookie`] with a JSON body, for routes whose handler needs
/// one to get past the auth gate. The auth gate runs first, so the body is
/// only ever read by a handler that has already been let in.
async fn request_with_cookie_and_body(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Vec<u8>,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body))
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

/// A seeded Analyst (no `identity:write`) is denied
/// `PUT /api/identity/users/{id}/tenants/{tenant_id}`, the route that gives
/// an existing user a tenant. The two loops above walk this entry through
/// `POLICY_TABLE`; this one names it, with a real principal.
#[tokio::test]
async fn a_seeded_analyst_is_denied_adding_a_user_to_a_tenant() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp =
        request_with_cookie(&router, "PUT", "/api/identity/users/x/tenants/x", &cookie).await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Analyst must be denied identity:write"
    );
}

/// BLD-1 / D2: `PUT /api/pipelines/{id}/run-as` decides whose permissions
/// every future run of that pipeline is governed by, so it is
/// `identity:write` — a governance decision, exactly like
/// `PUT /api/pipelines/{id}/tenant` above it in [`POLICY_TABLE`].
///
/// Both directions are named here with a real principal: a seeded Data
/// Engineer, who holds `pipeline:*` (so they can edit and run the
/// pipeline) but not `identity:write`, is refused; a Platform Admin is
/// not. Naming the Data Engineer rather than the Analyst is the stronger
/// half — `pipeline:write` alone must never be enough to reassign who a
/// pipeline reads as.
#[tokio::test]
async fn assigning_a_pipelines_author_is_denied_without_identity_write() {
    let TestApp { router, pool } = spin_up().await;
    let data_engineer = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;
    let admin = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let body = br#"{"userId":"00000000-0000-0000-0000-000000000001"}"#.to_vec();

    let denied = request_with_cookie_and_body(
        &router,
        "PUT",
        "/api/pipelines/pl-x/run-as",
        &data_engineer,
        body.clone(),
    )
    .await;
    assert_eq!(
        denied.status(),
        StatusCode::FORBIDDEN,
        "a seeded Data Engineer's pipeline:write must not be enough to \
         reassign who the pipeline reads as"
    );

    // The other half: the correctly-permissioned caller must never be
    // refused by the auth gate. This asserts only "not 401/403" — the id
    // does not exist, so a 404 from the handler is the right answer and
    // the body is deliberately not checked here (see this file's header).
    let allowed =
        request_with_cookie_and_body(&router, "PUT", "/api/pipelines/pl-x/run-as", &admin, body)
            .await;
    assert!(
        !matches!(
            allowed.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "a Platform Admin must pass the auth gate on this route, got {}",
        allowed.status()
    );
}

/// A seeded Analyst (no `governance:write`) is denied
/// `POST /api/lakehouse/tables/{ns}/{table}/maintenance` (WS2 §4),
/// a fifth `Policy::RequiresPermission` route added after the four-family
/// spot-check above — kept as its own test rather than folded into that
/// one so its doc comment's "four" stays accurate.
#[tokio::test]
async fn a_seeded_analyst_is_denied_the_maintenance_policy_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "POST",
        "/api/lakehouse/tables/x/x/maintenance",
        &cookie,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Analyst must be denied governance:write"
    );
}

/// A seeded Analyst (`sari@meridian.example` — `query:read, catalog:read,
/// lineage:read`, no `identity:write`) is denied
/// `POST /api/identity/service-identities/{id}/rotate` —
/// the service-identity rotation endpoint, gated by
/// `Policy::RequiresPermission("identity:write")`. The two table-driven
/// loops above cover the zero-permission and Platform-Admin directions
/// for this route by construction; this test pins the under-permissioned
/// direction against a real, independently-seeded role (a regression
/// that widened the gate to `Policy::RequiresAuth` would flip this 403 to
/// a non-403, and the message names the missing permission so that
/// failure cannot pass silently).
#[tokio::test]
async fn a_seeded_analyst_is_denied_service_identity_rotate() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "POST",
        "/api/identity/service-identities/00000000-0000-0000-0000-000000000000/rotate",
        &cookie,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "POST /api/identity/service-identities/{{id}}/rotate must refuse an Analyst \
         (missing identity:write) with 403"
    );
}

/// A seeded Analyst (`catalog:read`) is not denied
/// `GET /api/lakehouse/capacity` (WS2 §4). This route is
/// `Policy::RequiresPermission("catalog:read")`, the same seeded permission
/// as the rest of `/api/lakehouse/*` — not `RequiresAuth`, which an earlier
/// plan draft assumed by analogy to the now-cut `/api/storage*` routes.
/// The zero-permission-principal loop above already proves the OTHER
/// direction generically (any `RequiresPermission` entry, including this
/// one, gets 403 with no permissions at all); this is the matching
/// allowed-with-the-permission half.
#[tokio::test]
async fn a_seeded_analyst_is_not_denied_lakehouse_capacity() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(&router, "GET", "/api/lakehouse/capacity", &cookie).await;
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Analyst holding catalog:read must not be denied"
    );
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
}

/// A seeded Analyst (`catalog:read`, no `catalog:write`) is not denied
/// `GET /api/catalog/{id}/annotation` (WS2 §13) — that route is gated by
/// the same seeded `catalog:read` permission as the rest of the catalog
/// surface.
#[tokio::test]
async fn a_seeded_analyst_is_not_denied_catalog_annotation_read() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "GET",
        "/api/catalog/commerce_orders/annotation",
        &cookie,
    )
    .await;
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
    // SEC-16: the read now runs the catalog tenant gate too, and the seeded
    // app has several tenants and no `CATALOG_TENANT_ID`, so it answers 403
    // with the gate's fixed reason. Only that 403 is allowed here: the
    // permission middleware's body names the missing permission instead, so
    // the assertion still fails if the middleware denies the Analyst.
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    if status == StatusCode::FORBIDDEN {
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let error = body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            error.starts_with("permission_denied: per-dataset tenant ownership is not tracked"),
            "a seeded Analyst holding catalog:read must not be denied by the permission \
             middleware; the only 403 allowed here is the tenant gate's, got {error:?}"
        );
    }
}

/// A seeded Analyst (`catalog:read`, no `catalog:write`) IS denied
/// `PUT /api/catalog/{id}/annotation` (WS2 §13, WS2 plan review W8):
/// annotation writes reuse the already-seeded `catalog:write` permission,
/// which the seeded Analyst role does not hold.
#[tokio::test]
async fn a_seeded_analyst_is_denied_catalog_annotation_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "PUT",
        "/api/catalog/commerce_orders/annotation",
        &cookie,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Analyst must be denied catalog:write"
    );
}

/// A seeded Data Engineer (`catalog:write`, among others) is not denied
/// `PUT /api/catalog/{id}/annotation` by the permission middleware (WS2 §13,
/// WS2 plan review W8) — the matching allowed-with-the-permission half of
/// the test above.
///
/// PR #79 review, SEC-16: the handler now also runs the catalog tenant gate,
/// and the seeded app has several tenants and no `CATALOG_TENANT_ID`, so the
/// write still answers 403. That is a different 403 from the middleware's:
/// the middleware's body names the missing permission
/// (`permission_denied: catalog:write`), the tenant gate's body carries its
/// fixed `CATALOG_TENANT_REFUSAL_UNCONFIGURED` reason. The assertion
/// therefore accepts a 403 only with that reason, so it still fails if the
/// middleware denies the Data Engineer.
#[tokio::test]
async fn a_seeded_data_engineer_is_not_denied_catalog_annotation_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "PUT",
        "/api/catalog/commerce_orders/annotation",
        &cookie,
    )
    .await;
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    if status == StatusCode::FORBIDDEN {
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        let error = body
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        assert!(
            error.starts_with("permission_denied: per-dataset tenant ownership is not tracked"),
            "a seeded Data Engineer holding catalog:write must not be denied by the permission \
             middleware; the only 403 allowed here is the tenant gate's, got {error:?}"
        );
    }
}

/// `DATA-12` D1: `PUT /api/catalog/{id}/certification` needs
/// `governance:write`. A seeded Data Engineer holds `catalog:write` (which
/// edits the details) but not `governance:write`, so the permission
/// middleware refuses it with a 403 naming `governance:write`, and nothing
/// is stored. The table-driven loops above cover the new `POLICY_TABLE` row
/// both ways; this names the case the decision exists for.
#[tokio::test]
async fn a_seeded_data_engineer_is_denied_catalog_certification_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "PUT",
        "/api/catalog/commerce_orders/certification",
        &cookie,
    )
    .await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "catalog:write alone must not set a certification mark"
    );
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    assert_eq!(
        body.get("error").and_then(serde_json::Value::as_str),
        Some("permission_denied: governance:write"),
        "the 403 must come from the permission middleware, not the tenant gate"
    );
    let stored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM asset_annotation WHERE asset_id = 'commerce_orders'",
    )
    .fetch_one(&pool)
    .await
    .expect("count rows");
    assert_eq!(stored, 0, "a refused write must store nothing");
}

/// `DATA-13`: `GET /api/governance/lineage/columns` needs `lineage:read`.
/// A seeded Data Engineer holds `pipeline:*`, `catalog:write` and
/// `connector:manage` but not `lineage:read`, so the permission middleware
/// refuses it with a 403 naming the permission. The table-driven loops above
/// walk the new `POLICY_TABLE` row both ways; this names the case.
#[tokio::test]
async fn a_seeded_data_engineer_is_denied_column_lineage() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "GET",
        "/api/governance/lineage/columns?table=serving.x&column=y",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    assert_eq!(
        body.get("error").and_then(serde_json::Value::as_str),
        Some("permission_denied: lineage:read")
    );
}

/// `DATA-13`: the other half. A seeded Analyst holds `lineage:read`, so the
/// permission middleware lets it through to the handler (whatever the
/// handler then answers for an unreachable `ClickHouse` is not an auth
/// failure).
#[tokio::test]
async fn a_seeded_analyst_is_not_denied_column_lineage() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(
        &router,
        "GET",
        "/api/governance/lineage/columns?table=serving.x&column=y",
        &cookie,
    )
    .await;
    assert!(
        !matches!(
            resp.status(),
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
        ),
        "lineage:read must reach the handler, got {}",
        resp.status()
    );
}

/// `DATA-15` F3: listing, making and ending a grant from an asset's page
/// need `access:approve`, which only Governance Admin holds. A seeded Data
/// Engineer is refused by the permission middleware on all three, with the
/// permission named, and nothing is stored. The table-driven loops cover the
/// new `POLICY_TABLE` rows both ways; this names the case the rows exist for.
#[tokio::test]
async fn a_seeded_data_engineer_is_denied_all_three_catalog_grant_routes() {
    let TestApp { router, pool } = spin_up().await;
    for (method, path) in [
        ("GET", "/api/catalog/commerce_orders/grants"),
        ("POST", "/api/catalog/commerce_orders/grants"),
        ("DELETE", "/api/catalog/commerce_orders/grants/grant-x"),
    ] {
        let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        assert_eq!(
            body.get("error").and_then(serde_json::Value::as_str),
            Some("permission_denied: access:approve"),
            "{method} {path}: the 403 must come from the permission middleware"
        );
    }
    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM access_grant")
        .fetch_one(&pool)
        .await
        .expect("count rows");
    assert_eq!(stored, 0, "a refused grant must store nothing");
}

/// `DATA-15` F4: undropping a table needs `governance:write`. A seeded Data
/// Engineer holds `catalog:write` but not `governance:write`, so the
/// permission middleware refuses `POST /api/catalog/{id}/restore` with the
/// permission named.
#[tokio::test]
async fn a_seeded_data_engineer_is_denied_the_undrop() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;
    let resp = request_with_cookie(
        &router,
        "POST",
        "/api/catalog/commerce_orders/restore",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
    assert_eq!(
        body.get("error").and_then(serde_json::Value::as_str),
        Some("permission_denied: governance:write"),
    );
}

/// `DATA-15` F4: the restore status read is `catalog:read`, which a seeded
/// Analyst holds (and `governance:write` it does not), so the permission
/// middleware must not refuse it. The first version of this check used the
/// Data Engineer, who holds `catalog:write` only; CI failed with
/// `permission_denied: catalog:read` (`DATA-15` CI run).
#[tokio::test]
async fn a_seeded_analyst_is_not_denied_the_restore_status_read() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
    let resp = request_with_cookie(
        &router,
        "GET",
        "/api/catalog/commerce_orders/restore",
        &cookie,
    )
    .await;
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED);
    if resp.status() == StatusCode::FORBIDDEN {
        let bytes = to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("read body");
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
        assert_ne!(
            body.get("error").and_then(serde_json::Value::as_str),
            Some("permission_denied: catalog:read"),
            "catalog:read must pass the permission middleware"
        );
    }
}

/// A seeded Analyst (`catalog:read`) is not denied the two semantic-layer
/// reads (`GET /api/semantic`, `GET /api/semantic/{asset}`, AI-16): they
/// reuse the catalog's read permission.
#[tokio::test]
async fn a_seeded_analyst_is_not_denied_the_semantic_reads() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    for path in ["/api/semantic", "/api/semantic/serving.orders"] {
        let resp = request_with_cookie(&router, "GET", path, &cookie).await;
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "a seeded Analyst holding catalog:read must not be denied {path}"
        );
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "a valid session must never be treated as unauthenticated"
        );
    }
}

/// A seeded Analyst (`catalog:read`, no `catalog:write`) IS denied
/// `PUT /api/semantic/{asset}` (AI-16): confirming a description reuses the
/// catalog's write permission, which the Analyst role does not hold.
#[tokio::test]
async fn a_seeded_analyst_is_denied_the_semantic_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(&router, "PUT", "/api/semantic/serving.orders", &cookie).await;
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Analyst must be denied catalog:write"
    );
}

/// A seeded Data Engineer (`catalog:write`) is not denied
/// `PUT /api/semantic/{asset}`: the allowed half of the test above.
#[tokio::test]
async fn a_seeded_data_engineer_is_not_denied_the_semantic_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = request_with_cookie(&router, "PUT", "/api/semantic/serving.orders", &cookie).await;
    assert_ne!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Data Engineer holding catalog:write must not be denied"
    );
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
}

/// A principal holding ONLY `ingest:read` (not `connector:manage`) may
/// `GET /api/connectors/ingestible` but is refused the base
/// `GET /api/connectors` — the whole reason `/ingestible` exists as a
/// separate route rather than a query param on the `connector:manage`-gated
/// one: the Dagster ingest service identity (`dagster/dispar_orchestrate/ingest_factory.py`)
/// only ever holds `ingest:read`, never `connector:manage`.
#[tokio::test]
async fn ingest_read_scope_can_call_ingestible_but_not_the_base_connectors_route() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "ingest:read").await;

    let ingestible_cookie = session_cookie_for_user(&pool, user_id).await;
    let ingestible_resp = request_with_cookie(
        &router,
        "GET",
        "/api/connectors/ingestible",
        &ingestible_cookie,
    )
    .await;
    assert_ne!(
        ingestible_resp.status(),
        StatusCode::FORBIDDEN,
        "ingest:read alone must be enough to GET /api/connectors/ingestible"
    );
    assert_ne!(
        ingestible_resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );

    let base_cookie = session_cookie_for_user(&pool, user_id).await;
    let base_resp = request_with_cookie(&router, "GET", "/api/connectors", &base_cookie).await;
    assert_eq!(
        base_resp.status(),
        StatusCode::FORBIDDEN,
        "ingest:read alone must NOT be enough to GET the base /api/connectors route -- that \
         is a distinct, broader connector:manage-gated route"
    );
}

/// A principal holding ONLY `ingest:read` (not `connector:manage`) may
/// `GET /api/connectors/{id}/ingest-spec` and is refused `PUT` on the same
/// route.
///
/// This, together with the next two tests, replaces a wrong assertion this
/// plan originally specified ("a principal with only `connector:manage`
/// can do both") — `PermissionSet::has`
/// (`lakehouse-auth/src/permissions.rs`) matches resource+action exactly,
/// so `connector:manage` never satisfies `ingest:read`; a Data Engineer
/// can do both only because `0033_connector_ingest_spec.sql` grants it
/// BOTH permissions, not because one implies the other.
#[tokio::test]
async fn ingest_read_only_principal_may_get_but_not_put_ingest_spec() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "ingest:read").await;

    let get_cookie = session_cookie_for_user(&pool, user_id).await;
    let get_resp = request_with_cookie(
        &router,
        "GET",
        "/api/connectors/conn-pg-lakehouse/ingest-spec",
        &get_cookie,
    )
    .await;
    assert_ne!(
        get_resp.status(),
        StatusCode::FORBIDDEN,
        "ingest:read alone must be enough to GET the ingest-spec"
    );
    assert_ne!(
        get_resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );

    let put_cookie = session_cookie_for_user(&pool, user_id).await;
    let put_resp = request_with_cookie(
        &router,
        "PUT",
        "/api/connectors/conn-pg-lakehouse/ingest-spec",
        &put_cookie,
    )
    .await;
    assert_eq!(
        put_resp.status(),
        StatusCode::FORBIDDEN,
        "ingest:read alone must NOT be enough to PUT the ingest-spec -- that would hand a \
         read-only caller PUT-level authority"
    );
}

/// A principal holding ONLY `connector:manage` (not `ingest:read`) may
/// `PUT /api/connectors/{id}/ingest-spec` and is refused `GET` on the same
/// route -- the mirror image of the test above.
#[tokio::test]
async fn connector_manage_only_principal_may_put_but_not_get_ingest_spec() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "connector:manage").await;

    let get_cookie = session_cookie_for_user(&pool, user_id).await;
    let get_resp = request_with_cookie(
        &router,
        "GET",
        "/api/connectors/conn-pg-lakehouse/ingest-spec",
        &get_cookie,
    )
    .await;
    assert_eq!(
        get_resp.status(),
        StatusCode::FORBIDDEN,
        "connector:manage alone must NOT be enough to GET the ingest-spec -- ingest:read is a \
         distinct resource:action pair"
    );

    let put_cookie = session_cookie_for_user(&pool, user_id).await;
    let put_resp = request_with_cookie(
        &router,
        "PUT",
        "/api/connectors/conn-pg-lakehouse/ingest-spec",
        &put_cookie,
    )
    .await;
    assert_ne!(
        put_resp.status(),
        StatusCode::FORBIDDEN,
        "connector:manage alone must be enough to PUT the ingest-spec"
    );
    assert_ne!(
        put_resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
}

/// A seeded Data Engineer holds BOTH `ingest:read` and `connector:manage`
/// (`0033_connector_ingest_spec.sql`'s grant, on top of the seeded
/// `pipeline:*, catalog:write, connector:manage`), so it may do EACH verb
/// on `ingest-spec` -- unlike either single-permission principal above.
#[tokio::test]
async fn seeded_data_engineer_may_get_and_put_ingest_spec() {
    let TestApp { router, pool } = spin_up().await;

    let get_cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;
    let get_resp = request_with_cookie(
        &router,
        "GET",
        "/api/connectors/conn-pg-lakehouse/ingest-spec",
        &get_cookie,
    )
    .await;
    assert_ne!(
        get_resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Data Engineer holding ingest:read must not be denied GET"
    );
    assert_ne!(get_resp.status(), StatusCode::UNAUTHORIZED);

    let put_cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;
    let put_resp = request_with_cookie(
        &router,
        "PUT",
        "/api/connectors/conn-pg-lakehouse/ingest-spec",
        &put_cookie,
    )
    .await;
    assert_ne!(
        put_resp.status(),
        StatusCode::FORBIDDEN,
        "a seeded Data Engineer holding connector:manage must not be denied PUT"
    );
    assert_ne!(put_resp.status(), StatusCode::UNAUTHORIZED);
}

/// `SRC-11` decisions D9 and D11 at the route: leaving columns out is refused
/// with a 422 for the key column of a deduplicating table (on `PostgreSQL`, and
/// on Oracle since `SRC-4` T1.2 made column choice available for it; the
/// refusal for a source that cannot list its columns is the unit test
/// `excluded_columns_are_accepted_only_for_sources_that_list_their_columns`),
/// and a malformed list is a 400. All three are refused before the address check or any write, so no connection
/// is made and nothing is stored.
#[tokio::test]
async fn ingest_spec_put_refuses_excluded_columns_the_source_cannot_honour() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "connector:manage").await;
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) \
         VALUES ($1, '11111111-1111-4111-8111-000000000001')",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("join the connector's tenant");

    let body = |driver: &str, objects: serde_json::Value| {
        serde_json::json!({
            "adapter": "sql",
            "ingestMode": "batch",
            "dial": {
                "driver": driver, "host": "db.example", "port": 5432,
                "database": "orders", "user": "app_reader"
            },
            "sourceObjects": objects,
        })
    };
    let cases = [
        (
            "oracle",
            serde_json::json!([{
                "name": "APP.ORDERS", "target": "orders", "loadMode": "replace_dedup",
                "primaryKey": ["ID"], "excludedColumns": ["ID"]
            }]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "postgres",
            serde_json::json!([{
                "name": "public.orders", "target": "orders", "loadMode": "replace_dedup",
                "primaryKey": ["id"], "excludedColumns": ["id"]
            }]),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "postgres",
            serde_json::json!([{
                "name": "public.orders", "target": "orders", "excludedColumns": "note"
            }]),
            StatusCode::BAD_REQUEST,
        ),
    ];
    for (driver, objects, expected) in cases {
        let cookie = session_cookie_for_user(&pool, user_id).await;
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/connectors/conn-pg-lakehouse/ingest-spec")
                    .header("cookie", cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&body(driver, objects.clone())).expect("serialize body"),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        assert_eq!(response.status(), expected, "{driver} {objects}");
    }
}

/// `SRC-11`: the run history, a run's log and Resume are a person's routes
/// (`connector:manage`); the four run reports are the orchestrator's
/// (`ingest:read`, then the handler's service-identity check). Each direction
/// is asserted: a `connector:manage`-only principal passes the gate of the
/// person's routes and is refused by the gate on the reports; an
/// `ingest:read`-only principal is refused by the gate on the person's routes
/// and, on the reports, passes the gate and is refused by the handler (it is a
/// user, not the service identity). The table-driven loops above cover the
/// zero-permission and Platform-Admin directions for all seven entries.
#[tokio::test]
async fn run_history_log_and_resume_need_connector_manage_and_run_reports_need_ingest_read() {
    let TestApp { router, pool } = spin_up().await;
    let manage = create_principal_with_permissions(&pool, "connector:manage").await;
    let ingest = create_principal_with_permissions(&pool, "ingest:read").await;
    let base = "/api/connectors/conn-pg-lakehouse";
    let person_routes = [
        ("GET", format!("{base}/runs")),
        ("GET", format!("{base}/runs/run-1/log")),
        ("POST", format!("{base}/resume")),
    ];
    let report_routes = [
        ("POST", format!("{base}/run-reports")),
        ("POST", format!("{base}/run-reports/run-1/attempts")),
        ("POST", format!("{base}/run-reports/run-1/log")),
        ("POST", format!("{base}/run-reports/run-1/end")),
    ];
    for (method, path) in &person_routes {
        let cookie = session_cookie_for_user(&pool, manage).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_ne!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
        assert_ne!(resp.status(), StatusCode::UNAUTHORIZED, "{method} {path}");

        let cookie = session_cookie_for_user(&pool, ingest).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
    for (method, path) in &report_routes {
        let cookie = session_cookie_for_user(&pool, manage).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
        let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
            .into_owned();
        assert!(
            text.contains("ingest:read"),
            "the gate names the permission: {text}"
        );

        let cookie = session_cookie_for_user(&pool, ingest).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
        let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
            .into_owned();
        assert!(
            text.contains("service identity"),
            "passed the gate, refused by the handler: {text}"
        );
    }
}

/// `SRC-4` slice 4: the lag limit is a person's (`connector:manage`); the capture
/// observations are the orchestrator's (`ingest:read`, then the handler's
/// service-identity check). Both ways: a `connector:manage`-only principal passes
/// the gate of the limit route and is refused by the gate on the observations; an
/// `ingest:read`-only principal is refused by the gate on the limit route and, on
/// the observations, passes the gate and is refused by the handler (a user, not
/// the service identity). The table-driven loops above cover the zero-permission
/// and Platform-Admin directions for both entries.
#[tokio::test]
async fn the_lag_limit_needs_connector_manage_and_capture_observations_need_ingest_read() {
    let TestApp { router, pool } = spin_up().await;
    let manage = create_principal_with_permissions(&pool, "connector:manage").await;
    let ingest = create_principal_with_permissions(&pool, "ingest:read").await;
    let base = "/api/connectors/conn-pg-lakehouse";
    let limit = ("PUT", format!("{base}/lag-limit"));
    let observations = ("POST", format!("{base}/capture/observations"));

    let cookie = session_cookie_for_user(&pool, manage).await;
    let resp = request_with_cookie(&router, limit.0, &limit.1, &cookie).await;
    assert_ne!(resp.status(), StatusCode::FORBIDDEN, "{}", limit.1);
    assert_ne!(resp.status(), StatusCode::UNAUTHORIZED, "{}", limit.1);
    let cookie = session_cookie_for_user(&pool, ingest).await;
    let resp = request_with_cookie(&router, limit.0, &limit.1, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{}", limit.1);

    let cookie = session_cookie_for_user(&pool, manage).await;
    let resp = request_with_cookie(&router, observations.0, &observations.1, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(
        text.contains("ingest:read"),
        "the gate names the permission: {text}"
    );
    let cookie = session_cookie_for_user(&pool, ingest).await;
    let resp = request_with_cookie(&router, observations.0, &observations.1, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(
        text.contains("service identity"),
        "passed the gate, refused by the handler: {text}"
    );
}

/// `SRC-13` (decision D11): the discovery routes are a person's
/// (`connector:manage`), both ways, and the orchestrator's answer route is the
/// service identity's (`ingest:read`, then the handler's check). A
/// `connector:manage`-only principal passes the gate of the person's routes (the
/// handler answers 400 for the body it was sent, or 404 for an answer nobody
/// asked for) and is refused by the gate on the result route; an
/// `ingest:read`-only principal is refused by the gate on the person's routes
/// and, on the result route, passes the gate and is refused by the handler (it
/// is a user, not the service identity). The table-driven loops above cover the
/// zero-permission and Platform-Admin directions for all three entries.
#[tokio::test]
async fn discovery_routes_need_connector_manage_and_the_result_route_needs_ingest_read() {
    let TestApp { router, pool } = spin_up().await;
    let manage = create_principal_with_permissions(&pool, "connector:manage").await;
    let ingest = create_principal_with_permissions(&pool, "ingest:read").await;
    // SRC-13 review 3 BLOCKER 2: both principals join Meridian Group, the tenant of
    // `conn-pg-lakehouse`. The per-connector routes answer 404 for a connector outside
    // the caller's tenants (the route layer, before the handler), so without this the
    // person's routes answered 404 instead of reaching the handler's 400, and the
    // test's point (403 from the permission gate, not from the tenant gate) was lost.
    for user_id in [manage, ingest] {
        sqlx::query(
            "INSERT INTO app_user_tenant (user_id, tenant_id) \
             VALUES ($1, '11111111-1111-4111-8111-000000000001')",
        )
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("join the connector's tenant");
    }
    let base = "/api/connectors/conn-pg-lakehouse";
    // A preview with no object is a 400 before any dial; a poll with nothing
    // requested is a 404. Neither touches a source.
    let person_routes = [
        (
            "POST",
            format!("{base}/discovery/preview"),
            StatusCode::BAD_REQUEST,
        ),
        (
            "GET",
            format!("{base}/discovery/list"),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (method, path, reached) in &person_routes {
        let cookie = session_cookie_for_user(&pool, manage).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), *reached, "{method} {path}");

        let cookie = session_cookie_for_user(&pool, ingest).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }

    let result_route = format!("{base}/discovery-results");
    let cookie = session_cookie_for_user(&pool, manage).await;
    let resp = request_with_cookie(&router, "POST", &result_route, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(
        text.contains("ingest:read"),
        "the gate names the permission: {text}"
    );

    let cookie = session_cookie_for_user(&pool, ingest).await;
    let resp = request_with_cookie(&router, "POST", &result_route, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(
        text.contains("service identity"),
        "passed the gate, refused by the handler: {text}"
    );
}

/// `SRC-12` part C: the OAuth sign-in routes of a connector are a person's
/// (`connector:manage`), the rotated-token report is the orchestrator's
/// (`ingest:read`, then the handler's service-identity check), and the
/// provider's callback needs a signed-in user and nothing more at the gate.
/// Each direction: a `connector:manage`-only principal passes the gate of the
/// person's routes (the handler answers 409, the seeded Postgres connector is
/// not an OAuth one) and is refused by the gate on the report; an
/// `ingest:read`-only principal is refused by the gate on the person's routes
/// and passes the gate on the report to be refused by the handler (a user, not
/// the service identity). The callback refuses nobody at the gate who is signed
/// in (a principal with no permission at all gets the fixed "not valid" page)
/// and answers 401 to nobody signed in.
#[tokio::test]
async fn oauth_routes_need_connector_manage_the_report_needs_ingest_read_and_the_callback_a_user() {
    let TestApp { router, pool } = spin_up().await;
    let manage = create_principal_with_permissions(&pool, "connector:manage").await;
    let ingest = create_principal_with_permissions(&pool, "ingest:read").await;
    let nobody = create_zero_permission_principal(&pool).await;
    // Both join the connector's tenant, as in the discovery test above, so the
    // person's routes reach the handler and the 403 is the permission gate's.
    for user_id in [manage, ingest] {
        sqlx::query(
            "INSERT INTO app_user_tenant (user_id, tenant_id) \
             VALUES ($1, '11111111-1111-4111-8111-000000000001')",
        )
        .bind(user_id)
        .execute(&pool)
        .await
        .expect("join the connector's tenant");
    }
    let base = "/api/connectors/conn-pg-lakehouse";
    let person_routes = [
        ("GET", format!("{base}/oauth")),
        ("POST", format!("{base}/oauth/start")),
        ("POST", format!("{base}/oauth/sign-out")),
    ];
    for (method, path) in &person_routes {
        let cookie = session_cookie_for_user(&pool, manage).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::CONFLICT, "{method} {path}");

        let cookie = session_cookie_for_user(&pool, ingest).await;
        let resp = request_with_cookie(&router, method, path, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }

    let report = format!("{base}/oauth/refresh-token");
    let cookie = session_cookie_for_user(&pool, manage).await;
    let resp = request_with_cookie(&router, "POST", &report, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(
        text.contains("ingest:read"),
        "the gate names the permission: {text}"
    );
    let cookie = session_cookie_for_user(&pool, ingest).await;
    let resp = request_with_cookie(&router, "POST", &report, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(
        text.contains("service identity"),
        "passed the gate, refused by the handler: {text}"
    );

    // The callback: signed in, with no permission at all, passes the gate and
    // gets the fixed page (no `state`); not signed in, 401.
    let callback = "/api/connectors/oauth/callback";
    let cookie = session_cookie_for_user(&pool, nobody).await;
    let resp = request_with_cookie(&router, "GET", callback, &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let text = String::from_utf8_lossy(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
        .into_owned();
    assert!(text.contains("Sign-in not completed"), "{text}");
    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(callback)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

/// The upload routes (ADR 0014, decision 6; `DATA-17` added the four of a
/// chunked upload and slice 5 the comparison of several files) are gated by the existing
/// `connector:manage`, with no new permission: a seeded Data Engineer
/// (`bayu@meridian.example`) is never refused at the gate, a seeded Analyst
/// (`sari@meridian.example`, no `connector:manage`) always is. The two
/// table-driven loops above prove the zero-permission and Platform-Admin
/// directions for these entries by construction; this pins the real roles,
/// and `POST /api/uploads` by name.
///
/// What a Data Engineer's request then does is not asserted here: with no
/// storage credentials in `spin_up`, `POST /api/uploads` answers 503, and the
/// per-id routes answer 404 for an upload that does not exist.
/// `tests/upload_routes.rs` covers those.
#[tokio::test]
async fn a_data_engineer_is_not_refused_and_an_analyst_is_on_every_upload_route() {
    let TestApp { router, pool } = spin_up().await;

    for (method, path) in [
        ("POST", "/api/uploads"),
        ("GET", "/api/uploads"),
        ("GET", "/api/uploads/x"),
        ("DELETE", "/api/uploads/x"),
        ("GET", "/api/uploads/x/preview"),
        ("POST", "/api/uploads/x/ingest"),
        // DATA-17: the chunked upload. `sessions` belongs to no upload; the
        // other three answer 404 for an upload that does not exist.
        ("POST", "/api/uploads/sessions"),
        ("PUT", "/api/uploads/x/parts/1"),
        ("GET", "/api/uploads/x/parts"),
        ("POST", "/api/uploads/x/complete"),
        // DATA-18: the check of a column list.
        ("POST", "/api/uploads/x/check"),
        // DATA-17 slice 5: which other files can load into one table with this one.
        ("POST", "/api/uploads/x/compare"),
        // DATA-35: folders for uploaded files; unknown ids answer 404, not 401/403.
        ("GET", "/api/uploads/folders"),
        ("POST", "/api/uploads/folders"),
        ("PATCH", "/api/uploads/folders/x"),
        ("DELETE", "/api/uploads/folders/x"),
        ("POST", "/api/uploads/move"),
    ] {
        let engineer = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;
        let resp = request_with_cookie(&router, method, path, &engineer).await;
        assert!(
            !matches!(
                resp.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ),
            "{method} {path}: a seeded Data Engineer holds connector:manage and must pass the \
             gate, got {}",
            resp.status()
        );

        let analyst = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;
        let resp = request_with_cookie(&router, method, path, &analyst).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {path}: a seeded Analyst lacks connector:manage and must be refused"
        );
    }
}

/// # SSRF: `PUT .../ingest-spec` refuses an obviously-internal dial host at
/// save time
///
/// This is a SECOND, non-authoritative check, on top of the dial-time
/// guard `connector_probe::resolve_checked`/Dagster's own `ssrf_guard`
/// already run — see `routes::connectors::ingest_spec_put`'s doc comment
/// for why a save-time check can never be the guarantee (DNS can change
/// between this `PUT` and a later `POST .../ingest/run`) and exists only
/// to fail fast on the common case (WS3 plan judge review Z1).
#[tokio::test]
async fn ingest_spec_put_rejects_a_dial_whose_host_resolves_internal() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "connector:manage").await;
    // Meridian Group, `conn-pg-lakehouse`'s tenant: the per-connector
    // routes answer only for a connector in the caller's tenant.
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) \
         VALUES ($1, '11111111-1111-4111-8111-000000000001')",
    )
    .bind(user_id)
    .execute(&pool)
    .await
    .expect("join the connector's tenant");
    let cookie = session_cookie_for_user(&pool, user_id).await;

    let body = serde_json::json!({
        "adapter": "rest",
        "ingestMode": "batch",
        "dial": {
            "baseUrl": "http://127.0.0.1:9999",
            "auth": {"type": "bearer"},
            "pagination": {"type": "none"},
            "endpoints": [],
        },
        "sourceObjects": [],
        // SEC-14: this body moves `conn-pg-lakehouse` from `sql` to `rest`,
        // which is a re-point and so needs its credential in the same
        // request; without it the answer would be the 409 of that rule
        // (tests/connector_repoint.rs), not the address check's 400 this
        // test is about. The credential is a throwaway bearer token.
        "credential": { "primary": { "kind": "token", "value": "throwaway-token" } },
    });

    let response = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/connectors/conn-pg-lakehouse/ingest-spec")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a rest dial whose baseUrl resolves to a loopback address must be refused at save \
         time, not just discovered on the next scheduled ingest run"
    );
}

// ── Dataset SLA (WS5 item E1, Y6) ────────────────────────────────────────

/// A principal holding ONLY `policy:read` (not `governance:write`) may
/// `GET /api/governance/sla` and is refused `PUT` on the same route —
/// mirrors `ingest_read_only_principal_may_get_but_not_put_ingest_spec`'s
/// shape: `GET`/`PUT` on one path are gated by two DIFFERENT seeded
/// permissions, not by one permission implying the other.
#[tokio::test]
async fn policy_read_only_principal_may_get_but_not_put_sla() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "policy:read").await;

    let get_cookie = session_cookie_for_user(&pool, user_id).await;
    let get_resp = request_with_cookie(&router, "GET", "/api/governance/sla", &get_cookie).await;
    assert_ne!(
        get_resp.status(),
        StatusCode::FORBIDDEN,
        "policy:read alone must be enough to GET /api/governance/sla"
    );
    assert_ne!(
        get_resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );

    let put_cookie = session_cookie_for_user(&pool, user_id).await;
    let put_resp = request_with_cookie(&router, "PUT", "/api/governance/sla", &put_cookie).await;
    assert_eq!(
        put_resp.status(),
        StatusCode::FORBIDDEN,
        "policy:read alone must NOT be enough to PUT /api/governance/sla -- that would hand \
         a read-only caller write authority"
    );
}

/// A principal holding ONLY `governance:write` (not `policy:read`) may
/// `PUT /api/governance/sla` and is refused `GET` on the same route — the
/// mirror image of the test above.
#[tokio::test]
async fn governance_write_only_principal_may_put_but_not_get_sla() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "governance:write").await;

    let get_cookie = session_cookie_for_user(&pool, user_id).await;
    let get_resp = request_with_cookie(&router, "GET", "/api/governance/sla", &get_cookie).await;
    assert_eq!(
        get_resp.status(),
        StatusCode::FORBIDDEN,
        "governance:write alone must NOT be enough to GET /api/governance/sla -- that is a \
         distinct, policy:read-gated route"
    );

    let put_cookie = session_cookie_for_user(&pool, user_id).await;
    let put_resp = request_with_cookie(&router, "PUT", "/api/governance/sla", &put_cookie).await;
    assert_ne!(
        put_resp.status(),
        StatusCode::FORBIDDEN,
        "governance:write alone must be enough to PUT /api/governance/sla"
    );
    assert_ne!(
        put_resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
}

/// SEC-23 (F1/F2): a principal holding other write permissions but NOT
/// `governance:write` is refused (403) when it adds a quality,
/// classification or residency rule, and nothing is stored. The bodies are
/// valid, so a 403 can only come from the permission gate, and the row
/// counts prove the handler never ran.
#[tokio::test]
async fn adding_a_governance_rule_without_governance_write_is_refused_and_stores_nothing() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(
        &pool,
        "catalog:read,catalog:write,policy:read,policy:write,query:read,pipeline:write",
    )
    .await;

    let cases = [
        (
            "quality",
            "quality_rule",
            serde_json::json!({
                "name": "sec23-no-write", "asset": "gold.orders", "dimension": "completeness",
                "threshold": "99%", "severity": "high",
            }),
        ),
        (
            "classification",
            "classification_rule",
            serde_json::json!({
                "asset": "gold.orders", "column": "email", "classification": "PII",
                "maskingRule": "hash",
            }),
        ),
        (
            "residency",
            "residency_rule",
            serde_json::json!({
                "tenant": "sec23", "classification": "PII", "approvedSites": ["site-a"],
                "crossSiteAllowed": false, "allowedOutput": "none",
            }),
        ),
    ];
    for (kind, table, body) in cases {
        // safe: only the constant `table` from the array above is formatted
        // in; never user input.
        let count_sql = format!("SELECT count(*) FROM {table}");
        let before: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(count_sql.as_str()))
            .fetch_one(&pool)
            .await
            .expect("count rules before");
        let cookie = session_cookie_for_user(&pool, user_id).await;
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/governance/{kind}"))
                    .header("cookie", cookie)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&body).expect("serialize body"),
                    ))
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "adding a {kind} rule without governance:write must be refused"
        );
        // safe: `table` is a constant from the array above, never user input.
        let after: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(count_sql.as_str()))
            .fetch_one(&pool)
            .await
            .expect("count rules after");
        assert_eq!(before, after, "a refused {kind} rule must store nothing");
    }
}

/// # Input validation: `expectedIntervalMinutes: 0` is refused at the
/// route, never reaching the database (WS5 plan review U12 — both layers
/// tested, not just the store-level `CHECK` in
/// `lakehouse-store/tests/governance.rs`).
///
/// `put_sla` validates the body (`validate_namespaced_table`, then the
/// `expectedIntervalMinutes <= 0` check) BEFORE it ever calls `pool(&state)`
/// to reach Postgres, so this is a real 400, not a database error
/// reinterpreted as one — see `routes::governance::put_sla`'s doc comment.
#[tokio::test]
async fn put_sla_rejects_a_non_positive_interval_before_touching_the_database() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "governance:write").await;
    let cookie = session_cookie_for_user(&pool, user_id).await;

    let body = serde_json::json!({
        "tableName": "gold.orders",
        "expectedIntervalMinutes": 0,
    });

    let response = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/governance/sla")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a zero expectedIntervalMinutes must be refused at the route, not surfaced as a \
         500 from the database CHECK constraint"
    );
}

/// A malformed `tableName` (no `.`, so it cannot split into
/// `<namespace>.<table>`) is refused the same way, for the same reason.
#[tokio::test]
async fn put_sla_rejects_a_table_name_with_no_namespace() {
    let TestApp { router, pool } = spin_up().await;
    let user_id = create_principal_with_permissions(&pool, "governance:write").await;
    let cookie = session_cookie_for_user(&pool, user_id).await;

    let body = serde_json::json!({
        "tableName": "orders",
        "expectedIntervalMinutes": 60,
    });

    let response = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/governance/sla")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::to_vec(&body).expect("serialize body"),
                ))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "a tableName with no <namespace>.<table> split must be refused at the route"
    );
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

/// SEC-9: the natural-language box in Query Studio has no endpoint of its
/// own. The old `/api/agent*` ask and text-to-SQL routes ran model-written SQL
/// straight against the engine; #78 removed them, and the assistant now runs
/// SQL only through the tool gate (`/api/ai/chat`, `/api/ai/tool`). A route
/// of that shape coming back would be a path around the gate, so it fails
/// here by name.
#[test]
fn no_route_runs_model_written_sql_outside_the_assistant_tool_gate() {
    for (method, path, _) in POLICY_TABLE {
        let lower = path.to_ascii_lowercase();
        assert!(
            lower != "/api/agent" && !lower.starts_with("/api/agent/"),
            "{method} {path}: a standalone agent endpoint (SEC-9)"
        );
        assert!(
            !lower.contains("text-to-sql")
                && !lower.contains("text_to_sql")
                && !lower.contains("nl2sql"),
            "{method} {path}: a text-to-SQL endpoint (SEC-9)"
        );
        assert!(
            !(lower.starts_with("/api/ai/") && lower.contains("sql")),
            "{method} {path}: an AI route that names SQL outside the tool gate (SEC-9)"
        );
    }
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
        let Some(close) = rest[open + 1..].find('"') else {
            break;
        };
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
        .filter(|(m, p)| !POLICY_TABLE.iter().any(|(pm, pp, _)| pm == m && pp == p))
        .collect();

    assert!(
        missing.is_empty(),
        "these routes are registered but absent from POLICY_TABLE, so auth_gate \
         will 500 on every call to them: {missing:?}"
    );
}

/// `GET /api/auth/me` returns each membership as `{id, name, slug}` — not
/// the bare `Vec<String>` of UUID strings the previous shape used, so a
/// tenant-switcher has something to render. `sari@meridian.example` is
/// seeded into exactly one tenant (`meridian-retail`,
/// `0002_seed_identity.sql`), so the assertion is exact: one structured
/// entry, every field a string.
#[tokio::test]
async fn me_response_returns_structured_tenants_not_bare_uuid_strings() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "sari@meridian.example").await;

    let resp = request_with_cookie(&router, "GET", "/api/auth/me", &cookie).await;
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    let tenants = body["tenants"]
        .as_array()
        .expect("`tenants` is an array (not the legacy bare-uuid array)");
    assert_eq!(
        tenants.len(),
        1,
        "sari@meridian.example is seeded into exactly one tenant (0002_seed_identity.sql)"
    );
    let entry = &tenants[0];
    assert!(
        entry["id"].is_string(),
        "tenant[0].id must be a string (not a bare-UUID string), got {:?}",
        entry["id"]
    );
    assert!(
        entry["name"].is_string(),
        "tenant[0].name must be a string, got {:?}",
        entry["name"]
    );
    assert!(
        entry["slug"].is_string(),
        "tenant[0].slug must be a string, got {:?}",
        entry["slug"]
    );
    assert_eq!(
        entry["slug"].as_str(),
        Some("meridian-retail"),
        "sari's only seeded tenant is meridian-retail (0002_seed_identity.sql)"
    );
}

// ── The two non-OIDC admin actions under audit
// (`create_tenant`, `revoke_session`) each write a real `audit_event`
// row on success. The OIDC-callback audit test lives in
// `routes/auth.rs::tests` because its wiremock round-trip harness is
// local to that module. These two live here because they only need the
// shared `spin_up`/`session_cookie_for_user`/`request_with_cookie` triple
// and a `#[tokio::test]` over the real router, exactly the shape the
// rest of this file already follows. ──────────────────────────────────────

/// `POST /api/identity/tenants` writes a `audit_event` row on success
/// with `action = 'identity.tenant.create'`,
/// `resource_kind = 'tenant'`, `resource_id = <new tenant's id>`, and
/// `outcome = 'executed'`.
///
/// `spin_up()` does not override `LAKEKEEPER_ADMIN_TOKEN_FILE`, so
/// `AppState::lakekeeper_admin` is `None` and the route's
/// no-admin early return fires AFTER the audit write — exactly the
/// "audit fires whether or not provisioning is reachable" property the
/// audit-write comment in `routes::identity::create_tenant` calls out.
#[tokio::test]
async fn create_tenant_writes_an_audit_event_on_success() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;

    let body_bytes = serde_json::to_vec(&serde_json::json!({
        "name": "Acme Co",
        "slug": "acme-co",
        "plan": "Standard",
        "residency": "US",
    }))
    .expect("serialize body");

    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/identity/tenants")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body_bytes))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "create_tenant must succeed (the tenant row is created and the \
         audit row is written even when no Lakekeeper admin is configured \
         on this deployment)"
    );

    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("valid JSON body");
    let tenant_id = body["id"]
        .as_str()
        .expect("the response body carries the new tenant's id");

    let row: (String, Option<String>, Option<String>, String) = sqlx::query_as(
        "SELECT action, resource_kind, resource_id, outcome FROM audit_event \
         WHERE action = 'identity.tenant.create' ORDER BY at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("an audit_event row for identity.tenant.create must exist");

    assert_eq!(row.0, "identity.tenant.create");
    assert_eq!(row.1.as_deref(), Some("tenant"));
    assert_eq!(
        row.2.as_deref(),
        Some(tenant_id),
        "resource_id must point at the new tenant's id"
    );
    assert_eq!(row.3, "executed");
}

/// `DELETE /api/auth/sessions/{id}` writes a `audit_event` row on
/// success with `action = 'auth.session.revoke'`, `resource_id` =
/// the session's UUID, and `outcome = 'executed'`.
///
/// Two live sessions are minted for the same `app_user.id`: one
/// carries the cookie that drives the DELETE (so the auth gate
/// resolves the caller as that user), and the other is the target of
/// the DELETE. `revoke_session_as_caller`'s SQL predicate
/// (`WHERE id = $1 AND revoked_at IS NULL AND ($3 OR app_user_id = $2)`)
/// accepts the second because both sessions belong to the same user.
#[tokio::test]
async fn delete_session_writes_an_audit_event() {
    let TestApp { router, pool } = spin_up().await;
    let user_id: Uuid =
        sqlx::query_scalar("INSERT INTO app_user (name, email) VALUES ($1, $2) RETURNING id")
            .bind("caller")
            .bind("caller-audit-revoke@x.invalid")
            .fetch_one(&pool)
            .await
            .expect("insert a caller");

    // Two sessions: one carries the cookie doing the DELETE, one is
    // the target. `revoke_session` checks ownership via the SQL
    // predicate, so both have to belong to the same `app_user.id`.
    let caller_cookie = session_cookie_for_user(&pool, user_id).await;
    let _target_cookie = session_cookie_for_user(&pool, user_id).await;
    let target_session_id: Uuid = sqlx::query_scalar(
        "SELECT id FROM session \
         WHERE app_user_id = $1 AND revoked_at IS NULL \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .expect("look up the target session id");

    let response = request_with_cookie(
        &router,
        "DELETE",
        &format!("/api/auth/sessions/{target_session_id}"),
        &caller_cookie,
    )
    .await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let row: (String, Option<String>, String) = sqlx::query_as(
        "SELECT action, resource_id, outcome FROM audit_event \
         WHERE action = 'auth.session.revoke' ORDER BY at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("an audit_event row for auth.session.revoke must exist");

    assert_eq!(row.0, "auth.session.revoke");
    assert_eq!(row.2, "executed");
    assert_eq!(
        row.1.as_deref(),
        Some(target_session_id.to_string().as_str()),
        "resource_id must be the UUID string of the session that was \
         revoked (the request path's {{id}}, never the caller's bearer \
         token)"
    );
}

// ── Governed tag keys (DATA-12 part 2) ───────────────────────────────────

/// The four `/api/governance/tag-keys` routes, asserted in both directions:
/// a principal holding ONLY `catalog:read` may list the keys (an editor needs
/// them to render pickers) and is refused every write; a principal holding
/// ONLY `governance:write` may define, change and remove keys and is refused
/// the list, which is a distinct, `catalog:read`-gated route. Each write is
/// sent with an empty body, so a pass is anything but 401/403 (the handler
/// then answers 400/404 for the body or key) and a refusal stores nothing.
#[tokio::test]
async fn tag_key_routes_are_gated_in_both_directions() {
    // PR #106 review R2 SHOULD-FIX 1: catalog-tenant caller
    let mut env = std::collections::HashMap::new();
    let catalog_tenant = "11111111-1111-4111-8111-000000000001";
    env.insert("CATALOG_TENANT_ID".to_owned(), catalog_tenant.to_owned());
    let TestApp { router, pool } = common::spin_up_with_env(&env).await;

    let reader = create_principal_with_permissions(&pool, "catalog:read").await;
    let admin = create_principal_with_permissions(&pool, "governance:write").await;
    let tenant_uuid: Uuid = catalog_tenant.parse().unwrap();
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2), ($3, $2)")
        .bind(reader)
        .bind(tenant_uuid)
        .bind(admin)
        .execute(&pool)
        .await
        .expect("associate test users with catalog tenant");

    let writes = [
        ("POST", "/api/governance/tag-keys", StatusCode::BAD_REQUEST),
        (
            "PUT",
            "/api/governance/tag-keys/data_domain",
            StatusCode::BAD_REQUEST,
        ),
        (
            "DELETE",
            "/api/governance/tag-keys/data_domain",
            StatusCode::NOT_FOUND,
        ),
    ];

    let reader_cookie = session_cookie_for_user(&pool, reader).await;
    let listed =
        request_with_cookie(&router, "GET", "/api/governance/tag-keys", &reader_cookie).await;
    // PR #106 review R3 SHOULD-FIX 15: strict positive assertions
    assert_eq!(
        listed.status(),
        StatusCode::OK,
        "catalog:read alone must be enough to list the tag keys"
    );
    for (method, path, _) in writes {
        let resp = request_with_cookie(&router, method, path, &reader_cookie).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "catalog:read alone must NOT be enough to {method} {path}"
        );
    }

    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let refused =
        request_with_cookie(&router, "GET", "/api/governance/tag-keys", &admin_cookie).await;
    assert_eq!(
        refused.status(),
        StatusCode::FORBIDDEN,
        "governance:write alone must NOT be enough to list the tag keys"
    );
    // PR #106 review R3 SHOULD-FIX 15: strict positive assertions with exact handler status
    for (method, path, expected) in writes {
        let resp = request_with_cookie(&router, method, path, &admin_cookie).await;
        assert_eq!(
            resp.status(),
            expected,
            "governance:write must reach handler for {method} {path} (answering {expected})"
        );
    }

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM governed_tag_key")
        .fetch_one(&pool)
        .await
        .expect("count keys");
    assert_eq!(stored, 0, "no empty-bodied request may store a key");
}

// ── Quality incidents (DATA-14) ─────────────────────────────────────────

/// The two `/api/governance/quality/incidents` routes, asserted in both
/// directions: any login may list incidents (like the quality rule list), a
/// principal without `governance:write` is refused the resolve (403) and
/// nothing changes, and one holding `governance:write` alone is past the gate
/// (the unknown incident then answers 404, not 401/403).
#[tokio::test]
async fn quality_incident_routes_are_gated_in_both_directions() {
    // PR #106 review R2 SHOULD-FIX 1: catalog-tenant caller
    let mut env = std::collections::HashMap::new();
    let catalog_tenant = "11111111-1111-4111-8111-000000000001";
    env.insert("CATALOG_TENANT_ID".to_owned(), catalog_tenant.to_owned());
    let TestApp { router, pool } = common::spin_up_with_env(&env).await;

    let reader = create_principal_with_permissions(&pool, "catalog:read").await;
    let admin = create_principal_with_permissions(&pool, "governance:write").await;
    let tenant_uuid: Uuid = catalog_tenant.parse().unwrap();
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2), ($3, $2)")
        .bind(reader)
        .bind(tenant_uuid)
        .bind(admin)
        .execute(&pool)
        .await
        .expect("associate test users with catalog tenant");
    let resolve = "/api/governance/quality/incidents/00000000-0000-4000-8000-000000000000/resolve";

    let reader_cookie = session_cookie_for_user(&pool, reader).await;
    let listed = request_with_cookie(
        &router,
        "GET",
        "/api/governance/quality/incidents",
        &reader_cookie,
    )
    .await;
    // PR #106 review R3 SHOULD-FIX 15: strict positive assertion
    assert_eq!(
        listed.status(),
        StatusCode::OK,
        "any login may list incidents"
    );
    let refused = request_with_cookie(&router, "POST", resolve, &reader_cookie).await;
    assert_eq!(
        refused.status(),
        StatusCode::FORBIDDEN,
        "catalog:read alone must NOT be enough to resolve an incident"
    );

    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let allowed = request_with_cookie(&router, "POST", resolve, &admin_cookie).await;
    // PR #106 review R3 SHOULD-FIX 15: empty body answers 400
    assert_eq!(
        allowed.status(),
        StatusCode::BAD_REQUEST,
        "governance:write must be enough to reach resolve handler which answers 400 on empty body"
    );

    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/governance/quality/incidents")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

/// `DATA-14` (F3): any login may read the schedules, and setting one needs
/// `governance:write`. A principal without it is refused (403) and nothing is
/// stored; one with it is past the gate (an empty body then answers 400).
#[tokio::test]
async fn quality_schedule_routes_are_gated_in_both_directions() {
    // PR #106 review R2 SHOULD-FIX 1: catalog-tenant caller
    let mut env = std::collections::HashMap::new();
    let catalog_tenant = "11111111-1111-4111-8111-000000000001";
    env.insert("CATALOG_TENANT_ID".to_owned(), catalog_tenant.to_owned());
    let TestApp { router, pool } = common::spin_up_with_env(&env).await;

    let reader = create_principal_with_permissions(&pool, "catalog:read").await;
    let admin = create_principal_with_permissions(&pool, "governance:write").await;
    let tenant_uuid: Uuid = catalog_tenant.parse().unwrap();
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2), ($3, $2)")
        .bind(reader)
        .bind(tenant_uuid)
        .bind(admin)
        .execute(&pool)
        .await
        .expect("associate test users with catalog tenant");
    let set = "/api/governance/quality/schedules/silver.orders";

    let reader_cookie = session_cookie_for_user(&pool, reader).await;
    let listed = request_with_cookie(
        &router,
        "GET",
        "/api/governance/quality/schedules",
        &reader_cookie,
    )
    .await;
    // PR #106 review R3 SHOULD-FIX 15: strict positive assertion
    assert_eq!(
        listed.status(),
        StatusCode::OK,
        "any login may read schedules"
    );
    let refused = request_with_cookie(&router, "PUT", set, &reader_cookie).await;
    assert_eq!(
        refused.status(),
        StatusCode::FORBIDDEN,
        "catalog:read alone must NOT be enough to set a schedule"
    );

    let admin_cookie = session_cookie_for_user(&pool, admin).await;
    let allowed = request_with_cookie(&router, "PUT", set, &admin_cookie).await;
    // PR #106 review R3 SHOULD-FIX 15: empty body answers 400
    assert_eq!(
        allowed.status(),
        StatusCode::BAD_REQUEST,
        "governance:write must be enough to reach set handler which answers 400 on empty body"
    );

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM quality_schedule")
        .fetch_one(&pool)
        .await
        .expect("count schedules");
    assert_eq!(stored, 0, "no empty-bodied request may store a schedule");
}

/// `DATA-14` (F5): any login may read a table's learning state; an anonymous
/// caller is refused.
#[tokio::test]
async fn the_learning_state_is_readable_by_any_login_and_not_anonymously() {
    // PR #106 review R2 SHOULD-FIX 1: catalog-tenant caller
    let mut env = std::collections::HashMap::new();
    let catalog_tenant = "11111111-1111-4111-8111-000000000001";
    env.insert("CATALOG_TENANT_ID".to_owned(), catalog_tenant.to_owned());
    let TestApp { router, pool } = common::spin_up_with_env(&env).await;

    let reader = create_principal_with_permissions(&pool, "catalog:read").await;
    let tenant_uuid: Uuid = catalog_tenant.parse().unwrap();
    sqlx::query("INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2)")
        .bind(reader)
        .bind(tenant_uuid)
        .execute(&pool)
        .await
        .expect("associate test user with catalog tenant");
    let cookie = session_cookie_for_user(&pool, reader).await;
    let path = "/api/governance/quality/learning/silver.orders";
    let read = request_with_cookie(&router, "GET", path, &cookie).await;
    // PR #106 review R2 SHOULD-FIX 1: restore strict positive assertion
    assert_eq!(read.status(), StatusCode::OK);
    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

/// SEC-12: the two routes the embed work added are in `POLICY_TABLE` with the
/// policies the plan names, so the table-driven loops above walk them both
/// ways (a `dashboard:read`-only principal is refused on the withdrawal; the
/// frame answer is reachable with no credentials at all).
#[test]
fn the_sec12_embed_routes_are_registered_with_the_policies_the_plan_names() {
    let policy_of = |method: &str, path: &str| {
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == method && *p == path)
            .map(|(_, _, policy)| *policy)
    };
    assert_eq!(
        policy_of("POST", "/api/embed/frame"),
        Some(Policy::Public),
        "the frame answer is read by the console's proxy with only the embed token"
    );
    assert_eq!(
        policy_of("POST", "/api/dashboard/embed-revoke"),
        Some(Policy::RequiresPermission("dashboard:write")),
        "withdrawing a token is a write"
    );
}

/// BI-16 part A (T8): the whole-result table export needs `dashboard:read` and
/// nothing more, and not the Query Studio permission. Asserted both ways: a
/// principal holding only `dashboard:read` gets past the gate, one holding only
/// `query:read` is refused.
#[tokio::test]
async fn the_table_export_needs_dashboard_read_and_not_query_read() {
    let TestApp { router, pool } = spin_up().await;
    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "GET" && *p == "/api/dashboard/table-export")
            .map(|(_, _, policy)| *policy),
        Some(Policy::RequiresPermission("dashboard:read"))
    );
    let viewer = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = common::session_cookie_for_user(&pool, viewer).await;
    let resp = request_with_cookie(
        &router,
        "GET",
        "/api/dashboard/table-export?chart=x",
        &cookie,
    )
    .await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "a dashboard viewer must get past the gate, got {}",
        resp.status()
    );
    let analyst = common::create_principal_with_permissions(&pool, "query:read").await;
    let cookie = common::session_cookie_for_user(&pool, analyst).await;
    let resp = request_with_cookie(
        &router,
        "GET",
        "/api/dashboard/table-export?chart=x",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// BI-1 (D6): the chart download needs `dashboard:read` and nothing more,
/// and not the Query Studio permission. Both ways, as for the table export.
#[tokio::test]
async fn the_chart_export_needs_dashboard_read_and_not_query_read() {
    let TestApp { router, pool } = spin_up().await;
    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "GET" && *p == "/api/dashboard/chart-export")
            .map(|(_, _, policy)| *policy),
        Some(Policy::RequiresPermission("dashboard:read"))
    );
    let uri = "/api/dashboard/chart-export?chart=x&format=xlsx";
    let viewer = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = common::session_cookie_for_user(&pool, viewer).await;
    let resp = request_with_cookie(&router, "GET", uri, &cookie).await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "a dashboard viewer must get past the gate, got {}",
        resp.status()
    );
    let analyst = common::create_principal_with_permissions(&pool, "query:read").await;
    let cookie = common::session_cookie_for_user(&pool, analyst).await;
    let resp = request_with_cookie(&router, "GET", uri, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let resp = request_with_cookie(&router, "GET", uri, "").await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// BLD-2 (T4): the target-lag route needs `pipeline:write` and nothing less.
/// Asserted both ways: a principal holding `pipeline:write` gets past the gate,
/// one holding only `pipeline:read` is refused with 403.
#[tokio::test]
async fn the_target_lag_route_needs_pipeline_write_and_not_pipeline_read() {
    let TestApp { router, pool } = spin_up().await;
    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "PUT" && *p == "/api/pipelines/{id}/target-lag")
            .map(|(_, _, policy)| *policy),
        Some(Policy::RequiresPermission("pipeline:write"))
    );
    let writer = common::create_principal_with_permissions(&pool, "pipeline:write").await;
    let cookie = common::session_cookie_for_user(&pool, writer).await;
    let resp = request_with_cookie(
        &router,
        "PUT",
        "/api/pipelines/00000000-0000-0000-0000-000000000001/target-lag",
        &cookie,
    )
    .await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "a pipeline writer must get past the gate, got {}",
        resp.status()
    );
    let reader = common::create_principal_with_permissions(&pool, "pipeline:read").await;
    let cookie = common::session_cookie_for_user(&pool, reader).await;
    let resp = request_with_cookie(
        &router,
        "PUT",
        "/api/pipelines/00000000-0000-0000-0000-000000000001/target-lag",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// BI-7: the join suggestions need `dashboard:read` and nothing more. Asserted
/// both ways: a principal holding only `dashboard:read` gets past the gate, one
/// holding only `query:read` is refused.
#[tokio::test]
async fn the_join_suggestions_need_dashboard_read_and_not_query_read() {
    let TestApp { router, pool } = spin_up().await;
    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "GET" && *p == "/api/dashboard/join-suggestions")
            .map(|(_, _, policy)| *policy),
        Some(Policy::RequiresPermission("dashboard:read"))
    );
    let uri = "/api/dashboard/join-suggestions?left=a&right=b";
    let viewer = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = common::session_cookie_for_user(&pool, viewer).await;
    let resp = request_with_cookie(&router, "GET", uri, &cookie).await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "a dashboard viewer must get past the gate, got {}",
        resp.status()
    );
    let analyst = common::create_principal_with_permissions(&pool, "query:read").await;
    let cookie = common::session_cookie_for_user(&pool, analyst).await;
    let resp = request_with_cookie(&router, "GET", uri, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

/// BI-10: hierarchies are read with `dashboard:read` and defined, changed and
/// deleted with `dashboard:write`. Asserted both ways for every method: a
/// reader gets past the gate on `GET` and is refused on the writes, a writer
/// gets past on all four, and a principal with neither is refused on all four.
#[tokio::test]
async fn hierarchies_are_read_with_dashboard_read_and_defined_with_dashboard_write() {
    let TestApp { router, pool } = spin_up().await;
    let uri = "/api/dashboard/hierarchies?mart=m&id=x";
    let reader = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let reader = common::session_cookie_for_user(&pool, reader).await;
    let writer = common::create_principal_with_permissions(&pool, "dashboard:write").await;
    let writer = common::session_cookie_for_user(&pool, writer).await;
    let neither = common::create_principal_with_permissions(&pool, "query:read").await;
    let neither = common::session_cookie_for_user(&pool, neither).await;
    for (method, needs) in [
        ("GET", "dashboard:read"),
        ("POST", "dashboard:write"),
        ("PUT", "dashboard:write"),
        ("DELETE", "dashboard:write"),
    ] {
        assert_eq!(
            POLICY_TABLE
                .iter()
                .find(|(m, p, _)| *m == method && *p == "/api/dashboard/hierarchies")
                .map(|(_, _, policy)| *policy),
            Some(Policy::RequiresPermission(needs)),
            "{method}"
        );
        let allowed = if needs == "dashboard:read" {
            &reader
        } else {
            &writer
        };
        let refused = if needs == "dashboard:read" {
            &neither
        } else {
            &reader
        };
        let resp = request_with_cookie(&router, method, uri, allowed).await;
        assert!(
            resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
            "{method}: the right principal must get past the gate, got {}",
            resp.status()
        );
        let resp = request_with_cookie(&router, method, uri, refused).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method}");
        let resp = request_with_cookie(&router, method, uri, &neither).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} (neither)");
    }
}

/// BI-18 part C, part 2: history, the trash, the personal space, favourites,
/// recent items and search. Going back, restoring and creating the personal
/// space edit dashboards and need `dashboard:write` (restoring needs the
/// permission that deletes); everything else is read with `dashboard:read`.
/// Asserted both ways for every route: the right principal gets past the gate,
/// a principal holding only the other permission is refused, and so is one
/// holding neither.
#[tokio::test]
async fn history_trash_personal_space_favourites_and_search_have_the_right_permissions() {
    let TestApp { router, pool } = spin_up().await;
    let reader = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let reader = common::session_cookie_for_user(&pool, reader).await;
    let writer = common::create_principal_with_permissions(&pool, "dashboard:write").await;
    let writer = common::session_cookie_for_user(&pool, writer).await;
    let neither = common::create_principal_with_permissions(&pool, "query:read").await;
    let neither = common::session_cookie_for_user(&pool, neither).await;
    for (method, path, needs) in [
        ("GET", "/api/dashboard/versions", "dashboard:read"),
        ("POST", "/api/dashboard/versions/revert", "dashboard:write"),
        ("GET", "/api/dashboard/trash", "dashboard:read"),
        ("POST", "/api/dashboard/trash/restore", "dashboard:write"),
        ("GET", "/api/dashboard/shortcuts", "dashboard:read"),
        ("PUT", "/api/dashboard/favourites", "dashboard:read"),
        ("POST", "/api/dashboard/recent", "dashboard:read"),
        ("GET", "/api/dashboard/search", "dashboard:read"),
        ("POST", "/api/dashboard/personal", "dashboard:write"),
    ] {
        assert_eq!(
            POLICY_TABLE
                .iter()
                .find(|(m, p, _)| *m == method && *p == path)
                .map(|(_, _, policy)| *policy),
            Some(Policy::RequiresPermission(needs)),
            "{method} {path}"
        );
        let (allowed, other) = if needs == "dashboard:read" {
            (&reader, &writer)
        } else {
            (&writer, &reader)
        };
        let resp = request_with_cookie(&router, method, path, allowed).await;
        assert!(
            resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
            "{method} {path}: the right principal must get past the gate, got {}",
            resp.status()
        );
        // `dashboard:write` and `dashboard:read` are separate grants: neither
        // implies the other, so the other one is refused too.
        let resp = request_with_cookie(&router, method, path, other).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {path} (other)"
        );
        let resp = request_with_cookie(&router, method, path, &neither).await;
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {path} (neither)"
        );
    }
}

/// BI-11: parameters are read with `dashboard:read` and defined, changed and
/// deleted with `dashboard:write`. Asserted both ways for every method: a
/// reader gets past the gate on `GET` and is refused on the writes, a writer
/// gets past on all four, and a principal with neither is refused on all four.
#[tokio::test]
async fn parameters_are_read_with_dashboard_read_and_defined_with_dashboard_write() {
    let TestApp { router, pool } = spin_up().await;
    let uri = "/api/dashboard/params?id=x";
    let reader = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let reader = common::session_cookie_for_user(&pool, reader).await;
    let writer = common::create_principal_with_permissions(&pool, "dashboard:write").await;
    let writer = common::session_cookie_for_user(&pool, writer).await;
    let neither = common::create_principal_with_permissions(&pool, "query:read").await;
    let neither = common::session_cookie_for_user(&pool, neither).await;
    for (method, needs) in [
        ("GET", "dashboard:read"),
        ("POST", "dashboard:write"),
        ("PUT", "dashboard:write"),
        ("DELETE", "dashboard:write"),
    ] {
        assert_eq!(
            POLICY_TABLE
                .iter()
                .find(|(m, p, _)| *m == method && *p == "/api/dashboard/params")
                .map(|(_, _, policy)| *policy),
            Some(Policy::RequiresPermission(needs)),
            "{method}"
        );
        let allowed = if needs == "dashboard:read" {
            &reader
        } else {
            &writer
        };
        let refused = if needs == "dashboard:read" {
            &neither
        } else {
            &reader
        };
        let resp = request_with_cookie(&router, method, uri, allowed).await;
        assert!(
            resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
            "{method}: the right principal must get past the gate, got {}",
            resp.status()
        );
        let resp = request_with_cookie(&router, method, uri, refused).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method}");
        let resp = request_with_cookie(&router, method, uri, &neither).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} (neither)");
    }
}

/// BI-1 (D3): the print view's data route is `Policy::Public` (the renderer has
/// no session) and the one such route that reads as a person, so it is pinned
/// by name; the table-driven loops above walk it with no credentials, and the
/// route's own tests (`render_data.rs`) show a session cookie changes nothing.
#[test]
fn the_render_data_route_is_public_and_token_authenticated() {
    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "POST" && *p == "/api/render/data")
            .map(|(_, _, policy)| *policy),
        Some(Policy::Public)
    );
}

/// BI-1 review fix (SHOULD-FIX) R8: the PDF route needs `dashboard:read` and
/// nothing more, and not the Query Studio permission; both ways, as for the
/// chart download.
#[tokio::test]
async fn the_pdf_export_needs_dashboard_read_and_not_query_read() {
    let TestApp { router, pool } = spin_up().await;
    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "GET" && *p == "/api/dashboard/export/pdf")
            .map(|(_, _, policy)| *policy),
        Some(Policy::RequiresPermission("dashboard:read"))
    );
    let uri = "/api/dashboard/export/pdf?board=x";
    let viewer = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = common::session_cookie_for_user(&pool, viewer).await;
    let resp = request_with_cookie(&router, "GET", uri, &cookie).await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "a dashboard viewer must get past the gate, got {}",
        resp.status()
    );
    let analyst = common::create_principal_with_permissions(&pool, "query:read").await;
    let cookie = common::session_cookie_for_user(&pool, analyst).await;
    let resp = request_with_cookie(&router, "GET", uri, &cookie).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let resp = request_with_cookie(&router, "GET", uri, "").await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// `RPT-1` (D7): every scheduled-report route needs `dashboard:read` and nothing
/// less or other, asserted both ways: no session is 401, a principal without it
/// is 403, a dashboard viewer gets past the gate (the handler's own owner and
/// download rules are in `report_routes.rs`).
#[tokio::test]
async fn the_report_routes_need_dashboard_read_both_ways() {
    let TestApp { router, pool } = spin_up().await;
    let routes: Vec<(&str, &str)> = POLICY_TABLE
        .iter()
        .filter(|(_, p, _)| p.starts_with("/api/reports/"))
        .map(|(m, p, policy)| {
            assert_eq!(
                *policy,
                Policy::RequiresPermission("dashboard:read"),
                "{m} {p}"
            );
            (*m, *p)
        })
        .collect();
    assert_eq!(
        routes.len(),
        8,
        "the seven subscription routes and the picker"
    );
    let viewer = common::create_principal_with_permissions(&pool, "dashboard:read").await;
    let viewer_cookie = common::session_cookie_for_user(&pool, viewer).await;
    let analyst = common::create_principal_with_permissions(&pool, "query:read").await;
    let analyst_cookie = common::session_cookie_for_user(&pool, analyst).await;
    for (method, path) in routes {
        let uri = path.replace("{id}", "00000000-0000-0000-0000-000000000000");
        let none = request_with_cookie(&router, method, &uri, "").await;
        assert_eq!(none.status(), StatusCode::UNAUTHORIZED, "{method} {uri}");
        let refused = request_with_cookie(&router, method, &uri, &analyst_cookie).await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        let through = request_with_cookie(&router, method, &uri, &viewer_cookie).await;
        assert!(
            through.status() != StatusCode::UNAUTHORIZED
                && through.status() != StatusCode::FORBIDDEN,
            "{method} {uri} got {}",
            through.status()
        );
    }
}

/// BLD-2 review R1 BLOCKER 7: lag-check route authorization asserted both ways.
#[tokio::test]
async fn lag_check_route_authorization_asserted_both_ways() {
    let TestApp { router, pool } = spin_up().await;
    let uri = "/api/pipelines/runnable/pl-test/lag-check";

    assert_eq!(
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == "POST" && *p == "/api/pipelines/runnable/{id}/lag-check")
            .map(|(_, _, policy)| *policy),
        Some(Policy::RequiresPermission("pipeline:read")),
    );

    // Platform admin has pipeline:read and passes the service_or_admin handler check
    let allowed = session_cookie_for_seeded_user(&pool, "fajar@meridian.example").await;
    let refused_user = common::create_principal_with_permissions(&pool, "query:read").await;
    let refused = common::session_cookie_for_user(&pool, refused_user).await;

    // A user with wrong permission gets 403
    let resp = request_with_cookie(&router, "POST", uri, &refused).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // No credentials gets 401
    let resp = request_with_cookie(&router, "POST", uri, "").await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Allowed principal gets past auth gate (not 401/403)
    let resp = request_with_cookie(&router, "POST", uri, &allowed).await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "allowed principal must get past auth gate, got {}",
        resp.status()
    );
}

/// `BI-19` part A (D4, D5): the three link-management routes are in
/// `POLICY_TABLE` as `dashboard:write` and the unlock route as public, and the
/// gate agrees both ways: no session is 401, `dashboard:read` alone is 403,
/// `dashboard:write` gets past, and a visitor with no session reaches the
/// public routes (the handler then says 404, never 401 or 403).
#[tokio::test]
async fn the_public_link_routes_are_gated_in_both_directions() {
    let TestApp { router, pool } = spin_up().await;
    let policy_of = |method: &str, path: &str| {
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == method && *p == path)
            .map(|(_, _, policy)| *policy)
    };
    let managed = [
        ("GET", "/api/dashboard/public-link?board=b_x"),
        ("PUT", "/api/dashboard/public-link"),
        ("DELETE", "/api/dashboard/public-link?board=b_x"),
    ];
    for (method, uri) in managed {
        let pattern = uri.split('?').next().unwrap();
        assert_eq!(
            policy_of(method, pattern),
            Some(Policy::RequiresPermission("dashboard:write")),
            "{method} {pattern}"
        );
        let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
        let cookie = session_cookie_for_user(&pool, reader).await;
        let resp = request_with_cookie(&router, method, uri, &cookie).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} read-only");
        let writer = create_principal_with_permissions(&pool, "dashboard:write").await;
        let cookie = session_cookie_for_user(&pool, writer).await;
        let resp = request_with_cookie(&router, method, uri, &cookie).await;
        assert!(
            resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
            "{method} writer got {}",
            resp.status()
        );
        let resp = request_with_cookie(&router, method, uri, "").await;
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{method} anonymous"
        );
    }
    assert_eq!(
        policy_of("POST", "/api/public/dashboard/{token}/unlock"),
        Some(Policy::Public)
    );
    for (method, uri) in [
        ("POST", "/api/public/dashboard/p_nothing/unlock"),
        ("GET", "/api/public/dashboard/p_nothing"),
    ] {
        let resp = request_with_cookie(&router, method, uri, "").await;
        assert!(
            resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
            "{method} {uri} anonymous got {}",
            resp.status()
        );
    }
}

/// `BI-26` (D5, D8): the values route is public (the signed token is the
/// credential: with no token it answers 400, never 401 or 403, and nothing is
/// read), and the settings route is `dashboard:write`: no session is 401,
/// `dashboard:read` alone is 403, `dashboard:write` gets past the gate.
#[tokio::test]
async fn the_embed_values_and_settings_routes_are_gated_in_both_directions() {
    let TestApp { router, pool } = spin_up().await;
    let policy_of = |method: &str, path: &str| {
        POLICY_TABLE
            .iter()
            .find(|(m, p, _)| *m == method && *p == path)
            .map(|(_, _, policy)| *policy)
    };
    assert_eq!(policy_of("POST", "/api/embed/values"), Some(Policy::Public));
    let resp =
        request_with_cookie_and_body(&router, "POST", "/api/embed/values", "", b"{}".to_vec())
            .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    let path = "/api/dashboard/embed-settings";
    assert_eq!(
        policy_of("PUT", path),
        Some(Policy::RequiresPermission("dashboard:write"))
    );
    let body = br#"{"board":"b_x","filterModes":{}}"#.to_vec();
    let resp = request_with_cookie_and_body(&router, "PUT", path, "", body.clone()).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let resp = request_with_cookie_and_body(&router, "PUT", path, &cookie, body.clone()).await;
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let writer = create_principal_with_permissions(&pool, "dashboard:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let resp = request_with_cookie_and_body(&router, "PUT", path, &cookie, body).await;
    assert!(
        resp.status() != StatusCode::UNAUTHORIZED && resp.status() != StatusCode::FORBIDDEN,
        "a writer got {}",
        resp.status()
    );
}

// ── Query limits (BI-5) ─────────────────────────────────────────────────

/// `GET /api/settings/limits` is open to any login and `PUT` needs
/// `settings:write`, asserted both ways: a principal without it is refused
/// the write (403) and nothing is stored, one holding it alone is past the
/// gate (an empty body then answers 400, not 401/403), and a principal with
/// no permission at all may still read.
#[tokio::test]
async fn the_query_limits_routes_are_gated_in_both_directions() {
    let TestApp { router, pool } = common::spin_up().await;
    let plain = create_principal_with_permissions(&pool, "dashboard:read").await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let plain_cookie = session_cookie_for_user(&pool, plain).await;
    let admin_cookie = session_cookie_for_user(&pool, admin).await;

    let read = request_with_cookie(&router, "GET", "/api/settings/limits", &plain_cookie).await;
    assert_eq!(read.status(), StatusCode::OK, "any login may read");
    let refused = request_with_cookie(&router, "PUT", "/api/settings/limits", &plain_cookie).await;
    assert_eq!(refused.status(), StatusCode::FORBIDDEN);
    let reached = request_with_cookie(&router, "PUT", "/api/settings/limits", &admin_cookie).await;
    assert_eq!(
        reached.status(),
        StatusCode::BAD_REQUEST,
        "settings:write reaches the handler, which refuses the empty body"
    );
    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/settings/limits")
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router answers");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
}

/// `POST /api/query/cancel` (Stop) is open to any login (it can only end the
/// caller's own run) and refuses an anonymous caller; a body that is not a
/// run id UUID never reaches the engine.
#[tokio::test]
async fn the_query_cancel_route_needs_a_login_and_a_uuid_run_id() {
    let TestApp { router, pool } = common::spin_up().await;
    let plain = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, plain).await;

    let anonymous = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/query/cancel")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"runId":"22222222-2222-4222-8222-222222222222"}"#,
                ))
                .expect("build request"),
        )
        .await
        .expect("router answers");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let bad = request_with_cookie_and_body(
        &router,
        "POST",
        "/api/query/cancel",
        &cookie,
        br#"{"runId":"%"}"#.to_vec(),
    )
    .await;
    assert_eq!(
        bad.status(),
        StatusCode::BAD_REQUEST,
        "a login is enough to reach the handler"
    );
}

/// `PR #246 review BLOCKER 1`: the public link's and the embed's map
/// boundary routes are `Policy::Public`, because the token is the credential
/// there, exactly as it is for the board's data (`GET
/// /api/public/dashboard/{token}` and `POST /api/embed/data`).
///
/// The two table-driven loops above walk both entries by construction (a
/// `Policy::Public` entry is not refused by the gate, and the Platform Admin
/// is never refused on it); this names them, so a change of their
/// classification is caught here and not only in the table.
///
/// With no credentials at all the gate must let the request through to the
/// handler, which answers 404 (no such board) or 400 (no jwt) — never a 401,
/// which the console would turn into a redirect to the login page for a
/// viewer who has no account.
#[tokio::test]
async fn the_public_map_boundary_routes_are_not_refused_by_the_auth_gate() {
    let TestApp { router, .. } = spin_up().await;
    for (method, path) in [
        (
            "GET",
            "/api/public/dashboard/tok/maps/m_0123456789ab/geojson",
        ),
        ("POST", "/api/embed/maps/m_0123456789ab/geojson"),
    ] {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("router never fails a request outright");
        assert_ne!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "{method} {path}: a public route must not answer 401 with no credentials"
        );
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "{method} {path}: a public route must not answer 403 with no credentials"
        );
    }
}
