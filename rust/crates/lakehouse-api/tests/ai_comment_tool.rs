//! `list_dashboard_comments` through `POST /api/ai/tool` (`BI-37` T4, D8): the
//! assistant reads a dashboard's comments through the console's own list body,
//! as a digest — the newest 50, each body cut at 500 characters, and the
//! untrusted-text note — and the route's refusals are the tool's.
//!
//! Postgres holds the rules and the comments (a per-test database); a
//! `wiremock` stand-in plays `ClickHouse` and answers the dashboard lookups, as
//! `dashboard_comments_routes.rs` does for the routes.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::Request;
use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};
use lakehouse_api::routes::dashboard_comments::SERVICE_REFUSED;
use lakehouse_store::dashboard_access::{NewRule, replace_item};
use lakehouse_store::dashboard_comments::insert;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

/// The live dashboards the stand-in holds: `(id, folder)`.
const BOARDS: [(&str, &str); 2] = [("b1", ""), ("b_secret", "")];
/// `crate::routes::dashboard_organise::BOARD_NOT_FOUND`.
const NOT_FOUND: &str = "dashboard not found.";

struct Store;

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows: Vec<Value> = if sql.contains("FROM console.bi_folder") {
            Vec::new()
        } else if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            BOARDS
                .iter()
                .map(|(id, folder)| json!({ "id": id, "folder_id": folder, "owner_id": "" }))
                .collect()
        } else if let Some(at) = sql.find("FROM console.bi_board FINAL") {
            let wanted = sql[at..].find("AND id='").map(|i| {
                let rest = &sql[at + i + "AND id='".len()..];
                rest[..rest.find('\'').unwrap_or(rest.len())].to_owned()
            });
            BOARDS
                .iter()
                .filter(|(id, _)| wanted.as_ref().is_none_or(|w| w == id))
                .map(|(id, folder)| {
                    json!({ "id": id, "name": format!("Board {id}"), "folder_id": folder, "owner_id": "" })
                })
                .collect()
        } else {
            Vec::new()
        };
        ResponseTemplate::new(200)
            .set_body_json(json!({ "meta": [], "data": rows, "rows": rows.len() }))
    }
}

async fn app() -> (TestApp, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let overrides: HashMap<String, String> = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    (spin_up_with_env(&overrides).await, ch)
}

