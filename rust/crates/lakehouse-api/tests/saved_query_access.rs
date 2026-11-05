//! The saved-query and folder routes (`QS-3`): the acceptance checklist's
//! AC2, AC3, AC4 and AC7 as route tests, plus the rules every one of the
//! routes has to hold — a 404 for what the caller may not see, a 403 with
//! the fixed sentence for what it may see but not change, a 422 for a
//! statement that is not read-only and a 409 for a folder that still holds
//! something.
//!
//! The sharing routes (a rule on a query or on a folder) are a slice of
//! their own. Where this file needs a query another person may see, it
//! writes the rule row itself, so the read and the write rules are proved
//! without waiting for that slice.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, session_cookie_for_seeded_user, session_cookie_for_user, spin_up};

/// Meridian Group, the tenant the seeded Analyst and the seeded Data
/// Scientist both act for.
const GROUP_TENANT: &str = "11111111-1111-4111-8111-000000000001";

/// The sentence a caller that may not see a saved query is told. The same
/// sentence as for a query that does not exist, so the answer never says
/// which it was.
const QUERY_NOT_FOUND: &str = "That saved query could not be found.";

/// The sentence a caller that may not see a folder is told.
const FOLDER_NOT_FOUND: &str = "That folder could not be found.";

/// The sentence a caller that may not change a saved query is told.
const QUERY_FORBIDDEN: &str = "You do not have permission to change this saved query.";

/// The sentence a caller that may not change a folder is told.
const FOLDER_FORBIDDEN: &str = "You do not have permission to change this folder.";

/// The sentence a move into or out of a personal space at a level below
/// manage is refused with — the same sentence for a query and for a
/// folder, so the two moves answer identically (`QS-3` review finding 1).
const MOVE_ACROSS_SPACES_FORBIDDEN: &str =
    "Only someone who manages this can move it into or out of a personal space.";

/// The sentence a folder that still holds queries is refused with.
const FOLDER_HOLDS_QUERIES: &str =
    "This folder still holds queries, some of them in the trash. Move or restore them first.";

/// The sentence a folder that still holds folders is refused with.
const FOLDER_HOLDS_FOLDERS: &str =
    "This folder still holds other folders. Move or delete them first.";

/// The sentence a taken folder name is refused with.
const FOLDER_DUPLICATE_NAME: &str = "A folder with that name already exists here.";

/// The sentence a bad folder name is refused with.
const FOLDER_INVALID_NAME: &str = "A folder name must be between 1 and 100 characters.";

/// The sentence a too-deep tree is refused with.
const FOLDER_TOO_DEEP: &str = "A folder tree cannot be deeper than four levels.";

/// The sentence a cycle is refused with.
const FOLDER_CYCLE: &str = "A folder cannot be moved into itself.";

/// The sentence a statement that is not read-only is refused with.
const NOT_READ_ONLY: &str = "only read queries (SELECT/SHOW/DESCRIBE/EXPLAIN) can be saved";

/// The sentence a query string the route cannot read is told.
const QUERY_STRING_UNREADABLE: &str = "The query string could not be read.";

/// The sentence a folder reference that is neither an id nor the root is
/// refused with.
const FOLDER_ID_INVALID: &str = "A folder must be an id or \"root\".";

/// The sentence a body of another shape is refused with.
const ACCESS_BODY_SHAPE: &str = "Body must be JSON {rules: [{userId?, roleId?, level}]}.";

/// The sentence a rule that names two grantees or none, or a level
/// outside the set, is refused with.
const ACCESS_RULE_SHAPE: &str =
    "Each rule must name one person or one role, and one of the levels view, edit or manage.";

/// The sentence a grantee that is listed twice is refused with.
const ACCESS_GRANTEE_TWICE: &str = "The same person or role is listed twice.";

/// The sentence a grantee that does not exist, or a person outside the
/// organisation, is refused with.
const ACCESS_GRANTEE_OUTSIDE: &str =
    "A saved query can only be shared with a person or a role of your organisation.";

/// One request against the router, with `cookie` as the session.
///
/// The answer is the status and the JSON body (a 204 has no body, so it
/// reads as `null`).
async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = match body {
        Some(body) => Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", cookie)
            .header("content-type", "application/json")
            .body(Body::from(
                serde_json::to_vec(&body).expect("serialize the request body"),
            ))
            .expect("build request"),
        None => Request::builder()
            .method(method)
            .uri(path)
            .header("cookie", cookie)
            .body(Body::empty())
            .expect("build request"),
    };
    let resp = app
        .clone()
        .oneshot(request)
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the response body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// The error text of an answer.
fn error(body: &Value) -> &str {
    body.get("error")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// One request against the router with a body that is not JSON at all.
///
/// The shape of the body is the route's own business, so a body that is
/// not JSON has to be answered with the route's own sentence rather than
/// with the reader's own text.
async fn call_with_raw_body(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: &'static str,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the response body");
    let value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, value)
}

/// The id of a seeded user.
async fn user_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM app_user WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("the seeded user exists")
}

/// A saved query that holds a statement the read-only guard refuses.
///
/// The guard is on every path that stores SQL, so no route can produce
/// such a row: it is written here, the way a row written before the guard
/// existed would be.
async fn write_a_query_that_cannot_be_saved(pool: &PgPool, owner: Uuid, title: &str) -> String {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO saved_query (title, sql, owner, owner_id, tenant_id) \
         VALUES ($1, 'DROP TABLE serving.mart_wisman', 'Rina Wijaya', $2, $3) \
         RETURNING id",
    )
    .bind(title)
    .bind(owner)
    .bind(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id"))
    .fetch_one(pool)
    .await
    .expect("insert the query");
    id.to_string()
}

/// AC2: every change of the SQL or the title keeps a version, and going
/// back to version 3 writes a new version on top of the history.
#[tokio::test]
async fn a_query_keeps_every_version_and_can_go_back_to_one() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &cookie,
        Some(json!({"title": "Revenue", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    for (step, sql) in ["SELECT 2", "SELECT 3", "SELECT 4", "SELECT 5"]
        .iter()
        .enumerate()
    {
        let (status, patched) = call(
            &router,
            "PATCH",
            &format!("/api/query/saved/{id}"),
            &cookie,
            Some(json!({"sql": sql})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "step {step}: {patched}");
    }

    let (status, versions) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}/versions"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    let versions = versions.as_array().expect("the versions are an array");
    assert_eq!(versions.len(), 5, "the create and the four changes");
    assert_eq!(versions[0]["version"], 5, "the newest is first");

    let (status, reverted) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/versions/3/revert"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reverted}");
    assert_eq!(
        reverted["sql"], "SELECT 3",
        "the query holds the SQL of version 3 again"
    );

    let (status, versions) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}/versions"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let versions = versions.as_array().expect("the versions are an array");
    assert_eq!(versions[0]["label"], "Restored version 3");
    assert_eq!(versions[0]["sql"], "SELECT 3");
}

/// AC3: a deleted query waits in the trash with the days it has left, and
/// comes back in the folder it was in, with its history.
#[tokio::test]
async fn a_deleted_query_waits_in_the_trash_and_comes_back_in_its_folder() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Reports"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_id = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &cookie,
        Some(json!({"title": "Weekly", "sql": "SELECT 1", "folderId": folder_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    assert_eq!(saved["folderId"], folder_id, "it sits in the folder");

    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, listed) = call(&router, "GET", "/api/query/saved", &cookie, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !listed
            .as_array()
            .expect("the list is an array")
            .iter()
            .any(|query| query["id"] == id),
        "a trashed query is not listed"
    );

    let (status, trash) = call(&router, "GET", "/api/query/saved-trash", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{trash}");
    let entry = trash
        .as_array()
        .expect("the trash is an array")
        .iter()
        .find(|entry| entry["id"] == id)
        .expect("the trash holds the query");
    assert_eq!(entry["folderId"], folder_id);
    assert_eq!(entry["daysLeft"], 29, "a fresh entry has 29 days left");

    let (status, restored) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/restore"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        restored["folderId"], folder_id,
        "it comes back in its folder"
    );

    let (status, versions) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}/versions"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        versions
            .as_array()
            .expect("the versions are an array")
            .len(),
        1,
        "its history is still there"
    );
}

