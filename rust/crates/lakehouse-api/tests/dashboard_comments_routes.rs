//! `BI-19` part E (T17) through the real router: comments on a dashboard are as
//! visible as the dashboard, mentions cannot leak a restricted dashboard, and
//! only the author edits and only the author or a manager deletes. Postgres
//! holds the rules and the comments (a per-test database); a `wiremock`
//! stand-in plays `ClickHouse` and answers the dashboard lookups.

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

/// The live dashboards the stand-in holds: `(id, folder)`. `b_trashed` is only
/// in the placements, as in the real store (a dashboard in the trash is not
/// returned by the live lookup).
const BOARDS: [(&str, &str); 2] = [("b1", ""), ("b_secret", "")];

struct Store {
    trashed: bool,
}

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows: Vec<Value> = if sql.contains("FROM console.bi_folder") {
            Vec::new()
        } else if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            BOARDS
                .iter()
                .map(|(id, folder)| json!({ "id": id, "folder_id": folder, "owner_id": "" }))
                .chain(
                    self.trashed
                        .then(|| json!({ "id": "b_trashed", "folder_id": "", "owner_id": "" })),
                )
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

async fn app() -> (axum::Router, PgPool, MockServer) {
    let ch = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Store { trashed: true })
        .mount(&ch)
        .await;
    let mut overrides = std::collections::HashMap::new();
    overrides.insert("CH_URL".to_owned(), ch.uri());
    let TestApp { router, pool } = spin_up_with_env(&overrides).await;
    (router, pool, ch)
}

async fn call(
    router: &axum::Router,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
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

fn rule(user: Uuid, level: &str) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: level.to_owned(),
    }
}

async fn post(
    router: &axum::Router,
    cookie: &str,
    board: &str,
    text: &str,
    mentions: &[Uuid],
) -> (StatusCode, Value) {
    call(
        router,
        cookie,
        "POST",
        "/api/dashboard/comments",
        Some(json!({ "board": board, "body": text, "mentions": mentions })),
    )
    .await
}

