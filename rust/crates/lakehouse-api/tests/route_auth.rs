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
        StatusCode::FORBIDDEN,
        "a seeded Analyst holding catalog:read must not be denied"
    );
    assert_ne!(
        resp.status(),
        StatusCode::UNAUTHORIZED,
        "a valid session must never be treated as unauthenticated"
    );
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
/// `PUT /api/catalog/{id}/annotation` (WS2 §13, WS2 plan review W8) — the
/// matching allowed-with-the-permission half of the test above.
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