/// AC4: a query that was saved without a folder sits in the author's
/// personal space, and another person of the same organisation does not
/// find it — neither in the search nor by its id, nor the folder it is in.
#[tokio::test]
async fn a_query_in_a_personal_space_is_not_found_by_another_person() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    assert!(
        saved["folderId"].is_string(),
        "a query saved without a folder sits in the author's personal space"
    );

    let (status, found) = call(&router, "GET", "/api/query/saved?q=Private", &reza, None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert!(
        found.as_array().expect("the list is an array").is_empty(),
        "the other person does not find it: {found}"
    );

    let (status, body) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error(&body), QUERY_NOT_FOUND);

    let (status, folders) = call(&router, "GET", "/api/query/folders", &reza, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !folders
            .as_array()
            .expect("the list is an array")
            .iter()
            .any(|folder| folder["isPersonal"] == true),
        "another person's personal space is not in the tree: {folders}"
    );
}

/// AC7: a person without `query:read` is refused every route of this
/// surface, with the permission named and nothing written.
#[tokio::test]
async fn the_saved_query_routes_need_the_query_read_permission() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = common::create_zero_permission_principal(&pool).await;
    let cookie = common::session_cookie_for_user(&pool, nobody).await;

    for (method, path) in [
        ("GET", "/api/query/saved"),
        ("POST", "/api/query/saved"),
        (
            "GET",
            "/api/query/saved/11111111-1111-4111-8111-000000000009",
        ),
        (
            "PATCH",
            "/api/query/saved/11111111-1111-4111-8111-000000000009",
        ),
        (
            "DELETE",
            "/api/query/saved/11111111-1111-4111-8111-000000000009",
        ),
        (
            "POST",
            "/api/query/saved/11111111-1111-4111-8111-000000000009/duplicate",
        ),
        (
            "POST",
            "/api/query/saved/11111111-1111-4111-8111-000000000009/restore",
        ),
        ("GET", "/api/query/saved-trash"),
        (
            "GET",
            "/api/query/saved/11111111-1111-4111-8111-000000000009/versions",
        ),
        (
            "POST",
            "/api/query/saved/11111111-1111-4111-8111-000000000009/versions/1/revert",
        ),
        ("GET", "/api/query/folders"),
        ("POST", "/api/query/folders"),
        (
            "PATCH",
            "/api/query/folders/11111111-1111-4111-8111-000000000009",
        ),
        (
            "DELETE",
            "/api/query/folders/11111111-1111-4111-8111-000000000009",
        ),
        (
            "GET",
            "/api/query/saved/11111111-1111-4111-8111-000000000009/access",
        ),
        (
            "PUT",
            "/api/query/saved/11111111-1111-4111-8111-000000000009/access",
        ),
        (
            "GET",
            "/api/query/folders/11111111-1111-4111-8111-000000000009/access",
        ),
        (
            "PUT",
            "/api/query/folders/11111111-1111-4111-8111-000000000009/access",
        ),
        ("GET", "/api/query/grantees"),
    ] {
        let (status, body) = call(&router, method, path, &cookie, None).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {body}");
        assert!(
            error(&body).contains("query:read"),
            "{method} {path}: the refusal names the permission: {body}"
        );
    }
}

/// A saved query the caller may not see is a 404 with the fixed sentence,
/// on every route that answers about one.
#[tokio::test]
async fn an_unseen_saved_query_is_not_found_on_every_route() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let unseen = Uuid::new_v4().to_string();

    for (method, path) in [
        ("GET", format!("/api/query/saved/{unseen}")),
        ("PATCH", format!("/api/query/saved/{unseen}")),
        ("DELETE", format!("/api/query/saved/{unseen}")),
        ("POST", format!("/api/query/saved/{unseen}/duplicate")),
        ("POST", format!("/api/query/saved/{unseen}/restore")),
        ("GET", format!("/api/query/saved/{unseen}/versions")),
        (
            "POST",
            format!("/api/query/saved/{unseen}/versions/1/revert"),
        ),
    ] {
        let (status, body) = call(&router, method, &path, &cookie, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {body}");
        assert_eq!(error(&body), QUERY_NOT_FOUND, "{method} {path}");
    }
}

/// A folder the caller may not see is a 404 with the fixed sentence.
#[tokio::test]
async fn an_unseen_folder_is_not_found_on_every_route() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let unseen = Uuid::new_v4().to_string();

    for (method, path) in [
        ("PATCH", format!("/api/query/folders/{unseen}")),
        ("DELETE", format!("/api/query/folders/{unseen}")),
    ] {
        let (status, body) = call(&router, method, &path, &cookie, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {body}");
        assert_eq!(error(&body), FOLDER_NOT_FOUND, "{method} {path}");
    }
}

/// A query another person may only see is refused with the fixed sentence
/// on every route that changes it, and the answer never says the query
/// does not exist.
#[tokio::test]
async fn a_query_may_only_be_viewed_is_refused_with_the_fixed_sentence() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    // `folderId: null` is the shared space, which every member of the
    // organisation sees at `view`.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Shared", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    assert!(saved["folderId"].is_null(), "it sits in the shared space");

    let (status, seen) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["level"], "view", "the other person may only view it");

    for (method, path, body) in [
        (
            "PATCH",
            format!("/api/query/saved/{id}"),
            Some(json!({"title": "Renamed"})),
        ),
        ("DELETE", format!("/api/query/saved/{id}"), None),
        (
            "POST",
            format!("/api/query/saved/{id}/versions/1/revert"),
            None,
        ),
    ] {
        let (status, answer) = call(&router, method, &path, &reza, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
        assert_eq!(
            error(&answer),
            format!("permission_denied: {QUERY_FORBIDDEN}"),
            "{method} {path}"
        );
    }

    // Reading is allowed at `view`: the history is part of seeing it.
    let (status, versions) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}/versions"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
}

/// Every path that stores SQL goes through the same read-only guard the
/// save route uses.
#[tokio::test]
async fn a_statement_that_is_not_read_only_is_refused_everywhere_it_could_be_stored() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let owner = user_id(&pool, "rina@meridian.example").await;
    let id = write_a_query_that_cannot_be_saved(&pool, owner, "Doomed").await;

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &cookie,
        Some(json!({"sql": "DROP TABLE serving.mart_wisman"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(error(&body), NOT_READ_ONLY);

    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/duplicate"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(error(&body), NOT_READ_ONLY);

    // A version that holds a statement the guard refuses is refused again
    // when the query goes back to it.
    sqlx::query(
        "INSERT INTO saved_query_version (query_id, version, title, sql, author_name, label) \
         VALUES ($1, 2, 'Doomed', 'DROP TABLE serving.mart_wisman', 'Rina Wijaya', '')",
    )
    .bind(Uuid::parse_str(&id).expect("the query id"))
    .execute(&pool)
    .await
    .expect("write the version");
    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/versions/2/revert"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert_eq!(error(&body), NOT_READ_ONLY);
}

/// The move rules: a query moves into a folder the mover may edit, back
/// to the shared space, and nowhere else.
#[tokio::test]
async fn a_query_moves_into_a_folder_the_mover_may_edit_and_nowhere_else() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &rina,
        Some(json!({"name": "Mine"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let mine = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    // A folder of another person: visible at `view`, never at `edit`.
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "Theirs"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let theirs = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Movable", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, moved) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &rina,
        Some(json!({"folderId": mine})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(moved["folderId"], mine);

    let (status, moved) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &rina,
        Some(json!({"folderId": "root"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert!(moved["folderId"].is_null(), "the shared space is the root");

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &rina,
        Some(json!({"folderId": theirs})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {QUERY_FORBIDDEN}")
    );

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &rina,
        Some(json!({"folderId": Uuid::new_v4().to_string()})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(error(&body), FOLDER_NOT_FOUND);

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &rina,
        Some(json!({"folderId": "not-a-folder"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), FOLDER_ID_INVALID);
}

/// The name rules: a name holds between one and a hundred characters,
/// and no two folders in the same place share a name.
#[tokio::test]
async fn a_folder_name_is_between_one_and_a_hundred_characters_and_free_in_its_place() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, body) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "  "})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), FOLDER_INVALID_NAME);

    let (status, body) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "x".repeat(101)})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), FOLDER_INVALID_NAME);

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Finance"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");

    let (status, body) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "fInAnCe"})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(error(&body), FOLDER_DUPLICATE_NAME);
}

