//! `BI-19` part B1 through the real router: what a person given one dashboard,
//! and a person who is not listed on a restricted folder, can read. Postgres
//! holds the rules (a per-test database); a `wiremock` stand-in plays
//! `ClickHouse` and answers each statement the way the real store would for
//! the `id IN (...)` / `id NOT IN (...)` clause it carries, so a read that
//! forgot the clause would get the other dashboards back and fail here.
//!
//! The pure rules (inheritance, restrictions, the cap) are unit-tested in
//! `lakehouse_bi::access`; the statement shapes in `routes/organise_paths.rs`.
//! `route_auth.rs` asserts the router floor of every moved row both ways.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_store::dashboard_access::{NewRule, replace_item};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

use common::{
    TestApp, create_principal_with_permissions, session_cookie_for_user, spin_up_with_env,
};

/// The dashboards the stand-in store holds: `(id, folder)`.
const BOARDS: [(&str, &str); 3] = [("b1", ""), ("b2", ""), ("b_secret", "f1")];

/// The ids named by `AND id IN (...)` or `AND id NOT IN (...)` in a statement,
/// if it carries one.
fn listed(sql: &str, marker: &str) -> Option<Vec<String>> {
    let rest = &sql[sql.find(marker)? + marker.len()..];
    let inside = &rest[..rest.find(')')?];
    Some(
        inside
            .split(',')
            .map(|s| s.trim().trim_matches('\'').to_owned())
            .collect(),
    )
}

struct Store;

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows: Vec<Value> = if sql.contains("FROM console.bi_folder") {
            let hidden = listed(&sql, "AND id NOT IN (").unwrap_or_default();
            if hidden.iter().any(|h| h == "f1") {
                Vec::new()
            } else {
                vec![json!({ "id": "f1", "name": "Secret", "parent_id": "" })]
            }
        } else if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            BOARDS
                .iter()
                .map(|(id, folder)| json!({ "id": id, "folder_id": folder, "owner_id": "" }))
                .collect()
        } else if sql.contains("FROM console.bi_board FINAL") {
            let only = listed(&sql, "AND id IN (");
            let hidden = listed(&sql, "AND id NOT IN (").unwrap_or_default();
            BOARDS
                .iter()
                .filter(|(id, _)| only.as_ref().is_none_or(|o| o.iter().any(|i| i == id)))
                .filter(|(id, _)| !hidden.iter().any(|h| h == id))
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

async fn app() -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Store)
        .mount(&ch)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

async fn get(router: &axum::Router, cookie: &str, uri: &str) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn view_for(user: Uuid) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: "view".to_owned(),
    }
}

fn ids(body: &Value) -> Vec<String> {
    body["boards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn an_analyst_given_one_dashboard_sees_it_and_gets_a_404_for_any_other() {
    let (router, pool, _ch) = app().await;
    let analyst = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(&pool, "board", "b1", &[view_for(analyst)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, analyst).await;

    let (status, body) = get(&router, &cookie, "/api/dashboard/boards").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), ["b1"], "the list holds only what was given");

    let (status, _) = get(&router, &cookie, "/api/dashboard?board=b1").await;
    assert_eq!(status, StatusCode::OK);
    let (other, other_body) = get(&router, &cookie, "/api/dashboard?board=b2").await;
    let (unknown, unknown_body) = get(&router, &cookie, "/api/dashboard?board=nope").await;
    assert_eq!(other, StatusCode::NOT_FOUND);
    assert_eq!(
        (other, &other_body),
        (unknown, &unknown_body),
        "a dashboard that exists and one that does not answer alike"
    );
    // The built-in board belongs to the starting point.
    let (builtin, _) = get(&router, &cookie, "/api/dashboard?board=default").await;
    assert_eq!(builtin, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn somebody_with_no_rule_and_no_dashboard_permission_is_refused_as_before() {
    let (router, pool, _ch) = app().await;
    let nobody = create_principal_with_permissions(&pool, "query:read").await;
    let cookie = session_cookie_for_user(&pool, nobody).await;
    for uri in [
        "/api/dashboard/boards",
        "/api/dashboard/folders",
        "/api/dashboard?board=b1",
    ] {
        let (status, _) = get(&router, &cookie, uri).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}");
    }
}

#[tokio::test]
async fn a_restricted_folder_hides_its_dashboards_from_a_viewer_who_is_not_listed() {
    let (router, pool, _ch) = app().await;
    replace_item(&pool, "folder", "f1", &[], true, None)
        .await
        .unwrap();
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    let listed_viewer = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[view_for(listed_viewer)],
        true,
        None,
    )
    .await
    .unwrap();

    let cookie = session_cookie_for_user(&pool, outsider).await;
    let (status, body) = get(&router, &cookie, "/api/dashboard/boards").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let seen = ids(&body);
    assert!(
        seen.contains(&"b1".to_owned()) && !seen.contains(&"b_secret".to_owned()),
        "{seen:?}"
    );
    let (_, folders) = get(&router, &cookie, "/api/dashboard/folders").await;
    assert_eq!(folders["folders"].as_array().unwrap().len(), 0, "{folders}");
    let (status, _) = get(&router, &cookie, "/api/dashboard?board=b_secret").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let cookie = session_cookie_for_user(&pool, listed_viewer).await;
    let (_, body) = get(&router, &cookie, "/api/dashboard/boards").await;
    assert!(ids(&body).contains(&"b_secret".to_owned()), "{body}");
    let (status, _) = get(&router, &cookie, "/api/dashboard?board=b_secret").await;
    assert_eq!(status, StatusCode::OK);
}
