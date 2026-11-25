//! The audit trail records the first failure of a window and a count, not
//! every failure (`SEC-75`, plan `adm-130`, decisions 3 to 6).
//!
//! # What is measured
//!
//! `audit_event` is append-only (migration `0142`), so one row per failed
//! sign-in is a row an anonymous caller can grow without limit. The
//! FIRST failure of a window writes the row, the later ones only count,
//! and the sweep writes one summary row when the window has closed.
//!
//! # The real socket
//!
//! The sign-in tests go through a REAL socket: a request's source is
//! decided by the peer's address and by the `X-Forwarded-For` a trusted
//! proxy writes, and a request that never had a peer address comes from
//! the fixed `unknown` bucket. The peer of a loopback test is
//! `127.0.0.1`, so a deployment that trusts `127.0.0.1/32` stands in for
//! a deployment that trusts its console.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use futures::FutureExt;
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime};
use tower::ServiceExt;

use common::{
    TestApp, audit_rows, create_zero_permission_principal, session_cookie_for_user, spin_up,
    spin_up_with_env, spin_up_with_state,
};

/// The address the first source of the tests comes from.
const SOURCE: &str = "203.0.113.7";
/// A second source, for the test that two sources get a row of their own.
const OTHER_SOURCE: &str = "198.51.100.4";
/// A password that is wrong.
const WRONG_PASSWORD: &str = "not-the-password";
/// The domain of the identifiers the tests type. Nothing of it may reach
/// the audit trail.
const IDENTIFIER_DOMAIN: &str = "spray.invalid";
/// How many failed sign-ins the sign-in tests type.
const FAILURES: usize = 60;
/// How many seconds of its window a run of failures needs, so that it
/// cannot cross the boundary: the run is a few seconds long, and the
/// margin is generous.
const WINDOW_MARGIN_SECONDS: i64 = 60;

/// A deployment that trusts the loopback, so the header names the source.
fn env_trusting_the_loopback() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TRUSTED_PROXIES".to_owned(), "127.0.0.1/32".to_owned());
    env
}

/// Serve `router` on a real loopback socket and return its address.
///
/// The source resolution reads the PEER address, so a test that proves
/// anything about the peer needs a real socket.
async fn serve(router: axum::Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback listener");
    let addr = listener.local_addr().expect("the listener's address");
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });
    addr
}

/// The start of the window the counter counts in, right now.
///
/// The same expression the production statements use, so the test reads
/// the window the rows were written in and not a window of its own.
async fn current_window(pool: &sqlx::PgPool) -> OffsetDateTime {
    sqlx::query_scalar("SELECT to_timestamp(floor(extract(epoch from now()) / 900) * 900)")
        .fetch_one(pool)
        .await
        .expect("the current window")
}

/// The seconds left in the current window, by the database's clock.
async fn seconds_left_in_the_window(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT floor(extract(epoch FROM (to_timestamp(floor(extract(epoch FROM now()) / 900) * 900) \
         + interval '900 seconds' - now())))::bigint",
    )
    .fetch_one(pool)
    .await
    .expect("the time left in the window")
}

/// Wait until the window has room for a run of failures, and return its
/// start.
///
/// The counter's window is a wall-clock window of 15 minutes: a run that
/// crosses its boundary counts into two windows, and the scenario would
/// then measure the sum of two windows instead of one. The wait is
/// bounded by the window itself, and happens only when the window is
/// nearly over.
async fn wait_for_a_window_with_room(pool: &sqlx::PgPool) -> OffsetDateTime {
    for _ in 0..3 {
        let left = seconds_left_in_the_window(pool).await;
        if left >= WINDOW_MARGIN_SECONDS {
            return current_window(pool).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(
            u64::try_from(left + 1).unwrap_or(1),
        ))
        .await;
    }
    panic!("the window did not open again");
}

/// One failed sign-in from `source`, and the answer it got.
async fn fail_sign_in(
    client: &reqwest::Client,
    addr: SocketAddr,
    identifier: &str,
    source: &str,
) -> StatusCode {
    client
        .post(format!("http://{addr}/api/auth/login"))
        .header("x-forwarded-for", source)
        .json(&json!({ "email": identifier, "password": WRONG_PASSWORD }))
        .send()
        .await
        .expect("the sign-in request")
        .status()
}

