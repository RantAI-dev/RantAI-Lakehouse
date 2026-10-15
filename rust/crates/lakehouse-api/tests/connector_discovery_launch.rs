//! `SRC-13` task 3: the types only the orchestrator reads. The API launches
//! `discover_job` (a `wiremock` Dagster), answers `running`, and stores the
//! answer the orchestrator posts back on `discovery-results`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use common::{TestApp, session_cookie_for_seeded_user, spin_up_with_env};

const MERIDIAN_GROUP: &str = "11111111-1111-4111-8111-000000000001";

async fn app_with_dagster(server: &MockServer) -> (TestApp, String) {
    let mut env = HashMap::new();
    env.insert(
        "DAGSTER_URL".to_owned(),
        format!("{}/graphql", server.uri()),
    );
    let app = spin_up_with_env(&env).await;
    let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
    (app, cookie)
}

async fn seed_mongo(pool: &sqlx::PgPool, id: &str) {
    sqlx::query(
        "INSERT INTO connector (id, tenant_id, name, type, direction, host, secret_ref, \
         environment, tenant, adapter, ingest_mode, dial) VALUES ($1, $2, 'mongo discovery', \
         'MongoDB', 'source', 'unused', 'env:CONNECTOR_MONGO_TEST', 'production', 'meridian', \
         'mongodb', 'batch', $3)",
    )
    .bind(id)
    .bind(MERIDIAN_GROUP.parse::<uuid::Uuid>().unwrap())
    .bind(json!({
        "hosts": ["mongo.example:27017"], "database": "shop",
        "username": "u", "directConnection": true,
    }))
    .execute(pool)
    .await
    .expect("seed a mongodb connector");
}

async fn send(
    app: &TestApp,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("cookie", cookie);
    let body = match body {
        Some(body) => {
            builder = builder.header("content-type", "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    let response = app
        .router
        .clone()
        .oneshot(builder.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn mount_launch(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .and(body_string_contains("launchRun"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "LaunchRunSuccess", "run": { "runId": "run-disc-1" } } }
        })))
        .expect(1)
        .mount(server)
        .await;
}

/// The job is launched with the connector, the request id and what was asked, and
/// no credential; the answer is `running`; the orchestrator's post is stored and
/// the poll reads it.
#[tokio::test]
async fn a_mongodb_list_launches_the_job_and_the_posted_answer_is_what_the_poll_reads() {
    let server = MockServer::start().await;
    mount_launch(&server).await;
    let (app, cookie) = app_with_dagster(&server).await;
    seed_mongo(&app.pool, "conn-mongo-disc").await;

    let (status, answer) = send(
        &app,
        &cookie,
        "POST",
        "/api/connectors/conn-mongo-disc/discovery/list",
        Some(json!({ "search": "ord" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer, json!({ "kind": "list", "state": "running" }));

    let requests = server.received_requests().await.unwrap();
    let launch = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(launch.contains("discover_job"), "{launch}");
    assert!(launch.contains("conn-mongo-disc") && launch.contains("run_discover"));
    assert!(
        !launch.contains("CONNECTOR_MONGO_TEST"),
        "no credential reference travels"
    );

    let request_id: String = sqlx::query_scalar(
        "SELECT request_id FROM connector_discovery WHERE connector_id = 'conn-mongo-disc'",
    )
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert!(launch.contains(&request_id));

    // The orchestrator posts the answer (an unrestricted administrator stands in
    // for the service identity; route_auth.rs pins who may).
    let admin = session_cookie_for_seeded_user(&app.pool, "fajar@meridian.example").await;
    let url = "/api/connectors/conn-mongo-disc/discovery-results";
    let body = json!({
        "requestId": request_id, "kind": "list", "state": "done",
        "objects": [{ "name": "orders", "kind": "collection" }], "truncated": false,
    });
    let (status, recorded) = send(&app, &admin, "POST", url, Some(body.clone())).await;
    assert_eq!(status, StatusCode::OK, "{recorded}");
    assert_eq!(recorded, json!({ "recorded": true }));
    let (_, again) = send(&app, &admin, "POST", url, Some(body)).await;
    assert_eq!(again, json!({ "recorded": false }), "the first post wins");
    let stale = json!({ "requestId": "another", "kind": "list", "state": "failed", "errorClass": "timeout" });
    let (_, stale) = send(&app, &admin, "POST", url, Some(stale)).await;
    assert_eq!(stale, json!({ "recorded": false }));

    let (status, poll) = send(
        &app,
        &cookie,
        "GET",
        "/api/connectors/conn-mongo-disc/discovery/list",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(poll["state"], "done");
    assert_eq!(poll["objects"][0]["name"], "orders");

    // A body that contradicts itself, or smuggles a free-text field, stores nothing.
    for bad in [
        json!({ "requestId": request_id, "kind": "list", "state": "failed", "errorClass": "timeout", "message": "x" }),
        json!({ "requestId": request_id, "kind": "list", "state": "failed", "errorClass": "boom" }),
        json!({ "requestId": request_id, "kind": "list", "state": "done" }),
    ] {
        let (status, _) = send(&app, &admin, "POST", url, Some(bad)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

/// A launch the orchestrator refuses is a fixed 422, and the request ends at once as
/// a failure so the poll does not wait for a job that never ran.
#[tokio::test]
async fn a_refused_launch_is_a_fixed_422_and_ends_the_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/graphql"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "data": { "launchRun": { "__typename": "PythonError", "message": "secret upstream text" } }
        })))
        .mount(&server)
        .await;
    let (app, cookie) = app_with_dagster(&server).await;
    seed_mongo(&app.pool, "conn-mongo-disc").await;

    let (status, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/connectors/conn-mongo-disc/discovery/list",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(!body.to_string().contains("secret upstream text"));

    let (_, poll) = send(
        &app,
        &cookie,
        "GET",
        "/api/connectors/conn-mongo-disc/discovery/list",
        None,
    )
    .await;
    assert_eq!(poll["state"], "failed");
}

/// With Dagster unreachable the answer is a fixed 503, not the transport's text.
#[tokio::test]
async fn an_unreachable_orchestrator_is_a_fixed_503() {
    let (app, cookie) = {
        let app = common::spin_up().await;
        let cookie = session_cookie_for_seeded_user(&app.pool, "bayu@meridian.example").await;
        (app, cookie)
    };
    seed_mongo(&app.pool, "conn-mongo-disc").await;
    let (status, body) = send(
        &app,
        &cookie,
        "POST",
        "/api/connectors/conn-mongo-disc/discovery/preview",
        Some(json!({ "object": "orders" })),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(!body.to_string().contains("127.0.0.1"));
}
