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

/// The folders the stand-in holds: `(id, name, parent)`. `f4` is the child of
/// `f5` (BI-19 B2 re-review: the folder-delete test).
const FOLDERS: [(&str, &str, &str); 5] = [
    ("f1", "Secret", ""),
    ("f2", "Open", ""),
    ("f3", "Empty", ""),
    ("f4", "Child", "f5"),
    ("f5", "Parent", ""),
];

/// A dashboard in the trash: `(id, folder)`. It shows only in the placements
/// and the row-state lookups, as in the real store. `b_orphan` names a folder
/// that no longer exists.
const TRASHED: [(&str, &str); 2] = [("b_trashed", "f2"), ("b_orphan", "f_gone")];

/// The charts the stand-in holds: `(id, state, board)`; state 2 is the trash.
const CHARTS: [(&str, u8, &str); 4] = [
    ("c1", 0, "b1"),
    ("c2", 0, "b2"),
    ("c_secret", 0, "b_secret"),
    ("c_gone", 2, "b1"),
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

/// Every id listed after each occurrence of `marker` in `sql`.
fn all_lists(sql: &str, marker: &str) -> Vec<Vec<String>> {
    sql.match_indices(marker)
        .filter_map(|(at, _)| listed(&sql[at..], marker))
        .collect()
}

/// A chart statement keeps a chart of `board` when the viewer's fragments allow
/// its board (`AND board IN (...)`, `AND board NOT IN (...)`, `AND 1 = 0`).
fn chart_allowed(sql: &str, board: &str) -> bool {
    !sql.contains("AND 1 = 0")
        && all_lists(sql, "AND board IN (")
            .iter()
            .all(|l| l.iter().any(|i| i == board))
        && !all_lists(sql, "AND board NOT IN (")
            .iter()
            .any(|l| l.iter().any(|i| i == board))
}

/// The SQL sources the stand-in holds: `(id, folder)`.
const SOURCES: [(&str, &str); 2] = [("s_root", ""), ("s_secret", "f1")];

/// The id a `... WHERE id = '<id>' ...` statement of the given shape asks for.
fn state_lookup(sql: &str, prefix: &str) -> Option<String> {
    let rest = &sql[sql.find(prefix)? + prefix.len()..];
    Some(rest[..rest.find('\'')?].to_owned())
}

struct Store;

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        let rows: Vec<Value> = if sql.contains("FROM console.bi_folder") {
            let only = listed(&sql, "AND id IN (");
            let hidden = listed(&sql, "AND id NOT IN (").unwrap_or_default();
            FOLDERS
                .iter()
                .filter(|(id, _, _)| only.as_ref().is_none_or(|o| o.iter().any(|i| i == id)))
                .filter(|(id, _, _)| !hidden.iter().any(|h| h == id))
                .filter(|_| !sql.contains("AND 1 = 0"))
                .map(|(id, name, parent)| json!({ "id": id, "name": name, "parent_id": parent }))
                .collect()
        } else if sql.contains("SELECT id, folder_id, owner_id FROM console.bi_board") {
            BOARDS
                .iter()
                .map(|(id, folder, owner)| json!({ "id": id, "folder_id": folder, "owner_id": owner }))
                .chain(TRASHED.iter().map(|(id, folder)| json!({ "id": id, "folder_id": folder, "owner_id": "" })))
                .collect()
        } else if let Some(wanted) = state_lookup(
            &sql,
            "SELECT id, board FROM console.bi_chart FINAL WHERE is_deleted = 2 AND id = '",
        ) {
            // `restore_chart`: the chart as the trash lists it.
            CHARTS
                .iter()
                .filter(|(id, state, _)| *id == wanted && *state == 2)
                .map(|(id, _, board)| json!({ "id": id, "board": board }))
                .collect()
        } else if let Some(wanted) = state_lookup(
            &sql,
            "SELECT is_deleted, board FROM console.bi_chart FINAL WHERE id = '",
        ) {
            CHARTS
                .iter()
                .filter(|(id, _, _)| *id == wanted)
                .map(|(_, state, board)| json!({ "is_deleted": state, "board": board }))
                .collect()
        } else if let Some(wanted) = state_lookup(
            &sql,
            "SELECT is_deleted, owner_id FROM console.bi_board FINAL WHERE id = '",
        ) {
            BOARDS
                .iter()
                .map(|(id, _, owner)| (*id, 0, *owner))
                .chain(TRASHED.iter().map(|(id, _)| (*id, 2, "")))
                .filter(|(id, _, _)| *id == wanted)
                .map(|(_, state, owner)| json!({ "is_deleted": state, "owner_id": owner }))
                .collect()
        } else if sql.contains("SELECT id, spec_json, board, created_by") {
            // `list_stored_charts`: a chart per dashboard, filtered by the viewer's fragments.
            CHARTS
                .iter()
                .filter(|(_, state, _)| *state == 0)
                .filter(|(_, _, board)| chart_allowed(&sql, board))
                .map(|(id, _, board)| {
                    let spec = json!({
                        "id": id, "title": format!("Chart {id}"), "kind": "bar", "mart": "mart_x",
                        "sql": "SELECT 1", "x": "region", "y": "email"
                    });
                    json!({
                        "id": id, "board": board, "created_by": "ui",
                        "created_at": "2026-01-01 00:00:00",
                        "spec_json": json!({ "spec": spec, "def": null, "hasYear": false }).to_string(),
                    })
                })
                .collect()
        } else if sql.contains("FROM console.bi_source FINAL") {
            let wanted = state_lookup(&sql, "AND id = '");
            SOURCES
                .iter()
                .filter(|(id, _)| wanted.as_ref().is_none_or(|w| w == id))
                .map(|(id, folder)| {
                    json!({
                        "id": id, "title": id, "sql": "SELECT 1 AS n", "folder_id": folder,
                        "columns_json": "[{\"name\":\"n\",\"type\":\"UInt8\"}]",
                        "created_by": "t", "updated_at": "2026-10-10 00:00:00"
                    })
                })
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
    // The stand-in holds a second, unrestricted folder (`f2`) for the write
    // tests, so the check is that the restricted one is not listed.
    assert!(
        !folders["folders"].to_string().contains("\"f1\""),
        "{folders}"
    );
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
    // BI-19 B2 re-review (B2-3, test bug, rule 2): the manager lists themselves
    // with manage. Restricting b2 without that locks the caller out (Q-R14, as
    // designed), and the refusals below would then all read 404. The intent
    // (a manager's change is applied, audited, and the refusals hold) is kept.
    let ok = put(
        &router,
        &cookie,
        &json!({
            "kind": "board", "id": "b2", "restricted": true,
            "rules": [
                { "userId": ann, "level": "view" },
                { "userId": manager, "level": "manage" },
            ],
        }),
    )
    .await;
    assert_eq!(ok.0, StatusCode::OK, "{}", ok.1);
    assert_eq!(ok.1["restricted"], true);
    let named: Vec<&Value> = ok.1["rules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["id"])
        .collect();
    assert!(
        named.contains(&&json!(ann)) && named.contains(&&json!(manager)),
        "{}",
        ok.1
    );
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
    assert_eq!(rules.len(), 2, "ann and the manager, as first saved");
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
        Some(json!({ "drillOf": "c_on_secret", "to": "city" })),
    )
    .await;
    let unknown = send(
        &router,
        &cookie,
        "POST",
        preview,
        Some(json!({ "drillOf": "c_nope", "to": "city" })),
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
            "name": "Weekly", "type": "digest", "board": board,
            "channel": "email", "target": "x@example.test",
            "mart": "mart_x", "measure": "visitors",
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
            "name": "Weekly", "type": "digest", "board": "b_secret",
            "channel": "email", "target": "x@example.test",
            "mart": "mart_x", "measure": "visitors",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("restricted"), "{body}");
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

// ── BI-19 B2 review: T8's acceptance tests (B2-3), B2-2, S1, S6 ────────────

fn rule_for(user: Uuid, level: &str) -> NewRule {
    NewRule {
        user_id: Some(user),
        role_id: None,
        level: level.to_owned(),
    }
}

/// A status that says the level checks let the request through: whatever
/// the stand-in store makes of it afterwards, it is neither "not found" nor
/// "not allowed".
fn passed(status: StatusCode) -> bool {
    status != StatusCode::FORBIDDEN && status != StatusCode::NOT_FOUND
}

fn chart_body(id: &str, board: &str) -> Value {
    json!({ "id": id, "title": "T", "kind": "bar", "mart": "mart_x", "board": board })
}

/// B2-3: `edit` on `b1` lets a chart on `b1` be saved and not one on `b2`: a
/// rule-only person does not see `b2` at all (404), and somebody whose
/// starting point is `view` sees it and may not change it (403). Moving a
/// chart from `b1` to `b2` needs `edit` on both.
#[tokio::test]
async fn edit_on_one_dashboard_saves_a_chart_there_and_a_move_needs_edit_on_both() {
    let (router, pool, _ch) = app().await;
    let rule_only = create_principal_with_permissions(&pool, "query:read").await;
    let reader = create_principal_with_permissions(&pool, "dashboard:read").await;
    let both = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(
        &pool,
        "board",
        "b1",
        &[
            rule_for(rule_only, "edit"),
            rule_for(reader, "edit"),
            rule_for(both, "edit"),
        ],
        false,
        None,
    )
    .await
    .unwrap();
    replace_item(&pool, "board", "b2", &[rule_for(both, "edit")], false, None)
        .await
        .unwrap();

    let save = |cookie: String, body: Value| {
        let router = router.clone();
        async move { put_specs(&router, &cookie, &body).await }
    };
    let rule_only_cookie = session_cookie_for_user(&pool, rule_only).await;
    let reader_cookie = session_cookie_for_user(&pool, reader).await;
    let both_cookie = session_cookie_for_user(&pool, both).await;

    // On b1: through. On b2: 404 for the rule-only person, 403 for the reader.
    assert!(passed(
        save(rule_only_cookie.clone(), chart_body("c1", "b1")).await
    ));
    assert_eq!(
        save(rule_only_cookie.clone(), chart_body("c2", "b2")).await,
        StatusCode::NOT_FOUND
    );
    assert!(passed(
        save(reader_cookie.clone(), chart_body("c1", "b1")).await
    ));
    assert_eq!(
        save(reader_cookie.clone(), chart_body("c2", "b2")).await,
        StatusCode::FORBIDDEN
    );
    // A move b1 -> b2: edit on b1 only is not enough (404 / 403), on both it is.
    assert_eq!(
        save(rule_only_cookie, chart_body("c1", "b2")).await,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        save(reader_cookie, chart_body("c1", "b2")).await,
        StatusCode::FORBIDDEN
    );
    assert!(passed(save(both_cookie, chart_body("c1", "b2")).await));
}

async fn put_specs(router: &axum::Router, cookie: &str, body: &Value) -> StatusCode {
    send(
        router,
        cookie,
        "PUT",
        "/api/dashboard/specs",
        Some(body.clone()),
    )
    .await
    .0
}

/// B2-3: a folder move needs `manage` on the folder and `edit` on the
/// destination (feature B7); a rename needs `edit` on the folder.
#[tokio::test]
async fn a_folder_move_needs_manage_on_it_and_edit_on_the_destination() {
    let (router, pool, _ch) = app().await;
    let mover = create_principal_with_permissions(&pool, "dashboard:read").await;
    let no_manage = create_principal_with_permissions(&pool, "dashboard:read").await;
    let no_target = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[
            rule_for(mover, "manage"),
            rule_for(no_manage, "edit"),
            rule_for(no_target, "manage"),
        ],
        false,
        None,
    )
    .await
    .unwrap();
    replace_item(
        &pool,
        "folder",
        "f2",
        &[rule_for(mover, "edit"), rule_for(no_manage, "edit")],
        false,
        None,
    )
    .await
    .unwrap();
    let body = json!({ "id": "f1", "parentId": "f2" });
    let status_for = |who: Uuid| {
        let (router, pool, body) = (router.clone(), pool.clone(), body.clone());
        async move {
            let cookie = session_cookie_for_user(&pool, who).await;
            send(
                &router,
                &cookie,
                "PUT",
                "/api/dashboard/folders",
                Some(body),
            )
            .await
            .0
        }
    };
    assert!(
        passed(status_for(mover).await),
        "manage + edit on the target"
    );
    assert_eq!(
        status_for(no_manage).await,
        StatusCode::FORBIDDEN,
        "edit only"
    );
    // A view starting point on the destination is below edit.
    assert_eq!(
        status_for(no_target).await,
        StatusCode::FORBIDDEN,
        "no edit on f2"
    );
    // A rename (no new parent) is edit on the folder.
    let cookie = session_cookie_for_user(&pool, no_manage).await;
    let (status, _) = send(
        &router,
        &cookie,
        "PUT",
        "/api/dashboard/folders",
        Some(json!({ "id": "f1", "name": "Renamed" })),
    )
    .await;
    assert!(passed(status), "edit renames");
}

/// B2-3 and B2-2: restoring a dashboard needs `manage` (it undoes a delete), a
/// chart needs `edit` on its dashboard.
#[tokio::test]
async fn restoring_a_dashboard_needs_manage_and_a_chart_needs_edit() {
    let (router, pool, _ch) = app().await;
    let editor = create_principal_with_permissions(&pool, "dashboard:read").await;
    let manager = create_principal_with_permissions(&pool, "dashboard:read").await;
    let viewer = create_principal_with_permissions(&pool, "dashboard:read").await;
    // `b_trashed` is in the trash and filed in f2; `c_gone` is a trashed chart of b1.
    replace_item(
        &pool,
        "folder",
        "f2",
        &[rule_for(editor, "edit"), rule_for(manager, "manage")],
        false,
        None,
    )
    .await
    .unwrap();
    replace_item(
        &pool,
        "board",
        "b1",
        &[rule_for(editor, "edit")],
        false,
        None,
    )
    .await
    .unwrap();
    let restore = |who: Uuid, kind: &str, id: &str| {
        let (router, pool, kind, id) =
            (router.clone(), pool.clone(), kind.to_owned(), id.to_owned());
        async move {
            let cookie = session_cookie_for_user(&pool, who).await;
            send(
                &router,
                &cookie,
                "POST",
                "/api/dashboard/trash/restore",
                Some(json!({ "kind": kind, "id": id })),
            )
            .await
            .0
        }
    };
    assert_eq!(
        restore(editor, "board", "b_trashed").await,
        StatusCode::FORBIDDEN
    );
    assert!(passed(restore(manager, "board", "b_trashed").await));
    assert!(
        passed(restore(editor, "chart", "c_gone").await),
        "edit restores a chart"
    );
    assert_eq!(
        restore(viewer, "chart", "c_gone").await,
        StatusCode::FORBIDDEN
    );
}

/// B2-2: a dashboard in the trash still names its folder, so deleting the
/// folder (and its restriction) is refused while it is there, with a sentence.
/// The same call by somebody who lacks `manage` is a 403.
#[tokio::test]
async fn a_folder_with_a_dashboard_in_the_trash_is_not_deleted() {
    let (router, pool, _ch) = app().await;
    let manager = create_principal_with_permissions(&pool, "dashboard:read").await;
    let editor = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f2",
        &[rule_for(manager, "manage"), rule_for(editor, "edit")],
        true,
        None,
    )
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, manager).await;
    let (status, body) = send(
        &router,
        &cookie,
        "DELETE",
        "/api/dashboard/folders?id=f2",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("trash"), "{body}");
    // The restriction is still there, so the trashed dashboard stays hidden.
    let rules = lakehouse_store::dashboard_access::all_restrictions(&pool)
        .await
        .unwrap();
    assert!(rules.iter().any(|m| m.item_id == "f2"), "{rules:?}");
    let cookie = session_cookie_for_user(&pool, editor).await;
    let (status, _) = send(
        &router,
        &cookie,
        "DELETE",
        "/api/dashboard/folders?id=f2",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// B2-3 and S1: a copy is filed next to its original (or in the folder named),
/// so the copier needs `edit` there. `view` on the original is not enough.
#[tokio::test]
async fn a_copy_needs_edit_where_it_will_be_filed() {
    let (router, pool, _ch) = app().await;
    let viewer = create_principal_with_permissions(&pool, "dashboard:read").await;
    let editor = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[rule_for(viewer, "view"), rule_for(editor, "edit")],
        false,
        None,
    )
    .await
    .unwrap();
    replace_item(
        &pool,
        "folder",
        "f2",
        &[rule_for(editor, "edit")],
        false,
        None,
    )
    .await
    .unwrap();
    let copy = |who: Uuid, body: Value| {
        let (router, pool) = (router.clone(), pool.clone());
        async move {
            let cookie = session_cookie_for_user(&pool, who).await;
            send(
                &router,
                &cookie,
                "POST",
                "/api/dashboard/boards",
                Some(body),
            )
            .await
            .0
        }
    };
    // `b_secret` is in f1: the copy lands in f1 unless a folder is named.
    // A reader with edit somewhere (here f1 through the viewer's own rule is
    // view) is refused up front; the editor on f1 passes.
    assert_eq!(
        copy(viewer, json!({ "duplicate": "b_secret" })).await,
        StatusCode::FORBIDDEN
    );
    assert!(passed(
        copy(editor, json!({ "duplicate": "b_secret" })).await
    ));
    // Naming another folder asks edit there instead: the editor has it on f2.
    assert!(passed(
        copy(editor, json!({ "duplicate": "b_secret", "folderId": "f2" })).await
    ));
    // ... and the viewer does not, whatever the original.
    assert_eq!(
        copy(viewer, json!({ "duplicate": "b1", "folderId": "f2" })).await,
        StatusCode::FORBIDDEN
    );
}

