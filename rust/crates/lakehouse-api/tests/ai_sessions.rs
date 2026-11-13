//! `/api/ai/sessions`: a conversation is the caller's alone, a renamed
//! title survives later saves, a deleted conversation stays deleted, and the
//! list is newest first (`AI-24` T1).
//!
//! These tests need a real `ClickHouse`, because the session store is one
//! (`console.chat_session`, a `ReplacingMergeTree`) and a `wiremock` stand-in
//! would only prove the request's shape, not that `FINAL` returns the newest
//! version. The arrangement is `tests/query_result_cap.rs`'s: one disposable
//! `ClickHouse` per test binary, one fresh Postgres per test from
//! [`spin_up_with_env`]. Every test mints its own principals with random
//! ids, so the shared `ClickHouse` never shows one test another's rows.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{Value, json};
use testcontainers::ImageExt;
use testcontainers::runners::AsyncRunner;
use testcontainers_modules::clickhouse::ClickHouse;
use tower::ServiceExt;
use uuid::Uuid;

use common::{TestApp, create_assistant_principal, session_cookie_for_user, spin_up_with_env};

static CLICKHOUSE: tokio::sync::OnceCell<(String, testcontainers::ContainerAsync<ClickHouse>)> =
    tokio::sync::OnceCell::const_new();

/// ONE `ClickHouse` for the whole test binary; see `tests/query_result_cap.rs`
/// for why per-test containers race.
async fn shared_clickhouse() -> &'static str {
    &CLICKHOUSE
        .get_or_init(|| async {
            let container = ClickHouse::default()
                .with_env_var("CLICKHOUSE_USER", "default")
                .with_env_var("CLICKHOUSE_PASSWORD", "")
                .with_env_var("CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT", "1")
                .with_tag("26.8")
                .start()
                .await
                .expect("start the ClickHouse testcontainer");
            let host = container.get_host().await.expect("container host");
            let port = container
                .get_host_port_ipv4(8123)
                .await
                .expect("container HTTP port");
            (format!("http://{host}:{port}"), container)
        })
        .await
        .0
}

/// A test app pointed at the shared `ClickHouse`, plus two fresh principals:
/// the conversation's owner and somebody else.
async fn app_with_two_users() -> (TestApp, String, String, Uuid, Uuid) {
    let ch_url = shared_clickhouse().await;
    let mut env = HashMap::new();
    env.insert("CH_URL".to_owned(), ch_url.to_owned());
    let app = spin_up_with_env(&env).await;
    // `ADM-13` D5: both people may use the assistant and hold nothing else;
    // the tests are about whose conversation each may read, not permissions.
    let owner = create_assistant_principal(&app.pool, "").await;
    let other = create_assistant_principal(&app.pool, "").await;
    let owner_cookie = session_cookie_for_user(&app.pool, owner).await;
    let other_cookie = session_cookie_for_user(&app.pool, other).await;
    (app, owner_cookie, other_cookie, owner, other)
}

