//! Dashboard templates (`BI-18` part D, T3; feature page
//! `docs/core/features/reports-and-templates.md`, decisions T1 to T4), through
//! the real router, a per-test Postgres (the templates) and a recording
//! `wiremock` stand-in for `ClickHouse` holding one dashboard with tabs, a
//! filter, a text tile, a chart over a table that has since been dropped and a
//! chart over a SQL source.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{create_principal_with_permissions, session_cookie_for_user, spin_up_with_env};
use serde_json::{Value, json};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, Request as MockRequest, Respond, ResponseTemplate};

#[derive(Clone)]
struct Store {
    log: Arc<Mutex<Vec<String>>>,
}

fn quoted_after(sql: &str, marker: &str) -> Option<String> {
    let rest = &sql[sql.find(marker)? + marker.len()..];
    Some(rest[..rest.find('\'')?].to_owned())
}

fn chart_row(id: &str, def: &Value) -> Value {
    let mut def = def.clone();
    def["title"] = json!(format!("Chart {id}"));
    let spec = json!({
        "id": id, "title": format!("Chart {id}"), "kind": def["kind"], "mart": def["mart"],
    "sql": format!("SELECT region FROM serving.{}", def["mart"].as_str().unwrap_or("")),
        "x": "region", "y": "email"
    });
    json!({
        "id": id, "board": "b_src", "created_by": "ui", "created_at": "2026-01-01 00:00:00",
        "spec_json": json!({ "spec": spec, "def": def, "hasYear": false }).to_string(),
    })
}

fn board_row(id: &str, name: &str, with_content: bool) -> Value {
    let (layout, filters, tabs) = if with_content {
        (
            json!({
                "c_a": {"x": 0, "y": 0, "w": 6, "h": 3, "tab": "t1"},
                "c_gone": {"x": 6, "y": 0, "w": 6, "h": 3, "tab": "t2"},
                "c_note": {"x": 0, "y": 3, "w": 12, "h": 2, "tab": "t1"}
            })
            .to_string(),
            json!([{"column": "region", "values": ["north"]}]).to_string(),
            json!([{"id": "t1", "title": "First"}, {"id": "t2", "title": "Second"}]).to_string(),
        )
    } else {
        (String::new(), String::new(), String::new())
    };
    json!({
        "id": id, "name": name, "description": "Quarterly", "created_by": "t",
        "layout_json": layout, "filters_json": filters,
        // Only the source dashboard has a public link and embedding on.
        "public_token": if with_content { "p_secret_token" } else { "" },
        "embed_enabled": if with_content { "1" } else { "0" },
        "folder_id": "", "created_at": "2026-10-10 00:00:00", "owner_id": "",
        "tabs_json": tabs, "embed_revoked_before": "0", "embed_revoked_jti_json": "[]",
        "embed_origins_json": "[]"
    })
}

impl Respond for Store {
    fn respond(&self, request: &MockRequest) -> ResponseTemplate {
        let sql = String::from_utf8_lossy(&request.body).into_owned();
        self.log.lock().unwrap().push(sql.clone());
        let rows = |data: Vec<Value>| {
            let n = data.len();
            ResponseTemplate::new(200).set_body_json(json!({ "meta": [], "data": data, "rows": n }))
        };
        if sql.starts_with("INSERT") || sql.starts_with("ALTER") || sql.starts_with("CREATE") {
            return ResponseTemplate::new(200).set_body_string("");
        }
        if sql.contains("SELECT id, spec_json, board, created_by") {
            let bar = |mart: &str| json!({ "mart": mart, "kind": "bar", "dimension": "region", "measures": ["email"] });
            return rows(vec![
                chart_row("c_a", &bar("mart_a")),
                // `mart_gone` was dropped after the dashboard was made.
                chart_row("c_gone", &bar("mart_gone")),
                chart_row(
                    "c_note",
                    &json!({ "mart": "", "kind": "text", "text": "Read me" }),
                ),
                chart_row(
                    "c_sql",
                    &json!({ "mart": "", "sqlSource": "s_one", "kind": "bar", "dimension": "region", "measures": ["n"] }),
                ),
            ]);
        }
        // `s_one` is filed in `f_hidden`, a folder only the people listed may
        // open, and nobody is: the folder list leaves it out for a viewer whose
        // statement excludes it, as the real store does.
        if sql.contains("FROM console.bi_folder") {
            if sql.contains("'f_hidden'") {
                return rows(vec![]);
            }
            return rows(vec![json!({
                "id": "f_hidden", "name": "Hidden", "parent_id": "", "created_by": "t",
                "owner_id": "", "updated_at": "2026-01-01 00:00:00"
            })]);
        }
        if sql.contains("FROM console.bi_source") {
            return rows(vec![json!({
                "id": "s_one", "title": "One", "sql": "SELECT 1 AS region", "columns_json": "[]",
                "folder_id": "f_hidden", "created_by": "t", "updated_at": "2026-01-01 00:00:00"
            })]);
        }
        if sql.contains("FROM console.bi_board") {
            let id = quoted_after(&sql, "id='").unwrap_or_default();
            if id.is_empty() {
                return rows(vec![]);
            }
            return if id == "b_src" {
                rows(vec![board_row("b_src", "Quarterly sales", true)])
            } else {
                rows(vec![board_row(&id, "Started", false)])
            };
        }
        rows(vec![])
    }
}