/// Fail `count` sign-ins from `source`, each with an identifier of its
/// own, and return the window they landed in.
///
/// The identifiers differ so that no ACCOUNT walks toward its own lock:
/// the account throttle is keyed by the identifier, and the source's
/// budget is what this test spends.
async fn fail_sign_ins(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    addr: SocketAddr,
    source: &str,
    prefix: &str,
    count: usize,
) -> OffsetDateTime {
    let window = wait_for_a_window_with_room(pool).await;
    for attempt in 0..count {
        let identifier = format!("{prefix}-{attempt}@{IDENTIFIER_DOMAIN}");
        let status = fail_sign_in(client, addr, &identifier, source).await;
        // The first failures answer `401`; the ones over the source's
        // budget answer `429` (decision 2) and are refused before the
        // password is checked.
        assert!(
            status == StatusCode::UNAUTHORIZED || status == StatusCode::TOO_MANY_REQUESTS,
            "attempt {attempt} answered {status}"
        );
    }
    assert_eq!(
        current_window(pool).await,
        window,
        "the run crossed the window boundary: it took longer than the margin"
    );
    window
}

/// Fail `count` sign-ins from `source`, insisting every answer is `401`.
///
/// [`fail_sign_ins`] allows a `429` over the source's budget. With no
/// trusted edge declared (`SEC-75` decision B) there is no such answer, and
/// accepting one here would hide the regression this scenario guards.
async fn fail_sign_ins_with_no_source_limit(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    addr: SocketAddr,
    source: &str,
    prefix: &str,
    count: usize,
) -> OffsetDateTime {
    let window = wait_for_a_window_with_room(pool).await;
    for attempt in 0..count {
        let status = fail_sign_in(
            client,
            addr,
            &format!("{prefix}-{attempt}@{IDENTIFIER_DOMAIN}"),
            source,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "attempt {attempt} answered {status}; no trusted edge means no per-source 429"
        );
    }
    assert_eq!(
        current_window(pool).await,
        window,
        "the run crossed the window boundary: it took longer than the margin"
    );
    window
}

/// The `audit_event` rows with this `action` written in this window,
/// oldest first, each as the JSON of the whole row.
async fn audit_rows_in_window(
    pool: &sqlx::PgPool,
    action: &str,
    window: OffsetDateTime,
) -> Vec<Value> {
    sqlx::query_scalar(
        "SELECT to_jsonb(a) FROM audit_event a \
         WHERE action = $1 AND at >= $2 AND at < $3 ORDER BY at, id",
    )
    .bind(action)
    .bind(window)
    .bind(window + Duration::minutes(15))
    .fetch_all(pool)
    .await
    .expect("read audit_event")
}