/// The tree rules: a tree is at most four levels deep, and a folder may
/// not become its own ancestor.
#[tokio::test]
async fn a_folder_tree_is_at_most_four_levels_deep_and_never_a_cycle() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Finance"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let finance = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    // The tree is at most four levels deep.
    let mut parent = finance.clone();
    for depth in 2..=4 {
        let (status, folder) = call(
            &router,
            "POST",
            "/api/query/folders",
            &cookie,
            Some(json!({"name": format!("Level {depth}"), "parentId": parent})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "depth {depth}: {folder}");
        parent = folder["id"]
            .as_str()
            .expect("the folder has an id")
            .to_owned();
    }
    let (status, body) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Level 5", "parentId": parent})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), FOLDER_TOO_DEEP);

    // A folder may not become its own ancestor.
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{finance}"),
        &cookie,
        Some(json!({"parentId": parent})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), FOLDER_CYCLE);
}

/// The personal space is nobody's to rename, move or delete.
#[tokio::test]
async fn a_personal_space_may_not_be_renamed_moved_or_deleted() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &cookie,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let personal = saved["folderId"]
        .as_str()
        .expect("the query sits in the personal space");
    for (method, body) in [
        ("PATCH", Some(json!({"name": "Renamed"}))),
        ("PATCH", Some(json!({"parentId": null}))),
        ("DELETE", None),
    ] {
        let (status, answer) = call(
            &router,
            method,
            &format!("/api/query/folders/{personal}"),
            &cookie,
            body,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}: {answer}");
        assert_eq!(
            error(&answer),
            format!("permission_denied: {FOLDER_FORBIDDEN}")
        );
    }
}

/// A folder that holds a query is not deleted, and not when the query
/// waits in the trash.
#[tokio::test]
async fn a_folder_that_holds_a_query_is_not_deleted() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Full"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let full = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &cookie,
        Some(json!({"title": "Inside", "sql": "SELECT 1", "folderId": full})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, body) = call(
        &router,
        "DELETE",
        &format!("/api/query/folders/{full}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(error(&body), FOLDER_HOLDS_QUERIES);

    // The query goes to the trash: the folder still holds it.
    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, body) = call(
        &router,
        "DELETE",
        &format!("/api/query/folders/{full}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(error(&body), FOLDER_HOLDS_QUERIES);

    // With the query restored out of the way, the folder holds nothing.
    let (status, _) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/restore"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    sqlx::query("UPDATE saved_query SET folder_id = NULL WHERE id = $1")
        .bind(Uuid::parse_str(&id).expect("the query id"))
        .execute(&pool)
        .await
        .expect("move the query out of the folder");
}

/// A folder that holds another folder is not deleted either, and the
/// parent goes once the child is gone.
#[tokio::test]
async fn a_folder_that_holds_another_folder_is_not_deleted() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Parent"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let parent = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Child", "parentId": parent})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let child = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, body) = call(
        &router,
        "DELETE",
        &format!("/api/query/folders/{parent}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(error(&body), FOLDER_HOLDS_FOLDERS);

    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/folders/{child}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/folders/{parent}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// The tree a caller sees holds only the folders they may see, and the
/// trash holds only the queries they may manage.
#[tokio::test]
async fn the_tree_and_the_trash_show_only_what_the_caller_may_see() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &rina,
        Some(json!({"name": "Rina's"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");

    let (status, tree) = call(&router, "GET", "/api/query/folders", &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    let tree = tree.as_array().expect("the list is an array");
    assert!(
        tree.iter().any(|folder| folder["name"] == "Rina's"),
        "her own folder is in her tree: {tree:?}"
    );
    assert!(
        tree.iter().all(|folder| folder["level"] == "manage"
            || folder["level"] == "view"
            || folder["level"] == "edit"),
        "every folder carries the level the caller holds on it: {tree:?}"
    );

    let (status, tree) = call(&router, "GET", "/api/query/folders", &reza, None).await;
    assert_eq!(status, StatusCode::OK);
    let tree = tree.as_array().expect("the list is an array");
    let hers = tree
        .iter()
        .find(|folder| folder["name"] == "Rina's")
        .unwrap_or_else(|| panic!("the shared tree holds her folder: {tree:?}"));
    assert_eq!(
        hers["level"], "view",
        "an ordinary folder of the shared tree is everybody's to see: {hers:?}"
    );

    // The trash holds what the caller may manage, and only that.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Rina's trash", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{id}"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, trash) = call(&router, "GET", "/api/query/saved-trash", &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{trash}");
    assert!(
        trash
            .as_array()
            .expect("the trash is an array")
            .iter()
            .any(|entry| entry["id"] == id),
        "she sees her own trash: {trash}"
    );

    let (status, trash) = call(&router, "GET", "/api/query/saved-trash", &reza, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        trash.as_array().expect("the trash is an array").is_empty(),
        "another person's trash is not hers to see: {trash}"
    );

    // The personal space of another person is a wall: it is not in the
    // tree the other person sees, even though its owner sees it.
    let (status, tree) = call(&router, "GET", "/api/query/folders", &reza, None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        tree.as_array()
            .expect("the list is an array")
            .iter()
            .all(|folder| folder["isPersonal"] != true),
        "another person's personal space is not in the tree: {tree}"
    );
}

/// The list is filtered by the text typed and by the folder asked for.
#[tokio::test]
async fn the_list_is_filtered_by_text_and_by_folder() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Filter"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_id = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    for (title, sql, folder) in [
        (
            "Revenue",
            "SELECT gold_amount FROM serving.mart_revenue",
            Value::String(folder_id.clone()),
        ),
        (
            "Headcount",
            "SELECT count() FROM serving.mart_people",
            Value::Null,
        ),
    ] {
        let (status, saved) = call(
            &router,
            "POST",
            "/api/query/saved",
            &cookie,
            Some(json!({"title": title, "sql": sql, "folderId": folder})),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{saved}");
    }

    let (status, found) = call(&router, "GET", "/api/query/saved?q=revenue", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let found = found.as_array().expect("the list is an array");
    assert_eq!(found.len(), 1, "the title matches: {found:?}");
    assert_eq!(found[0]["title"], "Revenue");

    let (status, found) = call(
        &router,
        "GET",
        "/api/query/saved?q=mart_people",
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let found = found.as_array().expect("the list is an array");
    assert_eq!(found.len(), 1, "the SQL matches too: {found:?}");
    assert_eq!(found[0]["title"], "Headcount");

    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved?folder={folder_id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let found = found.as_array().expect("the list is an array");
    assert_eq!(found.len(), 1, "the folder filter: {found:?}");
    assert_eq!(found[0]["title"], "Revenue");

    let (status, found) = call(
        &router,
        "GET",
        "/api/query/saved?folder=root",
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let found = found.as_array().expect("the list is an array");
    assert_eq!(found.len(), 1, "the shared space: {found:?}");
    assert_eq!(found[0]["title"], "Headcount");

    let (status, body) = call(
        &router,
        "GET",
        "/api/query/saved?folder=nonsense",
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), FOLDER_ID_INVALID);
}

/// A query string the route cannot read is refused with the fixed
/// sentence, never with the reader's own text.
#[tokio::test]
async fn a_query_string_the_route_cannot_read_is_refused_with_the_fixed_sentence() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let resp = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/query/saved?q=a&q=b")
                .header("cookie", &cookie)
                .body(Body::empty())
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let bytes = to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("read the response body");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");
    assert_eq!(error(&body), QUERY_STRING_UNREADABLE);
}

/// A new query lands in the author's personal space unless the body names
/// another place, and a duplicate lands in the caller's own personal space.
#[tokio::test]
async fn a_new_query_and_a_duplicate_land_in_the_callers_own_space() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Default", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert!(
        saved["folderId"].is_string(),
        "without a folder it lands in the personal space: {saved}"
    );

    let (status, shared) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Shared", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{shared}");
    assert!(
        shared["folderId"].is_null(),
        "an explicit null is the shared space: {shared}"
    );
    let shared_id = shared["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, copy) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{shared_id}/duplicate"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{copy}");
    assert_eq!(copy["title"], "Shared", "the copy holds the same title");
    assert_eq!(copy["sql"], "SELECT 1", "and the same SQL");
    assert!(
        copy["folderId"].is_string(),
        "the copy lands in the caller's own personal space: {copy}"
    );
    assert_ne!(copy["id"], shared_id, "it is a new query");
}

// ── The access lists and the grantee search ─────────────────────────────

/// A rule opens a saved query to another person, at the level it names.
///
/// A query saved without a folder lands in the author's personal space,
/// which is a wall: nobody else sees it at all until a rule says so.
#[tokio::test]
async fn a_rule_opens_a_saved_query_to_another_person_at_the_level_it_names() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, body) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the personal space is a wall: {body}"
    );

    let (status, list) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["itemType"], "query");
    assert_eq!(list["itemId"], id);
    assert_eq!(list["yourLevel"], "manage");
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the list holds the one rule: {list}");
    assert_eq!(rules[0]["userId"], reza_id.to_string());
    assert_eq!(rules[0]["roleId"], Value::Null);
    assert_eq!(rules[0]["level"], "edit");
    assert!(
        rules[0]["grantedAt"]
            .as_str()
            .expect("the rule carries the moment it was written")
            .ends_with('Z'),
        "the rule carries the moment it was written: {list}"
    );
    assert!(
        list["inherited"]
            .as_array()
            .expect("the inherited list is an array")
            .is_empty(),
        "the personal space carries no rule of its own: {list}"
    );

    let (status, seen) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["level"], "edit", "the rule names the level: {seen}");
}