async fn send(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .header("cookie", cookie)
        .body(Body::from(body.map_or_else(String::new, |v| v.to_string())))
        .expect("build request");
    let response = router.clone().oneshot(request).await.expect("request");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

const SESSIONS: &str = "/api/ai/sessions";

fn conversation(text: &str) -> Value {
    json!({ "mode": "ask", "messages": [{ "role": "user", "content": text }] })
}

/// Saves one conversation and returns its id.
async fn save(router: &axum::Router, cookie: &str, text: &str) -> String {
    let (status, body) = send(router, "POST", SESSIONS, cookie, Some(conversation(text))).await;
    assert_eq!(status, StatusCode::OK, "save answered {body}");
    body["id"].as_str().expect("a new id").to_owned()
}

/// `updated_at` is both the row's version and the list's sort key, and it is
/// a `DateTime` (whole seconds). Two writes in the same second share a
/// version, and `ReplacingMergeTree` gives no order between equal versions —
/// so a test that asserts "the later write wins" waits out the second.
async fn next_second() {
    tokio::time::sleep(Duration::from_millis(1100)).await;
}

#[tokio::test]
async fn a_renamed_title_survives_a_later_save() {
    let (app, owner, _other, _owner_id, _other_id) = app_with_two_users().await;
    let id = save(&app.router, &owner, "Cari penjualan bulan lalu").await;

    let (status, renamed) = send(
        &app.router,
        "PATCH",
        SESSIONS,
        &owner,
        Some(json!({ "id": id, "title": "Penjualan bulanan" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "rename answered {renamed}");
    assert_eq!(renamed["title"], "Penjualan bulanan");

    next_second().await;
    let (status, saved) = send(
        &app.router,
        "POST",
        SESSIONS,
        &owner,
        Some(json!({
            "id": id,
            "mode": "ask",
            "messages": [
                { "role": "user", "content": "Cari penjualan bulan lalu" },
                { "role": "assistant", "content": "Berikut angkanya." },
            ],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "save answered {saved}");
    assert_eq!(
        saved["title"], "Penjualan bulanan",
        "a locked title is not re-derived from the first message"
    );

    let (status, opened) = send(
        &app.router,
        "GET",
        &format!("{SESSIONS}?id={id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["session"]["title"], "Penjualan bulanan");
}

#[tokio::test]
async fn a_deleted_conversation_is_not_found_afterwards() {
    let (app, owner, _other, _owner_id, _other_id) = app_with_two_users().await;
    let id = save(&app.router, &owner, "Hapus percakapan ini").await;

    let (status, deleted) = send(
        &app.router,
        "DELETE",
        &format!("{SESSIONS}?id={id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "delete answered {deleted}");

    let (status, body) = send(
        &app.router,
        "GET",
        &format!("{SESSIONS}?id={id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "opening it answered {body}");
    assert_eq!(body["error"], "sesi tidak ditemukan");

    let (status, listed) = send(&app.router, "GET", SESSIONS, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(listed["sessions"], json!([]));
}

#[tokio::test]
async fn a_save_to_a_deleted_id_is_not_found_and_does_not_bring_it_back() {
    let (app, owner, _other, _owner_id, _other_id) = app_with_two_users().await;
    let id = save(&app.router, &owner, "Percakapan yang dihapus").await;
    let (status, _) = send(
        &app.router,
        "DELETE",
        &format!("{SESSIONS}?id={id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, body) = send(
        &app.router,
        "POST",
        SESSIONS,
        &owner,
        Some(json!({
            "id": id,
            "mode": "ask",
            "messages": [{ "role": "user", "content": "Coba simpan lagi" }],
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "save answered {body}");
    assert_eq!(body["error"], "sesi tidak ditemukan");

    let (status, _) = send(
        &app.router,
        "GET",
        &format!("{SESSIONS}?id={id}"),
        &owner,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "the deleted row stayed deleted"
    );
    let (_, listed) = send(&app.router, "GET", SESSIONS, &owner, None).await;
    assert_eq!(listed["sessions"], json!([]));
}

#[tokio::test]
async fn another_users_conversation_is_not_found_for_get_rename_and_delete() {
    let (app, owner, other, _owner_id, _other_id) = app_with_two_users().await;
    let id = save(&app.router, &owner, "Punya pemilik").await;
    let uri = format!("{SESSIONS}?id={id}");

    // 404, not 403: an id must not be probeable.
    let (status, _) = send(&app.router, "GET", &uri, &other, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &app.router,
        "PATCH",
        SESSIONS,
        &other,
        Some(json!({ "id": id, "title": "Dibajak" })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = send(
        &app.router,
        "DELETE",
        &format!("{SESSIONS}?id={id}"),
        &other,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Nothing changed: the owner still opens it, under its own title.
    let (status, opened) = send(&app.router, "GET", &uri, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(opened["session"]["title"], "Punya pemilik");
    assert_eq!(opened["session"]["messages"][0]["content"], "Punya pemilik");

    let (_, theirs) = send(&app.router, "GET", SESSIONS, &other, None).await;
    assert_eq!(
        theirs["sessions"],
        json!([]),
        "another user's list holds none of the owner's conversations"
    );
}

#[tokio::test]
async fn the_list_is_newest_first_and_holds_only_the_callers() {
    let (app, owner, other, _owner_id, _other_id) = app_with_two_users().await;
    let first = save(&app.router, &owner, "Percakapan pertama").await;
    next_second().await;
    let second = save(&app.router, &owner, "Percakapan kedua").await;
    next_second().await;
    let someone_else = save(&app.router, &other, "Percakapan orang lain").await;

    let (status, listed) = send(&app.router, "GET", SESSIONS, &owner, None).await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = listed["sessions"]
        .as_array()
        .expect("sessions is a list")
        .iter()
        .map(|s| s["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, [second.as_str(), first.as_str()]);
    assert!(
        !ids.contains(&someone_else.as_str()),
        "the list holds only the caller's conversations: {ids:?}"
    );
}