/// B2-3: the six moved rows, both ways. A person with neither a starting point
/// nor a rule is refused by each (403); a writer passes each.
#[tokio::test]
async fn the_remaining_moved_rows_refuse_a_person_with_no_access_and_admit_a_writer() {
    let (router, pool, _ch) = app().await;
    let nobody = create_principal_with_permissions(&pool, "query:read").await;
    let writer = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let nobody_cookie = session_cookie_for_user(&pool, nobody).await;
    let writer_cookie = session_cookie_for_user(&pool, writer).await;

    let calls: [(&str, &str, Option<Value>, StatusCode); 6] = [
        (
            "POST",
            "/api/dashboard/folders",
            Some(json!({ "name": "New" })),
            StatusCode::OK,
        ),
        (
            "PUT",
            "/api/dashboard/folders",
            Some(json!({ "id": "f1", "name": "Renamed" })),
            StatusCode::OK,
        ),
        // f2 holds a dashboard in the trash: past the level check, refused as not empty.
        (
            "DELETE",
            "/api/dashboard/folders?id=f2",
            None,
            StatusCode::CONFLICT,
        ),
        ("GET", "/api/dashboard/specs", None, StatusCode::OK),
        ("GET", "/api/dashboard/sources", None, StatusCode::OK),
        ("GET", "/api/reports/recipients?q=a", None, StatusCode::OK),
    ];
    for (method, uri, body, admitted) in calls {
        let refused = send(&router, &nobody_cookie, method, uri, body.clone()).await;
        assert_eq!(
            refused.0,
            StatusCode::FORBIDDEN,
            "{method} {uri}: {refused:?}"
        );
        let allowed = send(&router, &writer_cookie, method, uri, body).await;
        assert_eq!(allowed.0, admitted, "{method} {uri}: {allowed:?}");
    }
}

