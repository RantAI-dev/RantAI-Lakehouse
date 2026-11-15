//! The sign-in and assistant resolvers, over a real database (`ADM-8`,
//! `ADM-9`; slice 2 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! What this proves:
//!
//! * a saved limit reaches the next sign-in with no restart: a saved
//!   `maxFailures` of 3 locks the fourth try (plan D12);
//! * the environment's numbers win over the saved ones (plan D1);
//! * the assistant's client is rebuilt when the saved provider changes, and
//!   a rebuild refuses an address that points inside the deployment's
//!   network (plan D13).
//!
//! The rows are written straight into the store, the way a row a console
//! save leaves looks. The routes that write them are proved in
//! `sign_in_settings_route.rs` and `assistant_settings_route.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{TestApp, spin_up_with_env, spin_up_with_state_without_env_llm_url};
use lakehouse_store::assistant_settings::AssistantSettings;
use lakehouse_store::sign_in_settings::SignInSettings;
use tower::ServiceExt;

const THROTTLED_EMAIL: &str = "fajar@meridian.example";
const REAL_PASSWORD: &str = "Co7rect!Passw0rd";
const WRONG_PASSWORD: &str = "not-the-password";

/// The provider address the console saved in these tests: `localhost`
/// resolves without a DNS lookup, so the check under test never needs a
/// network.
const CONSOLE_PROVIDER: &str = "https://localhost:8443/v1";

async fn login(app: &axum::Router, email: &str, password: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(format!(
                    r#"{{"email": "{email}", "password": "{password}"}}"#
                )))
                .expect("build request"),
        )
        .await
        .expect("router never fails a request outright")
        .status()
}

/// Seeds a working `provider = 'local'` password identity for the seeded
/// Platform Admin, so a wrong password is the only thing under test.
async fn seed_password_identity(pool: &sqlx::PgPool) {
    let (user_id,): (uuid::Uuid,) = sqlx::query_as("SELECT id FROM app_user WHERE email = $1")
        .bind(THROTTLED_EMAIL)
        .fetch_one(pool)
        .await
        .expect("seeded fajar@meridian.example");
    lakehouse_auth::password::create_local_identity(
        pool,
        user_id,
        &lakehouse_auth::Secret::new(REAL_PASSWORD),
        false,
    )
    .await
    .expect("create the local identity under test");
}

/// The row a console save of `max_failures` leaves behind.
async fn save_max_failures(pool: &sqlx::PgPool, max_failures: i32) {
    lakehouse_store::sign_in_settings::upsert(
        pool,
        &SignInSettings {
            max_failures: Some(max_failures),
            failure_window_secs: None,
            lockout_secs: None,
            retention_days: None,
            embed_max_lifetime_secs: None,
        },
        "user-1",
    )
    .await
    .expect("save the sign-in limits");
}

/// `ADM-8`: the saved limit reaches the next sign-in with no restart. Three
/// wrong passwords are refused as wrong passwords, and the fourth is
/// refused as locked — the limit the console saved, not the code's default
/// of five.
#[tokio::test]
async fn a_saved_max_failures_of_three_locks_the_fourth_try_without_a_restart() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    seed_password_identity(&pool).await;
    save_max_failures(&pool, 3).await;

    for try_number in 1..=3 {
        assert_eq!(
            login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await,
            StatusCode::UNAUTHORIZED,
            "failure {try_number} must be refused as a wrong password"
        );
    }
    assert_eq!(
        login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await,
        StatusCode::TOO_MANY_REQUESTS,
        "the fourth try must be refused as locked"
    );
}

/// Plan D1: the environment's numbers win over the saved ones. The
/// environment says five tries, the console's row says three: the fifth
/// try is still refused as a wrong password, and the sixth is the locked
/// one.
#[tokio::test]
async fn the_environments_limit_wins_over_the_saved_one() {
    let mut env = HashMap::new();
    env.insert("LOGIN_MAX_FAILURES".to_owned(), "5".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    seed_password_identity(&pool).await;
    save_max_failures(&pool, 3).await;

    for try_number in 1..=5 {
        assert_eq!(
            login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await,
            StatusCode::UNAUTHORIZED,
            "failure {try_number} must be refused as a wrong password"
        );
    }
    assert_eq!(
        login(&router, THROTTLED_EMAIL, WRONG_PASSWORD).await,
        StatusCode::TOO_MANY_REQUESTS,
        "the sixth try is the locked one: the environment's limit of five won"
    );
}

/// `ADM-9`: the client is rebuilt when the saved provider changes. The
/// address here is one the deployment's own operator policy allows
/// (`CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS`), so the rebuild goes through.
///
/// The deployment's environment says nothing about the address, so the
/// console owns it and the saved one is the one the client is rebuilt on
/// (`ADM-9 review R4`).
#[tokio::test]
async fn the_assistant_client_is_rebuilt_when_the_saved_provider_changes() {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    let (TestApp { pool, .. }, state) = spin_up_with_state_without_env_llm_url(&env, |_| {}).await;

    // Before the save, the client is the one this process started with: the
    // deployment's own `LLM_URL`.
    let before = state.llm.read().await.clone();
    let before_url = before.settings.url.clone();
    assert_ne!(
        before_url, CONSOLE_PROVIDER,
        "the client starts on the deployment's own address"
    );

    lakehouse_store::assistant_settings::upsert(
        &pool,
        &AssistantSettings {
            llm_url: Some(CONSOLE_PROVIDER.to_owned()),
            llm_model: Some("console-model".to_owned()),
            llm_key: None,
        },
        "user-1",
    )
    .await
    .expect("save the assistant settings");

    let after = lakehouse_api::runtime_settings::assistant(&state).await;
    assert_eq!(
        after.settings.url, CONSOLE_PROVIDER,
        "the client must be rebuilt from the saved provider"
    );
    assert_eq!(after.settings.model, "console-model");
}

/// `ADM-9`: the rebuild checks the address again, and an address that points
/// inside the deployment's network is refused — so the client this process
/// already has stays in use and never carries that address.
///
/// This is the check the console's own save makes; a rebuild makes it
/// again, because a name that resolved publicly when it was saved may
/// resolve inside the network now.
#[tokio::test]
async fn a_rebuild_refuses_a_provider_address_inside_the_network() {
    let (TestApp { pool, .. }, state) =
        spin_up_with_state_without_env_llm_url(&HashMap::new(), |_| {}).await;
    let before_url = state.llm.read().await.settings.url.clone();

    lakehouse_store::assistant_settings::upsert(
        &pool,
        &AssistantSettings {
            llm_url: Some(CONSOLE_PROVIDER.to_owned()),
            llm_model: Some("console-model".to_owned()),
            llm_key: None,
        },
        "user-1",
    )
    .await
    .expect("save the assistant settings");

    let after = lakehouse_api::runtime_settings::assistant(&state).await;
    assert_ne!(
        after.settings.url, CONSOLE_PROVIDER,
        "an address inside the network must never reach the client"
    );
    assert_eq!(
        after.settings.url, before_url,
        "the client this process had stays in use"
    );
}
