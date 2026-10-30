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

use common::{TestApp, session_cookie_for_seeded_user, spin_up};

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