/// B2-3: the recipient picker's two modes. The report picker needs
/// `dashboard:read`; the People mode (`roles=1`) is open to somebody who
/// manages something through a rule alone, and to nobody who manages nothing.
#[tokio::test]
async fn the_people_picker_serves_a_rule_only_manager_and_the_report_picker_does_not() {
    let (router, pool, _ch) = app().await;
    let manager = create_principal_with_permissions(&pool, "query:read").await;
    let mere_viewer = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(
        &pool,
        "board",
        "b1",
        &[rule_for(manager, "manage"), rule_for(mere_viewer, "view")],
        false,
        None,
    )
    .await
    .unwrap();
    let manager_cookie = session_cookie_for_user(&pool, manager).await;
    let viewer_cookie = session_cookie_for_user(&pool, mere_viewer).await;
    let people = "/api/reports/recipients?roles=1&q=a";
    let report = "/api/reports/recipients?q=a";
    assert_eq!(
        get(&router, &manager_cookie, people).await.0,
        StatusCode::OK
    );
    assert_eq!(
        get(&router, &manager_cookie, report).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        get(&router, &viewer_cookie, people).await.0,
        StatusCode::FORBIDDEN
    );
}

/// B2-1 (BI-19 B2 review): the assistant's chart-writing tools are judged by the
/// same level as the console. A `dashboard:write` holder who is only given
/// `view` on a restricted dashboard is refused by each; with `edit` the level
/// check lets the call through. (`delete_chart` waits for an approval first,
/// so it is covered by the unit tests in `ai::tools`.)
#[tokio::test]
async fn the_assistants_chart_tools_need_edit_on_a_restricted_dashboard() {
    let (router, pool, _ch) = app().await;
    let viewer = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let editor = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[rule_for(viewer, "view"), rule_for(editor, "edit")],
        true,
        None,
    )
    .await
    .unwrap();
    let calls = [
        (
            "create_chart",
            json!({ "title": "T", "kind": "bar", "board": "b_secret" }),
        ),
        (
            "update_chart",
            json!({ "id": "c_secret", "title": "T", "kind": "bar" }),
        ),
        (
            "move_chart",
            json!({ "id": "c_secret", "board": "b_secret" }),
        ),
        (
            "format_chart",
            json!({ "chart": "c_secret", "colors": { "palette": "muted" } }),
        ),
    ];
    for (who, refused) in [(viewer, true), (editor, false)] {
        let cookie = session_cookie_for_user(&pool, who).await;
        for (tool, args) in &calls {
            // `confirmed` is what the console's Confirm button adds.
            let mut args = args.clone();
            args["confirmed"] = json!(true);
            let (status, body) = send(
                &router,
                &cookie,
                "POST",
                "/api/ai/tool",
                Some(json!({ "tool": tool, "args": args, "mode": "build" })),
            )
            .await;
            let text = body.to_string();
            assert_eq!(status, StatusCode::OK, "{tool}: {text}");
            assert_eq!(
                text.contains("edit on this dashboard"),
                refused,
                "{tool} for {}: {text}",
                if refused {
                    "a view rule"
                } else {
                    "an edit rule"
                }
            );
        }
    }
}

