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

/// The dashboards the stand-in store holds: `(id, folder, owner)`. `b_private`
/// is somebody else's personal dashboard.
const BOARDS: [(&str, &str, &str); 4] = [
    ("b1", "", ""),
    ("b2", "", ""),
    ("b_secret", "f1", ""),
    ("b_private", "fp_x", "someone-else"),
];

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
                .map(|(id, folder, owner)| json!({ "id": id, "folder_id": folder, "owner_id": owner }))
                .collect()
        } else if sql.contains("FROM console.bi_board FINAL") {
            let only = listed(&sql, "AND id IN (");
            let hidden = listed(&sql, "AND id NOT IN (").unwrap_or_default();
            let by_id = sql.find("AND id='").map(|at| {
                let rest = &sql[at + "AND id='".len()..];
                rest[..rest.find('\'').unwrap_or(rest.len())].to_owned()
            });
            BOARDS
                .iter()
                .filter(|(id, _, _)| only.as_ref().is_none_or(|o| o.iter().any(|i| i == id)))
                .filter(|(id, _, _)| !hidden.iter().any(|h| h == id))
                // An empty "only" list is `AND 1 = 0`: nothing.
                .filter(|_| !sql.contains("AND 1 = 0"))
                // A read by id (`AND id='b1'`) answers that dashboard only.
                .filter(|(id, _, _)| by_id.as_ref().is_none_or(|wanted| wanted == id))
                .filter(|(_, _, owner)| owner.is_empty() || sql.contains(&format!("owner_id = '{owner}'")) || !sql.contains("owner_id = ''"))
                .map(|(id, folder, owner)| {
                    json!({ "id": id, "name": format!("Board {id}"), "folder_id": folder, "owner_id": owner })
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

async fn put(router: &axum::Router, cookie: &str, body: &Value) -> (StatusCode, Value) {
    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/dashboard/access")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
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

#[tokio::test]
async fn only_somebody_who_manages_a_dashboard_reads_or_changes_its_permissions() {
    let (router, pool, _ch) = app().await;
    let manager = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let nobody = create_principal_with_permissions(&pool, "query:read").await;
    let guest = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(&pool, "board", "b1", &[view_for(guest)], false, None)
        .await
        .unwrap();
    let get_access = "/api/dashboard/access?kind=board&id=b1";

    // A manager reads it.
    let cookie = session_cookie_for_user(&pool, manager).await;
    let (status, body) = get(&router, &cookie, get_access).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["yourLevel"], "manage");
    assert_eq!(body["rules"].as_array().unwrap().len(), 1, "{body}");

    // Somebody who can see it and does not manage it is refused; somebody who
    // cannot see it, and an id that does not exist, get the same 404.
    // BI-19 B1 review R10: this asserted 200 although the comment and
    // `managed_subject` say 403 (a reader holds view, not manage); corrected.
    let cookie = session_cookie_for_user(&pool, reader).await;
    assert_eq!(
        get(&router, &cookie, get_access).await.0,
        StatusCode::FORBIDDEN
    );
    let cookie = session_cookie_for_user(&pool, guest).await;
    assert_eq!(
        get(&router, &cookie, get_access).await.0,
        StatusCode::FORBIDDEN
    );
    let hidden = get(&router, &cookie, "/api/dashboard/access?kind=board&id=b2").await;
    let unknown = get(&router, &cookie, "/api/dashboard/access?kind=board&id=nope").await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND);
    assert_eq!(
        hidden, unknown,
        "an existing dashboard and an unknown id answer alike"
    );

    // Neither permission nor rule.
    let cookie = session_cookie_for_user(&pool, nobody).await;
    assert_eq!(
        get(&router, &cookie, get_access).await.0,
        StatusCode::FORBIDDEN
    );
    let (status, _) = put(
        &router,
        &cookie,
        &json!({ "kind": "board", "id": "b1", "rules": [], "restricted": true }),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        lakehouse_store::dashboard_access::restriction_for_item(&pool, "board", "b1")
            .await
            .unwrap()
            .is_none(),
        "a refused change wrote nothing"
    );
}

#[tokio::test]
async fn a_manager_replaces_the_rules_and_the_change_is_audited_and_the_refusals_hold() {
    let (router, pool, _ch) = app().await;
    let manager = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let ann = create_principal_with_permissions(&pool, "query:read").await;
    let cookie = session_cookie_for_user(&pool, manager).await;
    let ok = put(
        &router,
        &cookie,
        &json!({
            "kind": "board", "id": "b2", "restricted": true,
            "rules": [{ "userId": ann, "level": "view" }],
        }),
    )
    .await;
    assert_eq!(ok.0, StatusCode::OK, "{}", ok.1);
    assert_eq!(ok.1["restricted"], true);
    assert_eq!(ok.1["rules"][0]["id"], json!(ann));
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_event WHERE action = 'dashboard.access_change' AND resource_id = 'b2'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audited, 1);

    // Ann now reads b2 and nothing else.
    let ann_cookie = session_cookie_for_user(&pool, ann).await;
    let (_, list) = get(&router, &ann_cookie, "/api/dashboard/boards").await;
    assert_eq!(ids(&list), ["b2"]);

    // Refusals: an unknown person, a person twice, the built-in dashboard,
    // somebody else's personal dashboard (which does not exist for the manager).
    for (body, expected) in [
        (
            json!({ "kind": "board", "id": "b2", "restricted": false,
                    "rules": [{ "userId": Uuid::new_v4(), "level": "view" }] }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "kind": "board", "id": "b2", "restricted": false,
                    "rules": [{ "userId": ann, "level": "view" }, { "userId": ann, "level": "edit" }] }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "kind": "board", "id": "default", "restricted": false, "rules": [] }),
            StatusCode::BAD_REQUEST,
        ),
        (
            json!({ "kind": "board", "id": "b_private", "restricted": false, "rules": [] }),
            StatusCode::NOT_FOUND,
        ),
        (
            json!({ "kind": "chart", "id": "b2", "restricted": false, "rules": [] }),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, body_out) = put(&router, &cookie, &body).await;
        assert_eq!(status, expected, "{body}: {body_out}");
    }
    // The failed attempts changed nothing.
    let rules = lakehouse_store::dashboard_access::rules_for_item(&pool, "board", "b2")
        .await
        .unwrap();
    assert_eq!(rules.len(), 1);
}