/// A rule on a folder covers everything inside it, and the access list of
/// a query names the folder it inherits from.
#[tokio::test]
async fn a_rule_on_a_folder_covers_the_query_inside_it() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &rina,
        Some(json!({"name": "Reports"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_id = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Weekly", "sql": "SELECT 1", "folderId": folder_id})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, list) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{folder_id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["itemType"], "folder");
    assert_eq!(list["itemId"], folder_id);
    assert_eq!(list["yourLevel"], "manage");
    // The rule on a folder is the folder's own: it sits in `rules`, not in
    // `inherited`, and a folder at the root inherits nothing.
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the folder's own rule: {list}");
    assert_eq!(rules[0]["userId"], reza_id.to_string());
    assert_eq!(rules[0]["level"], "edit");
    assert!(
        list["inherited"]
            .as_array()
            .expect("the inherited list is an array")
            .is_empty(),
        "a folder at the root inherits nothing: {list}"
    );

    let (status, seen) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(
        seen["level"], "edit",
        "the folder's rule covers the query inside it: {seen}"
    );

    // The query's own rule is its own; the folder's rule is inherited, and
    // the inherited entry names the folder it sits on.
    let (status, list) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": yoga.to_string(), "level": "view"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the query's own rule: {list}");
    assert_eq!(rules[0]["userId"], yoga.to_string());
    let inherited = list["inherited"]
        .as_array()
        .expect("the inherited list is an array");
    assert_eq!(inherited.len(), 1, "the folder's rule is inherited: {list}");
    assert_eq!(inherited[0]["folderId"], folder_id);
    assert_eq!(inherited[0]["folderName"], "Reports");
    assert_eq!(inherited[0]["userId"], reza_id.to_string());
    assert_eq!(inherited[0]["roleId"], Value::Null);
    assert_eq!(inherited[0]["level"], "edit");
}

/// A saved query inside a folder inside a folder.
///
/// The ids come back as the chain reads them: the query first, then the
/// folder it sits in, then the folder above that one.
async fn a_query_two_folders_deep(router: &axum::Router, cookie: &str) -> (String, String, String) {
    let (status, folder) = call(
        router,
        "POST",
        "/api/query/folders",
        cookie,
        Some(json!({"name": "Reports"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let reports = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, folder) = call(
        router,
        "POST",
        "/api/query/folders",
        cookie,
        Some(json!({"name": "Weekly", "parentId": reports})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let weekly = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, saved) = call(
        router,
        "POST",
        "/api/query/saved",
        cookie,
        Some(json!({"title": "Numbers", "sql": "SELECT 1", "folderId": weekly})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let query = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    (query, weekly, reports)
}

/// The inherited list names every folder of the chain above the item, the
/// nearest folder first.
#[tokio::test]
async fn the_inherited_list_names_the_whole_chain_nearest_folder_first() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;
    let (id, weekly, reports) = a_query_two_folders_deep(&router, &rina).await;

    // A rule on the folder nearest the query, and one on the folder above
    // it.
    for (folder_id, person, level) in [(&reports, &reza_id, "edit"), (&weekly, &yoga, "view")] {
        let (status, list) = call(
            &router,
            "PUT",
            &format!("/api/query/folders/{folder_id}/access"),
            &rina,
            Some(json!({"rules": [{"userId": person.to_string(), "level": level}]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
    }

    // The query inherits from the folder it sits in and from that folder's
    // parent: the nearest folder first.
    let (status, list) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert!(
        list["rules"]
            .as_array()
            .expect("the rules are an array")
            .is_empty(),
        "the query carries no rule of its own: {list}"
    );
    let inherited = list["inherited"]
        .as_array()
        .expect("the inherited list is an array");
    assert_eq!(inherited.len(), 2, "the whole chain is inherited: {list}");
    assert_eq!(inherited[0]["folderId"], weekly);
    assert_eq!(inherited[0]["folderName"], "Weekly");
    assert_eq!(inherited[0]["userId"], yoga.to_string());
    assert_eq!(inherited[0]["level"], "view");
    assert_eq!(inherited[1]["folderId"], reports);
    assert_eq!(inherited[1]["folderName"], "Reports");
    assert_eq!(inherited[1]["userId"], reza_id.to_string());
    assert_eq!(inherited[1]["level"], "edit");
}

/// A folder carries its own rule in its own list, and inherits the rules
/// of its parents.
#[tokio::test]
async fn a_folder_carries_its_own_rule_and_inherits_its_parents() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;
    let (_id, weekly, reports) = a_query_two_folders_deep(&router, &rina).await;

    for (folder_id, person, level) in [(&reports, &reza_id, "edit"), (&weekly, &yoga, "view")] {
        let (status, list) = call(
            &router,
            "PUT",
            &format!("/api/query/folders/{folder_id}/access"),
            &rina,
            Some(json!({"rules": [{"userId": person.to_string(), "level": level}]})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
    }

    let (status, list) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the folder's own rule: {list}");
    assert_eq!(rules[0]["userId"], yoga.to_string());
    let inherited = list["inherited"]
        .as_array()
        .expect("the inherited list is an array");
    assert_eq!(inherited.len(), 1, "only the parent is inherited: {list}");
    assert_eq!(inherited[0]["folderId"], reports);
    assert_eq!(inherited[0]["userId"], reza_id.to_string());
}

/// The access list needs manage, for reading it as well as for changing
/// it: a person who may only see a query may not see who else has it.
#[tokio::test]
async fn the_access_list_needs_manage_to_be_read_or_changed() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    // A query in the shared space: every member of the organisation may
    // see it, and nobody but the author may manage it.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Shared", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &rina,
        Some(json!({"name": "Reports"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_id = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    for (method, path, body, sentence) in [
        (
            "GET",
            format!("/api/query/saved/{id}/access"),
            None,
            QUERY_FORBIDDEN,
        ),
        (
            "PUT",
            format!("/api/query/saved/{id}/access"),
            Some(json!({"rules": []})),
            QUERY_FORBIDDEN,
        ),
        (
            "GET",
            format!("/api/query/folders/{folder_id}/access"),
            None,
            FOLDER_FORBIDDEN,
        ),
        (
            "PUT",
            format!("/api/query/folders/{folder_id}/access"),
            Some(json!({"rules": []})),
            FOLDER_FORBIDDEN,
        ),
    ] {
        let (status, answer) = call(&router, method, &path, &reza, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
        assert_eq!(
            error(&answer),
            format!("permission_denied: {sentence}"),
            "{method} {path}"
        );
    }

    // The body is read only after the level: a caller that may see the
    // item but not manage it is refused with the level, whatever the body
    // holds.
    let (status, answer) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &reza,
        Some(json!({"nope": true})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(
        error(&answer),
        format!("permission_denied: {QUERY_FORBIDDEN}"),
        "the level is checked before the body"
    );
}

/// An item the caller may not see is a 404 on both access routes, never a
/// 403: the answer never says which it was.
#[tokio::test]
async fn an_item_the_caller_may_not_see_is_not_found_on_both_access_routes() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    // A query in the author's personal space, and an id nothing answers
    // to.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let unseen = Uuid::new_v4().to_string();

    for (method, path, body, sentence) in [
        (
            "GET",
            format!("/api/query/saved/{id}/access"),
            None,
            QUERY_NOT_FOUND,
        ),
        (
            "PUT",
            format!("/api/query/saved/{id}/access"),
            Some(json!({"rules": []})),
            QUERY_NOT_FOUND,
        ),
        (
            "GET",
            format!("/api/query/saved/{unseen}/access"),
            None,
            QUERY_NOT_FOUND,
        ),
        (
            "PUT",
            format!("/api/query/saved/{unseen}/access"),
            Some(json!({"rules": []})),
            QUERY_NOT_FOUND,
        ),
        (
            "GET",
            format!("/api/query/folders/{unseen}/access"),
            None,
            FOLDER_NOT_FOUND,
        ),
        (
            "PUT",
            format!("/api/query/folders/{unseen}/access"),
            Some(json!({"rules": []})),
            FOLDER_NOT_FOUND,
        ),
    ] {
        let (status, answer) = call(&router, method, &path, &reza, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{method} {path}: {answer}");
        assert_eq!(error(&answer), sentence, "{method} {path}");
    }

    // The body is read only after the item: a caller that may not see the
    // item is refused with the 404, whatever the body holds.
    let (status, answer) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &reza,
        Some(json!({"nope": true})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(
        error(&answer),
        QUERY_NOT_FOUND,
        "the item is read before the body"
    );
}

/// A body of another shape is refused with the fixed sentence, and a rule
/// that names two grantees or none, or a level outside the set, with its
/// own.
#[tokio::test]
async fn a_body_or_a_rule_of_another_shape_is_refused() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Shared", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let path = format!("/api/query/saved/{id}/access");

    for body in [
        json!({"nope": true}),
        json!([1, 2]),
        json!({"rules": "none"}),
        json!({"rules": [{"userId": yoga.to_string(), "level": "view"}], "extra": 1}),
    ] {
        let (status, answer) = call(&router, "PUT", &path, &rina, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(error(&answer), ACCESS_BODY_SHAPE);
    }

    // A body that is not JSON at all is refused the same way.
    let (status, answer) = call_with_raw_body(&router, "PUT", &path, &rina, "not json").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(error(&answer), ACCESS_BODY_SHAPE);

    for body in [
        json!({"rules": [{"userId": yoga.to_string(), "roleId": yoga.to_string(), "level": "view"}]}),
        json!({"rules": [{"level": "view"}]}),
        json!({"rules": [{"userId": yoga.to_string(), "level": "owner"}]}),
    ] {
        let (status, answer) = call(&router, "PUT", &path, &rina, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(error(&answer), ACCESS_RULE_SHAPE);
    }
}

/// The same grantee twice is refused with its own sentence.
#[tokio::test]
async fn the_same_grantee_listed_twice_is_refused() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Shared", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let (status, answer) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [
            {"userId": yoga.to_string(), "level": "view"},
            {"userId": yoga.to_string(), "level": "edit"}
        ]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
    assert_eq!(error(&answer), ACCESS_GRANTEE_TWICE);
}

/// A grantee that does not exist, or a person of another organisation, is
/// refused — and the rules that were there stay exactly as they were.
#[tokio::test]
async fn a_grantee_outside_the_organisation_is_refused_and_the_old_rules_stay() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;
    // Andi is a member of Meridian Retail, not of Meridian Group: he is
    // not a person of Rina's organisation.
    let andi = user_id(&pool, "andi@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Shared", "sql": "SELECT 1", "folderId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let path = format!("/api/query/saved/{id}/access");

    let (status, list) = call(
        &router,
        "PUT",
        &path,
        &rina,
        Some(json!({"rules": [{"userId": yoga.to_string(), "level": "view"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");

    for grantee in [andi.to_string(), Uuid::new_v4().to_string()] {
        let (status, answer) = call(
            &router,
            "PUT",
            &path,
            &rina,
            Some(json!({"rules": [{"userId": grantee, "level": "view"}]})),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{answer}");
        assert_eq!(error(&answer), ACCESS_GRANTEE_OUTSIDE);
    }

    // The refused changes left the old rules exactly as they were.
    let (status, list) = call(&router, "GET", &path, &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the old rule is still there: {list}");
    assert_eq!(rules[0]["userId"], yoga.to_string());
}

/// The grantee search lists the people of the caller's organisation and
/// the roles of the deployment, and nobody else.
#[tokio::test]
async fn the_grantee_search_lists_the_people_of_the_callers_organisation() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, found) = call(&router, "GET", "/api/query/grantees?q=", &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let people = found["people"].as_array().expect("the people are an array");
    assert!(
        people
            .iter()
            .any(|person| person["email"] == "reza@meridian.example"),
        "a member of the organisation is there: {found}"
    );
    assert!(
        !people
            .iter()
            .any(|person| person["email"] == "andi@meridian.example"),
        "a person of another organisation is not: {found}"
    );
    let roles = found["roles"].as_array().expect("the roles are an array");
    assert!(
        roles.iter().any(|role| role["name"] == "Analyst"),
        "the roles of the deployment are there: {found}"
    );

    // The search narrows by name.
    let (status, found) = call(&router, "GET", "/api/query/grantees?q=Reza", &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    let people = found["people"].as_array().expect("the people are an array");
    assert_eq!(people.len(), 1, "one person holds the name: {found}");
    assert_eq!(people[0]["email"], "reza@meridian.example");

    // A person of another organisation is not there to be found.
    let (status, found) = call(&router, "GET", "/api/query/grantees?q=Andi", &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert!(
        found["people"]
            .as_array()
            .expect("the people are an array")
            .is_empty(),
        "he is not of her organisation: {found}"
    );
}

// ── The assistant's own reads and writes ─────────────────────────────────
//
// The assistant reaches the saved queries through three tools
// (`routes/ai/registry.rs`): `save_query` writes one,
// `list_saved_queries` reads the list, and `run_saved_query` runs one by
// its id. They go through the same resolver as the console's own routes,
// so a personal space is a wall for the assistant too — and the answer
// the assistant gives about a query it may not see is the same answer it
// gives about a query that does not exist.

/// One assistant tool call over HTTP: `POST /api/ai/tool`.
///
/// The body is the console's own — `{"tool", "args", "mode"}` — and the
/// answer is the tool's own result, not the envelope around it. A read
/// tool needs `"mode": "ask"`; a write tool needs `"mode": "build"` and
/// `"confirmed": true` in its arguments, which is the console's Confirm
/// button and nothing else.
async fn call_tool(app: &axum::Router, cookie: &str, tool: &str, args: Value, mode: &str) -> Value {
    let (status, body) = call(
        app,
        "POST",
        "/api/ai/tool",
        cookie,
        Some(json!({ "tool": tool, "args": args, "mode": mode })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{tool}: {body}");
    body["result"].clone()
}

/// A saved query in the **shared space** of the group tenant: no folder,
/// the tenant's own row.
///
/// Written here, the way a row that was shared before the assistant
/// existed would be. The assistant's own save always lands in the
/// caller's personal space, so the shared space needs a row of its own
/// for the controls below.
async fn write_a_query_in_the_shared_space(pool: &PgPool, owner: Uuid, title: &str) -> String {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO saved_query (title, sql, owner, owner_id, tenant_id) \
         VALUES ($1, 'SELECT 1', 'Rina Wijaya', $2, $3) \
         RETURNING id",
    )
    .bind(title)
    .bind(owner)
    .bind(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id"))
    .fetch_one(pool)
    .await
    .expect("insert the query");
    id.to_string()
}

/// The id of the personal space of one person.
async fn personal_space_id(pool: &PgPool, owner: Uuid) -> String {
    let id: Uuid =
        sqlx::query_scalar("SELECT id FROM saved_query_folder WHERE is_personal AND owner_id = $1")
            .bind(owner)
            .fetch_one(pool)
            .await
            .expect("the person has a personal space");
    id.to_string()
}

/// A third reader of the organisation: a plain colleague of the same
/// tenant as the seeded cast, holding the Analyst role, with no rule of
/// their own. The session cookie is returned.
///
/// The seed gives `query:read` to the Analyst and the Data Scientist
/// only, and its only two holders in the cast's tenant are Rina and
/// Reza — the two the move tests already cast — so the colleague who
/// proves "the organisation still sees it" is written here. What hides
/// a query or a folder from them is the personal space and nothing
/// else: no authorship, no grant, one tenant.
async fn a_third_reader(pool: &PgPool) -> String {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO app_user (id, name, email, status) \
                 VALUES ($1, 'Nadia Rahma', 'nadia@meridian.example', 'active')",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("insert the third reader");
    sqlx::query(
        "INSERT INTO app_user_role (user_id, role_id) \
         SELECT $1, id FROM role WHERE name = 'Analyst'",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("give the third reader the Analyst role");
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) \
         SELECT $1, id FROM tenant WHERE slug = 'meridian-group'",
    )
    .bind(id)
    .execute(pool)
    .await
    .expect("put the third reader in the organisation's tenant");
    session_cookie_for_user(pool, id).await
}

/// Rina's private query, opened to Reza at the level named.
///
/// The query is saved through the route — the save is what creates her
/// personal space — and the rule is written through the access route,
/// the way the sharing slice grants it, so the fixture is the product's
/// own doing. Returns the query's id and the id of the space it sits in.
async fn rinas_private_query_opened_to_reza(
    router: &axum::Router,
    pool: &PgPool,
    level: &str,
) -> (String, String) {
    let rina = session_cookie_for_seeded_user(pool, "rina@meridian.example").await;
    let reza_id = user_id(pool, "reza@meridian.example").await;
    let (status, saved) = call(
        router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let (status, _) = call(
        router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": level}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let space = saved["folderId"]
        .as_str()
        .expect("the save created her personal space")
        .to_owned();
    (id, space)
}

/// A shared folder opened to Reza at the level named.
///
/// The folder row and the rule are written directly, the way the
/// fixture helpers in this file write their rows: the folder's creation
/// and the grant are not what these tests prove, the refusals are.
/// Returns the folder's id.
async fn a_shared_folder_opened_to_reza(pool: &PgPool, level: &str) -> String {
    let folder_id: Uuid = sqlx::query_scalar(
        "INSERT INTO saved_query_folder (tenant_id, name) \
         VALUES ($1, 'Reports') RETURNING id",
    )
    .bind(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id"))
    .fetch_one(pool)
    .await
    .expect("insert the folder");
    sqlx::query(
        "INSERT INTO saved_query_access_rule (folder_id, user_id, level, granted_by) \
         VALUES ($1, \
                 (SELECT id FROM app_user WHERE email = 'reza@meridian.example'), \
                 $2, \
                 (SELECT id FROM app_user WHERE email = 'rina@meridian.example'))",
    )
    .bind(folder_id)
    .bind(level)
    .execute(pool)
    .await
    .expect("grant the folder rule");
    folder_id.to_string()
}

/// The assistant's list does not name another person's personal query.
///
/// The two people act for the same organisation and the same tenant, so
/// what hides one person's query from the other is the personal space
/// and nothing else. CONTROL: the same author's query in the shared
/// space IS in the list, so the test cannot pass by an empty list.
#[tokio::test]
async fn the_assistants_list_does_not_name_another_persons_personal_query() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;

    // She saves a query with the assistant: it lands in her own personal
    // space.
    let saved = call_tool(
        &router,
        &rina,
        "save_query",
        json!({
            "title": "Rina's own",
            "sql": "SELECT 1",
            "owner": "Rina Wijaya",
            "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let private_id = saved["query"]["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    // CONTROL: the same author's query in the shared space, of the same
    // tenant. Only the space differs.
    let shared_id = write_a_query_in_the_shared_space(&pool, rina_id, "Everybody's").await;

    let listed = call_tool(&router, &reza, "list_saved_queries", json!({}), "ask").await;
    let listed = listed["queries"]
        .as_array()
        .expect("the list is an array")
        .clone();
    assert!(
        !listed.iter().any(|query| query["id"] == private_id),
        "another person's personal query is not in the list: {listed:?}"
    );
    assert!(
        listed.iter().any(|query| query["id"] == shared_id),
        "CONTROL: the shared space is in the list: {listed:?}"
    );
}

/// The assistant answers another person's personal query exactly as it
/// answers an id that does not exist.
///
/// The answer is the chat's own "not found" sentence: a refusal that
/// named the query would tell the caller it exists.
#[tokio::test]
async fn the_assistant_answers_another_persons_personal_query_as_an_unknown_id() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;

    let saved = call_tool(
        &router,
        &rina,
        "save_query",
        json!({
            "title": "Rina's own",
            "sql": "SELECT 1",
            "owner": "Rina Wijaya",
            "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let private_id = saved["query"]["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    let theirs = call_tool(
        &router,
        &reza,
        "run_saved_query",
        json!({ "id": private_id }),
        "ask",
    )
    .await;
    let unknown = call_tool(
        &router,
        &reza,
        "run_saved_query",
        json!({ "id": Uuid::new_v4().to_string() }),
        "ask",
    )
    .await;
    assert_eq!(
        theirs,
        json!({ "error": "saved query tidak ditemukan" }),
        "another person's personal query is not found"
    );
    assert_eq!(
        theirs, unknown,
        "the answer is the same as for an id that does not exist"
    );

    // CONTROL: a query of the shared space IS found, so the "not found"
    // above is about the personal space and not about a tool that finds
    // nothing at all.
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let shared_id = write_a_query_in_the_shared_space(&pool, rina_id, "Everybody's").await;
    let shared = call_tool(
        &router,
        &reza,
        "run_saved_query",
        json!({ "id": shared_id }),
        "ask",
    )
    .await;
    assert_ne!(
        shared, theirs,
        "a query of the shared space is found, not reported as missing: {shared}"
    );
}

/// The assistant's save puts the query in the caller's own personal space.
///
/// CONTROL: the same call by the other person lands in that person's own
/// personal space, not in the first one's.
#[tokio::test]
async fn the_assistants_save_puts_the_query_in_the_callers_own_personal_space() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    let saved = call_tool(
        &router,
        &rina,
        "save_query",
        json!({
            "title": "Rina's own",
            "sql": "SELECT 1",
            "owner": "Rina Wijaya",
            "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let id = saved["query"]["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let folder_id = saved["query"]["folderId"]
        .as_str()
        .expect("the query sits in a folder")
        .to_owned();
    assert_eq!(
        folder_id,
        personal_space_id(&pool, rina_id).await,
        "the query sits in her own personal space"
    );
    let owner: Option<Uuid> = sqlx::query_scalar("SELECT owner_id FROM saved_query WHERE id = $1")
        .bind(Uuid::parse_str(&id).expect("the query id"))
        .fetch_one(&pool)
        .await
        .expect("the query row");
    assert_eq!(owner, Some(rina_id), "she is the author of the query");

    // CONTROL: the same call by the other person lands in that person's
    // own personal space.
    let saved = call_tool(
        &router,
        &reza,
        "save_query",
        json!({
            "title": "Reza's own",
            "sql": "SELECT 2",
            "owner": "Reza Hakim",
            "confirmed": true,
        }),
        "build",
    )
    .await;
    assert_eq!(saved["ok"], true, "{saved}");
    let folder_id = saved["query"]["folderId"]
        .as_str()
        .expect("the query sits in a folder")
        .to_owned();
    assert_eq!(
        folder_id,
        personal_space_id(&pool, reza_id).await,
        "the query sits in his own personal space"
    );
}

// ── A deployment with a single tenant ───────────────────────────────────

/// The deployment holds one tenant: every other tenant and its
/// memberships go, and the caller joins the one that is left.
///
/// A deployment with one tenant reaches every row of every table
/// (`SEC-20`), so the reach of a request no longer names the tenant a
/// new row belongs to: the tenant the request acts as does.
async fn leave_one_tenant(pool: &PgPool, caller: Uuid) {
    for tenant in [
        "11111111-1111-4111-8111-000000000002",
        "11111111-1111-4111-8111-000000000003",
        "11111111-1111-4111-8111-000000000004",
    ] {
        let tenant_id: Uuid = tenant.parse().expect("a seeded tenant id");
        // The membership rows go first: `app_user_tenant.tenant_id`
        // refuses the deletion of a tenant that still has members.
        sqlx::query("DELETE FROM app_user_tenant WHERE tenant_id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .expect("drop the memberships of the tenant that goes");
        sqlx::query("DELETE FROM tenant WHERE id = $1")
            .bind(tenant_id)
            .execute(pool)
            .await
            .expect("drop the tenant");
    }
    sqlx::query(
        "INSERT INTO app_user_tenant (user_id, tenant_id) VALUES ($1, $2) \
         ON CONFLICT DO NOTHING",
    )
    .bind(caller)
    .bind(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id"))
    .execute(pool)
    .await
    .expect("add the caller to the one tenant that is left");
}

/// The tenant a row belongs to.
///
/// The wire shapes of a saved query and of a folder carry no tenant, so
/// the tenant of a row is read from the row itself.
async fn tenant_of_saved_query(pool: &PgPool, id: &str) -> Option<Uuid> {
    sqlx::query_scalar("SELECT tenant_id FROM saved_query WHERE id = $1")
        .bind(Uuid::parse_str(id).expect("a saved query id"))
        .fetch_one(pool)
        .await
        .expect("the saved query row")
}

/// The tenant a folder belongs to.
///
/// The wire shape of a folder carries no tenant either.
async fn tenant_of_folder(pool: &PgPool, id: &str) -> Uuid {
    sqlx::query_scalar("SELECT tenant_id FROM saved_query_folder WHERE id = $1")
        .bind(Uuid::parse_str(id).expect("a folder id"))
        .fetch_one(pool)
        .await
        .expect("the folder row")
}

/// In a deployment with a single tenant, the query a caller saves, the
/// copy of a query and the folder a caller makes all belong to that
/// tenant.
///
/// A personal space hangs off the tenant of the person it belongs to, so
/// a row that carries no tenant has no personal space to land in: it
/// lands in the shared space instead, and the caller's own work is
/// nobody's private work at all.
#[tokio::test]
async fn in_a_single_tenant_deployment_a_query_a_copy_and_a_folder_belong_to_that_tenant() {
    let TestApp { router, pool } = spin_up().await;
    let rina = user_id(&pool, "rina@meridian.example").await;
    leave_one_tenant(&pool, rina).await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    // A query saved without a folder lands in the caller's own personal
    // space, and the row carries the tenant of the deployment.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &cookie,
        Some(json!({"title": "Single tenant", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let saved_id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let space = personal_space_id(&pool, rina).await;
    assert_eq!(
        saved["folderId"].as_str(),
        Some(space.as_str()),
        "the query lands in the caller's own personal space: {saved}"
    );
    assert_eq!(
        tenant_of_saved_query(&pool, &saved_id).await,
        Some(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id")),
        "the row carries the tenant of the deployment"
    );

    // The copy of a query is a row of the caller's own, so it lands in
    // the caller's own personal space as well.
    let (status, copy) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{saved_id}/duplicate"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{copy}");
    assert_eq!(
        copy["folderId"].as_str(),
        Some(space.as_str()),
        "the copy lands in the caller's own personal space: {copy}"
    );
    let copy_id = copy["id"].as_str().expect("the copy has an id").to_owned();
    assert_eq!(
        tenant_of_saved_query(&pool, &copy_id).await,
        Some(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id")),
        "the copy carries the tenant of the deployment"
    );

    // A folder belongs to the tenant the request acts as, and the
    // console sees it in the list of the caller's folders.
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &cookie,
        Some(json!({"name": "Single tenant folder"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let folder_id = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();
    let (status, folders) = call(&router, "GET", "/api/query/folders", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{folders}");
    assert!(
        folders
            .as_array()
            .expect("the folders are a list")
            .iter()
            .any(|folder| folder["id"] == folder_id.as_str()),
        "the console sees the folder: {folders}"
    );
    assert_eq!(
        tenant_of_folder(&pool, &folder_id).await,
        Uuid::parse_str(GROUP_TENANT).expect("the group tenant id"),
        "the folder belongs to the tenant of the deployment"
    );
}

/// An edit grantee cannot publish a personal query to the organisation:
/// the move into the shared space needs manage on the query, and the
/// query stays unseen by everybody else (`QS-3` review finding 1).
#[tokio::test]
async fn an_edit_grantee_cannot_move_a_personal_query_into_the_shared_space() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    // The third reader is a plain colleague of the organisation: no
    // authorship, no grant, one tenant.
    let colleague = a_third_reader(&pool).await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    assert!(
        saved["folderId"].is_string(),
        "a query saved without a folder sits in the author's personal space: {saved}"
    );

    // Rina opens the query to Reza at edit, the way the sharing slice
    // grants it: the grant carries edit, not manage.
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Moving the query to the shared root would publish it to the whole
    // organisation, and edit is not manage: refused with the one move
    // sentence.
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &reza,
        Some(json!({"folderId": "root"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_ACROSS_SPACES_FORBIDDEN}")
    );

    // The organisation still cannot see the query: it never moved.
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error(&found), QUERY_NOT_FOUND);
}

/// An edit grantee on a folder inside a personal space cannot lift the
/// folder — and the queries inside it — into the shared space: the move
/// needs manage on the folder (`QS-3` review finding 1).
#[tokio::test]
async fn an_edit_grantee_cannot_move_a_personal_sub_folder_into_the_shared_space() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    // The third reader is a plain colleague of the organisation: no
    // authorship, no grant, one tenant.
    let colleague = a_third_reader(&pool).await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    // Rina saves a private query first: the save creates her personal
    // space, and the sub-folder is built inside it.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let space = saved["folderId"]
        .as_str()
        .expect("the save created her personal space")
        .to_owned();
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &rina,
        Some(json!({"name": "Weekly", "parentId": space})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let weekly = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Numbers", "sql": "SELECT 1", "folderId": weekly})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let numbers = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();

    // Reza holds edit on the sub-folder — on that folder alone.
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Moving the folder to the shared root would publish it and the
    // query inside it to the whole organisation: refused with the one
    // move sentence.
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{weekly}"),
        &reza,
        Some(json!({"parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_ACROSS_SPACES_FORBIDDEN}")
    );

    // Nothing moved: the folder and the query inside it stay invisible
    // to the organisation.
    let (status, tree) = call(&router, "GET", "/api/query/folders", &colleague, None).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert!(
        !tree
            .as_array()
            .expect("the list is an array")
            .iter()
            .any(|folder| folder["id"] == weekly.as_str()),
        "the sub-folder is still nobody else's to see: {tree}"
    );
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{numbers}"),
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(error(&found), QUERY_NOT_FOUND);
}

/// An edit grantee cannot hide a shared query inside a personal space:
/// the move out of the shared space needs manage on the query, and the
/// organisation still sees the query afterwards (`QS-3` review finding 1).
#[tokio::test]
async fn an_edit_grantee_cannot_move_a_shared_query_into_a_personal_space() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    // The third reader is a plain colleague of the organisation: no
    // authorship, no grant, one tenant.
    let colleague = a_third_reader(&pool).await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    let id = write_a_query_in_the_shared_space(&pool, rina_id, "SharedNumbers").await;

    // Reza's own personal space exists — his first private save creates
    // it — and the move below names it.
    let (status, own) = call(
        &router,
        "POST",
        "/api/query/saved",
        &reza,
        Some(json!({"title": "Reza's own", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{own}");
    let reza_space = personal_space_id(&pool, reza_id).await;

    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &reza,
        Some(json!({"folderId": reza_space})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_ACROSS_SPACES_FORBIDDEN}")
    );

    // The organisation still sees the query, still in the shared space.
    let (status, list) = call(&router, "GET", "/api/query/saved", &colleague, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let shared = list
        .as_array()
        .expect("the list is an array")
        .iter()
        .find(|query| query["id"] == id.as_str())
        .unwrap_or_else(|| panic!("the organisation still sees the query: {list}"));
    assert!(
        shared["folderId"].is_null(),
        "the query is still in the shared space: {shared}"
    );
}

/// The owner holds manage on their own queries, so the same two moves an
/// edit grantee is refused land: a private query out to the shared space
/// and a shared query into the owner's personal space.
#[tokio::test]
async fn the_owner_moves_into_and_out_of_a_personal_space_at_manage() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;

    // Out of the personal space: her private save created it, and the
    // answer carries the query's new place.
    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let space = saved["folderId"]
        .as_str()
        .expect("the save created her personal space")
        .to_owned();
    let (status, moved) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &rina,
        Some(json!({"folderId": "root"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert!(
        moved["folderId"].is_null(),
        "the query is in the shared space now: {moved}"
    );

    // Into the personal space: the shared query of hers moves into it,
    // and the moved state is what a later read sees.
    let shared_id = write_a_query_in_the_shared_space(&pool, rina_id, "SharedNumbers").await;
    let (status, moved) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{shared_id}"),
        &rina,
        Some(json!({"folderId": space})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert_eq!(
        moved["folderId"].as_str(),
        Some(space.as_str()),
        "the query is in her personal space now: {moved}"
    );
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{shared_id}"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(
        found["folderId"].as_str(),
        Some(space.as_str()),
        "the moved state is what a read sees: {found}"
    );
}

/// A refused move writes nothing: a title in the same PATCH is not
/// written either, and the query stays where and as it was.
#[tokio::test]
async fn a_refused_move_writes_nothing_not_even_the_title_in_the_same_patch() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    let (status, saved) = call(
        &router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Private", "sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let space = saved["folderId"]
        .as_str()
        .expect("the save created her personal space")
        .to_owned();
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // The move is refused, and the title in the same body is not
    // written: the check runs before the write.
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &reza,
        Some(json!({"folderId": "root", "title": "Renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_ACROSS_SPACES_FORBIDDEN}")
    );

    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(
        found["title"], "Private",
        "the title never changed: {found}"
    );
    assert_eq!(
        found["folderId"].as_str(),
        Some(space.as_str()),
        "the query never moved: {found}"
    );
}

/// A move that names a folder of another tenant than the item's is a
/// move into an unseen folder: a 404 for the query move and for the
/// folder move, never a 403 (`QS-3` review finding 6).
#[tokio::test]
async fn another_tenants_folder_is_not_found_on_a_query_move_and_on_a_folder_move() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;

    // A folder of the tenant next door: it exists, but it is not in the
    // tree the request acts as, so naming it names an unseen folder.
    let foreign: Uuid = sqlx::query_scalar(
        "INSERT INTO saved_query_folder (tenant_id, name) \
         VALUES ($1, 'Retail shared') RETURNING id",
    )
    .bind(Uuid::parse_str("11111111-1111-4111-8111-000000000002").expect("a seeded tenant id"))
    .fetch_one(&pool)
    .await
    .expect("insert the other tenant's folder");
    let foreign = foreign.to_string();

    // The query move: refused as unseen, not as forbidden.
    let shared_id = write_a_query_in_the_shared_space(&pool, rina_id, "SharedNumbers").await;
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{shared_id}"),
        &rina,
        Some(json!({"folderId": foreign})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(error(&body), FOLDER_NOT_FOUND);

    // The folder move: the same answer for the parent a folder moves
    // under.
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &rina,
        Some(json!({"name": "Reports"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let reports = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{reports}"),
        &rina,
        Some(json!({"parentId": foreign})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(error(&body), FOLDER_NOT_FOUND);
}

/// Decision 6 (plan 55 D5): edit may delete and restore a saved query,
/// and the trash shows the queries the caller may edit — so an edit
/// grantee on someone's personal query can take it to the trash, find
/// it waiting there, and bring it back, all without manage.
#[tokio::test]
async fn an_edit_grantee_deletes_a_personal_query_and_restores_it_from_their_trash() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let (id, space) = rinas_private_query_opened_to_reza(&router, &pool, "edit").await;

    // Reza may delete it: edit is enough since decision 6.
    let (status, _) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "edit may delete a query");

    // The trash shows it to the edit grantee: the trash lists the
    // queries the caller may edit.
    let (status, trash) = call(&router, "GET", "/api/query/saved-trash", &reza, None).await;
    assert_eq!(status, StatusCode::OK, "{trash}");
    let entry = trash
        .as_array()
        .expect("the trash is an array")
        .iter()
        .find(|entry| entry["id"] == id)
        .expect("the edit grantee sees the query waiting in the trash");
    assert_eq!(entry["title"], "Private");

    // And edit may restore it, into the folder it was in.
    let (status, restored) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/restore"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{restored}");
    assert_eq!(
        restored["folderId"], space,
        "it comes back in the folder it was in"
    );

    // Back where it was: out of the trash and live for the grantee
    // again.
    let (status, trash) = call(&router, "GET", "/api/query/saved-trash", &reza, None).await;
    assert_eq!(status, StatusCode::OK, "{trash}");
    assert!(
        !trash
            .as_array()
            .expect("the trash is an array")
            .iter()
            .any(|entry| entry["id"] == id),
        "the restored query is out of the trash: {trash}"
    );
    let (status, seen) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["level"], "edit", "the grant still names its level");
}

/// A view grantee may neither delete nor restore a personal query:
/// decision 6 lifts a query's delete and restore to edit, and view
/// stays below edit. The trash does not show it to them either.
#[tokio::test]
async fn a_view_grantee_neither_deletes_nor_restores_a_personal_query() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let (id, _) = rinas_private_query_opened_to_reza(&router, &pool, "view").await;

    // Delete is refused, with the change sentence — the same sentence
    // the other change routes answer.
    let (status, body) = call(
        &router,
        "DELETE",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {QUERY_FORBIDDEN}")
    );

    // Restore is refused the same way.
    let (status, body) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/restore"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {QUERY_FORBIDDEN}")
    );

    // Nothing was written: the query is still live for the view grantee
    // and never in their trash.
    let (status, seen) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{seen}");
    assert_eq!(seen["level"], "view", "the query never left");
    let (status, trash) = call(&router, "GET", "/api/query/saved-trash", &reza, None).await;
    assert_eq!(status, StatusCode::OK, "{trash}");
    assert!(
        !trash
            .as_array()
            .expect("the trash is an array")
            .iter()
            .any(|entry| entry["id"] == id),
        "a view grantee sees no trash entry for it: {trash}"
    );
}

/// Review finding 3: decision 6 lifts only a query's delete and restore
/// to edit. Reading and writing the access list of a query, reading and
/// writing the access list of a folder, and deleting a folder, all
/// still need manage — an edit grantee is refused all five, and the
/// refused writes change nothing.
#[tokio::test]
async fn an_edit_grantee_is_refused_the_access_lists_and_a_folder_delete() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let (id, _) = rinas_private_query_opened_to_reza(&router, &pool, "edit").await;
    let folder_id = a_shared_folder_opened_to_reza(&pool, "edit").await;

    // All five, each refused with its item's own sentence: the sharing
    // surface keeps manage even after decision 6.
    for (method, path, body, sentence) in [
        (
            "GET",
            format!("/api/query/saved/{id}/access"),
            None,
            QUERY_FORBIDDEN,
        ),
        (
            "PUT",
            format!("/api/query/saved/{id}/access"),
            Some(json!({"rules": []})),
            QUERY_FORBIDDEN,
        ),
        (
            "GET",
            format!("/api/query/folders/{folder_id}/access"),
            None,
            FOLDER_FORBIDDEN,
        ),
        (
            "PUT",
            format!("/api/query/folders/{folder_id}/access"),
            Some(json!({"rules": []})),
            FOLDER_FORBIDDEN,
        ),
        (
            "DELETE",
            format!("/api/query/folders/{folder_id}"),
            None,
            FOLDER_FORBIDDEN,
        ),
    ] {
        let (status, answer) = call(&router, method, &path, &reza, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method} {path}: {answer}");
        assert_eq!(
            error(&answer),
            format!("permission_denied: {sentence}"),
            "{method} {path}"
        );
    }

    // The refused writes changed nothing: the query's rule is still
    // Rina's edit grant, and the folder still stands.
    let (status, list) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the rule is untouched: {list}");
    assert_eq!(rules[0]["userId"], reza_id.to_string());
    assert_eq!(rules[0]["level"], "edit");
    let (status, folders) = call(&router, "GET", "/api/query/folders", &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{folders}");
    assert!(
        folders
            .as_array()
            .expect("the tree is an array")
            .iter()
            .any(|folder| folder["id"] == folder_id),
        "the folder is still there: {folders}"
    );
}
