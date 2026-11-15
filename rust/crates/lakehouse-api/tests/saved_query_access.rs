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

use crate::common;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

use common::{
    TestApp, session_cookie_for_seeded_user, session_cookie_for_user, spin_up,
    spin_up_with_env_counting_acquires,
};

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

/// The sentence a caller that may not read a folder's access list is told.
///
/// It is not the sentence of a refusal to change: this route changes
/// nothing, and it says what the caller would need to see the list.
const FOLDER_ACCESS_FORBIDDEN: &str =
    "Only someone who can edit this folder can see who it is shared with.";

/// The sentence a move at a level below manage is refused with — the same
/// sentence for a query and for a folder, so the two moves answer
/// identically (`QS-3` review of #382: a move needs manage).
const MOVE_NEEDS_MANAGE: &str = "Only someone who manages this can move it.";

/// The sentence a rule on a personal space itself is refused with: a
/// personal space is shared through a query or a folder inside it.
const PERSONAL_SPACE_NOT_SHARED: &str =
    "A personal space is not shared: share a query or a folder inside it.";

/// The sentence a share of a folder is refused with when the caller does
/// not manage everything in it (`QS-3` review of #393).
const SHARE_NEEDS_MANAGE_OF_CONTENTS: &str =
    "Only someone who manages everything in this folder can share it.";

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

/// The three causes the read-only guard tells apart (QS-14), as the
/// statement that has each and the sentence a store of it is refused with.
/// A write keeps the save route's own sentence; a curly quote and a typo
/// use the sentences Query Studio's run and estimate give.
const REFUSED_WHEN_STORED: [(&str, &str); 3] = [
    (
        "SELEC 1",
        "Query Studio runs only statements that start with SELECT, WITH, SHOW, DESCRIBE or EXPLAIN. Check the first word for a typo.",
    ),
    (
        "\u{2018}SELECT\u{2019} 1",
        "The query has a character Query Studio cannot read safely, such as a curly quote (\u{2018} \u{2019} \u{201C} \u{201D}) or a `$$` sequence. Type straight quotes (' or \") and run it again.",
    ),
    ("DROP TABLE t", NOT_READ_ONLY),
];