/// One request against the router; the answer is the status and the JSON
/// body (`null` when there is none).
async fn call(
    app: &axum::Router,
    method: &str,
    path: &str,
    cookie: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder().method(method).uri(path);
    builder = builder.header("cookie", cookie);
    let body = body.map(|value| value.to_string());
    if let Some(body) = &body {
        builder = builder.header("content-type", "application/json");
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

/// Assert that nothing the visitor typed is in a row: not the identifier,
/// not the password, and not a hash of the identifier.
///
/// The `args` and the `detail` are checked as they are stored, and the
/// whole row with them: a value in any column is then caught, not only one
/// of the two the plan names.
fn assert_nothing_the_visitor_typed(row: &Value, prefix: &str, count: usize) {
    for text in [
        row["args"].to_string(),
        row["detail"].to_string(),
        row.to_string(),
    ] {
        for attempt in 0..count {
            let identifier = format!("{prefix}-{attempt}@{IDENTIFIER_DOMAIN}");
            let key = lakehouse_auth::throttle::key_for(&identifier);
            for forbidden in [identifier.as_str(), &key[..12]] {
                assert!(
                    !text.contains(forbidden),
                    "the row must not carry {forbidden}: {text}"
                );
            }
        }
        assert!(
            !text.contains(WRONG_PASSWORD),
            "the row must not carry the password: {text}"
        );
    }
}

/// Sixty failed sign-ins from one source write ONE refused row before the
/// sweep, and ONE summary row with the count of the rest after it
/// (`SEC-75` decision 3).
///
/// The row names the source (decision 5) and nothing the visitor typed:
/// not the identifier, not the password, and not a hash of either. The
/// summary names the same things and the count: the sixty failures minus
/// the one the first row already records.
#[tokio::test]
async fn sixty_failed_signins_from_one_source_write_one_refused_row_and_one_summary() {
    let (TestApp { router, pool }, state) =
        spin_up_with_state(&env_trusting_the_loopback(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    let window = fail_sign_ins(&pool, &client, addr, SOURCE, "spray", FAILURES).await;

    let rows = audit_rows_in_window(&pool, "auth.login", window).await;
    assert_eq!(rows.len(), 1, "sixty failures, one row: {rows:#?}");
    let row = &rows[0];
    assert_eq!(row["outcome"], "refused");
    assert_eq!(
        row["actor_label"],
        lakehouse_store::audit::ACTOR_SIGN_IN_FORM
    );
    assert!(row["principal_id"].is_null(), "nobody signed in: {row}");
    // `SEC-75` decision 5: the row names the source and nothing else the
    // caller typed.
    assert_eq!(row["args"]["source"], SOURCE, "the row names the source");
    assert_nothing_the_visitor_typed(row, "spray", FAILURES);

    // The window has closed: the sweep writes the summary of the sixty
    // failures, and the count is the ones that came after the first row.
    close_the_window_of_a_source(&pool, SOURCE).await;
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 1, "one summary: {counts:?}");

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(rows.len(), 2, "one refused row and one summary: {rows:#?}");
    let summary = summary_of(
        &rows,
        "59 more failed sign-ins from this source in 15 minutes.",
    );
    assert_eq!(summary["outcome"], "refused");
    assert_eq!(
        summary["actor_label"],
        lakehouse_store::audit::ACTOR_SIGN_IN_FORM
    );
    assert!(
        summary["principal_id"].is_null(),
        "nobody signed in: {summary}"
    );
    assert_eq!(
        summary["args"]["source"], SOURCE,
        "the summary names the source"
    );
    assert_eq!(
        summary["args"]["count"], 59,
        "the count of the later failures"
    );
    assert!(
        summary["args"]["windowStartedAt"].is_string(),
        "the summary names the window: {summary}"
    );
    assert!(
        summary["args"]["windowEndedAt"].is_string(),
        "the summary names the window: {summary}"
    );
    assert_nothing_the_visitor_typed(summary, "spray", FAILURES);
}

/// With no trusted edge declared (`SEC-75` decision B), sixty failures from
/// one peer all answer `401`: the per-source `429` needs an operator-declared
/// proxy. Counting and compaction still run, so the trail keeps one first
/// row and the sweep writes one summary with the count of the other 59.
#[tokio::test]
async fn with_no_trusted_edge_sixty_failures_get_no_429_but_still_compact() {
    let (TestApp { router, pool }, state) = spin_up_with_state(&HashMap::new(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();
    // Nothing is trusted, so the header the helper sends is ignored and
    // the loopback peer is the source.
    let peer = "127.0.0.1";

    let window =
        fail_sign_ins_with_no_source_limit(&pool, &client, addr, peer, "spray", FAILURES).await;

    let rows = audit_rows_in_window(&pool, "auth.login", window).await;
    assert_eq!(rows.len(), 1, "sixty failures, one row: {rows:#?}");
    assert_eq!(rows[0]["outcome"], "refused");
    assert_eq!(rows[0]["args"]["source"], peer, "the row names the peer");
    assert_nothing_the_visitor_typed(&rows[0], "spray", FAILURES);

    close_the_window_of_a_source(&pool, peer).await;
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 1, "one summary: {counts:?}");

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(rows.len(), 2, "one refused row and one summary: {rows:#?}");
    let summary = summary_of(
        &rows,
        "59 more failed sign-ins from this source in 15 minutes.",
    );
    assert_eq!(
        summary["args"]["source"], peer,
        "the summary names the peer"
    );
    assert_eq!(
        summary["args"]["count"], 59,
        "the count of the later failures"
    );
    assert_nothing_the_visitor_typed(summary, "spray", FAILURES);
}

/// Two sources get a refused row of their own.
#[tokio::test]
async fn two_sources_get_a_refused_row_of_their_own() {
    let TestApp { router, pool } = spin_up_with_env(&env_trusting_the_loopback()).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    let window = wait_for_a_window_with_room(&pool).await;
    for attempt in 0..3 {
        let status = fail_sign_in(
            &client,
            addr,
            &format!("first-{attempt}@{IDENTIFIER_DOMAIN}"),
            SOURCE,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        let status = fail_sign_in(
            &client,
            addr,
            &format!("second-{attempt}@{IDENTIFIER_DOMAIN}"),
            OTHER_SOURCE,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    let rows = audit_rows_in_window(&pool, "auth.login", window).await;
    assert_eq!(rows.len(), 2, "two sources, a row each: {rows:#?}");
    let mut sources: Vec<&str> = rows
        .iter()
        .map(|row| row["args"]["source"].as_str().unwrap_or(""))
        .collect();
    sources.sort_unstable();
    assert_eq!(sources, vec![OTHER_SOURCE, SOURCE]);
    // Neither row holds what the visitor typed.
    for row in &rows {
        assert_nothing_the_visitor_typed(row, "first", 3);
        assert_nothing_the_visitor_typed(row, "second", 3);
    }
}

/// Five refusals at the gate by one principal on one route write ONE
/// refused row (`SEC-75` decision 4).
#[tokio::test]
async fn five_gate_refusals_of_one_principal_and_route_write_one_row() {
    let TestApp { router, pool } = spin_up().await;
    let nobody = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, nobody).await;

    for _ in 0..5 {
        let (status, _) = call(
            &router,
            "POST",
            "/api/identity/roles",
            &cookie,
            Some(json!({"name": "Sneaky", "permissions": "*:*"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    let rows = audit_rows(&pool, "identity.role.create").await;
    assert_eq!(rows.len(), 1, "five refusals, one row: {rows:#?}");
    let row = &rows[0];
    assert_eq!(row["outcome"], "refused");
    assert_eq!(row["detail"], "identity:write");
    assert_eq!(row["principal_id"], json!(nobody.to_string()));
}

/// The gate's refusals are counted per principal AND per route: a second
/// principal on the same route, and the same principal on another route,
/// each get a row of their own.
#[tokio::test]
async fn two_principals_and_two_routes_get_a_refused_row_of_their_own() {
    let TestApp { router, pool } = spin_up().await;
    let first = create_zero_permission_principal(&pool).await;
    let second = create_zero_permission_principal(&pool).await;
    let first_cookie = session_cookie_for_user(&pool, first).await;
    let second_cookie = session_cookie_for_user(&pool, second).await;

    let (status, _) = call(
        &router,
        "POST",
        "/api/identity/roles",
        &first_cookie,
        Some(json!({"name": "Sneaky", "permissions": "*:*"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &router,
        "PUT",
        &format!("/api/identity/roles/{}", uuid::Uuid::new_v4()),
        &first_cookie,
        Some(json!({"permissions": "*:*"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = call(
        &router,
        "POST",
        "/api/identity/roles",
        &second_cookie,
        Some(json!({"name": "Sneaky", "permissions": "*:*"})),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let creates = audit_rows(&pool, "identity.role.create").await;
    assert_eq!(creates.len(), 2, "two principals, a row each: {creates:#?}");
    let mut principals: Vec<&str> = creates
        .iter()
        .map(|row| row["principal_id"].as_str().unwrap_or(""))
        .collect();
    principals.sort_unstable();
    // The two principals are uuids, so their order is the coin toss of the
    // run: the expectation is sorted too, or the test fails half the time.
    let mut expected = vec![first.to_string(), second.to_string()];
    expected.sort_unstable();
    assert_eq!(principals, expected);

    let updates = audit_rows(&pool, "identity.role.update").await;
    assert_eq!(updates.len(), 1, "another route, another row: {updates:#?}");
    assert_eq!(updates[0]["principal_id"], json!(first.to_string()));
}

/// The row that carries the count: the summary the sweep writes for a
/// closed window.
fn summary_of<'a>(rows: &'a [Value], detail: &str) -> &'a Value {
    rows.iter()
        .find(|row| row["detail"] == json!(detail))
        .unwrap_or_else(|| panic!("no row with the detail {detail:?}: {rows:#?}"))
}

/// Move the counter rows of one source into the window before the current
/// one, so the sweep finds them closed.
///
/// The window is a wall-clock boundary of fifteen minutes; a test cannot
/// wait for one to close, so it moves the counter row into the window
/// that has closed. The window expression is the same one the production
/// statements use.
async fn close_the_window_of_a_source(pool: &sqlx::PgPool, source: &str) {
    sqlx::query(
        "UPDATE login_source_throttle SET window_started_at = \
         to_timestamp(floor(extract(epoch from now()) / 900) * 900) - interval '900 seconds' \
         WHERE source = $1",
    )
    .bind(source)
    .execute(pool)
    .await
    .expect("move the counter row into the closed window");
}

/// Move the counter rows of one principal and route into the window before
/// the current one, so the sweep finds them closed.
async fn close_the_window_of_a_route(pool: &sqlx::PgPool, method: &str, pattern: &str) {
    sqlx::query(
        "UPDATE gate_refusal_throttle SET window_started_at = \
         to_timestamp(floor(extract(epoch from now()) / 900) * 900) - interval '900 seconds' \
         WHERE method = $1 AND route_pattern = $2",
    )
    .bind(method)
    .bind(pattern)
    .execute(pool)
    .await
    .expect("move the counter row into the closed window");
}

/// How many counter rows one source holds.
async fn counter_rows_of_a_source(pool: &sqlx::PgPool, source: &str) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM login_source_throttle WHERE source = $1")
        .bind(source)
        .fetch_one(pool)
        .await
        .expect("count the counter rows")
}

/// A closed window of failed sign-ins gets ONE summary row, with the count
/// of the failures that came after the first row (`SEC-75` decision 3).
///
/// The summary names the source and the count, and nothing the visitor
/// typed: not the identifier, not the password, and not a hash of either.
#[tokio::test]
async fn a_closed_window_gets_one_summary_row_with_the_count() {
    let (TestApp { router, pool }, state) =
        spin_up_with_state(&env_trusting_the_loopback(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, SOURCE, "spray", 3).await;
    let refused = audit_rows(&pool, "auth.login").await;
    assert_eq!(refused.len(), 1, "three failures, one row: {refused:#?}");

    close_the_window_of_a_source(&pool, SOURCE).await;
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 1, "one summary: {counts:?}");

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(rows.len(), 2, "one refused row and one summary: {rows:#?}");
    let summary = summary_of(
        &rows,
        "2 more failed sign-ins from this source in 15 minutes.",
    );
    assert_eq!(summary["outcome"], "refused");
    assert_eq!(
        summary["actor_label"],
        lakehouse_store::audit::ACTOR_SIGN_IN_FORM
    );
    assert!(
        summary["principal_id"].is_null(),
        "nobody signed in: {summary}"
    );
    assert_eq!(
        summary["args"]["source"], SOURCE,
        "the summary names the source"
    );
    assert_eq!(
        summary["args"]["count"], 2,
        "the count of the later failures"
    );
    assert!(
        summary["args"]["windowStartedAt"].is_string(),
        "the summary names the window: {summary}"
    );
    assert!(
        summary["args"]["windowEndedAt"].is_string(),
        "the summary names the window: {summary}"
    );

    // Nothing the visitor typed is in the summary.
    assert_nothing_the_visitor_typed(summary, "spray", 3);

    // The counter row is gone: it has been summarised.
    assert_eq!(
        counter_rows_of_a_source(&pool, SOURCE).await,
        0,
        "the summarised counter row is deleted"
    );
}

/// A closed window whose counter says its first row was NEVER written
/// counts every failure in the summary, not one fewer
/// (`SEC-75` decision 3; gov-dev review item A1).
///
/// The flag is set directly after the failures: the first row really was
/// written, but a counter that never got marked (a crash between the write
/// and the mark) must not make the summary under-count. The false branch of
/// `summarised_count` is what this scenario pins down.
#[tokio::test]
async fn a_window_without_a_first_row_counts_every_failure() {
    let (TestApp { router, pool }, state) =
        spin_up_with_state(&env_trusting_the_loopback(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, SOURCE, "spray", 3).await;

    // Stand in for a first row that never got marked on the counter, then
    // close the window the sweep reads.
    sqlx::query("UPDATE login_source_throttle SET first_row_written = FALSE WHERE source = $1")
        .bind(SOURCE)
        .execute(&pool)
        .await
        .expect("clear the first-row flag");
    close_the_window_of_a_source(&pool, SOURCE).await;

    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 1, "one summary: {counts:?}");

    let rows = audit_rows(&pool, "auth.login").await;
    // The summary reports the first row AND the later two: all three
    // failures, not the two that came after a first row.
    let summary = summary_of(
        &rows,
        "3 more failed sign-ins from this source in 15 minutes.",
    );
    assert_eq!(
        summary["args"]["count"], 3,
        "every failure is counted when no first row was written"
    );
}

/// A window whose summary write fails keeps its counter, and the NEXT sweep
/// writes the summary (`SEC-75` decision 3; gov-dev review item A2).
///
/// The audit insert is made to fail with a temporary trigger that refuses
/// summary rows, and the trigger is dropped before the second sweep. The
/// `continue` after a failed `record_best_effort_checked` is what keeps the
/// counter for the retry: without it the counter is deleted and the retry
/// has nothing to summarise.
#[tokio::test]
async fn a_failed_summary_write_keeps_the_counter_for_the_next_sweep() {
    let (TestApp { router, pool }, state) =
        spin_up_with_state(&env_trusting_the_loopback(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, SOURCE, "spray", 3).await;
    // Make "full count" literal: this window owes the summary all three
    // failures, so the retried summary must report three, not two.
    sqlx::query("UPDATE login_source_throttle SET first_row_written = FALSE WHERE source = $1")
        .bind(SOURCE)
        .execute(&pool)
        .await
        .expect("clear the first-row flag");
    close_the_window_of_a_source(&pool, SOURCE).await;

    // A nonce keeps the trigger's name unique even if the suite runs in
    // parallel against the same test Postgres (one database per test).
    let nonce = uuid::Uuid::new_v4().simple();
    let fun_name = format!("fail_summary_writes_{nonce}");
    let trigger_name = format!("summary_unwritable_{nonce}");

    sqlx::query(sqlx::AssertSqlSafe(format!(
        r"CREATE OR REPLACE FUNCTION {fun_name}() RETURNS trigger
           AS 'BEGIN RAISE EXCEPTION ''injected summary write failure''; END;'
           LANGUAGE plpgsql"
    )))
    .execute(&pool)
    .await
    .expect("create the failing trigger function");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER IF EXISTS {trigger_name} ON audit_event"
    )))
    .execute(&pool)
    .await
    .expect("clear any stale trigger");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        r"CREATE TRIGGER {trigger_name}
           BEFORE INSERT ON audit_event
           FOR EACH ROW WHEN (NEW.detail LIKE '%more failed sign-ins%')
           EXECUTE FUNCTION {fun_name}()"
    )))
    .execute(&pool)
    .await
    .expect("arm the failing trigger");

    // Assert under a panic catch so the DROP runs on every path.
    let result = std::panic::AssertUnwindSafe(async {
        let counts = lakehouse_api::audit_compaction::sweep(&state)
            .await
            .expect("the sweep runs");
        assert_eq!(
            counts.summaries, 0,
            "a failed summary is not counted as written: {counts:?}"
        );
        let rows = audit_rows(&pool, "auth.login").await;
        assert_eq!(rows.len(), 1, "no summary was written: {rows:#?}");
        assert_eq!(
            counter_rows_of_a_source(&pool, SOURCE).await,
            1,
            "the counter row stays for the next sweep"
        );
    })
    .catch_unwind()
    .await;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER IF EXISTS {trigger_name} ON audit_event"
    )))
    .execute(&pool)
    .await
    .expect("disarm the failing trigger");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP FUNCTION IF EXISTS {fun_name}()"
    )))
    .execute(&pool)
    .await
    .expect("drop the failing trigger function");

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }

    // The write works again: the next sweep writes exactly one summary of
    // the whole window and deletes the counter row.
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 1, "the retried summary lands: {counts:?}");

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(
        rows.len(),
        2,
        "one refused row and one summary after the retry: {rows:#?}"
    );
    let summary = summary_of(
        &rows,
        "3 more failed sign-ins from this source in 15 minutes.",
    );
    assert_eq!(
        summary["args"]["count"], 3,
        "the retry reports the full count"
    );
    assert_eq!(
        counter_rows_of_a_source(&pool, SOURCE).await,
        0,
        "the summarised counter row is deleted"
    );
}

/// A sweep cannot run while another API process holds the sweep lock
/// (gov-dev review item B). The holder is a separate transaction on another
/// connection from the pool: it stands in for a replica or the old process
/// during a rolling deploy that is already sweeping the same database.
#[tokio::test]
async fn a_sweep_cannot_run_while_another_process_holds_the_lock() {
    let (TestApp { router, pool }, state) =
        spin_up_with_state(&env_trusting_the_loopback(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, SOURCE, "spray", 3).await;
    close_the_window_of_a_source(&pool, SOURCE).await;

    let mut holder = pool.begin().await.expect("a holder transaction");
    let held: bool =
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))")
            .bind("audit_compaction.sweep")
            .fetch_one(&mut *holder)
            .await
            .expect("take the sweep lock");
    assert!(held, "the other process holds the sweep lock");

    // The locked sweep is a no-op: another process owns the window.
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(
        counts,
        lakehouse_api::audit_compaction::SweepCounts::default(),
        "the sweep that cannot take the lock does nothing: {counts:?}"
    );
    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(
        rows.len(),
        1,
        "no summary while the lock is held: {rows:#?}"
    );
    assert_eq!(
        counter_rows_of_a_source(&pool, SOURCE).await,
        1,
        "no counter row is deleted while the lock is held"
    );

    holder.rollback().await.expect("release the sweep lock");

    // Once the lock is free, one sweep writes the summary and deletes the
    // counter row.
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(
        counts.summaries, 1,
        "the second sweep writes the summary: {counts:?}"
    );
    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(
        rows.len(),
        2,
        "one refused row and one summary after the lock is released: {rows:#?}"
    );
    assert_eq!(
        counter_rows_of_a_source(&pool, SOURCE).await,
        0,
        "the counter row is deleted by the sweep that owns the lock"
    );
}

/// A window with nothing after its first row writes NO summary: there is
/// nothing to summarise (`SEC-75` decision 3).
#[tokio::test]
async fn a_window_with_one_failure_writes_no_summary() {
    let (TestApp { router, pool }, state) =
        spin_up_with_state(&env_trusting_the_loopback(), |_| {}).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, SOURCE, "spray", 1).await;

    close_the_window_of_a_source(&pool, SOURCE).await;
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 0, "one failure, no summary: {counts:?}");

    let rows = audit_rows(&pool, "auth.login").await;
    assert_eq!(rows.len(), 1, "the refused row and nothing else: {rows:#?}");
    assert_eq!(
        counter_rows_of_a_source(&pool, SOURCE).await,
        0,
        "the summarised counter row is deleted"
    );
}