struct App {
    router: axum::Router,
    pool: sqlx::PgPool,
    log: Arc<Mutex<Vec<String>>>,
    _ch: MockServer,
}

async fn app() -> App {
    let log = Arc::new(Mutex::new(Vec::new()));
    let ch = MockServer::start().await;
    Mock::given(wiremock::matchers::method("POST"))
        .respond_with(Store { log: log.clone() })
        .mount(&ch)
        .await;
    let env = HashMap::from([("CH_URL".to_owned(), ch.uri())]);
    let common::TestApp { router, pool } = spin_up_with_env(&env).await;
    App {
        router,
        pool,
        log,
        _ch: ch,
    }
}

async fn send(
    app: &App,
    cookie: &str,
    method: &str,
    uri: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let resp = app
        .router
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

async fn person(app: &App, permissions: &str) -> String {
    let user = create_principal_with_permissions(&app.pool, permissions).await;
    session_cookie_for_user(&app.pool, user).await
}

fn inserts_into(app: &App, table: &str, from: usize) -> Vec<String> {
    app.log.lock().unwrap()[from..]
        .iter()
        .filter(|s| s.starts_with(&format!("INSERT INTO console.{table}")))
        .cloned()
        .collect()
}

async fn saved_template(app: &App, cookie: &str) -> String {
    let (status, saved) = send(
        app,
        cookie,
        "POST",
        "/api/dashboard/templates",
        Some(json!({ "board": "b_src", "name": "  Sales overview " })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    saved["template"]["id"].as_str().unwrap().to_owned()
}

async fn definition(app: &App, id: &str) -> Value {
    sqlx::query_scalar::<_, Value>("SELECT definition FROM dashboard_template WHERE id = $1::uuid")
        .bind(id)
        .fetch_one(&app.pool)
        .await
        .unwrap()
}

/// Decision T1: a template is the dashboard's definition (layout, charts,
/// filters, tabs, text tiles) and nothing else: no data, no sharing state.
#[tokio::test]
async fn a_dashboard_saves_as_a_template_with_its_definition_and_no_sharing_state() {
    let app = app().await;
    let cookie = person(&app, "dashboard:read, dashboard:write").await;
    let id = saved_template(&app, &cookie).await;

    let (status, list) = send(&app, &cookie, "GET", "/api/dashboard/templates", None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let row = &list["templates"][0];
    assert_eq!(row["id"], id.as_str());
    assert_eq!(row["name"], "Sales overview");
    assert_eq!(row["sourceBoard"], "b_src");
    assert_eq!(row["sourceBoardName"], "Quarterly sales");
    assert_eq!(row["chartCount"], 4);

    let stored = definition(&app, &id).await;
    let file = &stored["file"];
    assert_eq!(file["kind"], "rantai-dashboard");
    assert_eq!(file["board"]["tabs"].as_array().unwrap().len(), 2);
    assert_eq!(file["board"]["filters"][0]["column"], "region");
    assert_eq!(file["charts"].as_array().unwrap().len(), 4);
    assert_eq!(stored["charts"].as_object().unwrap().len(), 4);
    let text = stored.to_string();
    for forbidden in [
        "p_secret_token",
        "publicToken",
        "embedEnabled",
        "ownerId",
        "\"rows\"",
        "\"results\"",
    ] {
        assert!(!text.contains(forbidden), "{forbidden} is in the template");
    }
}

/// Decision T2: starting from a template makes an independent copy in the
/// chosen folder: fresh chart ids on a new dashboard, the layout and tabs
/// re-keyed to them, and the template unchanged. A chart whose table is gone
/// is still copied and is not run; a chart over a SQL source is left out with
/// the import's reason for a person without `dashboard:sql`.
#[tokio::test]
async fn a_dashboard_started_from_a_template_is_an_independent_copy() {
    let app = app().await;
    let cookie = person(&app, "dashboard:read, dashboard:write").await;
    let id = saved_template(&app, &cookie).await;
    let before = definition(&app, &id).await;

    let from = app.log.lock().unwrap().len();
    let (status, started) = send(
        &app,
        &cookie,
        "POST",
        &format!("/api/dashboard/templates/{id}/use"),
        Some(json!({ "name": "Q4 sales", "folderId": "" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    let new_board = started["board"]["id"].as_str().unwrap().to_owned();
    assert_ne!(new_board, "b_src");
    assert_eq!(started["board"]["name"], "Q4 sales");
    assert_eq!(
        started["leftOut"],
        json!([{ "title": "Chart c_sql", "reason": "It reads a SQL source, which needs the dashboard SQL permission." }])
    );

    let charts = inserts_into(&app, "bi_chart", from);
    assert_eq!(charts.len(), 3, "{charts:?}");
    assert!(
        charts.iter().all(|s| s.contains(&format!("'{new_board}'"))),
        "{charts:?}"
    );
    for old in ["'c_a'", "'c_gone'", "'c_note'"] {
        assert!(charts.iter().all(|s| !s.contains(old)), "{old} was reused");
    }
    // The chart over the dropped table is copied as it was, and nothing ran it.
    assert!(charts.iter().any(|s| s.contains("mart_gone")), "{charts:?}");
    let ran_gone = app.log.lock().unwrap()[from..]
        .iter()
        .any(|s| s.starts_with("SELECT region FROM serving.mart_gone"));
    assert!(!ran_gone, "a chart copied from a template is not run first");

    // The settings landed on the new board with the new ids. The stand-in does
    // not remember a write, so each full-row save carries the one setting it
    // changes: the writes are judged together.
    let boards: Vec<String> = inserts_into(&app, "bi_board (", from)
        .into_iter()
        .filter(|s| s.contains(&format!("'{new_board}'")))
        .collect();
    assert!(!boards.is_empty(), "the new board was saved");
    let all = boards.join("\n");
    assert!(
        all.contains("First") && all.contains("Second"),
        "tabs: {all}"
    );
    assert!(all.contains("north"), "filters: {all}");
    let new_ids: Vec<String> = charts
        .iter()
        .filter_map(|c| c.split('\'').nth(1).map(str::to_owned))
        .collect();
    assert!(
        new_ids.iter().any(|id| all.contains(&format!("\"{id}\":"))),
        "the layout is keyed by the new chart ids: {all}"
    );
    for old in ["c_a", "c_gone", "c_note"] {
        assert!(
            !all.contains(&format!("\"{old}\":")),
            "layout still keyed by {old}: {all}"
        );
    }
    assert!(!all.contains("p_secret_token"), "no public link: {all}");

    // A second copy gets ids of its own, and the template has not changed.
    let from_second = app.log.lock().unwrap().len();
    let (status, second) = send(
        &app,
        &cookie,
        "POST",
        &format!("/api/dashboard/templates/{id}/use"),
        Some(json!({ "name": "Q1 sales" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{second}");
    assert_ne!(second["board"]["id"], new_board.as_str());
    let second_charts = inserts_into(&app, "bi_chart", from_second);
    assert_eq!(second_charts.len(), 3);
    for chart in &charts {
        let id_of = |s: &str| s.split('\'').nth(1).unwrap_or_default().to_owned();
        assert!(second_charts.iter().all(|c| id_of(c) != id_of(chart)));
    }
    assert_eq!(
        definition(&app, &id).await,
        before,
        "the template is unchanged"
    );
}

/// A person with `dashboard:sql` is still left without a SQL-source chart
/// whose source sits in a folder they may not open, with the import's reason.
#[tokio::test]
async fn a_sql_source_chart_in_a_folder_the_person_cannot_open_is_left_out() {
    let app = app().await;
    let cookie = person(&app, "dashboard:read, dashboard:write, dashboard:sql").await;
    let id = saved_template(&app, &cookie).await;
    // `f_hidden` is "only the people listed", and nobody is listed.
    sqlx::query(
        "INSERT INTO dashboard_restriction (item_kind, item_id) VALUES ('folder', 'f_hidden')",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let from = app.log.lock().unwrap().len();
    let (status, started) = send(
        &app,
        &cookie,
        "POST",
        &format!("/api/dashboard/templates/{id}/use"),
        Some(json!({ "name": "With SQL" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{started}");
    assert_eq!(
        started["leftOut"][0]["reason"],
        "The SQL source it reads is not available here."
    );
    assert_eq!(inserts_into(&app, "bi_chart", from).len(), 3);
}

/// The built-in dashboard, an unknown template, a blank name and a delete.
#[tokio::test]
async fn templates_refuse_what_they_cannot_copy_and_delete_for_good() {
    let app = app().await;
    let cookie = person(&app, "dashboard:read, dashboard:write").await;
    let (status, said) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/templates",
        Some(json!({ "board": "default", "name": "Main" })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        said["error"]
            .as_str()
            .unwrap()
            .starts_with("The built-in dashboard"),
        "{said}"
    );
    let (status, said) = send(
        &app,
        &cookie,
        "POST",
        "/api/dashboard/templates",
        Some(json!({ "board": "b_src", "name": "  " })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(said["error"], "A template needs a name.");

    let id = saved_template(&app, &cookie).await;
    let (status, _) = send(
        &app,
        &cookie,
        "DELETE",
        &format!("/api/dashboard/templates/{id}"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    for (method, uri, body) in [
        ("DELETE", format!("/api/dashboard/templates/{id}"), None),
        (
            "POST",
            format!("/api/dashboard/templates/{id}/use"),
            Some(json!({ "name": "X" })),
        ),
        (
            "POST",
            "/api/dashboard/templates/not-a-uuid/use".to_owned(),
            Some(json!({ "name": "X" })),
        ),
    ] {
        let (status, said) = send(&app, &cookie, method, &uri, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {uri}");
        assert_eq!(said["error"], "template not found.");
    }
    let (_, list) = send(&app, &cookie, "GET", "/api/dashboard/templates", None).await;
    assert!(list["templates"].as_array().unwrap().is_empty(), "{list}");
}

/// `BI-18D-AC8`: `dashboard:read` alone is refused saving and starting with 403.
#[tokio::test]
async fn a_reader_cannot_save_or_use_a_template() {
    let app = app().await;
    let writer = person(&app, "dashboard:read, dashboard:write").await;
    let id = saved_template(&app, &writer).await;
    let reader = person(&app, "dashboard:read").await;
    for (method, uri, body) in [
        (
            "POST",
            "/api/dashboard/templates".to_owned(),
            json!({ "board": "b_src", "name": "X" }),
        ),
        (
            "POST",
            format!("/api/dashboard/templates/{id}/use"),
            json!({ "name": "X" }),
        ),
    ] {
        let (status, _) = send(&app, &reader, method, &uri, Some(body)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {uri}");
    }
}

/// Review round 1, BLOCKER 1: saving a template runs the dashboard file
/// export's checks in its order, then needs `edit`. A hidden dashboard is the
/// same 404 whatever else is true; with it visible, downloads switched off for
/// the person's roles is the download refusal; with downloads on, `view`
/// without `edit` is 403. Nothing is saved in any of the three.
#[tokio::test]
async fn saving_a_template_follows_the_dashboard_file_checks_then_needs_edit() {
    let app = app().await;
    let save = |cookie: String| {
        let app = &app;
        async move {
            send(
                app,
                &cookie,
                "POST",
                "/api/dashboard/templates",
                Some(json!({ "board": "b_src", "name": "Copy" })),
            )
            .await
        }
    };
    // `b_src` is "only the people listed"; `viewer` is listed with view.
    let viewer =
        create_principal_with_permissions(&app.pool, "dashboard:read, dashboard:write").await;
    let stranger =
        create_principal_with_permissions(&app.pool, "dashboard:read, dashboard:write").await;
    lakehouse_store::dashboard_access::replace_item(
        &app.pool,
        "board",
        "b_src",
        &[lakehouse_store::dashboard_access::NewRule {
            user_id: Some(viewer),
            role_id: None,
            level: "view".to_owned(),
        }],
        true,
        None,
    )
    .await
    .unwrap();
    let viewer = session_cookie_for_user(&app.pool, viewer).await;
    let stranger = session_cookie_for_user(&app.pool, stranger).await;

    // Downloads off for every role.
    sqlx::query(
        "INSERT INTO reporting_settings (singleton, time_zone, week_start, download_denied_roles) \
         SELECT TRUE, 'UTC', 'monday', array_agg(r.name) FROM role r",
    )
    .execute(&app.pool)
    .await
    .unwrap();
    let (status, said) = save(stranger.clone()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "hidden first: {said}");
    assert_eq!(said["error"], "dashboard not found.");
    let (status, said) = save(viewer.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{said}");
    assert!(
        said["error"]
            .as_str()
            .unwrap()
            .contains("Downloads are switched off"),
        "{said}"
    );

    // Downloads on: the stranger still finds nothing, and view is not edit.
    sqlx::query("DELETE FROM reporting_settings")
        .execute(&app.pool)
        .await
        .unwrap();
    let (status, said) = save(stranger).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{said}");
    assert_eq!(said["error"], "dashboard not found.");
    let (status, said) = save(viewer).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{said}");
    assert!(
        said["error"]
            .as_str()
            .unwrap()
            .contains("edit on this dashboard"),
        "{said}"
    );

    let saved: i64 = sqlx::query_scalar("SELECT count(*) FROM dashboard_template")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(saved, 0, "nothing is saved");
}