/// A refused save, edit, copy or revert names its cause: a typo in the
/// first word and a curly quote get their own sentence, a write keeps the
/// old one.
#[tokio::test]
async fn a_statement_refused_for_storing_says_whether_it_is_a_typo_a_curly_quote_or_a_write() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let owner = user_id(&pool, "rina@meridian.example").await;
    let id = write_a_query_that_cannot_be_saved(&pool, owner, "Doomed").await;

    for (sql, sentence) in REFUSED_WHEN_STORED {
        let (status, body) = call(
            &router,
            "POST",
            "/api/query/saved",
            &cookie,
            Some(json!({"title": "Refused", "sql": sql})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "save {sql}: {body}"
        );
        assert_eq!(error(&body), sentence, "save {sql}");

        let (status, body) = call(
            &router,
            "PATCH",
            &format!("/api/query/saved/{id}"),
            &cookie,
            Some(json!({"sql": sql})),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "edit {sql}: {body}"
        );
        assert_eq!(error(&body), sentence, "edit {sql}");

        // The copy and the revert read a statement that is already stored,
        // so the row is written with it, the way a row older than the
        // guard would be.
        sqlx::query("UPDATE saved_query SET sql = $1 WHERE id = $2")
            .bind(sql)
            .bind(Uuid::parse_str(&id).expect("the query id"))
            .execute(&pool)
            .await
            .expect("store the statement");
        let (status, body) = call(
            &router,
            "POST",
            &format!("/api/query/saved/{id}/duplicate"),
            &cookie,
            None,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "copy {sql}: {body}"
        );
        assert_eq!(error(&body), sentence, "copy {sql}");

        sqlx::query("DELETE FROM saved_query_version WHERE query_id = $1 AND version = 2")
            .bind(Uuid::parse_str(&id).expect("the query id"))
            .execute(&pool)
            .await
            .expect("clear the version");
        sqlx::query(
            "INSERT INTO saved_query_version (query_id, version, title, sql, author_name, label) \
             VALUES ($1, 2, 'Doomed', $2, 'Rina Wijaya', '')",
        )
        .bind(Uuid::parse_str(&id).expect("the query id"))
        .bind(sql)
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
        assert_eq!(
            status,
            StatusCode::UNPROCESSABLE_ENTITY,
            "revert {sql}: {body}"
        );
        assert_eq!(error(&body), sentence, "revert {sql}");
    }
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

/// The access list of a query needs manage, for reading it as well as for
/// changing it: a person who may only see a query may not see who else has
/// it. The list of a folder is read from edit up and changed with manage, so
/// a person who may only see a folder is refused both (QS-3, folder sharing
/// warning: the case below has view only on the folder). The folder's two
/// refusals carry two sentences: the one for a refusal to change, and the
/// one that says what the caller would need to read the list.
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
            FOLDER_ACCESS_FORBIDDEN,
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
        "INSERT INTO app_user (id, name, username, email, status) \
                 VALUES ($1, 'Nadia Rahma', 'nadia', 'nadia@meridian.example', 'active')",
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
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
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
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
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
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
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
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
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
/// to edit. Reading and writing the access list of a query, writing the
/// access list of a folder, and deleting a folder still need manage — an
/// edit grantee is refused all four, and the refused writes change nothing.
/// Reading the access list of a folder is the one exception, from edit up
/// (QS-3, folder sharing warning): the save and move dialogs name who a
/// folder is shared with before the user confirms.
#[tokio::test]
async fn an_edit_grantee_is_refused_the_access_lists_and_a_folder_delete() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let (id, _) = rinas_private_query_opened_to_reza(&router, &pool, "edit").await;
    let folder_id = a_shared_folder_opened_to_reza(&pool, "edit").await;

    // All four, each refused with its item's own sentence: changing the
    // rules, and reading a query's list, keep manage even after decision 6.
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

    // QS-3, folder sharing warning: the one list an edit grantee may read
    // is the folder's. It carries the folder's own rule (Reza's) and the
    // level he reads it at.
    let (status, list) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{folder_id}/access"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["yourLevel"], "edit", "{list}");
    let rules = list["rules"].as_array().expect("the rules are an array");
    assert_eq!(rules.len(), 1, "the folder's own rule: {list}");
    assert_eq!(rules[0]["userId"], reza_id.to_string());
    assert_eq!(rules[0]["level"], "edit");

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

// ── A move needs manage (`QS-3` review of #382) ─────────────────

/// The folder with `id` in the tree the caller sees, or a panic when it is
/// not there.
async fn folder_in_tree(router: &axum::Router, cookie: &str, id: &str) -> Value {
    let (status, tree) = call(router, "GET", "/api/query/folders", cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    tree.as_array()
        .expect("the tree is an array")
        .iter()
        .find(|folder| folder["id"] == id)
        .unwrap_or_else(|| panic!("the caller sees folder {id}: {tree}"))
        .clone()
}

/// A folder `name` created by the caller under `parent` (the root when
/// `None`); its id.
async fn folder_created(
    router: &axum::Router,
    cookie: &str,
    name: &str,
    parent: Option<&str>,
) -> String {
    // No `parentId` key makes a root folder.
    let mut body = json!({"name": name});
    if let Some(parent) = parent {
        body["parentId"] = json!(parent);
    }
    let (status, folder) = call(router, "POST", "/api/query/folders", cookie, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned()
}

/// A query `title` saved by the caller into `folder` (their personal space
/// when `None`); its id and the folder it landed in.
async fn query_saved(
    router: &axum::Router,
    cookie: &str,
    title: &str,
    folder: Option<&str>,
) -> (String, String) {
    // No `folderId` key saves into the caller's personal space.
    let mut body = json!({"title": title, "sql": "SELECT 1"});
    if let Some(folder) = folder {
        body["folderId"] = json!(folder);
    }
    let (status, saved) = call(router, "POST", "/api/query/saved", cookie, Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let folder_id = saved["folderId"]
        .as_str()
        .expect("the query sits in a folder")
        .to_owned();
    (id, folder_id)
}

/// A shared folder `Reports` that Reza edits, a folder `X` that Reza
/// created inside it (so he manages `X`), and a query of Rina's saved
/// into `X` — Rina holds edit on `X` through a rule Reza wrote. Reza's
/// level on Rina's query is edit, which `Reports` gives him: his
/// authorship of `X` does not reach what other people put in it.
///
/// Returns the ids of `Reports`, `X` and Rina's query.
async fn x_in_reports_holding_rinas_query(
    router: &axum::Router,
    pool: &PgPool,
) -> (String, String, String) {
    let rina = session_cookie_for_seeded_user(pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(pool, "reza@meridian.example").await;
    let rina_id = user_id(pool, "rina@meridian.example").await;
    let reports = a_shared_folder_opened_to_reza(pool, "edit").await;

    let (status, folder) = call(
        router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "X", "parentId": reports})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let x = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();
    let (status, body) = call(
        router,
        "PUT",
        &format!("/api/query/folders/{x}/access"),
        &reza,
        Some(json!({"rules": [{"userId": rina_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, saved) = call(
        router,
        "POST",
        "/api/query/saved",
        &rina,
        Some(json!({"title": "Rina's numbers", "sql": "SELECT 1", "folderId": x})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let query = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    (reports, x, query)
}

/// Review scenario (a): Reza holds edit on Rina's `Weekly` sub-folder
/// and creates a folder `X` of his own inside it, which he manages.
/// Moving Rina's query into `X` is a move, and a move needs manage on the
/// query, so it is refused: the query was the first leg of publishing it.
/// A colleague still cannot see the query afterwards.
#[tokio::test]
async fn an_edit_holder_cannot_move_a_query_into_a_folder_of_their_own() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
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
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // Reza creates X inside Weekly: he is its author, so he manages it.
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "X", "parentId": weekly})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let x = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();
    assert_eq!(folder["level"], "manage", "{folder}");

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{numbers}"),
        &reza,
        Some(json!({"folderId": x})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
    );

    // The query never moved, and the organisation cannot see it.
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{numbers}"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["folderId"].as_str(), Some(weekly.as_str()), "{found}");
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

/// Review scenario (a) as the review wrote it (`QS-3` review of #382, follow-up): the
/// folder `X` sits in Rina's PERSONAL space, under `Weekly`, and Rina
/// herself puts her query into it. Reza, who created `X` and manages it,
/// holds only edit on that query through the rule on `Weekly`. Moving `X`
/// to the shared root would publish Rina's query to the organisation, so
/// it is refused with the move sentence, and a colleague with no grant
/// still neither finds the query by id nor in their list.
#[tokio::test]
async fn an_author_of_a_folder_in_a_personal_space_cannot_publish_it_with_her_query_inside() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let colleague = a_third_reader(&pool).await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;

    let (_private, space) = query_saved(&router, &rina, "Private", None).await;
    let weekly = folder_created(&router, &rina, "Weekly", Some(&space)).await;
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Reza creates X inside Weekly: he is its author, so he manages it.
    let x = folder_created(&router, &reza, "X", Some(&weekly)).await;
    let tree_x = folder_in_tree(&router, &reza, &x).await;
    assert_eq!(tree_x["level"], "manage", "{tree_x}");

    // Rina manages the whole of her space, so she saves a query into X.
    let (numbers, _) = query_saved(&router, &rina, "Rina's numbers", Some(&x)).await;
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{numbers}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(
        found["level"], "edit",
        "authoring X does not reach Rina's query: {found}"
    );

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{x}"),
        &reza,
        Some(json!({"parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
    );

    // X is still in Weekly, and the organisation does not see the query.
    let tree_x = folder_in_tree(&router, &reza, &x).await;
    assert_eq!(
        tree_x["parentId"].as_str(),
        Some(weekly.as_str()),
        "{tree_x}"
    );
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{numbers}"),
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{found}");
    assert_eq!(error(&found), QUERY_NOT_FOUND);
    let (status, listed) = call(
        &router,
        "GET",
        "/api/query/saved?q=numbers",
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert!(
        listed.as_array().expect("the list is an array").is_empty(),
        "the colleague's list does not hold the query: {listed}"
    );
}

/// Review scenario (b): Reza created `X` inside `Reports` (so he manages
/// it) and Rina's query sits in it. Moving `X` to the shared root would
/// change which rules cover Rina's query, and Reza holds only edit on it,
/// so the move is refused. A colleague who holds edit through a rule on
/// `Reports` keeps that edit on the query afterwards. CONTROL: an empty
/// folder of Reza's moves to the root.
#[tokio::test]
async fn an_author_of_a_folder_cannot_move_it_with_another_persons_query_inside() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let colleague = a_third_reader(&pool).await;
    let colleague_id = user_id(&pool, "nadia@meridian.example").await;
    let (reports, x, query) = x_in_reports_holding_rinas_query(&router, &pool).await;
    // The colleague edits everything in `Reports` through a rule on it.
    sqlx::query(
        "INSERT INTO saved_query_access_rule (folder_id, user_id, level, granted_by) \
         VALUES ($1, $2, 'edit', (SELECT id FROM app_user WHERE email = 'rina@meridian.example'))",
    )
    .bind(Uuid::parse_str(&reports).expect("the folder id"))
    .bind(colleague_id)
    .execute(&pool)
    .await
    .expect("grant the colleague edit on Reports");
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{query}"),
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(
        found["level"], "edit",
        "the rule on Reports covers it: {found}"
    );

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{x}"),
        &reza,
        Some(json!({"parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
    );

    // Nothing moved: X is still in Reports and the colleague still holds
    // edit on the query.
    let tree_x = folder_in_tree(&router, &reza, &x).await;
    assert_eq!(
        tree_x["parentId"].as_str(),
        Some(reports.as_str()),
        "{tree_x}"
    );
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{query}"),
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert_eq!(found["level"], "edit", "{found}");

    // CONTROL: a folder of his own with nothing of anybody else's moves.
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "Y", "parentId": reports})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let y = folder["id"].as_str().expect("the folder has an id");
    let (status, moved) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{y}"),
        &reza,
        Some(json!({"parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{moved}");
    assert!(moved["parentId"].is_null(), "{moved}");
}

/// Review scenario (c): Reza holds edit on Rina's shared query and has a
/// folder of his own. Moving the query into it is refused, so he cannot
/// hide it from the organisation or raise his level on it. The colleague
/// still sees it in the shared space.
#[tokio::test]
async fn an_edit_holder_of_a_shared_query_cannot_move_it_into_their_own_folder() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let colleague = a_third_reader(&pool).await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let id = write_a_query_in_the_shared_space(&pool, rina_id, "SharedNumbers").await;
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{id}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "edit"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "Mine"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let mine = folder["id"]
        .as_str()
        .expect("the folder has an id")
        .to_owned();

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &reza,
        Some(json!({"folderId": mine})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
    );

    // Still in the shared space, where the colleague sees it at view.
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &colleague,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    assert!(found["folderId"].is_null(), "{found}");
    assert_eq!(found["level"], "view", "{found}");
    // An edit holder still edits it: only the move is refused.
    let (status, edited) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &reza,
        Some(json!({"title": "Renamed by Reza"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["title"], "Renamed by Reza");
}

/// A move to the folder the query is already in is no move, so an edit
/// holder who re-sends it with a new title is not refused.
#[tokio::test]
async fn an_edit_holder_who_names_the_current_folder_is_not_moving() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let (_reports, x, query) = x_in_reports_holding_rinas_query(&router, &pool).await;
    // Reza holds edit on the query through `Reports`.
    let (status, edited) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{query}"),
        &reza,
        Some(json!({"folderId": x, "title": "Same place"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["title"], "Same place");
    assert_eq!(edited["folderId"].as_str(), Some(x.as_str()));
}

/// D4: a folder PATCH whose move is refused writes nothing, the new name
/// in the same body included, and a name the destination refuses does not
/// leave the folder moved.
#[tokio::test]
async fn a_refused_folder_move_writes_nothing_not_even_the_name_in_the_same_patch() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let (reports, x, _query) = x_in_reports_holding_rinas_query(&router, &pool).await;

    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{x}"),
        &reza,
        Some(json!({"name": "Renamed", "parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {MOVE_NEEDS_MANAGE}")
    );
    let tree_x = folder_in_tree(&router, &reza, &x).await;
    assert_eq!(tree_x["name"], "X", "the name never changed: {tree_x}");
    assert_eq!(
        tree_x["parentId"].as_str(),
        Some(reports.as_str()),
        "{tree_x}"
    );

    // The other order of refusal: the move is allowed (an empty folder of
    // his own) but the new name is taken at the destination, and the
    // folder must not be left moved with its old name.
    let (status, taken) = call(
        &router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "Taken"})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{taken}");
    let (status, folder) = call(
        &router,
        "POST",
        "/api/query/folders",
        &reza,
        Some(json!({"name": "Y", "parentId": reports})),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{folder}");
    let y = folder["id"].as_str().expect("the folder has an id");
    let (status, body) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{y}"),
        &reza,
        Some(json!({"name": "Taken", "parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(error(&body), FOLDER_DUPLICATE_NAME);
    let tree_y = folder_in_tree(&router, &reza, y).await;
    assert_eq!(tree_y["name"], "Y", "{tree_y}");
    assert_eq!(
        tree_y["parentId"].as_str(),
        Some(reports.as_str()),
        "{tree_y}"
    );

    // CONTROL: the same body with a free name does both.
    let (status, done) = call(
        &router,
        "PATCH",
        &format!("/api/query/folders/{y}"),
        &reza,
        Some(json!({"name": "Free", "parentId": null})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{done}");
    assert_eq!(done["name"], "Free");
    assert!(done["parentId"].is_null(), "{done}");
}

/// D5: a personal space is not shared. `PUT .../access` on one is refused
/// with its own sentence and writes no rule, and a person who cannot see
/// the space still gets the 404. CONTROL: a folder inside the space is
/// shared, and it shows in the other person's tree.
#[tokio::test]
async fn a_personal_space_is_not_shared_but_a_folder_inside_it_is() {
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
    let space = saved["folderId"]
        .as_str()
        .expect("the save created her personal space")
        .to_owned();

    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{space}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "view"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {PERSONAL_SPACE_NOT_SHARED}")
    );
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saved_query_access_rule WHERE folder_id = $1")
            .bind(Uuid::parse_str(&space).expect("the space id"))
            .fetch_one(&pool)
            .await
            .expect("count the rules");
    assert_eq!(stored, 0, "the refused PUT wrote no rule");
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{space}/access"),
        &reza,
        Some(json!({"rules": []})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(error(&body), FOLDER_NOT_FOUND);

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
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        Some(json!({"rules": [{"userId": reza_id.to_string(), "level": "view"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let shared = folder_in_tree(&router, &reza, &weekly).await;
    assert_eq!(shared["level"], "view", "{shared}");
}

/// A rule that is stored on a personal space all the same (written before
/// the refusal existed) gives nobody anything: the person it names does not
/// see the space or what is in it.
#[tokio::test]
async fn a_rule_stored_on_a_personal_space_opens_nothing() {
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
    let private = saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned();
    let space = saved["folderId"]
        .as_str()
        .expect("the save created her personal space")
        .to_owned();
    sqlx::query(
        "INSERT INTO saved_query_access_rule (folder_id, user_id, level, granted_by) \
         VALUES ($1, $2, 'manage', $2)",
    )
    .bind(Uuid::parse_str(&space).expect("the space id"))
    .bind(reza_id)
    .execute(&pool)
    .await
    .expect("seed the rule on the personal space");

    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{private}"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{found}");
    assert_eq!(error(&found), QUERY_NOT_FOUND);
    let (status, tree) = call(&router, "GET", "/api/query/folders", &reza, None).await;
    assert_eq!(status, StatusCode::OK, "{tree}");
    assert!(
        !tree
            .as_array()
            .expect("the tree is an array")
            .iter()
            .any(|folder| folder["id"] == space.as_str()),
        "the space is not in his tree: {tree}"
    );
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{space}/access"),
        &reza,
        Some(json!({"rules": []})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

/// The access list names only rules that grant something (`QS-3` review of
/// #382, follow-up). A rule stored on a personal space gives nobody anything,
/// so the list of a query, of a folder inside the space and of the space itself
/// does not show it. CONTROL: a rule on a folder inside the space still
/// shows as inherited on what sits below that folder.
#[tokio::test]
async fn a_rule_stored_on_a_personal_space_is_not_listed_as_access() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let yoga = user_id(&pool, "yoga@meridian.example").await;
    let (private, space) = query_saved(&router, &rina, "Private", None).await;
    let weekly = folder_created(&router, &rina, "Weekly", Some(&space)).await;
    let archive = folder_created(&router, &rina, "Archive", Some(&weekly)).await;
    let (numbers, _) = query_saved(&router, &rina, "Numbers", Some(&archive)).await;
    // The rule on `Weekly` is real: the route accepts it.
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        Some(json!({"rules": [{"userId": yoga.to_string(), "level": "view"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    // The rule on the space is the leftover the resolver ignores; the
    // route refuses to write one, so it is seeded.
    sqlx::query(
        "INSERT INTO saved_query_access_rule (folder_id, user_id, level, granted_by) \
         VALUES ($1, $2, 'manage', $2)",
    )
    .bind(Uuid::parse_str(&space).expect("the space id"))
    .bind(reza_id)
    .execute(&pool)
    .await
    .expect("seed the rule on the personal space");

    // The query in the space, the sub-folder and the space itself list no
    // rule from the space.
    for path in [
        format!("/api/query/saved/{private}/access"),
        format!("/api/query/folders/{weekly}/access"),
    ] {
        let (status, list) = call(&router, "GET", &path, &rina, None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        assert!(
            list["inherited"]
                .as_array()
                .expect("the inherited list is an array")
                .is_empty(),
            "{path} inherits nothing from the space: {list}"
        );
    }
    let (status, list) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{space}/access"),
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
        "the space lists no rule of its own: {list}"
    );

    // `Weekly` still lists the rule seeded for it, as its own rule and not
    // as an inherited one.
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
    assert_eq!(rules.len(), 1, "Weekly lists its own rule: {list}");
    assert_eq!(rules[0]["userId"], yoga.to_string(), "{list}");
    assert_eq!(rules[0]["level"], "view", "{list}");

    // CONTROL: below `Weekly` the real rule is inherited, and the space's
    // rule is still left out.
    for path in [
        format!("/api/query/saved/{numbers}/access"),
        format!("/api/query/folders/{archive}/access"),
    ] {
        let (status, list) = call(&router, "GET", &path, &rina, None).await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let inherited = list["inherited"]
            .as_array()
            .expect("the inherited list is an array");
        assert_eq!(inherited.len(), 1, "{path} inherits only Weekly: {list}");
        assert_eq!(inherited[0]["folderId"], weekly, "{list}");
        assert_eq!(inherited[0]["userId"], yoga.to_string(), "{list}");
    }
}

// ── Sharing a folder needs manage on what it holds (`QS-3` review of #393) ──

/// `PUT /api/query/folders/{folder}/access` with `rules`.
async fn put_folder_rules(
    router: &axum::Router,
    cookie: &str,
    folder: &str,
    rules: Value,
) -> (StatusCode, Value) {
    call(
        router,
        "PUT",
        &format!("/api/query/folders/{folder}/access"),
        cookie,
        Some(json!({ "rules": rules })),
    )
    .await
}

/// The rules written on `folder` itself, read from the database.
async fn stored_rules(pool: &PgPool, folder: &str) -> Vec<(Uuid, String)> {
    sqlx::query_as(
        "SELECT user_id, level::text FROM saved_query_access_rule \
         WHERE folder_id = $1 AND user_id IS NOT NULL ORDER BY user_id",
    )
    .bind(Uuid::parse_str(folder).expect("the folder id"))
    .fetch_all(pool)
    .await
    .expect("read the stored rules")
}

/// The level `cookie` holds on the saved query `query`.
async fn level_on_query(router: &axum::Router, cookie: &str, query: &str) -> Value {
    let (status, found) = call(
        router,
        "GET",
        &format!("/api/query/saved/{query}"),
        cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{found}");
    found["level"].clone()
}

/// Review scenario (a): Reza created `X` inside `Reports` (so he manages
/// it) and Rina's query sits in it. Giving himself manage on `X` would make
/// Rina's query his to manage, so the share is refused, no rule is
/// written and his level on her query is what it was.
#[tokio::test]
async fn an_author_of_a_folder_cannot_give_himself_manage_over_another_persons_query_in_it() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let (_reports, x, query) = x_in_reports_holding_rinas_query(&router, &pool).await;
    let before = stored_rules(&pool, &x).await;
    assert_eq!(before.len(), 1, "Rina's edit rule is the only one");
    assert_eq!(level_on_query(&router, &reza, &query).await, "edit");

    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([
            {"userId": rina_id.to_string(), "level": "edit"},
            {"userId": reza_id.to_string(), "level": "manage"}
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );

    assert_eq!(stored_rules(&pool, &x).await, before, "nothing was written");
    assert_eq!(
        level_on_query(&router, &reza, &query).await,
        "edit",
        "his level on her query is unchanged"
    );
}

/// Review scenario (b): sharing `X` with a colleague would change what the
/// colleague may do with Rina's query, so it is refused and the colleague's
/// level on it is what it was. (The organisation reads a query that sits in
/// a shared folder, so the colleague holds view before and after.)
#[tokio::test]
async fn a_folder_holding_another_persons_query_is_not_shared_with_a_colleague() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let bob = a_third_reader(&pool).await;
    let bob_id = user_id(&pool, "nadia@meridian.example").await;
    let (_reports, x, query) = x_in_reports_holding_rinas_query(&router, &pool).await;
    assert_eq!(level_on_query(&router, &bob, &query).await, "view");
    let before = stored_rules(&pool, &x).await;

    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([
            {"userId": rina_id.to_string(), "level": "edit"},
            {"userId": bob_id.to_string(), "level": "edit"}
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );
    assert_eq!(stored_rules(&pool, &x).await, before, "nothing was written");
    assert_eq!(
        level_on_query(&router, &bob, &query).await,
        "view",
        "the colleague's level on her query is unchanged"
    );
}

/// Review scenario (b) in the place where it matters: `X` sits in Rina's
/// PERSONAL space, under her `Weekly` folder that she shares with Reza at
/// edit. Reza created `X` (he manages it) and Rina saved a query into it.
/// A colleague holds no level anywhere in that space, so sharing `X` with
/// the colleague would turn her invisible query into a visible one: it is
/// refused with the contents sentence, no rule is written, and the
/// colleague still gets the 404 on the query. Reza giving himself manage on
/// `X` is refused the same way.
#[tokio::test]
async fn a_folder_in_a_personal_space_holding_another_persons_query_is_not_shared_with_a_colleague()
{
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let bob = a_third_reader(&pool).await;
    let bob_id = user_id(&pool, "nadia@meridian.example").await;

    let (_, space) = query_saved(&router, &rina, "Private", None).await;
    let weekly = folder_created(&router, &rina, "Weekly", Some(&space)).await;
    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &weekly,
        json!([{"userId": reza_id.to_string(), "level": "edit"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let x = folder_created(&router, &reza, "X", Some(&weekly)).await;
    let (query, _) = query_saved(&router, &rina, "Rina's numbers", Some(&x)).await;

    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{query}"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{found}");
    let before = stored_rules(&pool, &x).await;
    assert!(before.is_empty(), "X has no rule yet: {before:?}");

    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([{"userId": bob_id.to_string(), "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );
    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([{"userId": reza_id.to_string(), "level": "manage"}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );

    assert!(
        stored_rules(&pool, &x).await.is_empty(),
        "no rule was written on X"
    );
    let (status, found) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{query}"),
        &bob,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{found}");
    assert_eq!(error(&found), QUERY_NOT_FOUND);
}

/// CONTROL for (a) and (b), review scenario (c): a folder that holds only
/// the caller's own queries and sub-folders is shared.
#[tokio::test]
async fn a_folder_holding_only_the_callers_own_work_is_shared() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let bob = a_third_reader(&pool).await;
    let bob_id = user_id(&pool, "nadia@meridian.example").await;
    let reports = a_shared_folder_opened_to_reza(&pool, "edit").await;
    let x = folder_created(&router, &reza, "X", Some(&reports)).await;
    let inner = folder_created(&router, &reza, "Inner", Some(&x)).await;
    let (own, _) = query_saved(&router, &reza, "Reza's own", Some(&x)).await;
    query_saved(&router, &reza, "Reza's deeper", Some(&inner)).await;

    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([{"userId": bob_id.to_string(), "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(stored_rules(&pool, &x).await.len(), 1);
    assert_eq!(level_on_query(&router, &bob, &own).await, "view");
}

/// Review scenario (d): a query of Rina's that is in the trash still
/// counts, because a restore brings it back into the folder.
#[tokio::test]
async fn a_trashed_query_of_another_person_blocks_the_share_of_its_folder() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    a_third_reader(&pool).await;
    let bob_id = user_id(&pool, "nadia@meridian.example").await;
    let (_reports, x, query) = x_in_reports_holding_rinas_query(&router, &pool).await;
    sqlx::query("UPDATE saved_query SET deleted_at = now() WHERE id = $1")
        .bind(Uuid::parse_str(&query).expect("the query id"))
        .execute(&pool)
        .await
        .expect("move Rina's query to the trash");
    let before = stored_rules(&pool, &x).await;

    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([{"userId": bob_id.to_string(), "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );
    assert_eq!(stored_rules(&pool, &x).await, before, "nothing was written");
}

/// Review scenario (e): a sub-folder that Rina authored under `X`, with
/// nothing in it, still blocks the share of `X`: Reza holds only edit on it.
#[tokio::test]
async fn an_empty_sub_folder_of_another_person_blocks_the_share_of_its_parent() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    a_third_reader(&pool).await;
    let bob_id = user_id(&pool, "nadia@meridian.example").await;
    let reports = a_shared_folder_opened_to_reza(&pool, "edit").await;
    let x = folder_created(&router, &reza, "X", Some(&reports)).await;
    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([{"userId": rina_id.to_string(), "level": "edit"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    folder_created(&router, &rina, "Rina's sub-folder", Some(&x)).await;
    let before = stored_rules(&pool, &x).await;

    let (status, body) = put_folder_rules(
        &router,
        &reza,
        &x,
        json!([{"userId": bob_id.to_string(), "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );
    assert_eq!(stored_rules(&pool, &x).await, before, "nothing was written");
}

/// Review scenario (f): Rina manages everything under her own folder, so
/// she shares it.
#[tokio::test]
async fn someone_who_manages_everything_in_a_folder_shares_it() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let weekly = folder_created(&router, &rina, "Weekly", None).await;
    let inner = folder_created(&router, &rina, "Inner", Some(&weekly)).await;
    query_saved(&router, &rina, "Top", Some(&weekly)).await;
    let (deep, _) = query_saved(&router, &rina, "Deep", Some(&inner)).await;

    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &weekly,
        json!([{"userId": reza_id.to_string(), "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(level_on_query(&router, &reza, &deep).await, "view");
}

/// Review scenario (g): a PUT with no rule at all removes the rules of the
/// folder, which changes who is covered, so it is refused in the same way.
#[tokio::test]
async fn an_empty_rule_list_is_refused_for_a_folder_holding_another_persons_query() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let (_reports, x, _query) = x_in_reports_holding_rinas_query(&router, &pool).await;
    let before = stored_rules(&pool, &x).await;
    assert_eq!(before.len(), 1, "Rina's edit rule is there to be removed");

    let (status, body) = put_folder_rules(&router, &reza, &x, json!([])).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {SHARE_NEEDS_MANAGE_OF_CONTENTS}")
    );
    assert_eq!(stored_rules(&pool, &x).await, before, "nothing was written");
}

/// Review scenario (h): the checks before the new one keep their order. A
/// caller who may not see the folder gets the 404, and a caller who may see
/// it but not manage it gets the manage 403; both win over the contents 403
/// for a folder that holds another person's query. A personal space gets its
/// own 403. Its owner manages all content in the space, so the contents rule
/// cannot also refuse there.
#[tokio::test]
async fn the_earlier_refusals_of_a_folder_share_win_over_the_contents_rule() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let bob = a_third_reader(&pool).await;
    let (reports, x, _query) = x_in_reports_holding_rinas_query(&router, &pool).await;

    // Manage 403: Reza edits `Reports` and Bob sees `X` through the
    // organisation, and neither manages what they ask to share.
    let (status, body) = put_folder_rules(&router, &reza, &reports, json!([])).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {FOLDER_FORBIDDEN}")
    );
    let (status, body) = put_folder_rules(&router, &bob, &x, json!([])).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {FOLDER_FORBIDDEN}")
    );

    // Rina shares `Weekly`, a folder inside her personal space, with Reza.
    // Reza holds edit on it and still gets the 404 for the space below.
    let (_private, space) = query_saved(&router, &rina, "Private", None).await;
    let weekly = folder_created(&router, &rina, "Weekly", Some(&space)).await;
    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &weekly,
        json!([{"userId": reza_id.to_string(), "level": "edit"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // 404: Reza cannot see the space at all.
    let (status, body) = put_folder_rules(&router, &reza, &space, json!([])).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(error(&body), FOLDER_NOT_FOUND);

    // Personal-space 403 for Rina, the owner. She manages all content in her
    // space, so the contents rule cannot also refuse here.
    let (status, body) = put_folder_rules(&router, &rina, &space, json!([])).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        error(&body),
        format!("permission_denied: {PERSONAL_SPACE_NOT_SHARED}")
    );
}

// ── A folder says who it is shared with, to edit and up (`QS-3`, folder sharing warning) ──

/// The rule of `list` (`rules` or `inherited`) that names the person or the
/// role `grantee`, under the key `key` (`"userId"` or `"roleId"`).
fn rule_naming<'a>(list: &'a Value, which: &str, key: &str, grantee: &str) -> &'a Value {
    list[which]
        .as_array()
        .unwrap_or_else(|| panic!("{which} is an array: {list}"))
        .iter()
        .find(|rule| rule[key] == grantee)
        .unwrap_or_else(|| panic!("{which} holds a rule for {key} {grantee}: {list}"))
}

/// The Analyst role of the seed.
const ANALYST_ROLE: &str = "22222222-2222-4222-8222-000000000001";
/// The Approver role of the seed.
const APPROVER_ROLE: &str = "22222222-2222-4222-8222-000000000002";

/// Who may read the access list of a folder: edit and up. The list is the
/// folder's own rules and the rules it inherits, with the level the reader
/// holds, and `grantedBy`, `grantedAt` and the inherited rule's `folderName`
/// are there as a manager sees them. The rules are still changed with
/// manage only.
#[tokio::test]
async fn an_edit_holder_reads_a_folders_own_and_inherited_rules() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let rina_id = user_id(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await;
    let yoga_id = user_id(&pool, "yoga@meridian.example").await;
    let reports = folder_created(&router, &rina, "Reports", None).await;
    let weekly = folder_created(&router, &rina, "Weekly", Some(&reports)).await;
    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &reports,
        json!([{"userId": reza_id.to_string(), "level": "edit"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &weekly,
        json!([{"userId": yoga_id.to_string(), "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Reza holds edit on `Weekly` through `Reports`: the list is his to read.
    let (status, list) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{weekly}/access"),
        &reza,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{list}");
    assert_eq!(list["yourLevel"], "edit", "{list}");
    let own = rule_naming(&list, "rules", "userId", &yoga_id.to_string());
    assert_eq!(own["level"], "view", "{list}");
    assert_eq!(own["grantedBy"], rina_id.to_string(), "{list}");
    assert!(own["grantedAt"].is_string(), "{list}");
    let inherited = rule_naming(&list, "inherited", "userId", &reza_id.to_string());
    assert_eq!(inherited["level"], "edit", "{list}");
    assert_eq!(inherited["folderId"], reports, "{list}");
    assert_eq!(inherited["folderName"], "Reports", "{list}");

    // Reading is all that edit gives: the rules stay manage's to change.
    let (status, answer) = put_folder_rules(&router, &reza, &weekly, json!([])).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(
        error(&answer),
        format!("permission_denied: {FOLDER_FORBIDDEN}")
    );
}

/// A person who may only view a folder is told no: the sentence says what
/// the caller would need to see the list, and the list stays closed below
/// edit.
#[tokio::test]
async fn a_view_holder_is_refused_a_folders_access_list() {
    let TestApp { router, pool } = spin_up().await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let folder_id = a_shared_folder_opened_to_reza(&pool, "view").await;

    let (status, answer) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{folder_id}/access"),
        &reza,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::FORBIDDEN, "{answer}");
    assert_eq!(
        error(&answer),
        format!("permission_denied: {FOLDER_ACCESS_FORBIDDEN}")
    );
}

/// A folder the caller may not see stays a 404 at the lower edit bar: the
/// personal space of another person.
#[tokio::test]
async fn a_folder_the_caller_may_not_see_is_still_not_found_for_its_access_list() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza = session_cookie_for_seeded_user(&pool, "reza@meridian.example").await;
    let (_, space) = query_saved(&router, &rina, "Private", None).await;

    let (status, answer) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{space}/access"),
        &reza,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::NOT_FOUND, "{answer}");
    assert_eq!(error(&answer), FOLDER_NOT_FOUND);
}

/// Every access list names its grantees, so the console shows who a rule is
/// for and not an id: a person by name and email, a role by name with no
/// email. The names are on the own and on the inherited rules of a query and
/// of a folder, and on the answer of a `PUT` as on a `GET`.
#[tokio::test]
async fn the_access_lists_name_their_people_and_roles() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let reza_id = user_id(&pool, "reza@meridian.example").await.to_string();
    let yoga_id = user_id(&pool, "yoga@meridian.example").await.to_string();
    let citra_id = user_id(&pool, "citra@meridian.example").await.to_string();
    let reports = folder_created(&router, &rina, "Reports", None).await;
    let weekly = folder_created(&router, &rina, "Weekly", Some(&reports)).await;
    let (query, _) = query_saved(&router, &rina, "Numbers", Some(&weekly)).await;
    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &reports,
        json!([
            {"userId": reza_id, "level": "edit"},
            {"roleId": ANALYST_ROLE, "level": "view"},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, put_answer) = put_folder_rules(
        &router,
        &rina,
        &weekly,
        json!([{"userId": citra_id, "level": "view"}]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{put_answer}");
    let (status, body) = call(
        &router,
        "PUT",
        &format!("/api/query/saved/{query}/access"),
        &rina,
        Some(json!({"rules": [{"userId": yoga_id, "level": "view"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, folder_list) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{weekly}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{folder_list}");
    let (status, query_list) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{query}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{query_list}");

    // The folder's list, read and as written: its own rule, and the two it
    // inherits from `Reports`.
    for list in [&folder_list, &put_answer] {
        let citra = rule_naming(list, "rules", "userId", &citra_id);
        assert_eq!(citra["granteeName"], "Citra Amelia", "{list}");
        assert_eq!(citra["granteeEmail"], "citra@meridian.example", "{list}");
        let reza = rule_naming(list, "inherited", "userId", &reza_id);
        assert_eq!(reza["granteeName"], "Reza Hakim", "{list}");
        assert_eq!(reza["granteeEmail"], "reza@meridian.example", "{list}");
        let analyst = rule_naming(list, "inherited", "roleId", ANALYST_ROLE);
        assert_eq!(analyst["granteeName"], "Analyst", "{list}");
        assert_eq!(
            analyst.get("granteeEmail"),
            Some(&Value::Null),
            "a role has no email, and the key is there: {list}"
        );
    }

    // The query's list: its own rule, and both folders above it.
    let yoga = rule_naming(&query_list, "rules", "userId", &yoga_id);
    assert_eq!(yoga["granteeName"], "Yoga Prasetya", "{query_list}");
    assert_eq!(yoga["granteeEmail"], "yoga@meridian.example");
    let citra = rule_naming(&query_list, "inherited", "userId", &citra_id);
    assert_eq!(citra["granteeName"], "Citra Amelia", "{query_list}");
    let analyst = rule_naming(&query_list, "inherited", "roleId", ANALYST_ROLE);
    assert_eq!(analyst["granteeName"], "Analyst", "{query_list}");
    assert_eq!(analyst.get("granteeEmail"), Some(&Value::Null));
}

/// A rule whose person is not in the caller's organisation names nobody:
/// the list carries the id and `null` for the name and the email, never the
/// name of another organisation's person. A person who is no longer active
/// is still named, because the rule that names them still applies.
#[tokio::test]
async fn a_person_outside_the_tenant_is_not_named_and_an_inactive_one_is() {
    let TestApp { router, pool } = spin_up().await;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    // Sari belongs to the Retail tenant only; Yoga to the Group's, and is
    // deactivated here.
    let sari_id = user_id(&pool, "sari@meridian.example").await;
    let yoga_id = user_id(&pool, "yoga@meridian.example").await;
    sqlx::query("UPDATE app_user SET status = 'inactive' WHERE id = $1")
        .bind(yoga_id)
        .execute(&pool)
        .await
        .expect("deactivate the person");
    let reports = folder_created(&router, &rina, "Reports", None).await;
    let (query, _) = query_saved(&router, &rina, "Numbers", Some(&reports)).await;
    // The route refuses a grantee outside the tenant, so a rule that
    // outlived one is written by hand.
    for grantee in [sari_id, yoga_id] {
        sqlx::query(
            "INSERT INTO saved_query_access_rule (folder_id, user_id, level, granted_by) \
             VALUES ($1, $2, 'view', $3)",
        )
        .bind(Uuid::parse_str(&reports).expect("the folder id"))
        .bind(grantee)
        .bind(user_id(&pool, "rina@meridian.example").await)
        .execute(&pool)
        .await
        .expect("write the rule");
    }

    // The folder lists the rules as its own, the query as inherited.
    let (status, folder_list) = call(
        &router,
        "GET",
        &format!("/api/query/folders/{reports}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{folder_list}");
    let (status, query_list) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{query}/access"),
        &rina,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{query_list}");

    for (list, which) in [(&folder_list, "rules"), (&query_list, "inherited")] {
        let sari = rule_naming(list, which, "userId", &sari_id.to_string());
        assert_eq!(sari.get("granteeName"), Some(&Value::Null), "{list}");
        assert_eq!(sari.get("granteeEmail"), Some(&Value::Null), "{list}");
        let yoga = rule_naming(list, which, "userId", &yoga_id.to_string());
        assert_eq!(yoga["granteeName"], "Yoga Prasetya", "{list}");
        assert_eq!(yoga["granteeEmail"], "yoga@meridian.example", "{list}");
    }
}

/// The names cost two queries at most, whatever the number of rules: one
/// list holds a person and a role, the other five people and four roles, and
/// both cost the same number of pool checkouts.
#[tokio::test]
async fn more_rules_do_not_add_a_round_trip_to_an_access_list() {
    let (app, checkouts) =
        spin_up_with_env_counting_acquires(&std::collections::HashMap::new()).await;
    let TestApp { router, pool } = app;
    let rina = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let small = folder_created(&router, &rina, "Small", None).await;
    let large = folder_created(&router, &rina, "Large", None).await;
    let mut people = Vec::new();
    for email in [
        "reza@meridian.example",
        "bayu@meridian.example",
        "dewi@meridian.example",
        "citra@meridian.example",
        "yoga@meridian.example",
    ] {
        people.push(user_id(&pool, email).await.to_string());
    }
    let (status, body) = put_folder_rules(
        &router,
        &rina,
        &small,
        json!([
            {"userId": people[0], "level": "view"},
            {"roleId": ANALYST_ROLE, "level": "view"},
        ]),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let mut many: Vec<Value> = people
        .iter()
        .map(|id| json!({"userId": id, "level": "view"}))
        .collect();
    for role in [
        ANALYST_ROLE,
        APPROVER_ROLE,
        "22222222-2222-4222-8222-000000000003",
        "22222222-2222-4222-8222-000000000004",
    ] {
        many.push(json!({"roleId": role, "level": "view"}));
    }
    let (status, body) = put_folder_rules(&router, &rina, &large, Value::Array(many)).await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // Warm-up: the first request of a fresh harness opens what later ones
    // reuse, so it is not part of the measurement.
    let path = |folder: &String| format!("/api/query/folders/{folder}/access");
    let (status, _) = call(&router, "GET", &path(&small), &rina, None).await;
    assert_eq!(status, StatusCode::OK);

    let before = checkouts.load(std::sync::atomic::Ordering::SeqCst);
    let (status, list) = call(&router, "GET", &path(&small), &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let small_cost = checkouts.load(std::sync::atomic::Ordering::SeqCst) - before;
    assert_eq!(list["rules"].as_array().map(Vec::len), Some(2), "{list}");

    let before = checkouts.load(std::sync::atomic::Ordering::SeqCst);
    let (status, list) = call(&router, "GET", &path(&large), &rina, None).await;
    assert_eq!(status, StatusCode::OK, "{list}");
    let large_cost = checkouts.load(std::sync::atomic::Ordering::SeqCst) - before;
    assert_eq!(list["rules"].as_array().map(Vec::len), Some(9), "{list}");

    assert!(small_cost > 0, "the counter counts: {small_cost}");
    assert_eq!(
        large_cost, small_cost,
        "nine rules cost as many checkouts as two"
    );
}

// ── Parameters (`query-parameters.md`, decision 3) ───────────────────────────────────────

/// The sentence a `parameters` value that is not a list of definitions is
/// refused with. Fixed: it never quotes the caller's JSON back.
const PARAMETERS_SHAPE: &str = "parameters must be a list of parameter definitions, each with a \
     name, a type, a default and options.";

/// The SQL the parameter tests save: one placeholder, `{{region}}`.
const BY_REGION: &str = "SELECT * FROM serving.mart_wisman WHERE region = {{region}}";

/// A text parameter named `region` whose default is `default`.
fn region(default: &str) -> Value {
    json!({"name": "region", "type": "text", "default": default})
}

/// Save `body` and answer the status and the answer.
async fn save(router: &axum::Router, cookie: &str, body: Value) -> (StatusCode, Value) {
    call(router, "POST", "/api/query/saved", cookie, Some(body)).await
}

/// The id of a saved-query answer.
fn id_of(saved: &Value) -> String {
    saved["id"]
        .as_str()
        .expect("the saved query has an id")
        .to_owned()
}

/// The versions of a query, newest first.
async fn versions_of(router: &axum::Router, cookie: &str, id: &str) -> Vec<Value> {
    let (status, versions) = call(
        router,
        "GET",
        &format!("/api/query/saved/{id}/versions"),
        cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{versions}");
    versions
        .as_array()
        .expect("the versions are an array")
        .clone()
}

/// `QS-5-AC4`: a saved query keeps its parameters. Changing only a
/// parameter's default writes a version, a revert brings back the
/// parameters of the version with its SQL, and a duplicate carries them.
#[tokio::test]
async fn a_saved_query_keeps_its_parameters_in_every_version_and_a_revert_and_a_copy_carry_them() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, saved) = save(
        &router,
        &cookie,
        json!({"title": "By region", "sql": BY_REGION, "parameters": [region("EU")]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["parameters"], json!([region("EU")]));
    let id = id_of(&saved);

    let (status, read) = call(
        &router,
        "GET",
        &format!("/api/query/saved/{id}"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{read}");
    assert_eq!(
        read["parameters"],
        json!([region("EU")]),
        "get carries them"
    );
    let (status, listed) = call(&router, "GET", "/api/query/saved", &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let listed = listed
        .as_array()
        .expect("the list is an array")
        .iter()
        .find(|q| q["id"] == id)
        .expect("the query is listed")
        .clone();
    assert_eq!(listed["parameters"], json!([region("EU")]));
    let versions = versions_of(&router, &cookie, &id).await;
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["parameters"], json!([region("EU")]));

    // Only the default changes: that is a version.
    let (status, patched) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &cookie,
        Some(json!({"parameters": [region("US")]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["parameters"], json!([region("US")]));
    assert_eq!(patched["sql"], BY_REGION, "the SQL did not move");
    let versions = versions_of(&router, &cookie, &id).await;
    assert_eq!(versions.len(), 2, "a parameters-only change is a version");
    assert_eq!(versions[0]["parameters"], json!([region("US")]));
    assert_eq!(versions[1]["parameters"], json!([region("EU")]));

    // The same list again writes nothing, and a new title keeps the list.
    let (status, again) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &cookie,
        Some(json!({"parameters": [region("US")]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(versions_of(&router, &cookie, &id).await.len(), 2);
    let (status, renamed) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{id}"),
        &cookie,
        Some(json!({"title": "By region, renamed"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["parameters"], json!([region("US")]));

    // Back to version 1: its SQL and its parameters together.
    let (status, reverted) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/versions/1/revert"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{reverted}");
    assert_eq!(reverted["parameters"], json!([region("EU")]));
    assert_eq!(reverted["sql"], BY_REGION);
    let versions = versions_of(&router, &cookie, &id).await;
    assert_eq!(versions[0]["label"], "Restored version 1");
    assert_eq!(versions[0]["parameters"], json!([region("EU")]));

    let (status, copy) = call(
        &router,
        "POST",
        &format!("/api/query/saved/{id}/duplicate"),
        &cookie,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{copy}");
    assert_ne!(copy["id"], id.as_str());
    assert_eq!(copy["parameters"], json!([region("EU")]));
    let copy_versions = versions_of(&router, &cookie, &id_of(&copy)).await;
    assert_eq!(copy_versions[0]["parameters"], json!([region("EU")]));
}

/// A query with no placeholder and no parameters saves exactly as before,
/// with an empty list; and a patch that leaves the SQL and the parameters
/// alone does not read the stored SQL again, so a query saved before `QS-5`
/// that holds a `{{` still renames.
#[tokio::test]
async fn a_query_without_parameters_saves_as_before_and_a_rename_does_not_rescan_old_sql() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let (status, saved) = save(
        &router,
        &cookie,
        json!({"title": "Plain", "sql": "SELECT 1"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["parameters"], json!([]));
    let (status, saved) = save(
        &router,
        &cookie,
        json!({"title": "Explicit", "sql": "SELECT 1", "parameters": null}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    assert_eq!(saved["parameters"], json!([]));

    // A row written before the scanner existed, with a placeholder in quotes.
    let owner = user_id(&pool, "rina@meridian.example").await;
    let old: Uuid = sqlx::query_scalar(
        "INSERT INTO saved_query (title, sql, owner, owner_id, tenant_id) \
         VALUES ('Old', 'SELECT ''{{x}}''', 'Rina Wijaya', $1, $2) RETURNING id",
    )
    .bind(owner)
    .bind(Uuid::parse_str(GROUP_TENANT).expect("the group tenant id"))
    .fetch_one(&pool)
    .await
    .expect("insert the old query");
    let (status, renamed) = call(
        &router,
        "PATCH",
        &format!("/api/query/saved/{old}"),
        &cookie,
        Some(json!({"title": "Old, renamed", "tags": ["legacy"]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{renamed}");
    assert_eq!(renamed["title"], "Old, renamed");
    assert_eq!(renamed["parameters"], json!([]));
}

/// `QS-5` decision 11 and the limits of decision 6, on save: a definition
/// the SQL does not use, a placeholder with no definition, a placeholder in
/// quotes, a definition of the wrong shape, and 21 / 201 are each a 400 with
/// a fixed sentence, and nothing is stored.
#[tokio::test]
async fn a_save_that_the_definitions_do_not_fit_is_a_400_with_a_fixed_sentence() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;

    let options: Vec<Value> = (0..=200)
        .map(|n| json!({"value": format!("v{n}"), "label": format!("V{n}")}))
        .collect();
    let too_many_definitions: Vec<Value> = (0..=20)
        .map(|n| json!({"name": format!("p{n}"), "type": "text"}))
        .collect();
    let all_placeholders = (0..=20)
        .map(|n| format!("{{{{p{n}}}}}"))
        .collect::<Vec<_>>()
        .join(", ");

    let cases: Vec<(&str, Value, &str)> = vec![
        (
            "SELECT 1",
            json!([region("EU")]),
            "{{region}} has a definition but the SQL does not use it.",
        ),
        (
            BY_REGION,
            json!([]),
            "{{region}} is used in the SQL but has no definition.",
        ),
        (
            "SELECT '{{region}}'",
            json!([region("EU")]),
            "{{region}} is inside quotes: write it without them, for example = {{region}}.",
        ),
        (
            "SELECT {{ not a parameter }}",
            json!([]),
            "This query has a {{ that is not a parameter.",
        ),
        (
            BY_REGION,
            json!([region("EU"), region("US")]),
            "Two parameters are named {{region}}.",
        ),
        (BY_REGION, json!("nope"), PARAMETERS_SHAPE),
        (BY_REGION, json!({"name": "region"}), PARAMETERS_SHAPE),
        (
            BY_REGION,
            json!([{"name": "region", "type": "secret-bogus"}]),
            PARAMETERS_SHAPE,
        ),
        (
            BY_REGION,
            json!([{"name": "region", "type": "text", "surprise": 1}]),
            PARAMETERS_SHAPE,
        ),
        (
            "SELECT 1",
            json!(too_many_definitions),
            "A query takes at most 20 parameters.",
        ),
        (
            "SELECT {{c}}",
            json!([{"name": "c", "type": "dropdown", "options": options}]),
            "{{c}} has more than 200 options; a dropdown or multi-select takes at most 200.",
        ),
    ];
    for (sql, parameters, expected) in cases {
        let (status, body) = save(
            &router,
            &cookie,
            json!({"title": "Refused", "sql": sql, "parameters": parameters}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{sql}: {body}");
        assert_eq!(error(&body), expected, "{sql}");
        assert!(
            !error(&body).contains("secret-bogus") && !error(&body).contains("surprise"),
            "the sentence never repeats the caller's own JSON: {body}"
        );
    }

    // 21 placeholders and no definition for any: the limit, not 21 sentences.
    let (status, body) = save(
        &router,
        &cookie,
        json!({"title": "Refused", "sql": format!("SELECT {all_placeholders}")}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), "A query takes at most 20 parameters.");

    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM saved_query WHERE title = 'Refused'")
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(stored, 0, "a refused save stores nothing");
}

/// A patch is checked against the SQL the query will have: the new SQL when
/// the body carries one, the stored SQL otherwise, with the new definitions
/// or the stored ones. A refused patch writes nothing.
#[tokio::test]
async fn a_patch_is_checked_against_the_sql_the_query_will_have() {
    let TestApp { router, pool } = spin_up().await;
    let cookie = session_cookie_for_seeded_user(&pool, "rina@meridian.example").await;
    let (status, saved) = save(
        &router,
        &cookie,
        json!({"title": "By region", "sql": BY_REGION, "parameters": [region("EU")]}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{saved}");
    let id = id_of(&saved);
    let path = format!("/api/query/saved/{id}");

    // New SQL that drops the placeholder strands the stored definition.
    let (status, body) = call(
        &router,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"sql": "SELECT 1"})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        error(&body),
        "{{region}} has a definition but the SQL does not use it."
    );

    // New SQL with a placeholder nobody defined.
    let (status, body) = call(
        &router,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"sql": format!("{BY_REGION} AND year = {{{{year}}}}")})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        error(&body),
        "{{year}} is used in the SQL but has no definition."
    );

    // New definitions the stored SQL does not use.
    let (status, body) = call(
        &router,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"parameters": [region("EU"), {"name": "year", "type": "number"}]})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(
        error(&body),
        "{{year}} has a definition but the SQL does not use it."
    );

    // A malformed list.
    let (status, body) = call(
        &router,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"parameters": {"region": "EU"}})),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert_eq!(error(&body), PARAMETERS_SHAPE);

    let (status, unchanged) = call(&router, "GET", &path, &cookie, None).await;
    assert_eq!(status, StatusCode::OK, "{unchanged}");
    assert_eq!(unchanged["sql"], BY_REGION);
    assert_eq!(unchanged["parameters"], json!([region("EU")]));
    assert_eq!(
        versions_of(&router, &cookie, &id).await.len(),
        1,
        "a refused patch writes no version"
    );

    // The SQL and the definitions move together: valid against the new SQL.
    let (status, patched) = call(
        &router,
        "PATCH",
        &path,
        &cookie,
        Some(json!({
            "sql": "SELECT * FROM serving.mart_wisman WHERE year = {{year}}",
            "parameters": [{"name": "year", "type": "number", "default": 2025}],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(
        patched["parameters"],
        json!([{"name": "year", "type": "number", "default": 2025}])
    );
    assert_eq!(versions_of(&router, &cookie, &id).await.len(), 2);

    // Clearing the list with SQL that no longer needs it.
    let (status, patched) = call(
        &router,
        "PATCH",
        &path,
        &cookie,
        Some(json!({"sql": "SELECT 1", "parameters": []})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{patched}");
    assert_eq!(patched["parameters"], json!([]));
}