/// A restricted `b_secret` that lists `listed` only (view), and a person who is
/// not listed but holds `dashboard:read`.
async fn restricted(pool: &PgPool, listed: &[Uuid]) {
    let rules: Vec<NewRule> = listed.iter().map(|u| rule(*u, "view")).collect();
    replace_item(pool, "board", "b_secret", &rules, true, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_reader_posts_and_lists_comments_in_order_with_text_kept_as_typed() {
    let (router, pool, _ch) = app().await;
    let ann = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, ann).await;
    let markup = "<img src=x onerror=alert(1)> **b**";
    let (status, first) = post(&router, &cookie, "b1", markup, &[]).await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["body"], markup, "stored and returned as typed");
    assert_eq!(first["canEdit"], true);
    post(&router, &cookie, "b1", "second", &[]).await;

    let (status, body) = call(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/comments?board=b1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let texts: Vec<&str> = body["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["body"].as_str().unwrap())
        .collect();
    assert_eq!(texts, [markup, "second"]);
}

#[tokio::test]
async fn a_comment_must_be_between_one_and_2000_characters_without_control_characters() {
    let (router, pool, _ch) = app().await;
    let ann = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, ann).await;
    for text in ["", "   ", &"x".repeat(2001), "a\u{0}b"] {
        let (status, _) = post(&router, &cookie, "b1", text, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{text:?}");
    }
    let (status, _) = post(&router, &cookie, "b1", &"x".repeat(2000), &[]).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn somebody_who_cannot_see_a_dashboard_gets_the_same_404_as_for_one_that_does_not_exist() {
    let (router, pool, _ch) = app().await;
    let listed = create_principal_with_permissions(&pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    restricted(&pool, &[listed]).await;
    let inside = session_cookie_for_user(&pool, listed).await;
    let outside = session_cookie_for_user(&pool, outsider).await;
    let (status, posted) = post(&router, &inside, "b_secret", "private words", &[]).await;
    assert_eq!(status, StatusCode::OK, "{posted}");

    let hidden = call(
        &router,
        &outside,
        "GET",
        "/api/dashboard/comments?board=b_secret",
        None,
    )
    .await;
    let unknown = call(
        &router,
        &outside,
        "GET",
        "/api/dashboard/comments?board=nope",
        None,
    )
    .await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert_eq!(hidden, unknown);
    let hidden = post(&router, &outside, "b_secret", "hi", &[]).await;
    let unknown = post(&router, &outside, "nope", "hi", &[]).await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert_eq!(hidden, unknown);
    // The people picker is the same 404.
    let (status, _) = call(
        &router,
        &outside,
        "GET",
        "/api/dashboard/comments/people?board=b_secret",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // A comment id on the hidden dashboard answers like an unknown id.
    let id = posted["id"].as_str().unwrap();
    let hidden_id = call(
        &router,
        &outside,
        "DELETE",
        &format!("/api/dashboard/comments/{id}"),
        None,
    )
    .await;
    let unknown_id = call(
        &router,
        &outside,
        "DELETE",
        &format!("/api/dashboard/comments/{}", Uuid::new_v4()),
        None,
    )
    .await;
    assert_eq!(hidden_id.0, StatusCode::NOT_FOUND);
    assert_eq!(hidden_id, unknown_id);
    let hidden_edit = call(
        &router,
        &outside,
        "PUT",
        &format!("/api/dashboard/comments/{id}"),
        Some(json!({ "body": "x" })),
    )
    .await;
    assert_eq!(hidden_edit.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn nobody_with_no_access_to_dashboards_is_refused_and_a_dashboard_in_the_trash_is_a_404() {
    let (router, pool, _ch) = app().await;
    let nobody = create_principal_with_permissions(&pool, "query:read").await;
    let cookie = session_cookie_for_user(&pool, nobody).await;
    let (status, _) = call(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/comments?board=b1",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, _) = call(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/comments?board=b_trashed",
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "comments follow the board into the trash"
    );
    let (status, _) = post(&router, &cookie, "b_trashed", "hi", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_person_given_view_on_one_dashboard_can_comment_on_it_and_on_no_other() {
    let (router, pool, _ch) = app().await;
    let ruled = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(&pool, "board", "b1", &[rule(ruled, "view")], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, ruled).await;
    let (status, body) = post(&router, &cookie, "b1", "hello", &[]).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = post(&router, &cookie, "b_secret", "hello", &[]).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_mention_of_somebody_who_cannot_see_the_dashboard_is_refused_and_nothing_is_stored() {
    let (router, pool, _ch) = app().await;
    let author = create_principal_with_permissions(&pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    let disabled = create_principal_with_permissions(&pool, "dashboard:read").await;
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(disabled)
        .execute(&pool)
        .await
        .unwrap();
    restricted(&pool, &[author, disabled]).await;
    let cookie = session_cookie_for_user(&pool, author).await;

    let expected = json!("One of the people mentioned cannot see this dashboard.");
    for target in [outsider, disabled, Uuid::new_v4()] {
        let (status, body) = post(&router, &cookie, "b_secret", "hi @them", &[target]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], expected, "one sentence for every reason");
    }
    let (_, listed) = call(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/comments?board=b_secret",
        None,
    )
    .await;
    assert_eq!(listed["comments"].as_array().unwrap().len(), 0);
    let (unread,): (i64,) = sqlx::query_as("SELECT count(*) FROM dashboard_comment_mention")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(unread, 0);
}

#[tokio::test]
async fn a_mention_reaches_the_bell_without_the_comment_text_and_only_while_the_person_can_see_it()
{
    let (router, pool, _ch) = app().await;
    let author = create_principal_with_permissions(&pool, "dashboard:read").await;
    let friend = create_principal_with_permissions(&pool, "query:read").await;
    restricted(&pool, &[author, friend]).await;
    let author_cookie = session_cookie_for_user(&pool, author).await;
    let friend_cookie = session_cookie_for_user(&pool, friend).await;

    let (status, body) = post(
        &router,
        &author_cookie,
        "b_secret",
        "the revenue is 12345",
        &[friend],
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, bell) = call(&router, &friend_cookie, "GET", "/api/notifications", None).await;
    assert_eq!(status, StatusCode::OK, "{bell}");
    let lines = bell["unreadMentions"].as_array().unwrap();
    assert_eq!(lines.len(), 1, "{bell}");
    let title = lines[0]["title"].as_str().unwrap();
    assert!(title.contains("mentioned you on Board b_secret"), "{title}");
    assert!(
        !bell.to_string().contains("12345"),
        "no comment text in the bell"
    );

    // The author is not told about their own mention.
    let (_, own) = call(&router, &author_cookie, "GET", "/api/notifications", None).await;
    assert_eq!(own["unreadMentions"].as_array().unwrap().len(), 0);

    // Access taken away: the line is gone, with nothing about the dashboard.
    restricted(&pool, &[author]).await;
    let (_, after) = call(&router, &friend_cookie, "GET", "/api/notifications", None).await;
    assert_eq!(after["unreadMentions"].as_array().unwrap().len(), 0);
    assert!(!after.to_string().contains("b_secret"));

    // Access back, then read: the bell empties.
    restricted(&pool, &[author, friend]).await;
    let (status, marked) = call(
        &router,
        &friend_cookie,
        "POST",
        "/api/dashboard/comments/mentions/read",
        Some(json!({})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(marked["marked"], 1);
    let (_, empty) = call(&router, &friend_cookie, "GET", "/api/notifications", None).await;
    assert_eq!(empty["unreadMentions"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn the_people_picker_offers_only_people_who_can_see_the_dashboard() {
    let (router, pool, _ch) = app().await;
    let author = create_principal_with_permissions(&pool, "dashboard:read").await;
    let listed = create_principal_with_permissions(&pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    restricted(&pool, &[author, listed]).await;
    sqlx::query("UPDATE app_user SET name = $2 WHERE id = $1")
        .bind(listed)
        .bind("Zed Listed")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE app_user SET name = $2 WHERE id = $1")
        .bind(outsider)
        .bind("Zed Outsider")
        .execute(&pool)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, author).await;
    let (status, body) = call(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/comments/people?board=b_secret&q=Zed",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let names: Vec<&str> = body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Zed Listed"]);
    assert!(
        body["items"][0].get("email").is_none(),
        "the picker is not a directory of addresses"
    );
}

#[tokio::test]
async fn only_the_author_edits_and_the_author_or_a_manager_deletes() {
    let (router, pool, _ch) = app().await;
    let author = create_principal_with_permissions(&pool, "query:read").await;
    let viewer = create_principal_with_permissions(&pool, "query:read").await;
    let manager = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(
        &pool,
        "board",
        "b1",
        &[
            rule(author, "view"),
            rule(viewer, "view"),
            rule(manager, "manage"),
        ],
        false,
        None,
    )
    .await
    .unwrap();
    let a = session_cookie_for_user(&pool, author).await;
    let v = session_cookie_for_user(&pool, viewer).await;
    let m = session_cookie_for_user(&pool, manager).await;
    let (_, posted) = post(&router, &a, "b1", "mine", &[]).await;
    let id = posted["id"].as_str().unwrap().to_owned();
    let uri = format!("/api/dashboard/comments/{id}");

    let (status, edited) = call(
        &router,
        &a,
        "PUT",
        &uri,
        Some(json!({ "body": "mine, changed" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["body"], "mine, changed");
    assert!(edited["editedAt"].is_string());

    for who in [&v, &m] {
        let (status, _) = call(&router, who, "PUT", &uri, Some(json!({ "body": "hijack" }))).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "not even a manager edits another's words"
        );
    }
    let (status, _) = call(&router, &v, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (_, listed) = call(&router, &v, "GET", "/api/dashboard/comments?board=b1", None).await;
    assert_eq!(listed["comments"][0]["canDelete"], false);
    let (_, listed) = call(&router, &m, "GET", "/api/dashboard/comments?board=b1", None).await;
    assert_eq!(listed["comments"][0]["canDelete"], true);

    let (status, _) = call(&router, &m, "DELETE", &uri, None).await;
    assert_eq!(status, StatusCode::OK, "a manager removes it");
    let (_, listed) = call(&router, &a, "GET", "/api/dashboard/comments?board=b1", None).await;
    assert_eq!(listed["comments"].as_array().unwrap().len(), 0);
    let (status, _) = call(&router, &a, "DELETE", &uri, None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a deleted comment is gone for its author too"
    );

    let (_, mine) = post(&router, &a, "b1", "again", &[]).await;
    let own = format!("/api/dashboard/comments/{}", mine["id"].as_str().unwrap());
    let (status, _) = call(&router, &a, "DELETE", &own, None).await;
    assert_eq!(status, StatusCode::OK, "the author deletes their own");
}

#[tokio::test]
async fn an_edit_that_adds_a_mention_checks_the_new_person_only() {
    let (router, pool, _ch) = app().await;
    let author = create_principal_with_permissions(&pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    restricted(&pool, &[author]).await;
    let cookie = session_cookie_for_user(&pool, author).await;
    let (_, posted) = post(&router, &cookie, "b_secret", "hi", &[]).await;
    let uri = format!("/api/dashboard/comments/{}", posted["id"].as_str().unwrap());
    let (status, _) = call(
        &router,
        &cookie,
        "PUT",
        &uri,
        Some(json!({ "body": "hi", "mentions": [outsider] })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// `BI-19` E review E-5: a service identity has no `app_user` row, so it is told
/// so in a sentence (403) on every route that would write its id, not a 500 from
/// the author foreign key.
#[tokio::test]
async fn a_service_token_is_refused_in_a_sentence_on_post_edit_delete_and_the_picker() {
    const TOKEN: &str = "svc-comments-fixture-token";
    let (router, pool, _ch) = app().await;
    let identity = lakehouse_store::identity::create_service_identity(
        &pool,
        &lakehouse_store::identity::CreateServiceIdentityInput {
            name: "test-comments".to_owned(),
            scopes: vec!["dashboard:read".to_owned(), "dashboard:write".to_owned()],
            environment: "test".to_owned(),
        },
    )
    .await
    .unwrap();
    lakehouse_auth::service_token::ensure_service_credential(
        &pool,
        identity.id.parse().unwrap(),
        &lakehouse_auth::Secret::new(TOKEN.to_owned()),
    )
    .await
    .unwrap();
    let id = Uuid::new_v4();
    let calls = [
        (
            "POST",
            "/api/dashboard/comments".to_owned(),
            Some(json!({ "board": "b1", "body": "hi" })),
        ),
        (
            "PUT",
            format!("/api/dashboard/comments/{id}"),
            Some(json!({ "body": "hi" })),
        ),
        ("DELETE", format!("/api/dashboard/comments/{id}"), None),
        (
            "GET",
            "/api/dashboard/comments/people?board=b1&q=a".to_owned(),
            None,
        ),
    ];
    for (method, uri, body) in calls {
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&uri)
                    .header("authorization", format!("Bearer {TOKEN}"))
                    .header("content-type", "application/json")
                    .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let body: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            body["error"],
            "Comments are for people; a service identity cannot post, edit, delete or look people up.",
            "{method} {uri}"
        );
    }
}