/// S4 (BI-19 B2 review, owner decision): a SQL source filed in a restricted
/// folder is listed, read and changed only by people who may see that folder,
/// and is the unknown-source 404 for anybody else. A source at the root stays
/// for everyone entitled.
#[tokio::test]
async fn a_sql_source_in_a_restricted_folder_follows_the_folder() {
    let (router, pool, _ch) = app().await;
    // `dashboard:sql` is the router floor of the source routes (B2 re-review:
    // the principals lacked it and every call was a 403 before the filter ran).
    let listed =
        create_principal_with_permissions(&pool, "dashboard:read, dashboard:write, dashboard:sql")
            .await;
    let unlisted =
        create_principal_with_permissions(&pool, "dashboard:read, dashboard:write, dashboard:sql")
            .await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[rule_for(listed, "view")],
        true,
        None,
    )
    .await
    .unwrap();
    let ids = |body: &Value| -> Vec<String> {
        body["sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap().to_owned())
            .collect()
    };

    let cookie = session_cookie_for_user(&pool, unlisted).await;
    let (status, body) = get(&router, &cookie, "/api/dashboard/sources").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(ids(&body), ["s_root"], "{body}");
    let unknown = send(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/fields?source=s_nope",
        None,
    )
    .await;
    let hidden = send(
        &router,
        &cookie,
        "GET",
        "/api/dashboard/fields?source=s_secret",
        None,
    )
    .await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND, "{hidden:?}");
    assert_eq!(hidden.0, unknown.0);
    let change = send(
        &router,
        &cookie,
        "PUT",
        "/api/dashboard/sources",
        Some(json!({ "id": "s_secret", "title": "x", "sql": "SELECT 1" })),
    )
    .await;
    assert_eq!(change.0, StatusCode::NOT_FOUND, "{change:?}");
    let gone = send(
        &router,
        &cookie,
        "DELETE",
        "/api/dashboard/sources?id=s_secret",
        None,
    )
    .await;
    assert_eq!(gone.0, StatusCode::NOT_FOUND, "{gone:?}");

    let cookie = session_cookie_for_user(&pool, listed).await;
    let (_, body) = get(&router, &cookie, "/api/dashboard/sources").await;
    assert_eq!(ids(&body), ["s_root", "s_secret"], "{body}");
    let (status, _) = get(&router, &cookie, "/api/dashboard/fields?source=s_secret").await;
    assert_eq!(status, StatusCode::OK);
}