/// A closed window of refusals at the gate gets ONE summary row, naming
/// the route and the count (`SEC-75` decision 4).
#[tokio::test]
async fn a_closed_window_of_gate_refusals_gets_one_summary_row() {
    let (TestApp { router, pool }, state) = spin_up_with_state(&HashMap::new(), |_| {}).await;
    let nobody = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, nobody).await;

    for _ in 0..3 {
        let (status, _) = call(
            &router,
            "POST",
            "/api/identity/roles",
            &cookie,
            Some(json!({"name": "Sneaky", "permissions": "*:*"})),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }
    let refused = audit_rows(&pool, "identity.role.create").await;
    assert_eq!(refused.len(), 1, "three refusals, one row: {refused:#?}");

    close_the_window_of_a_route(&pool, "POST", "/api/identity/roles").await;
    let counts = lakehouse_api::audit_compaction::sweep(&state)
        .await
        .expect("the sweep runs");
    assert_eq!(counts.summaries, 1, "one summary: {counts:?}");

    let rows = audit_rows(&pool, "identity.role.create").await;
    assert_eq!(rows.len(), 2, "one refused row and one summary: {rows:#?}");
    let summary = summary_of(
        &rows,
        "2 more refusals of POST /api/identity/roles in 15 minutes.",
    );
    assert_eq!(summary["outcome"], "refused");
    assert_eq!(summary["principal_id"], json!(nobody.to_string()));
    assert_eq!(summary["resource_kind"], "role");
    assert_eq!(summary["args"]["route"], "/api/identity/roles");
    assert_eq!(summary["args"]["method"], "POST");
    assert_eq!(summary["args"]["count"], 2);

    let counter_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM gate_refusal_throttle")
        .fetch_one(&pool)
        .await
        .expect("count the counter rows");
    assert_eq!(counter_rows, 0, "the summarised counter row is deleted");
}