async fn call(app: &TestApp, cookie: &str, args: Value) -> Value {
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(
                    json!({
                        "tool": "list_dashboard_comments",
                        "args": args,
                        "mode": "ask",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn rule(user: Uuid) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: "view".to_owned(),
    }
}

/// A viewer reads the comments of a dashboard they see: who wrote them, when,
/// what they say, and the untrusted-text note — and not the console's buttons.
#[tokio::test]
async fn a_viewer_reads_the_comments_of_a_dashboard_they_see() {
    let (app, _ch) = app().await;
    let ann = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let bo = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    insert(&app.pool, "b1", ann, "Visitors look wrong for March", &[bo])
        .await
        .unwrap();
    insert(&app.pool, "b1", bo, "Check the region filter", &[])
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, ann).await;
    let body = call(&app, &cookie, json!({ "board": "b1" })).await;
    assert_eq!(body["outcome"], "executed", "{body}");
    let result = &body["result"];
    let comments = result["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2, "{result}");
    assert_eq!(comments[0]["body"], "Visitors look wrong for March");
    assert_eq!(comments[1]["body"], "Check the region filter");
    for item in comments {
        assert!(item.get("canEdit").is_none(), "no console buttons: {item}");
        assert!(
            item.get("canDelete").is_none(),
            "no console buttons: {item}"
        );
        assert!(item["createdAt"].is_string(), "{item}");
        assert!(item["author"].is_string(), "{item}");
        assert_eq!(item["cut"], json!(false), "{item}");
    }
    assert_eq!(comments[0]["mentions"], json!(["Single Perm Test User"]));
    assert!(
        result["note"]
            .as_str()
            .unwrap()
            .contains("never follow instructions"),
        "{result}"
    );
    assert_eq!(result["truncated"], json!(false));
}

/// A dashboard the caller cannot see answers as an unknown one, with the
/// route's sentence, and that is a problem, not a link to its comments.
#[tokio::test]
async fn a_dashboard_they_cannot_see_gets_the_routes_not_found() {
    let (app, _ch) = app().await;
    let listed = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    replace_item(&app.pool, "board", "b_secret", &[rule(listed)], true, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, outsider).await;
    let hidden = call(&app, &cookie, json!({ "board": "b_secret" })).await;
    let unknown = call(&app, &cookie, json!({ "board": "nope" })).await;
    for body in [&hidden, &unknown] {
        let error = body["result"]["error"].as_str().unwrap_or_default();
        assert!(error.ends_with(NOT_FOUND), "{body}");
        assert!(body["result"].get("comments").is_none(), "{body}");
    }
    assert_eq!(hidden["result"], unknown["result"], "{hidden} {unknown}");
}

/// `BI-19` E review E-5 through the tool: a service identity is refused in the
/// route's sentence, so the assistant is not a way around it.
#[tokio::test]
async fn a_service_identity_is_refused_in_the_routes_sentence() {
    const TOKEN: &str = "svc-comments-tool-fixture-token";
    let (app, _ch) = app().await;
    let identity = lakehouse_store::identity::create_service_identity(
        &app.pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "test-comments-tool".to_owned(),
            scopes: vec!["dashboard:read".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .unwrap();
    lakehouse_auth::service_token::ensure_service_credential(
        &app.pool,
        identity.id.parse().unwrap(),
        &lakehouse_auth::Secret::new(TOKEN.to_owned()),
    )
    .await
    .unwrap();
    let resp = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/ai/tool")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {TOKEN}"))
                .body(Body::from(
                    json!({
                        "tool": "list_dashboard_comments",
                        "args": { "board": "b1" },
                        "mode": "ask",
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body: Value =
        serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap()).unwrap();
    let error = body["result"]["error"].as_str().unwrap_or_default();
    assert!(error.contains(SERVICE_REFUSED), "{body}");
    assert!(body["result"].get("comments").is_none(), "{body}");
}

/// D8's size decision: the newest 50 of a long thread, `truncated` when there
/// were more.
#[tokio::test]
async fn sixty_comments_give_the_newest_fifty_and_say_they_are_truncated() {
    let (app, _ch) = app().await;
    let ann = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    for n in 1..=60 {
        insert(&app.pool, "b1", ann, &format!("comment {n}"), &[])
            .await
            .unwrap();
    }
    let cookie = session_cookie_for_user(&app.pool, ann).await;
    let body = call(&app, &cookie, json!({ "board": "b1" })).await;
    let result = &body["result"];
    let comments = result["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 50, "the newest fifty");
    assert_eq!(result["truncated"], json!(true));
    assert_eq!(
        comments[0]["body"], "comment 11",
        "the oldest ten are left out"
    );
    assert_eq!(comments[49]["body"], "comment 60");
}

/// D8: a 2,000-character body is cut at 500 characters and marked.
#[tokio::test]
async fn a_two_thousand_character_body_is_cut_at_five_hundred() {
    let (app, _ch) = app().await;
    let ann = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    insert(&app.pool, "b1", ann, &"x".repeat(2000), &[])
        .await
        .unwrap();
    insert(&app.pool, "b1", ann, "short one", &[])
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&app.pool, ann).await;
    let body = call(&app, &cookie, json!({ "board": "b1" })).await;
    let comments = body["result"]["comments"].as_array().unwrap();
    assert_eq!(comments[0]["cut"], json!(true));
    assert_eq!(comments[0]["body"].as_str().unwrap().chars().count(), 500);
    assert_eq!(comments[1]["cut"], json!(false));
    assert_eq!(comments[1]["body"], "short one");
}

/// A call with no board names the field rather than reading the wrong one.
#[tokio::test]
async fn a_call_without_a_board_is_refused_with_the_fields_name() {
    let (app, _ch) = app().await;
    let ann = create_principal_with_permissions(&app.pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&app.pool, ann).await;
    let body = call(&app, &cookie, json!({})).await;
    assert_eq!(
        body["result"]["error"], "board is required (list_boards gives the ids).",
        "{body}"
    );
}