// ── BI-19 B1 review R1 to R6 ───────────────────────────────────────────────

async fn send(
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

/// `f1` is restricted to nobody in particular; `b_secret` sits in it.
async fn restricted_folder(pool: &PgPool) {
    replace_item(pool, "folder", "f1", &[], true, None)
        .await
        .unwrap();
}

/// R1: the assistant's `list_boards` shows an unlisted `dashboard:read`
/// holder what the console shows them, not the restricted dashboard.
#[tokio::test]
async fn the_assistant_does_not_list_a_restricted_dashboard_to_an_unlisted_reader() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    let (status, body) = send(
        &router,
        &cookie,
        "POST",
        "/api/ai/tool",
        Some(json!({ "tool": "list_boards", "args": {}, "mode": "ask" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let text = body["result"].to_string();
    assert!(text.contains("b1"), "{text}");
    assert!(!text.contains("b_secret"), "{text}");
}

/// R5: a writer who is not listed reaches a restricted dashboard through no
/// write route: the public link, a move and a duplicate answer as for an id
/// that does not exist.
#[tokio::test]
async fn an_unlisted_writer_gets_the_unknown_dashboard_answer_for_a_restricted_one() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let writer = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;

    for (method, uri, body) in [
        ("GET", "/api/dashboard/public-link?board={}", None),
        (
            "POST",
            "/api/dashboard/boards",
            Some(json!({ "duplicate": "{}" })),
        ),
        (
            "PUT",
            "/api/dashboard/boards",
            Some(json!({ "id": "{}", "folderId": "" })),
        ),
    ] {
        let fill = |id: &str| {
            (
                uri.replace("{}", id),
                body.as_ref().map(|b| {
                    serde_json::from_str::<Value>(&b.to_string().replace("{}", id)).unwrap()
                }),
            )
        };
        let (hidden_uri, hidden_body) = fill("b_secret");
        let (nope_uri, nope_body) = fill("nope");
        let hidden = send(&router, &cookie, method, &hidden_uri, hidden_body).await;
        let unknown = send(&router, &cookie, method, &nope_uri, nope_body).await;
        assert!(hidden.0.is_client_error(), "{method} {uri}: {hidden:?}");
        assert_eq!(hidden, unknown, "{method} {uri}");
    }
}

/// R4: somebody who holds only a rule cannot read any mart through `records`
/// or `values` without naming what they were given.
#[tokio::test]
async fn a_rule_only_person_cannot_read_any_mart_through_records_or_values() {
    let (router, pool, _ch) = app().await;
    let guest = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(&pool, "board", "b1", &[view_for(guest)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, guest).await;
    for uri in [
        "/api/dashboard/records?mart=other",
        "/api/dashboard/values?column=province",
        "/api/dashboard/values?column=province&board=all",
    ] {
        let (status, body) = send(&router, &cookie, "GET", uri, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
    }
    // A board they were not given is the unknown-dashboard answer.
    let (status, _) = send(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/values?column=province&board=b2",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// R2: a drill of a chart the caller cannot see is the unknown-chart answer,
/// and any other preview still needs `dashboard:read`.
#[tokio::test]
async fn a_drill_preview_is_a_404_for_an_unseen_chart_and_other_previews_need_read() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let guest = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(&pool, "board", "b1", &[view_for(guest)], false, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, guest).await;
    let preview = "/api/dashboard/specs/preview";
    let hidden = send(
        &router,
        &cookie,
        "POST",
        preview,
        Some(json!({ "drillOf": "c_on_secret", "level": 1 })),
    )
    .await;
    let unknown = send(
        &router,
        &cookie,
        "POST",
        preview,
        Some(json!({ "drillOf": "c_nope", "level": 1 })),
    )
    .await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND, "{hidden:?}");
    assert_eq!(hidden, unknown);
    let (status, _) = send(
        &router,
        &cookie,
        "POST",
        preview,
        Some(json!({ "title": "t", "kind": "bar", "mart": "m" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// R6: an outsider (not listed on the restricted folder) cannot create a
/// digest of a dashboard in it.
#[tokio::test]
async fn an_unlisted_person_cannot_create_a_digest_of_a_restricted_dashboard() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read, alert:write").await;
    let cookie = session_cookie_for_user(&pool, outsider).await;
    let digest = |board: &str| {
        json!({
            "name": "Weekly", "kind": "digest", "board": board,
            "channel": "email", "target": "x@example.test",
        })
    };
    let hidden = send(
        &router,
        &cookie,
        "POST",
        "/api/alerts",
        Some(digest("b_secret")),
    )
    .await;
    let unknown = send(
        &router,
        &cookie,
        "POST",
        "/api/alerts",
        Some(digest("nope")),
    )
    .await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND, "{hidden:?}");
    assert_eq!(hidden, unknown);
}

/// Q-R12: a listed person can see the restricted dashboard but still cannot
/// send its digest outside the console.
#[tokio::test]
async fn even_a_listed_person_cannot_save_a_digest_of_a_restricted_dashboard() {
    let (router, pool, _ch) = app().await;
    let insider = create_principal_with_permissions(&pool, "dashboard:read, alert:write").await;
    replace_item(&pool, "folder", "f1", &[view_for(insider)], true, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, insider).await;
    let (status, body) = send(
        &router,
        &cookie,
        "POST",
        "/api/alerts",
        Some(json!({
            "name": "Weekly", "kind": "digest", "board": "b_secret",
            "channel": "email", "target": "x@example.test",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

/// Q-R14: managing a parent folder through a rule does not open a restricted
/// child.
#[tokio::test]
async fn a_manager_of_the_parent_folder_does_not_reach_a_restricted_child() {
    let (router, pool, _ch) = app().await;
    let guest = create_principal_with_permissions(&pool, "query:read").await;
    // The rule is on the dashboard's own folder `f1`; the dashboard itself is
    // restricted with an empty list, which ends what `f1` gives.
    replace_item(&pool, "folder", "f1", &[view_for(guest)], false, None)
        .await
        .unwrap();
    replace_item(&pool, "board", "b_secret", &[], true, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, guest).await;
    let (status, _) = send(
        &router,
        &cookie,
        "GET",
        "/api/dashboard?board=b_secret",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// R6 (assistant path): the assistant's `create_alert_rule` with `type=digest`
/// goes through the same guard as the route, so a restricted dashboard's digest
/// is refused whoever asks (the unlisted get the unknown-dashboard answer).
#[tokio::test]
async fn the_assistant_cannot_save_a_digest_of_a_restricted_dashboard() {
    let (router, pool, _ch) = app().await;
    let insider = create_principal_with_permissions(&pool, "dashboard:read, alert:write").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read, alert:write").await;
    replace_item(&pool, "folder", "f1", &[view_for(insider)], true, None)
        .await
        .unwrap();
    for (who, expect) in [(insider, "restricted"), (outsider, "not found")] {
        let cookie = session_cookie_for_user(&pool, who).await;
        let (status, body) = send(
            &router,
            &cookie,
            "POST",
            "/api/ai/tool",
            Some(json!({
                "tool": "create_alert_rule",
                "args": {
                    "name": "Weekly", "type": "digest", "board": "b_secret",
                    "channel": "email", "target": "x@example.test", "confirmed": true,
                },
                "mode": "build",
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let text = body.to_string().to_lowercase();
        assert!(text.contains(expect), "{text}");
        assert!(!text.contains("\"ok\":true"), "{text}");
    }
}
