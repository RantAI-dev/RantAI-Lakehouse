//! `PUT /api/connectors/{id}/secret` — probe-first credential-reference
//! rotation (`routes::connectors::rotate_secret`).
//!
//! # Why every test here ends in a refusal, never a landed swap
//!
//! `rotate_secret` refuses `newSecretRef` with
//! `reject_allowlisted_secret_ref` — the SAME check `create` applies to a
//! caller-supplied `secretRef` — before ever probing. That check refuses
//! exactly the ref SHAPES [`crate::state::CONNECTOR_ALLOWED_SECRET_REF_PATTERNS`]
//! names; [`crate::state::AppState::connector_secret_resolver`]
//! (`AllowlistedSecretResolver`) resolves ONLY those same shapes. The two
//! checks are deliberately the mirror image of each other (see
//! `reject_allowlisted_secret_ref`'s doc comment) — which means a
//! `newSecretRef` that passes the refusal check can never resolve, and a
//! `newSecretRef` that would resolve never passes the refusal check. So a
//! real, HTTP-level "rotation lands" test is not possible against the
//! public API surface with an honest `newSecretRef` — this is the SAME
//! property `POST .../test` already has for every connector this build's
//! `create` route can itself produce (`0023_connector_dedicated_secret_refs.sql`'s
//! header: "the only connectors that can dial with [an allowlisted ref]
//! are the ones seeded by migration"), not a new gap this route
//! introduces. `swap_secret_ref`'s own success path is proven at the
//! store layer instead (`lakehouse-store/tests/connectors.rs`) — see the
//! task's report for the full reasoning.
//!
//! Keeping `reject_allowlisted_secret_ref` here (rather than skipping it,
//! which the brief's literal wording for this route did not call out) is
//! deliberate: without it, a `connector:manage` principal could create an
//! ordinary connector pointed at a host they control (passing `create`'s
//! own check with an innocuous ref), then use THIS route to rotate that
//! connector's ref to a reserved, deployment-owned pattern -- reopening
//! the exact exfiltration path `0023_connector_dedicated_secret_refs.sql`
//! closed for `create`, just via a second write path instead of the
//! first.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use common::{session_cookie_for_seeded_user, spin_up};

async fn put(
    app: &axum::Router,
    path: &str,
    cookie: &str,
    body: Value,
) -> axum::http::Response<Body> {
    app.clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

async fn secret_ref_columns(pool: &PgPool, id: &str) -> (String, Option<String>) {
    sqlx::query_as("SELECT secret_ref, secret_ref_secondary FROM connector WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("connector row must exist")
}

/// The candidate credential reference does not resolve
/// (`AllowlistedSecretResolver` returns `NotAllowed` -- `newSecretRef`
/// deliberately names no reserved pattern, see the module doc comment) --
/// `supported: true` (this build CAN dial a `sql`-adapter Postgres
/// connector), but the probe cannot even attempt a dial, so it must
/// refuse the rotation with 422, and the stored `secret_ref` must be
/// left exactly as seeded.
#[tokio::test]
async fn a_probe_that_cannot_resolve_the_candidate_ref_refuses_the_rotation_and_leaves_the_ref_unchanged()
 {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let before = secret_ref_columns(&app.pool, "conn-pg-lakehouse").await;

    let response = put(
        &app.router,
        "/api/connectors/conn-pg-lakehouse/secret",
        &cookie,
        json!({ "slot": "primary", "newSecretRef": "env:CUSTOM_ROTATION_TEST_REF" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.to_lowercase().contains("not") || message.to_lowercase().contains("allow"),
        "expected a resolver-refusal-shaped message, got {message:?}"
    );

    let after = secret_ref_columns(&app.pool, "conn-pg-lakehouse").await;
    assert_eq!(
        after, before,
        "a refused rotation must never touch the stored ref"
    );
}

/// A connector whose `type` this build has no dial implementation for at
/// all (`Outcome::unsupported`, `connector_probe::probe_by_kind`) must
/// refuse the rotation with 422 — a rotation for a type that can never be
/// verified is never applied — and leave the stored ref untouched.
#[tokio::test]
async fn an_unsupported_connector_type_refuses_the_rotation_and_leaves_the_ref_unchanged() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    sqlx::query(
        "INSERT INTO connector (id, name, type, direction, host, secret_ref, environment, tenant) \
         VALUES ('conn-kafka-unsupported-test', 'kafka test', 'Kafka', 'source', 'unused', \
         'env:CONNECTOR_KAFKA_TOKEN', 'production', 'meridian')",
    )
    .execute(&app.pool)
    .await
    .expect("seed a kind=Kafka, adapter=NULL connector (no dial implementation)");

    let response = put(
        &app.router,
        "/api/connectors/conn-kafka-unsupported-test/secret",
        &cookie,
        json!({ "slot": "primary", "newSecretRef": "env:CUSTOM_ROTATION_TEST_REF" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("cannot be probed by this build"),
        "expected the unsupported-type refusal text, got {message:?}"
    );

    let after = secret_ref_columns(&app.pool, "conn-kafka-unsupported-test").await;
    assert_eq!(after.0, "env:CONNECTOR_KAFKA_TOKEN");
}

/// An unknown connector id is a 404 — before any probe is attempted.
#[tokio::test]
async fn rotating_an_unknown_connector_is_a_404() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = put(
        &app.router,
        "/api/connectors/conn-does-not-exist/secret",
        &cookie,
        json!({ "slot": "primary", "newSecretRef": "env:CUSTOM_ROTATION_TEST_REF" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

/// A `slot` outside `"primary" | "secondary"` fails to deserialize --
/// this must be a 400, not a panic or a silent default.
#[tokio::test]
async fn an_unrecognized_slot_is_a_400() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = put(
        &app.router,
        "/api/connectors/conn-pg-lakehouse/secret",
        &cookie,
        json!({ "slot": "tertiary", "newSecretRef": "env:CUSTOM_ROTATION_TEST_REF" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// `newSecretRef` shaped like a raw credential value (not a reference
/// name) must be refused with 400 — the same
/// `connectors::looks_like_raw_secret` shape check `create` applies to a
/// caller-supplied `secretRef`, reused here rather than re-implemented.
#[tokio::test]
async fn a_raw_credential_shaped_new_secret_ref_is_a_400() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = put(
        &app.router,
        "/api/connectors/conn-pg-lakehouse/secret",
        &cookie,
        json!({
            "slot": "primary",
            "newSecretRef": "postgres://admin:hunter2@db.internal:5432/oms",
        }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// A `newSecretRef` naming one of the deployment's own reserved
/// connector-credential patterns is refused with 400 before any probe --
/// the regression guard for the exfiltration path the module doc comment
/// describes (rotating an attacker-hosted connector onto a real
/// deployment credential).
#[tokio::test]
async fn a_new_secret_ref_naming_a_reserved_connector_credential_pattern_is_refused() {
    let app = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;

    let response = put(
        &app.router,
        "/api/connectors/conn-pg-lakehouse/secret",
        &cookie,
        json!({ "slot": "primary", "newSecretRef": "env:CONNECTOR_S3_SECRET_KEY" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let message = body["error"].as_str().unwrap_or_default();
    assert!(
        message.contains("reserved"),
        "expected reject_allowlisted_secret_ref's own wording, got {message:?}"
    );

    let after = secret_ref_columns(&app.pool, "conn-pg-lakehouse").await;
    assert_eq!(
        after.0, "env:CONNECTOR_PG_PASSWORD",
        "refused before any write"
    );
}