/// A counter that cannot be written still writes the refused row: a
/// refused sign-in stays visible on the trail, which is what the trail
/// held before the compaction, and never for fewer (`SEC-75` decision 3,
/// the gate's fail-visible fallback).
///
/// The write failure is injected with a trigger, because the budget
/// check reads the counter with a `SELECT` and the failure being
/// reproduced is a failed WRITE: the check still answers, and the
/// counting of this failure is what fails.
#[tokio::test]
async fn a_failed_counter_write_still_writes_the_refused_row() {
    let TestApp { router, pool } = spin_up_with_env(&env_trusting_the_loopback()).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    // Fixed global names collide when the suite runs in parallel against
    // the same test Postgres (one process's objects block another's
    // CREATE) and a panic that skips the DROP leaves residue for later
    // runs. A nonce per run keeps processes isolated; the setup is also
    // idempotent and the DROP runs in all paths below.
    let nonce = uuid::Uuid::new_v4().simple();
    let fun_name = format!("fail_source_counter_writes_{nonce}");
    let trigger_name = format!("source_counter_unwritable_{nonce}");

    sqlx::query(sqlx::AssertSqlSafe(format!(
        r"CREATE OR REPLACE FUNCTION {fun_name}() RETURNS trigger
           AS 'BEGIN RAISE EXCEPTION ''injected counter write failure''; END;'
           LANGUAGE plpgsql"
    )))
    .execute(&pool)
    .await
    .expect("create the failing trigger function");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER IF EXISTS {trigger_name} ON login_source_throttle"
    )))
    .execute(&pool)
    .await
    .expect("clear any stale trigger");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        r"CREATE TRIGGER {trigger_name}
           BEFORE INSERT OR UPDATE ON login_source_throttle
           FOR EACH ROW EXECUTE FUNCTION {fun_name}()"
    )))
    .execute(&pool)
    .await
    .expect("arm the failing trigger");

    // Run the assertions under a panic catch so the DROP below runs even
    // if one fails; the panic is re-raised after cleanup.
    let result = std::panic::AssertUnwindSafe(async {
        for attempt in 0..2 {
            let identifier = format!("counter-fail-{attempt}@{IDENTIFIER_DOMAIN}");
            let status = fail_sign_in(&client, addr, &identifier, SOURCE).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "a counter failure changes nothing about the answer"
            );
        }

        let rows = audit_rows(&pool, "auth.login").await;
        assert_eq!(rows.len(), 2, "two failures, a refused row each: {rows:#?}");
        for row in &rows {
            assert_eq!(row["outcome"], "refused");
            assert_eq!(row["args"]["source"], SOURCE, "the row names the source");
            assert_nothing_the_visitor_typed(row, "counter-fail", 2);
        }
    })
    .catch_unwind()
    .await;

    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP TRIGGER IF EXISTS {trigger_name} ON login_source_throttle"
    )))
    .execute(&pool)
    .await
    .expect("disarm the failing trigger");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP FUNCTION IF EXISTS {fun_name}()"
    )))
    .execute(&pool)
    .await
    .expect("drop the failing trigger function");

    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}
