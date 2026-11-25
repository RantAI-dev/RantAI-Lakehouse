//! The source of a request, and the per-source limit on failed sign-ins
//! (`SEC-75`, plan `adm-130`, decisions 1, 2 and 7).
//!
//! These tests go through a REAL socket, because the source of a
//! request is decided by the peer's address: a request that never had
//! one (`tower::ServiceExt::oneshot`) is a request from
//! `client_source::UNKNOWN_SOURCE`, and every assertion about a peer,
//! a proxy or a forged header would be about nothing at all.
//!
//! The peer of a loopback test is `127.0.0.1`, so a deployment that
//! trusts `127.0.0.1/32` stands in for a deployment that trusts its
//! console: the peer is a trusted proxy, and the header is what names
//! the source.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::http::StatusCode;
use time::OffsetDateTime;

use common::{TestApp, audit_rows, spin_up_with_env};

/// The account the tests sign in as. Seeded by `0002_seed_identity.sql`.
const SIGNED_IN_EMAIL: &str = "fajar@meridian.example";
/// The password of that account, set by [`seed_password_identity`].
const REAL_PASSWORD: &str = "Co7rect!Passw0rd";
/// A password that is wrong, for the failures.
const WRONG_PASSWORD: &str = "not-the-password";

/// The address the test's peer has on the loopback interface.
const LOOPBACK: &str = "127.0.0.1";
/// The client a trusted peer names in `X-Forwarded-For`.
const NAMED_CLIENT: &str = "203.0.113.7";

/// How many seconds of its window a run of failures needs, so that it
/// cannot cross the boundary.
///
/// Every failed sign-in costs an `Argon2id` verification, so the run of
/// fifty failures of the biggest scenario takes tens of seconds on a
/// loaded machine. The margin is two to four times that, so the run
/// finishes inside its window; the wait it can cause is bounded by the
/// margin itself and happens only when the window is nearly over.
const WINDOW_MARGIN_SECONDS: i64 = 120;

/// A deployment that trusts `trusted` as a proxy.
fn env_with_trusted(trusted: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert("TRUSTED_PROXIES".to_owned(), trusted.to_owned());
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

/// One sign-in, with the `X-Forwarded-For` the caller wants the API to
/// believe.
async fn sign_in(
    client: &reqwest::Client,
    addr: SocketAddr,
    email: &str,
    password: &str,
    forwarded_for: Option<&str>,
) -> reqwest::Response {
    let mut request = client
        .post(format!("http://{addr}/api/auth/login"))
        .json(&serde_json::json!({ "email": email, "password": password }));
    if let Some(forwarded_for) = forwarded_for {
        request = request.header("x-forwarded-for", forwarded_for);
    }
    request.send().await.expect("the sign-in request")
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
/// crosses its boundary splits its own count over two windows, and the
/// scenario would then measure the sum of two windows instead of one.
/// The wait is bounded by the window itself, and happens only when the
/// window is nearly over.
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

/// Fail `count` sign-ins from one source, each with an identifier of its
/// own.
///
/// The source is the one the REQUEST names — the peer, or the header's
/// right-most untrusted entry when a trusted proxy writes one — so the
/// scenario, not this helper, says which source it counts.
///
/// The counter's window is a wall-clock window of 15 minutes: a run that
/// crosses its boundary splits its own count over two windows, and the
/// scenario would then measure the sum of two windows instead of one.
/// The run therefore starts in a window with room for it, and a run that
/// still crosses the boundary fails here instead of quietly measuring
/// two windows.
///
/// `prefix` names the identifiers of the run (a fresh identifier per
/// attempt, so the run does not walk an account toward its own lock).
async fn fail_sign_ins(
    pool: &sqlx::PgPool,
    client: &reqwest::Client,
    addr: SocketAddr,
    prefix: &str,
    count: usize,
    forwarded_for: Option<&str>,
) {
    let window = wait_for_a_window_with_room(pool).await;
    for attempt in 0..count {
        let resp = sign_in(
            client,
            addr,
            &format!("{prefix}-{attempt}@meridian.example"),
            WRONG_PASSWORD,
            forwarded_for,
        )
        .await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED, "attempt {attempt}");
    }
    assert_eq!(
        current_window(pool).await,
        window,
        "the run crossed the window boundary: it took longer than the margin"
    );
}

/// The per-source counters, one row per source, in the order the
/// database returns them.
async fn source_counts(pool: &sqlx::PgPool) -> Vec<(String, i32)> {
    sqlx::query_as("SELECT source, failures FROM login_source_throttle ORDER BY source")
        .fetch_all(pool)
        .await
        .expect("read the per-source counters")
}

/// How many rows the account throttle holds for `email`.
async fn account_throttle_rows(pool: &sqlx::PgPool, email: &str) -> i64 {
    let key = lakehouse_auth::throttle::key_for(email);
    let (count,): (i64,) =
        sqlx::query_as("SELECT count(*) FROM login_throttle WHERE key_hash = $1")
            .bind(&key)
            .fetch_one(pool)
            .await
            .expect("read the account throttle");
    count
}

/// Seed a working `provider = 'local'` password identity for the seeded
/// Platform Admin, with `must_change_password = false` so a correct
/// sign-in is a 200.
async fn seed_password_identity(pool: &sqlx::PgPool) {
    let (user_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(SIGNED_IN_EMAIL)
        .fetch_one(pool)
        .await
        .expect("the seeded user (0002_seed_identity.sql)");
    lakehouse_auth::password::create_local_identity(
        pool,
        user_id,
        &lakehouse_auth::Secret::new(REAL_PASSWORD),
        false,
    )
    .await
    .expect("create the local identity under test");
}

/// A header a peer nobody vouched for wrote: the source is the peer, and
/// the header is not read at all.
///
/// The non-empty trusted list here does not vouch for the 127.0.0.1
/// peer, so the forged address never gets a counter or a budget of its own
/// (`SEC-75 decision B`). With an empty list the same is true of every
/// peer: the header is never read and there is no per-source 429.
#[tokio::test]
async fn a_forged_header_from_an_untrusted_peer_is_ignored() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("192.0.2.0/24")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, "ghost", 1, Some("203.0.113.7")).await;

    assert_eq!(
        source_counts(&pool).await,
        vec![(LOOPBACK.to_owned(), 1)],
        "the peer is the source; the forged header was not read"
    );
}