// ── BI-19 B2 review S5/S6: the leak tests and the positive cases ───────────

fn chart_ids(body: &Value, key: &str) -> Vec<String> {
    body[key]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| c["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// R10 (specs) and S6 (`list_charts`): an unlisted person is not shown the chart
/// of a restricted dashboard, by the console's list or by the assistant's; a
/// rule-only person on `b1` sees `c1` and nothing else.
#[tokio::test]
async fn the_chart_lists_leave_out_a_restricted_dashboards_charts() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    let rule_only = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(
        &pool,
        "board",
        "b1",
        &[rule_for(rule_only, "view")],
        false,
        None,
    )
    .await
    .unwrap();

    for (who, expect) in [(outsider, vec!["c1", "c2"]), (rule_only, vec!["c1"])] {
        let cookie = session_cookie_for_user(&pool, who).await;
        let (status, specs) = get(&router, &cookie, "/api/dashboard/specs").await;
        assert_eq!(status, StatusCode::OK, "{specs}");
        let got = chart_ids(&specs, "charts");
        assert_eq!(got, expect, "specs: {specs}");
        let (status, tool) = send(
            &router,
            &cookie,
            "POST",
            "/api/ai/tool",
            Some(json!({ "tool": "list_charts", "args": {}, "mode": "ask" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{tool}");
        let text = tool["result"].to_string();
        assert!(!text.contains("c_secret"), "{text}");
        if who == rule_only {
            // BI-19 B2 re-review (test bug, rule 2): the assistant's chart
            // tools need `dashboard:read`, which a rule-only person lacks, so
            // the tool refuses them; the leak check above still holds.
            assert!(text.contains("dashboard:read"), "{text}");
            continue;
        }
        for id in &expect {
            assert!(text.contains(id), "{id} missing from {text}");
        }
    }
}

/// S6: `records?chart=` for a rule-only person is a 404 for a chart on a
/// dashboard they were not given, and passes the access check for the chart on
/// the dashboard they were (whatever the stand-in store makes of the read).
#[tokio::test]
async fn records_of_a_chart_need_a_dashboard_the_person_was_given() {
    let (router, pool, _ch) = app().await;
    let rule_only = create_principal_with_permissions(&pool, "query:read").await;
    replace_item(
        &pool,
        "board",
        "b1",
        &[rule_for(rule_only, "view")],
        false,
        None,
    )
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, rule_only).await;
    let hidden = get(&router, &cookie, "/api/dashboard/records?chart=c2").await;
    let unknown = get(&router, &cookie, "/api/dashboard/records?chart=c_nope").await;
    assert_eq!(hidden.0, StatusCode::NOT_FOUND, "{hidden:?}");
    assert_eq!(hidden, unknown);
    let given = get(&router, &cookie, "/api/dashboard/records?chart=c1").await;
    assert!(passed(given.0), "{given:?}");
}

/// R10 (versions, favourites): a restricted dashboard answers like one
/// that does not exist, to somebody the rules leave out.
#[tokio::test]
async fn versions_and_favourites_of_a_restricted_dashboard_read_as_unknown() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    let cookie = session_cookie_for_user(&pool, outsider).await;
    for (method, uri, body) in [
        ("GET", "/api/dashboard/versions?board={}", None),
        (
            "PUT",
            "/api/dashboard/favourites",
            Some(json!({ "kind": "board", "id": "{}", "on": true })),
        ),
    ] {
        let fill = |id: &str| {
            (
                uri.replace("{}", id),
                body.as_ref().map(|b: &Value| {
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

/// R10 (trash, search): the statements an unlisted person's trash list and
/// search send carry the viewer's exclusion of the restricted dashboard, and a
/// listed person's do not.
#[tokio::test]
async fn trash_and_search_statements_exclude_a_restricted_dashboard_for_the_unlisted() {
    let (router, pool, ch) = app().await;
    let listed = create_principal_with_permissions(&pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[rule_for(listed, "view")],
        true,
        None,
    )
    .await
    .unwrap();
    let excluded = |statements: &[String], needle: &str| {
        statements
            .iter()
            .filter(|s| s.contains(needle))
            .any(|s| s.contains("NOT IN ('b_secret')"))
    };
    for (who, hides) in [(outsider, true), (listed, false)] {
        let cookie = session_cookie_for_user(&pool, who).await;
        ch.reset().await;
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(Store)
            .mount(&ch)
            .await;
        for uri in ["/api/dashboard/trash", "/api/dashboard/search?q=a"] {
            let (status, body) = get(&router, &cookie, uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        }
        let statements: Vec<String> = ch
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|r| String::from_utf8_lossy(&r.body).into_owned())
            .collect();
        for table in [
            "FROM console.bi_board FINAL WHERE is_deleted = 2",
            "FROM console.bi_board FINAL WHERE is_deleted = 0",
        ] {
            assert_eq!(
                excluded(&statements, table),
                hides,
                "{table}: {statements:?}"
            );
        }
    }
}

/// R8: deleting an (empty) folder takes its rules and its mark with it, so a
/// later folder cannot inherit them through a reused id.
#[tokio::test]
async fn deleting_a_restricted_folder_takes_its_rules_and_mark_with_it() {
    let (router, pool, _ch) = app().await;
    let manager = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f3",
        &[rule_for(manager, "manage")],
        true,
        None,
    )
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, manager).await;
    let (status, body) = send(
        &router,
        &cookie,
        "DELETE",
        "/api/dashboard/folders?id=f3",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let marks = lakehouse_store::dashboard_access::all_restrictions(&pool)
        .await
        .unwrap();
    assert!(marks.iter().all(|m| m.item_id != "f3"), "{marks:?}");
    let rules = lakehouse_store::dashboard_access::rules_for_item(&pool, "folder", "f3")
        .await
        .unwrap();
    assert!(rules.is_empty(), "{rules:?}");
}

/// R10 (export): the dashboard-as-code file is the caller's view of the
/// dashboards, so the restricted dashboard and its chart are not in an
/// unlisted person's file, and are in a listed person's.
#[tokio::test]
async fn the_export_file_leaves_out_a_restricted_dashboard_for_the_unlisted() {
    let (router, pool, _ch) = app().await;
    let listed = create_principal_with_permissions(&pool, "dashboard:read").await;
    let outsider = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[rule_for(listed, "view")],
        true,
        None,
    )
    .await
    .unwrap();
    for (who, sees) in [(outsider, false), (listed, true)] {
        let cookie = session_cookie_for_user(&pool, who).await;
        let resp = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/api/dashboard/export")
                    .header("cookie", &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let text = String::from_utf8_lossy(&bytes).into_owned();
        assert!(text.contains("b1"), "{text}");
        assert_eq!(text.contains("b_secret"), sees, "{text}");
        assert_eq!(text.contains("c_secret"), sees, "{text}");
    }
}

// ── BI-19 B2 re-review: S4-b (every by-id source path), folder delete, restore ──

/// A reply with the id of the source taken out, so the answer for a hidden
/// source and for an unknown one can be compared whole (the sentence names the
/// id that was asked for).
fn without_id(reply: &(StatusCode, Value), id: &str) -> (StatusCode, String) {
    (reply.0, reply.1.to_string().replace(id, "<id>"))
}

/// One request per place that takes a SQL source's id from the caller, for the
/// source `id`: `(method, path, body)`.
fn source_calls(id: &str) -> Vec<(&'static str, String, Option<Value>)> {
    let chart = json!({
        "title": "T", "kind": "bar", "mart": "", "sqlSource": id,
        "dimension": "n", "measures": ["n"], "aggregate": "sum", "board": "b1",
    });
    let field = json!({ "source": id, "name": "twice", "formula": "[n] * 2" });
    let metric = json!({ "source": id, "name": "total", "formula": "Sum([n])" });
    let hierarchy = json!({ "source": id, "name": "h", "levels": ["n"] });
    vec![
        (
            "GET",
            format!("/api/dashboard/records?sqlSource={id}"),
            None,
        ),
        (
            "GET",
            format!("/api/dashboard/calc-fields?source={id}"),
            None,
        ),
        (
            "POST",
            "/api/dashboard/calc-fields/validate".to_owned(),
            Some(field.clone()),
        ),
        ("POST", "/api/dashboard/calc-fields".to_owned(), Some(field)),
        (
            "POST",
            "/api/dashboard/metrics/validate".to_owned(),
            Some(metric.clone()),
        ),
        ("POST", "/api/dashboard/metrics".to_owned(), Some(metric)),
        (
            "POST",
            "/api/dashboard/hierarchies".to_owned(),
            Some(hierarchy),
        ),
        (
            "POST",
            "/api/dashboard/specs".to_owned(),
            Some(chart.clone()),
        ),
        (
            "POST",
            "/api/dashboard/specs/preview".to_owned(),
            Some(chart),
        ),
    ]
}

/// S4-b: each place that takes a SQL source's id from the caller answers a
/// source in a folder they cannot see exactly as it answers an unknown id, and
/// answers a person who may see the folder normally.
#[tokio::test]
async fn every_path_that_names_a_sql_source_hides_one_in_a_restricted_folder() {
    let (router, pool, _ch) = app().await;
    let power = "dashboard:read, dashboard:write, metric:write";
    let listed = create_principal_with_permissions(&pool, power).await;
    let unlisted = create_principal_with_permissions(&pool, power).await;
    replace_item(
        &pool,
        "folder",
        "f1",
        &[rule_for(listed, "manage")],
        true,
        None,
    )
    .await
    .unwrap();

    let cookie = session_cookie_for_user(&pool, unlisted).await;
    let listed_cookie = session_cookie_for_user(&pool, listed).await;
    for n in 0..source_calls("s_secret").len() {
        let (method, path, body) = source_calls("s_secret").swap_remove(n);
        let hidden = send(&router, &cookie, method, &path, body).await;
        let (method, path, body) = source_calls("s_nope").swap_remove(n);
        let unknown = send(&router, &cookie, method, &path, body).await;
        assert!(hidden.0.is_client_error(), "path {n}: {hidden:?}");
        assert_eq!(
            without_id(&hidden, "s_secret"),
            without_id(&unknown, "s_nope"),
            "path {n} ({method} {path}): a hidden source and an unknown one answer alike"
        );
        // The listed person gets past the source check (what follows is the
        // stand-in store's business).
        let (method, path, body) = source_calls("s_secret").swap_remove(n);
        let seen = send(&router, &listed_cookie, method, &path, body).await;
        assert!(
            !(seen.0 == StatusCode::NOT_FOUND && seen.1.to_string().contains("s_secret")),
            "path {n}: a person who may see the folder is not told it is unknown: {seen:?}"
        );
    }
    // A source at the root stays reachable for everyone entitled.
    let (status, _) = get(&router, &cookie, "/api/dashboard/calc-fields?source=s_root").await;
    assert_eq!(status, StatusCode::OK);
}

/// S4-b, the assistant: `list_calculated_fields` and `validate_formula` answer
/// a hidden source as an unknown one.
#[tokio::test]
async fn the_assistant_does_not_resolve_a_hidden_sql_source_either() {
    let (router, pool, _ch) = app().await;
    restricted_folder(&pool).await;
    let outsider =
        create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let cookie = session_cookie_for_user(&pool, outsider).await;
    for (tool, args) in [
        ("list_calculated_fields", json!({ "sqlSource": "{}" })),
        (
            "validate_formula",
            json!({ "sqlSource": "{}", "name": "x", "formula": "[n] * 2" }),
        ),
        (
            "create_chart",
            json!({ "title": "T", "kind": "bar", "sqlSource": "{}", "dimension": "n",
                    "measures": ["n"], "aggregate": "sum", "board": "b1", "confirmed": true }),
        ),
    ] {
        let ask = |id: &str| {
            let args: Value = serde_json::from_str(&args.to_string().replace("{}", id)).unwrap();
            let router = router.clone();
            let cookie = cookie.clone();
            async move {
                send(
                    &router,
                    &cookie,
                    "POST",
                    "/api/ai/tool",
                    Some(json!({ "tool": tool, "args": args, "mode": "build" })),
                )
                .await
            }
        };
        let hidden = ask("s_secret").await;
        let unknown = ask("s_nope").await;
        assert_eq!(
            without_id(&hidden, "s_secret"),
            without_id(&unknown, "s_nope"),
            "{tool}"
        );
        assert!(
            !hidden.1.to_string().contains("SELECT 1"),
            "{tool}: {hidden:?}"
        );
    }
}

/// Re-review (pre-existing since R5): a folder holding a restricted sub-folder
/// the caller cannot see is not empty. The count of subfolders goes through
/// `Viewer::system()`, so the manager of the parent is told 409 instead of
/// deleting it and orphaning the child.
#[tokio::test]
async fn a_folder_holding_a_hidden_restricted_subfolder_is_not_deleted() {
    let (router, pool, _ch) = app().await;
    let manager = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "folder",
        "f5",
        &[rule_for(manager, "manage")],
        true,
        None,
    )
    .await
    .unwrap();
    // `f4` (its child) is restricted to nobody in particular.
    replace_item(&pool, "folder", "f4", &[], true, None)
        .await
        .unwrap();
    let cookie = session_cookie_for_user(&pool, manager).await;
    let (status, body) = send(
        &router,
        &cookie,
        "DELETE",
        "/api/dashboard/folders?id=f5",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert!(body.to_string().contains("1 subfolder"), "{body}");
    assert!(
        lakehouse_store::dashboard_access::restriction_for_item(&pool, "folder", "f5")
            .await
            .unwrap()
            .is_some(),
        "the refused delete took nothing away"
    );
}

/// B2-2 (re-review): a dashboard whose folder is gone comes back at the root,
/// so restoring it needs `edit` there, on top of `manage` on the dashboard.
#[tokio::test]
async fn restoring_a_dashboard_whose_folder_is_gone_needs_edit_at_the_root() {
    let (router, pool, _ch) = app().await;
    // Manage on the dashboard through a rule, with only a view-level starting
    // point and no write permission: not allowed to file anything at the root.
    let rule_manager = create_principal_with_permissions(&pool, "dashboard:read").await;
    replace_item(
        &pool,
        "board",
        "b_orphan",
        &[rule_for(rule_manager, "manage")],
        false,
        None,
    )
    .await
    .unwrap();
    let cookie = session_cookie_for_user(&pool, rule_manager).await;
    let (status, body) = send(
        &router,
        &cookie,
        "POST",
        "/api/dashboard/trash/restore",
        Some(json!({ "kind": "board", "id": "b_orphan" })),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    // Edit at the root (the starting point of a writer) lets it through the
    // level checks.
    let writer = create_principal_with_permissions(&pool, "dashboard:read, dashboard:write").await;
    let cookie = session_cookie_for_user(&pool, writer).await;
    let (status, body) = send(
        &router,
        &cookie,
        "POST",
        "/api/dashboard/trash/restore",
        Some(json!({ "kind": "board", "id": "b_orphan" })),
    )
    .await;
    assert!(passed(status), "{status} {body}");
}
