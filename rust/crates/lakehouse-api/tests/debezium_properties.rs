//! WS0: the route must never return a resolved secret value — only
//! `${ENV_VAR_NAME}` references the deployment's own shell expands at
//! container start, exactly like `docker-compose.yml`'s `debezium-server`
//! heredoc already does.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

use common::{TestApp, session_cookie_for_seeded_user, spin_up, spin_up_with_env};

async fn get_with_cookie(
    router: &axum::Router,
    path: &str,
    cookie: &str,
) -> axum::http::Response<Body> {
    router
        .clone()
        .oneshot(
            Request::builder()
                .uri(path)
                .header("cookie", cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
}

/// The live-pool half of the no-secret-leak property: a real HTTP round
/// trip through the router, authenticated as a real `connector:manage`
/// principal, against the real seeded `conn-pg-lakehouse` connector,
/// returns a `200` whose `properties` field names the connector's own
/// `secretRef` (`env:CONNECTOR_PG_PASSWORD`, per migration `0023`) as an
/// `${...}` reference. This does NOT mutate process environment: the
/// type-level guarantee that no secret VALUE can appear is already proven
/// by the pure `validate_env_var_ref_rejects_every_dangerous_character` and
/// `template_renders_env_var_references_never_a_resolved_value` unit
/// tests in `lakehouse-store::cdc` — `render_debezium_properties_template`
/// takes `&str` reference NAMES and has no parameter through which a
/// resolved secret could enter its output at all, so there is nothing
/// left for an env-mutating integration test to prove that the type
/// signature does not already guarantee. This test instead proves the
/// HTTP-level wiring: the right connector's own `secretRef` name flows
/// through to the right `${...}` reference in a real response body.
#[tokio::test]
async fn response_names_the_connectors_own_secret_ref_as_a_reference() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = get_with_cookie(
        &router,
        "/api/connectors/conn-pg-lakehouse/debezium-properties?table=public.orders",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    let properties = body["properties"].as_str().expect("properties field");

    // conn-pg-lakehouse's secretRef is env:CONNECTOR_PG_PASSWORD, not
    // env:POSTGRES_PASSWORD — migration 0023 moved it off the API's own
    // secret name. This assertion fails if that ever silently reverts,
    // or if the handler ever hardcodes a literal name instead of
    // deriving it from the connector's own dial_info.secret_ref.
    assert!(
        properties.contains("debezium.source.database.password=${CONNECTOR_PG_PASSWORD}"),
        "must reference this connector's own secretRef by name: {properties}"
    );
    assert_eq!(body["table"].as_str(), Some("public.orders"));
    assert!(
        body["note"].as_str().is_some(),
        "note field must be present"
    );
}

#[tokio::test]
async fn non_postgres_connector_is_rejected_with_400() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = get_with_cookie(
        &router,
        "/api/connectors/conn-s3-warehouse/debezium-properties?table=x",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn missing_table_query_param_is_rejected_with_400() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = get_with_cookie(
        &router,
        "/api/connectors/conn-pg-lakehouse/debezium-properties",
        &cookie,
    )
    .await;
    // Missing required query param: axum's own `Query` extractor
    // rejection (400 Bad Request) fires before this handler's body
    // runs at all.
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// An unknown connector id is a 404, not a 400 or a leaked internal error.
#[tokio::test]
async fn unknown_connector_id_is_a_404() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let cookie = session_cookie_for_seeded_user(&pool, "bayu@meridian.example").await;

    let resp = get_with_cookie(
        &router,
        "/api/connectors/conn-does-not-exist/debezium-properties?table=public.orders",
        &cookie,
    )
    .await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
