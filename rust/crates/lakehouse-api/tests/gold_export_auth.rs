//! HTTP-level acceptance tests for gold-publish-per-mart plan T1: the
//! scheduled Gold export (`dagster/dispar_orchestrate/gold_export.py`)
//! can authenticate against `POST /api/gold/export/{mart}`.
//!
//! Before T1, no `bootstrap_gold_export_service` existed: the schedule
//! sent only `x-run-token`, `auth_gate`'s `Policy::RequiresAuth` floor ran
//! first, found no real principal behind the request, and the nightly run
//! got `401` before `routes::gold::check_export_token` was ever reached.
//! These tests prove the fixed chain end to end over the real router: the
//! bearer credential minted from `GOLD_EXPORT_RUN_TOKEN` clears the
//! floor, and the same value in `x-run-token` clears
//! `check_export_token` — with nothing else configured.
//!
//! The seeding uses the SAME production functions
//! `main::bootstrap_gold_export_service` calls
//! (`lakehouse_store::identity::create_service_identity` +
//! `lakehouse_auth::service_token::ensure_service_credential`) because
//! that function is private to the `bin` crate and an integration test
//! cannot call it — the established constraint and precedent is
//! `security_regressions.rs`'s `seed_agent_run_service_credential`. The
//! identity's fixed name and empty-scope shape are pinned by `main.rs`'s
//! own unit tests, which DO call the real function.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use lakehouse_auth::Secret;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, spin_up_with_env};

/// Seeds the `gold-export-scheduler` identity + credential exactly the way
/// `main::bootstrap_gold_export_service` does — same production
/// functions, same fixed name, EMPTY scopes (see that function's "Why
/// this identity gets NO scopes" doc section for why) — because that
/// function is private to the `bin` crate and this HTTP-level harness
/// cannot call it directly.
async fn seed_gold_export_service_credential(pool: &sqlx::PgPool, token: &str) {
    let identity = lakehouse_store::identity::create_service_identity(
        pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "gold-export-scheduler".to_owned(),
            scopes: Vec::new(),
            environment: "test".to_owned(),
        },
    )
    .await
    .expect("seed the gold-export service identity fixture");
    let service_identity_id: Uuid = identity.id.parse().expect("a UUID identity id");
    lakehouse_auth::service_token::ensure_service_credential(
        pool,
        service_identity_id,
        &Secret::new(token.to_owned()),
    )
    .await
    .expect("seed the gold-export service credential fixture");
}

/// The exact header pair `gold_export.py::_headers` sends once T1's
/// Python half lands: the shared token as the bearer credential AND as
/// the `x-run-token` fallback — nothing else.
async fn post_export_with_scheduler_headers(
    app: &axum::Router,
    token: &str,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/gold/export/gold_export_smoke")
                .header("authorization", format!("Bearer {token}"))
                .header("x-run-token", token)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// T1 acceptance: with `GOLD_EXPORT_RUN_TOKEN` set, a request carrying
/// ONLY those two headers gets past `auth_gate` (no more nightly `401`)
/// AND past `check_export_token` (no `403`, and none of its "not
/// configured" text — with the token configured, a failed token check is
/// a `401`, so "not 401" plus "not 403" plus that body check is exactly
/// "both gates passed"). The request then proceeds into the export body
/// and fails against the dead `ClickHouse`/Lakekeeper upstreams the
/// shared harness points at; whatever classified 4xx/5xx that produces
/// is irrelevant here — only the auth outcome is under test.
#[tokio::test]
async fn scheduler_headers_clear_auth_gate_and_check_export_token_once_the_identity_is_seeded() {
    const TOKEN: &str = "gold-export-scheduler-fixture-token";
    let mut overrides = HashMap::new();
    overrides.insert("GOLD_EXPORT_RUN_TOKEN".to_owned(), TOKEN.to_owned());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;

    // Negative control — the pre-T1 bug: the same two headers with NO
    // seeded identity get `401` at `auth_gate`, because the opaque bearer
    // matches no credential. This is exactly what the nightly schedule
    // hit before the bootstrap existed.
    let unseeded = post_export_with_scheduler_headers(&router, TOKEN).await;
    assert_eq!(
        unseeded.status(),
        StatusCode::UNAUTHORIZED,
        "without a seeded identity the request must still be refused"
    );

    seed_gold_export_service_credential(&pool, TOKEN).await;

    let resp = post_export_with_scheduler_headers(&router, TOKEN).await;
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let text = String::from_utf8_lossy(&body);
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "auth_gate floor not cleared by the seeded credential: {text}"
    );
    assert_ne!(
        status,
        StatusCode::FORBIDDEN,
        "check_export_token denied the authenticated principal: {text}"
    );
    assert!(
        !text.contains("gold export is not configured"),
        "check_export_token treated the export as unconfigured despite the \
         configured token: {text}"
    );
}