/// A trusted proxy names the source in the header: the right-most entry
/// that is not itself trusted is the source.
#[tokio::test]
async fn a_trusted_peer_takes_the_rightmost_untrusted_entry_of_the_header() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("127.0.0.1/32")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(
        &pool,
        &client,
        addr,
        "ghost",
        1,
        Some("203.0.113.7, 198.51.100.4"),
    )
    .await;

    assert_eq!(
        source_counts(&pool).await,
        vec![("198.51.100.4".to_owned(), 1)],
        "the right-most untrusted entry is the source"
    );
}

/// A trusted proxy that says nothing leaves the peer as the source.
#[tokio::test]
async fn a_trusted_peer_without_a_header_falls_back_to_the_peer() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("127.0.0.1/32")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, "ghost", 1, None).await;

    assert_eq!(source_counts(&pool).await, vec![(LOOPBACK.to_owned(), 1)]);
}

/// The sign-in after a trusted edge's named client has spent its budget
/// is refused with `429` and a `Retry-After`, before the password is
/// checked — and the account it named is never touched (`SEC-75 decision B`:
/// the per-source 429 needs a declared edge).
#[tokio::test]
async fn the_signin_after_the_per_source_limit_is_refused_with_429() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("127.0.0.1/32")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();
    seed_password_identity(&pool).await;

    // 50 failures, each with an identifier of its own, so that no
    // ACCOUNT is locked on the way (the account throttle is keyed by the
    // identifier) and only the CLIENT the trusted peer names is spent.
    fail_sign_ins(&pool, &client, addr, "spray", 50, Some(NAMED_CLIENT)).await;
    assert_eq!(
        source_counts(&pool).await,
        vec![(NAMED_CLIENT.to_owned(), 50)],
        "the trusted peer named the client whose budget was spent"
    );

    let rows_before = audit_rows(&pool, "auth.login").await.len();
    // The 51st sign-in, with the CORRECT password: it is refused, so the
    // password was never checked.
    let resp = sign_in(
        &client,
        addr,
        SIGNED_IN_EMAIL,
        REAL_PASSWORD,
        Some(NAMED_CLIENT),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = resp
        .headers()
        .get("retry-after")
        .expect("429 carries Retry-After")
        .to_str()
        .expect("ASCII header value")
        .parse::<u64>()
        .expect("Retry-After is an integer");
    assert!(retry_after >= 1, "Retry-After must be a positive wait");

    // The refusal wrote no audit row of its own.
    assert_eq!(
        audit_rows(&pool, "auth.login").await.len(),
        rows_before,
        "the 429 path writes no audit row"
    );
    // And the account was never touched: the refusal happens before the
    // account throttle.
    assert_eq!(
        account_throttle_rows(&pool, SIGNED_IN_EMAIL).await,
        0,
        "the account throttle was not touched by the refused sign-in"
    );
}

/// Two sources have two budgets.
#[tokio::test]
async fn two_sources_count_separately() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("127.0.0.1/32")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();

    fail_sign_ins(&pool, &client, addr, "first", 3, Some("203.0.113.7")).await;
    fail_sign_ins(&pool, &client, addr, "second", 3, Some("198.51.100.4")).await;

    assert_eq!(
        source_counts(&pool).await,
        vec![
            ("198.51.100.4".to_owned(), 3),
            ("203.0.113.7".to_owned(), 3)
        ],
        "each source has its own count"
    );
}

/// A successful sign-in does not hand the source its budget back: the
/// budget belongs to the address.
#[tokio::test]
async fn a_successful_signin_does_not_reset_the_source_count() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("127.0.0.1/32")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();
    seed_password_identity(&pool).await;

    fail_sign_ins(&pool, &client, addr, "spray", 3, Some("203.0.113.7")).await;
    assert_eq!(
        source_counts(&pool).await,
        vec![("203.0.113.7".to_owned(), 3)]
    );

    let resp = sign_in(
        &client,
        addr,
        SIGNED_IN_EMAIL,
        REAL_PASSWORD,
        Some("203.0.113.7"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    assert_eq!(
        source_counts(&pool).await,
        vec![("203.0.113.7".to_owned(), 3)],
        "the success did not reset the source's count"
    );
}

/// The session records no network address, even when a trusted proxy named a
/// client in `X-Forwarded-For` — the Sessions page says "Not recorded"
/// (`ADM-14` D10; `SEC-82` may change this).
#[tokio::test]
async fn the_session_does_not_record_the_source_of_the_signin() {
    let TestApp { router, pool } = spin_up_with_env(&env_with_trusted("127.0.0.1/32")).await;
    let addr = serve(router).await;
    let client = reqwest::Client::new();
    seed_password_identity(&pool).await;

    let resp = sign_in(
        &client,
        addr,
        SIGNED_IN_EMAIL,
        REAL_PASSWORD,
        Some("203.0.113.7"),
    )
    .await;
    assert_eq!(resp.status(), StatusCode::OK);

    let (created_ip,): (Option<String>,) =
        sqlx::query_as("SELECT created_ip FROM session ORDER BY created_at DESC LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("the session of the sign-in");
    assert_eq!(created_ip, None, "the session records no network address");
}
