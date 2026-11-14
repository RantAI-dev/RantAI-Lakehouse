//! The audit coverage guard and the central audit writer (`GOV-1`).
//!
//! Two kinds of test live here. The guard tests read the two route tables and
//! the route sources and fail when a route that changes something has no audit
//! decision, the way `sec11_guard.rs` fails on a response built from an
//! upstream error. The route tests drive the real router over a real
//! per-test database and read `audit_event` back: what the central writer
//! records for a change, for a refusal at the gate and for a request that
//! never reaches a route, and what it leaves out of a row.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use lakehouse_api::audit_coverage::{AUDIT_TABLE, Coverage, IdParam};
use lakehouse_api::policy::{ANY_METHOD, POLICY_TABLE};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use crate::common::{
    TestApp, audit_rows, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_seeded_user, session_cookie_for_user, spin_up,
};

// ── The guard ──────────────────────────────────────────────────────────────

/// The methods that change something. `HEAD` and the Iceberg door's `*`
/// entries are reads or are refused by the door itself, so they are not in
/// `AUDIT_TABLE` (see its module doc comment).
fn changes_something(method: &str) -> bool {
    method != "GET" && method != "HEAD" && method != ANY_METHOD
}

/// Every excluded route with its reason, one per line: what a failing guard
/// prints, and what a person reads to review the exclusion list.
fn excluded_listing() -> String {
    AUDIT_TABLE
        .iter()
        .filter_map(|(method, pattern, coverage)| match coverage {
            Coverage::Excluded { reason } => Some(format!("{method} {pattern}: {reason}")),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn routes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/routes")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read src/routes") {
        let path = entry.expect("read a directory entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// `^[a-z_]+(\.[a-z_]+)+$`, written out: the crate has no regex dependency.
fn is_module_verb(action: &str) -> bool {
    let mut parts = 0;
    for part in action.split('.') {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_lowercase() || b == b'_') {
            return false;
        }
        parts += 1;
    }
    parts >= 2
}

/// `GOV-1` AC12, check 1: every route that changes something has one entry in
/// `AUDIT_TABLE`, and the table has nothing else. A new route fails here
/// until somebody decides who audits it.
#[test]
fn every_route_that_changes_something_is_in_the_audit_table_exactly_once() {
    let policy: BTreeSet<(&str, &str)> = POLICY_TABLE
        .iter()
        .filter(|(method, _, _)| changes_something(method))
        .map(|(method, pattern, _)| (*method, *pattern))
        .collect();
    let mut listed: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    for (method, pattern, _) in AUDIT_TABLE {
        *listed.entry((*method, *pattern)).or_default() += 1;
    }

    let missing: Vec<_> = policy
        .iter()
        .filter(|route| !listed.contains_key(*route))
        .collect();
    let unknown: Vec<_> = listed
        .keys()
        .filter(|route| !policy.contains(*route))
        .collect();
    let twice: Vec<_> = listed.iter().filter(|(_, n)| **n > 1).collect();
    assert!(
        missing.is_empty() && unknown.is_empty() && twice.is_empty(),
        "AUDIT_TABLE must hold every changing route of POLICY_TABLE exactly once and nothing \
         else.\nnot in AUDIT_TABLE ({}): {missing:?}\nin AUDIT_TABLE but not a changing route of \
         POLICY_TABLE ({}): {unknown:?}\nlisted more than once ({}): {twice:?}\n\
         A route is covered by its own handler (`Coverage::Handler`), by the central writer \
         (`Coverage::Central`), or excluded for a reason the feature page lists as its \
         Decision 1 (`Coverage::Excluded`).",
        missing.len(),
        unknown.len(),
        twice.len(),
    );
}

/// Check 2: an exclusion always says why. The list is printed on every run
/// (and with every failure) so a reviewer reads exactly what is excluded.
#[test]
fn every_excluded_route_gives_a_reason() {
    let listing = excluded_listing();
    println!("Routes excluded from the audit trail:\n{listing}");
    for (method, pattern, coverage) in AUDIT_TABLE {
        if let Coverage::Excluded { reason } = coverage {
            assert!(
                !reason.trim().is_empty(),
                "{method} {pattern} is excluded without a reason.\nExcluded set:\n{listing}"
            );
        }
    }
}

/// Check 3: a `Handler` entry is not a promise nobody keeps. The action it
/// names must exist as a string literal in a route source, so a table entry
/// cannot name an action no handler writes.
#[test]
fn every_handler_action_is_a_string_literal_in_a_route_source() {
    let mut files = Vec::new();
    rust_files(&routes_dir(), &mut files);
    let sources: Vec<String> = files
        .iter()
        .map(|path| fs::read_to_string(path).expect("read a route file"))
        .collect();
    let mut absent = Vec::new();
    for (method, pattern, coverage) in AUDIT_TABLE {
        if let Coverage::Handler { action } = coverage {
            let literal = format!("\"{action}\"");
            if !sources.iter().any(|source| source.contains(&literal)) {
                absent.push(format!("{method} {pattern}: {action}"));
            }
        }
    }
    assert!(
        absent.is_empty(),
        "these Handler routes name an action that is not a string literal under src/routes/:\n{}",
        absent.join("\n")
    );
}

/// Check 4: a `Central` action is `module.verb`, no two `Central` routes (or
/// a `Central` and a `Handler` route) share one, and its `id_param` is a
/// path parameter of its own pattern, or a query-string name the table marks
/// as such.
///
/// The `module.verb` shape is asked of `Central` actions only: a `Handler`
/// action is whatever its handler has written since before this table (the
/// assistant's tool names such as `create_chart`, the map actions
/// `map_upload`, `map_rename` and `map_delete`), and renaming one would split
/// the history an auditor reads.
#[test]
fn central_actions_are_module_verb_unique_and_read_their_id_from_the_path_or_the_query() {
    let handler_actions: BTreeSet<&str> = AUDIT_TABLE
        .iter()
        .filter_map(|(_, _, coverage)| match coverage {
            Coverage::Handler { action } => Some(*action),
            _ => None,
        })
        .collect();
    let mut seen: BTreeMap<&str, String> = BTreeMap::new();
    for (method, pattern, coverage) in AUDIT_TABLE {
        let Coverage::Central {
            action,
            resource_kind,
            id_param,
        } = coverage
        else {
            continue;
        };
        let route = format!("{method} {pattern}");
        assert!(
            is_module_verb(action),
            "{route}: Central action {action:?} is not module.verb"
        );
        assert!(
            !resource_kind.is_empty(),
            "{route}: a Central row needs a resource kind"
        );
        assert!(
            !handler_actions.contains(action),
            "{route}: Central action {action:?} is also a Handler action"
        );
        if let Some(previous) = seen.insert(action, route.clone()) {
            panic!("{route}: Central action {action:?} is already {previous}'s");
        }
        // A path id must be a parameter of the route's own pattern; a query
        // id is marked explicitly, and cannot also be a path parameter (that
        // row would read a name the router never set).
        match id_param {
            IdParam::None => {}
            IdParam::Path(param) => assert!(
                pattern.contains(&format!("{{{param}}}")),
                "{route}: id_param {param:?} is not a path parameter of the pattern"
            ),
            IdParam::Query(param) => assert!(
                !pattern.contains(&format!("{{{param}}}")),
                "{route}: id_param {param:?} is marked as a query parameter but is a path \
                 parameter of the pattern"
            ),
        }
    }
}

// ── The central writer, through the router ─────────────────────────────────

/// One request against the router; the answer is the status and the JSON
/// body (`null` when there is none).
///
/// A client states the length of a body it sends in one piece, and the router
/// is called without a client here, so the length is declared by hand.
async fn send(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    content_type: Option<&str>,
    body: Option<String>,
) -> (StatusCode, Value) {
    send_declaring(app, method, path, cookie, content_type, body, true).await
}

/// [`send`], with the choice of whether the request declares a length (a
/// chunked body does not).
async fn send_declaring(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    content_type: Option<&str>,
    body: Option<String>,
    declare_length: bool,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(cookie) = cookie {
        builder = builder.header("cookie", cookie);
    }
    if let Some(content_type) = content_type {
        builder = builder.header("content-type", content_type);
    }
    if declare_length && let Some(body) = &body {
        builder = builder.header("content-length", body.len());
    }
    let request = builder
        .body(body.map_or_else(Body::empty, Body::from))
        .expect("build request");
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("the router never fails a request outright");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the response body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A JSON request.
async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    send(
        app,
        method,
        path,
        Some(cookie),
        body.as_ref().map(|_| "application/json"),
        body.map(|value| value.to_string()),
    )
    .await
}

async fn audit_event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit_event")
        .fetch_one(pool)
        .await
        .expect("count audit_event")
}

/// The one row of `action`, failing when there are none or several.
async fn the_row(pool: &PgPool, action: &str) -> Value {
    let mut rows = audit_rows(pool, action).await;
    assert_eq!(rows.len(), 1, "exactly one {action} row, got {rows:#?}");
    rows.remove(0)
}

/// Saves a query as the seeded Analyst and returns its id.
async fn save_a_query(app: &axum::Router, cookie: &str, title: &str) -> String {
    let (status, saved) = call(
        app,
        "POST",
        "/api/query/saved",
        cookie,
        Some(json!({"title": title, "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    saved["id"].as_str().expect("the saved query id").to_owned()
}

/// AC1: a saved query's update and delete each leave one row naming the
/// person, the action, the query and the field names, with the outcome of the
/// answer, and the handler still got the body the writer read.
#[tokio::test]
async fn updating_and_deleting_a_saved_query_each_write_one_row() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let id = save_a_query(&router, &cookie, "Revenue").await;

    let (status, patched) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &cookie,
        Some(json!({"title": "Revenue by region", "sql": "SELECT 2"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(
        patched["title"], "Revenue by region",
        "the handler received the body the writer had already read"
    );

    let update = the_row(&pool, "saved_query.update").await;
    assert_eq!(update["outcome"], "executed");
    assert_eq!(update["detail"], "200");
    assert_eq!(update["resource_kind"], "saved_query");
    assert_eq!(update["resource_id"], json!(id));
    assert_eq!(update["principal_kind"], "user");
    assert_eq!(update["actor_label"], "Rina Wijaya");
    assert_eq!(update["args"], json!({"fields": ["sql", "title"]}));
    assert!(
        !update.to_string().contains("Revenue by region"),
        "a row holds field names, never a value: {update}"
    );

    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{id}"),
        &cookie,
        None,
    )
    .await;
    assert!(status.is_success(), "{status}");
    let delete = the_row(&pool, "saved_query.delete").await;
    assert_eq!(delete["outcome"], "executed");
    assert_eq!(delete["resource_id"], json!(id));
    assert_eq!(
        delete["args"],
        json!({"fields": null}),
        "a request with no JSON body names no fields"
    );
}

/// AC2 and AC3 over a stack with no `ClickHouse`: a dashboard create and
/// delete, and an alert create, reach their handler and fail there. A change
/// that fails is still a row, with the outcome `failed`, so the trail shows
/// the attempt. The id of these routes travels in the body or the query, so
/// the row names the kind and the action and leaves the id empty.
#[tokio::test]
async fn dashboard_and_alert_changes_write_a_row_even_when_they_fail() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "dashboard:write,alert:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (status, _) = call(
        &router,
        "POST",
        "/api/dashboard/boards",
        &cookie,
        Some(json!({"name": "Sales"})),
    )
    .await;
    assert!(status.is_server_error(), "no ClickHouse: {status}");
    let create = the_row(&pool, "dashboard.create").await;
    assert_eq!(create["outcome"], "failed");
    assert_eq!(create["detail"], json!(status.as_u16().to_string()));
    assert_eq!(create["resource_kind"], "dashboard");
    assert_eq!(create["resource_id"], Value::Null);
    assert_eq!(create["args"], json!({"fields": ["name"]}));

    let (status, _) = call(
        &router,
        "DELETE",
        "/api/dashboard/boards?id=board-1",
        &cookie,
        None,
    )
    .await;
    assert!(status.is_server_error(), "no ClickHouse: {status}");
    let delete = the_row(&pool, "dashboard.delete").await;
    assert_eq!(delete["outcome"], "failed");
    assert_eq!(delete["resource_kind"], "dashboard");

    let (status, _) = call(
        &router,
        "POST",
        "/api/alerts",
        &cookie,
        Some(json!({"name": "Late load", "kind": "freshness"})),
    )
    .await;
    let alert = the_row(&pool, "alert.create").await;
    assert_eq!(alert["detail"], json!(status.as_u16().to_string()));
    assert_eq!(alert["resource_kind"], "alert");
    assert_eq!(alert["args"], json!({"fields": ["kind", "name"]}));
}

/// An object whose id travels in the query string is named on its row: the
/// value is percent-decoded, and a missing or empty `id` leaves the id null.
#[tokio::test]
async fn a_delete_names_the_object_whose_id_is_in_the_query_string() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "dashboard:write").await;
    let cookie = session_cookie_for_user(&pool, user).await;

    let (_, _) = call(
        &router,
        "DELETE",
        "/api/dashboard/boards?id=board%2042&other=ignored",
        &cookie,
        None,
    )
    .await;
    let (_, _) = call(&router, "DELETE", "/api/dashboard/boards", &cookie, None).await;
    let long = "b".repeat(300);
    let (_, _) = call(
        &router,
        "DELETE",
        &format!("/api/dashboard/boards?id={long}"),
        &cookie,
        None,
    )
    .await;

    let rows = audit_rows(&pool, "dashboard.delete").await;
    assert_eq!(rows.len(), 3, "{rows:#?}");
    let ids: Vec<&Value> = rows.iter().map(|row| &row["resource_id"]).collect();
    assert!(ids.contains(&&json!("board 42")), "decoded id: {ids:?}");
    assert!(ids.contains(&&Value::Null), "no id in the query: {ids:?}");
    assert!(
        ids.contains(&&json!("b".repeat(200))),
        "an id is cut at 200 characters: {ids:?}"
    );
}

/// The same id is named on the `refused` row the gate writes when it turns
/// the caller away before the handler runs.
#[tokio::test]
async fn a_refusal_at_the_gate_names_the_object_in_the_query_string() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, nobody).await;
    let (status, _) = call(
        &router,
        "DELETE",
        "/api/dashboard/boards?id=board-7",
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let row = the_row(&pool, "dashboard.delete").await;
    assert_eq!(row["outcome"], "refused");
    assert_eq!(row["resource_id"], json!("board-7"));
}

/// AC4: the query-limits save names the fields it carried and none of the
/// values, not a number and not the role it named.
#[tokio::test]
async fn the_query_limits_update_names_its_fields_and_holds_no_value() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let role: Uuid = sqlx::query_scalar(
        "SELECT r.id FROM role r JOIN app_user_role ur ON ur.role_id = r.id \
         WHERE ur.user_id = $1 LIMIT 1",
    )
    .bind(admin)
    .fetch_one(&pool)
    .await
    .unwrap();
    let (status, saved) = call(
        &router,
        "PUT",
        "/api/settings/limits",
        &cookie,
        Some(json!({
            "install": {"timeoutSeconds": 30, "chartRows": 500, "mapPoints": 1000, "studioRows": 4000},
            "roles": [{"roleId": role, "timeoutSeconds": 300}]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{saved}");

    let row = the_row(&pool, "settings.limits.update").await;
    assert_eq!(row["outcome"], "executed");
    assert_eq!(row["resource_kind"], "settings");
    assert_eq!(row["args"], json!({"fields": ["install", "roles"]}));
    assert!(
        !row.to_string().contains(&role.to_string()),
        "the role id is a value of the body and is not in the row: {row}"
    );
    assert!(
        !row["args"].to_string().contains("timeoutSeconds"),
        "only top-level names are kept: {row}"
    );
}

/// AC5: creating a role leaves a row from the central writer.
#[tokio::test]
async fn creating_a_role_writes_a_row_that_names_fields_and_no_value() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "identity:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "POST",
        "/api/identity/roles",
        &cookie,
        Some(json!({"name": "Auditor-9d1c", "permissions": "query:read", "description": "x"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let row = the_row(&pool, "identity.role.create").await;
    assert_eq!(row["outcome"], "executed");
    assert_eq!(row["detail"], "201");
    assert_eq!(row["resource_kind"], "role");
    assert_eq!(row["principal_id"], json!(admin.to_string()));
    assert_eq!(
        row["args"],
        json!({"fields": ["description", "name", "permissions"]})
    );
    assert!(
        !row.to_string().contains("Auditor-9d1c"),
        "the new role's name is a value: {row}"
    );
}

/// AC6 on a `Central` route: the gate turns a caller without the permission
/// away, the handler never runs, and the central writer is never reached, so
/// the gate writes the one `refused` row, with the missing permission.
#[tokio::test]
async fn a_refusal_at_the_gate_on_a_central_route_writes_one_refused_row() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, nobody).await;
    let (status, _) = call(
        &router,
        "POST",
        "/api/identity/roles",
        &cookie,
        Some(json!({"name": "Sneaky", "permissions": "*:*"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let row = the_row(&pool, "identity.role.create").await;
    assert_eq!(row["outcome"], "refused");
    assert_eq!(row["detail"], "identity:write");
    assert_eq!(row["principal_id"], json!(nobody.to_string()));
    assert_eq!(row["args"], json!({}));
    assert!(
        !row.to_string().contains("Sneaky"),
        "the gate row is built from the route, not from the body: {row}"
    );
    let roles: i64 = sqlx::query_scalar("SELECT count(*) FROM role WHERE name = 'Sneaky'")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(roles, 0, "the refused change did not happen");
}

/// AC6 on a `Handler` route: the handler never ran either, so nothing else
/// would have written the refusal; the gate writes it under the route's own
/// action.
#[tokio::test]
async fn a_refusal_at_the_gate_on_a_handler_route_writes_one_refused_row() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, nobody).await;
    let role = Uuid::new_v4();
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/identity/roles/{role}"),
        &cookie,
        Some(json!({"permissions": "*:*"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let row = the_row(&pool, "identity.role.update").await;
    assert_eq!(row["outcome"], "refused");
    assert_eq!(row["detail"], "identity:write");
    assert_eq!(row["principal_id"], json!(nobody.to_string()));
}

/// The assistant's routes write one row per tool call under the tool's own
/// name, so the name the table gives them is only the label of a refusal at
/// the gate. It must not be a tool's name: a refused request to the chat or
/// tool route would read as a refused `create_chart`.
#[tokio::test]
async fn a_refusal_at_the_gate_on_the_assistant_routes_is_named_for_the_route() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, nobody).await;
    for (path, action) in [
        ("/api/ai/tool", "assistant.tool"),
        ("/api/ai/chat", "assistant.chat"),
    ] {
        let (status, _) = call(
            &router,
            "POST",
            path,
            &cookie,
            Some(json!({"tool": "create_chart"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}");
        let row = the_row(&pool, action).await;
        assert_eq!(row["outcome"], "refused", "{path}");
        assert_eq!(row["principal_id"], json!(nobody.to_string()));
    }
    assert!(
        audit_rows(&pool, "create_chart").await.is_empty(),
        "a refused route is not a refused create_chart"
    );
}

/// A caller that has to rotate its password is turned away from every route
/// but three; that refusal is audited under the route's action, with the
/// sentence the gate wrote.
#[tokio::test]
async fn a_refusal_for_a_pending_password_rotation_is_audited() {
    let TestApp { router, pool } = spin_up().await;
    let user = create_principal_with_permissions(&pool, "query:read").await;
    lakehouse_auth::password::create_local_identity(
        &pool,
        user,
        &lakehouse_auth::Secret::new("Boots7rapPassw0rd!"),
        true,
    )
    .await
    .expect("give the user a local identity whose password is due");
    let cookie = session_cookie_for_user(&pool, user).await;
    let (status, _) = call(
        &router,
        "POST",
        "/api/query/saved",
        &cookie,
        Some(json!({"title": "t", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let row = the_row(&pool, "saved_query.create").await;
    assert_eq!(row["outcome"], "refused");
    assert_eq!(
        row["detail"],
        "password rotation required before this route is reachable"
    );
}

/// A change that finds nothing answers 404; the central writer records the
/// attempt as `refused`, with the id the person named.
#[tokio::test]
async fn a_404_on_a_central_delete_is_a_refused_row() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let missing = Uuid::new_v4();
    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{missing}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let row = the_row(&pool, "saved_query.delete").await;
    assert_eq!(row["outcome"], "refused");
    assert_eq!(row["detail"], "404");
    assert_eq!(row["resource_id"], json!(missing.to_string()));
}

/// Nothing is recorded for a request nobody was signed in for (401), for an
/// address no route serves (404) or for a method a route does not serve (405):
/// none of them reaches a route or has a principal.
#[tokio::test]
async fn a_401_a_404_and_a_405_write_nothing() {
    let TestApp { router, pool } = spin_up().await;
    let before = audit_event_count(&pool).await;

    let (status, _) = send(
        &router,
        "POST",
        "/api/identity/roles",
        None,
        Some("application/json"),
        Some(json!({"name": "x"}).to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _) = send(&router, "POST", "/api/no-such-route", None, None, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(&router, "TRACE", "/api/identity/roles", None, None, None).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);

    assert_eq!(audit_event_count(&pool).await, before);
}

/// A body that is not a JSON object, not JSON, or not declared as JSON is
/// passed through unread; the row says `{"fields": null}` and never holds the
/// bytes.
#[tokio::test]
async fn a_body_that_is_not_a_json_object_is_not_read() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (_, _) = send(
        &router,
        "POST",
        "/api/query/saved",
        Some(&cookie),
        Some("application/json"),
        Some(r#"["distinctive-array-value"]"#.to_owned()),
    )
    .await;
    let (_, _) = send(
        &router,
        "POST",
        "/api/query/saved",
        Some(&cookie),
        Some("text/plain"),
        Some(r#"{"title": "distinctive-text-value"}"#.to_owned()),
    )
    .await;
    let rows = audit_rows(&pool, "saved_query.create").await;
    assert_eq!(rows.len(), 2, "{rows:#?}");
    for row in &rows {
        assert_eq!(row["args"], json!({"fields": null}), "{row}");
        assert!(!row.to_string().contains("distinctive"), "{row}");
    }
}

/// A body larger than the limit the writer reads is not held: the handler is
/// handed the request as it came, and the row says `{"fields": null}`.
#[tokio::test]
async fn a_body_above_the_read_limit_is_passed_through_unread() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let padding = "x".repeat(lakehouse_api::audit_coverage::MAX_BODY_PEEK_BYTES + 1);
    let (_, _) = send(
        &router,
        "POST",
        "/api/query/saved",
        Some(&cookie),
        Some("application/json"),
        Some(json!({"title": "big", "sql": padding}).to_string()),
    )
    .await;
    let row = the_row(&pool, "saved_query.create").await;
    assert_eq!(row["args"], json!({"fields": null}));
}

/// A body sent in chunks declares no length, so it is not read; the handler
/// still gets it whole.
#[tokio::test]
async fn a_body_with_no_declared_length_is_not_read_and_still_reaches_the_handler() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let (status, saved) = send_declaring(
        &router,
        "POST",
        "/api/query/saved",
        Some(&cookie),
        Some("application/json"),
        Some(json!({"title": "Chunked", "sql": "SELECT 1"}).to_string()),
        false,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["title"], "Chunked");
    let row = the_row(&pool, "saved_query.create").await;
    assert_eq!(row["args"], json!({"fields": null}));
    assert_eq!(row["outcome"], "executed");
}

// ── Old and new values of a policy change ───────────────────────────────────

/// Authors a policy that binds no table, so its rows sit on the policy
/// itself, and returns its id.
async fn author_policy(app: &axum::Router, cookie: &str, name: &str, activate: bool) -> String {
    let (status, created) = call(
        app,
        "POST",
        "/api/governance/policies",
        cookie,
        Some(json!({
            "name": name,
            "kind": "Agent autonomy",
            "subjects": "Every digital employee",
            "resources": "Every tool",
            "effect": "Permit with obligation",
            "activate": activate,
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    created["id"].as_str().expect("the policy id").to_owned()
}

/// A policy change names what it changed from and to: creating one records
/// the definition it created (also for a policy that binds no table, which
/// used to leave no row), switching it on and off records both statuses, and
/// deleting it records what it was.
#[tokio::test]
async fn a_policy_change_records_the_old_and_the_new_value() {
    let TestApp { router, pool } = spin_up().await;
    let admin = create_principal_with_permissions(&pool, "policy:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    let id = author_policy(&router, &cookie, "Hold writes at night", false).await;
    let create = the_row(&pool, "policy.create").await;
    assert_eq!(create["resource_kind"], "policy");
    assert_eq!(create["resource_id"], json!(id));
    assert_eq!(create["args"]["name"], "Hold writes at night");
    assert_eq!(create["args"]["newStatus"], "draft");
    assert_eq!(create["args"]["kind"], "Agent autonomy");
    assert_eq!(create["args"]["effect"], "Permit with obligation");

    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/governance/policies/{id}/status"),
        &cookie,
        Some(json!({"status": "ready"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let enforce = the_row(&pool, "policy.enforce").await;
    assert_eq!(enforce["args"]["oldStatus"], "draft");
    assert_eq!(enforce["args"]["newStatus"], "ready");

    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/governance/policies/{id}/status"),
        &cookie,
        Some(json!({"status": "draft"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let suspend = the_row(&pool, "policy.suspend").await;
    assert_eq!(suspend["args"]["oldStatus"], "ready");
    assert_eq!(suspend["args"]["newStatus"], "draft");

    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/governance/policies/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let delete = the_row(&pool, "policy.delete").await;
    assert_eq!(delete["args"]["name"], "Hold writes at night");
    assert_eq!(delete["args"]["oldStatus"], "draft");
    assert_eq!(delete["args"]["kind"], "Agent autonomy");
    assert_eq!(delete["args"]["effect"], "Permit with obligation");
}
