//! `GET`/`PUT /api/settings/assistant` and `POST
//! /api/settings/assistant/test` (`ADM-9`, slice 2 of
//! `docs/superpowers/plans/2026-11-06-adm-6-9-console-settings.md`).
//!
//! What this proves:
//!
//! * every route needs `settings:write`, reading included (plan D4);
//! * the key is write-only: it never comes back in an answer, and the row
//!   holds it sealed (plan D5);
//! * the environment's key owns the whole provider: a save that sends any
//!   of the three fields is refused with a 409 and nothing is written, and
//!   the environment's key is never sent to an address the console chose
//!   (the `ADM-7 review R1` lesson, applied to the assistant);
//! * without a key, the environment's own address and model win on their
//!   own, each by itself (`ADM-9 review R4`): the field is the server's,
//!   the console's saved value of it is not used, and a save that sends it
//!   is refused with a 409;
//! * a deployment whose environment is built the way `docker-compose.yml`
//!   builds it — the four variables present and empty — may set the
//!   console's own address and port (`ADM-9 review R6`);
//! * the address must be `https://` and must not point inside the
//!   deployment's network (plan D13);
//! * every save writes one audit row, naming the fields that changed and
//!   never the key (plan D6);
//! * the test answers one of the fixed sentences, never the provider's own
//!   words (plan D13).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use crate::common;

use std::collections::HashMap;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use common::{
    TestApp, create_principal_with_permissions, create_zero_permission_principal,
    session_cookie_for_user, spin_up_with_env, spin_up_without_env_llm_url,
};
use serde_json::{Value, json};
use tower::ServiceExt;

/// What the person is told when the environment owns a field (plan D1).
const SET_BY_THE_SERVER: &str =
    "This setting is set in the server's environment and cannot be changed here.";

/// The assistant settings route.
const ASSISTANT: &str = "/api/settings/assistant";
/// The assistant test route.
const ASSISTANT_TEST: &str = "/api/settings/assistant/test";
/// The email settings route. It is here for one case only: the four
/// variables of `ADM-9 review R6` are ONE deployment's environment, so the
/// case that proves the console may set them asks both routes.
const EMAIL: &str = "/api/settings/email";

/// The provider address the tests save. `localhost` resolves without a DNS
/// lookup, so the address check never needs a network — and the deployment's
/// own policy is opened for it, so the check passes the same way it would
/// for a public provider.
const CONSOLE_PROVIDER: &str = "https://localhost:8443/v1";

/// The environment that lets the address check reach an internal address:
/// the same switch the connector dials read.
fn env_with_internal_hosts() -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "CONNECTOR_PROBE_ALLOW_INTERNAL_HOSTS".to_owned(),
        "true".to_owned(),
    );
    env
}

/// Call one of the routes: its status and its body.
async fn call(
    app: &axum::Router,
    verb: &str,
    path: &str,
    cookie: &str,
    body: &str,
) -> (StatusCode, Value) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method(verb)
                .uri(path)
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// A deployment with nothing saved answers with the deployment's own
/// address and model, and says that no key is set.
///
/// The deployment's environment says nothing about the address and the
/// model (`ADM-9 review R4`), so both are the code's defaults and the
/// console's to change.
#[tokio::test]
async fn a_fresh_deployment_answers_every_field_with_its_source() {
    let TestApp { router, pool } = spin_up_without_env_llm_url(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["url"]["source"], json!("default"), "{body}");
    assert_eq!(body["model"]["source"], json!("default"), "{body}");
    assert_eq!(body["key"], json!({"set": false, "source": "default"}));
}

/// Plan D4: reading the provider needs `settings:write` like changing it.
#[tokio::test]
async fn a_caller_without_settings_write_is_refused_on_all_three_routes() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let reader = create_zero_permission_principal(&pool).await;
    let cookie = session_cookie_for_user(&pool, reader).await;
    for (verb, path) in [
        ("GET", ASSISTANT),
        ("PUT", ASSISTANT),
        ("POST", ASSISTANT_TEST),
    ] {
        let (status, _) = call(&router, verb, path, &cookie, r#"{"model":"m"}"#).await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "{verb} {path} must need settings:write"
        );
    }
    // Nothing was written by the refused save.
    assert!(
        lakehouse_store::assistant_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );
}

/// Plan D5: the key is write-only. It never comes back in an answer, and
/// the row holds it sealed.
#[tokio::test]
async fn the_key_never_comes_back_in_an_answer() {
    let TestApp { router, pool } = spin_up_without_env_llm_url(&env_with_internal_hosts()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":"https://localhost:8443/v1","model":"console-model","key":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["key"], json!({"set": true, "source": "console"}));
    assert!(!body.to_string().contains("not-a-real-secret"), "{body}");
    assert_eq!(
        body["url"],
        json!({"value": CONSOLE_PROVIDER, "source": "console"})
    );
    assert_eq!(
        body["model"],
        json!({"value": "console-model", "source": "console"})
    );

    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["key"], json!({"set": true, "source": "console"}));
    assert!(!read.to_string().contains("not-a-real-secret"), "{read}");

    // The row holds the sealed bytes, not the text.
    let row = lakehouse_store::assistant_settings::get(&pool)
        .await
        .unwrap()
        .expect("the save wrote a row");
    let sealed = row.llm_key.expect("the key was stored");
    assert!(!String::from_utf8_lossy(&sealed).contains("not-a-real-secret"));
}

/// The `ADM-7 review R1` lesson, applied to the assistant: the environment's
/// key owns the whole provider. A save that sends any of the three fields is
/// refused with a 409 and nothing is written, and the console's saved
/// address is not used — so the environment's key is never sent to an
/// address the console chose.
#[tokio::test]
async fn the_environments_key_owns_the_whole_provider() {
    let mut env = env_with_internal_hosts();
    env.insert("LLM_KEY".to_owned(), "an-environment-key".to_owned());
    env.insert(
        "LLM_URL".to_owned(),
        "https://env.example.com/v1".to_owned(),
    );
    env.insert("LLM_MODEL".to_owned(), "env-model".to_owned());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    // A console's own row, saved before the operator put the key in the
    // environment.
    lakehouse_store::assistant_settings::upsert(
        &pool,
        &lakehouse_store::assistant_settings::AssistantSettings {
            llm_url: Some(CONSOLE_PROVIDER.to_owned()),
            llm_model: Some("console-model".to_owned()),
            llm_key: None,
        },
        &admin.to_string(),
    )
    .await
    .unwrap();

    // Every field is the server's, and a save that sends one is refused.
    for body in [
        r#"{"url":"https://localhost:8443/v1"}"#,
        r#"{"model":"console-model"}"#,
        r#"{"key":"not-a-real-secret"}"#,
    ] {
        let (status, got) = call(&router, "PUT", ASSISTANT, &cookie, body).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}: {got}");
        assert_eq!(
            got["error"].as_str(),
            Some(SET_BY_THE_SERVER),
            "{body}: {got}"
        );
    }
    // Nothing was written by the refused saves.
    let row = lakehouse_store::assistant_settings::get(&pool)
        .await
        .unwrap()
        .expect("the console's own row");
    assert_eq!(row.llm_url.as_deref(), Some(CONSOLE_PROVIDER));
    assert_eq!(row.llm_model.as_deref(), Some("console-model"));
    assert!(row.llm_key.is_none(), "the key was not written");

    // The answer names the environment's provider, never the console's.
    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["url"],
        json!({"value": "https://env.example.com/v1", "source": "server"})
    );
    assert_eq!(
        read["model"],
        json!({"value": "env-model", "source": "server"})
    );
    assert_eq!(read["key"], json!({"set": true, "source": "server"}));
    assert!(!read.to_string().contains("an-environment-key"), "{read}");
}

/// `ADM-9 review R4`: the environment's own address wins on its own, with
/// no key beside it. The address is the server's, so a save that sends it
/// is refused with a 409 and writes nothing, and the answer names the
/// environment's address — not the console's.
///
/// The model is not in this case, so it stays the console's: each field of
/// the provider answers for itself, and only the key makes all three one
/// unit.
#[tokio::test]
async fn the_environments_own_address_is_refused_and_nothing_is_written() {
    let mut env = env_with_internal_hosts();
    env.insert(
        "LLM_URL".to_owned(),
        "https://env.example.com/v1".to_owned(),
    );
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    // A console's own row, saved before the operator put the address in
    // the environment.
    lakehouse_store::assistant_settings::upsert(
        &pool,
        &lakehouse_store::assistant_settings::AssistantSettings {
            llm_url: Some(CONSOLE_PROVIDER.to_owned()),
            llm_model: Some("console-model".to_owned()),
            llm_key: None,
        },
        &admin.to_string(),
    )
    .await
    .unwrap();

    // A save that sends the address is refused, and nothing is written.
    let (status, got) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":"https://localhost:8443/v1"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert_eq!(got["error"].as_str(), Some(SET_BY_THE_SERVER), "{got}");

    let row = lakehouse_store::assistant_settings::get(&pool)
        .await
        .unwrap()
        .expect("the console's own row");
    assert_eq!(row.llm_url.as_deref(), Some(CONSOLE_PROVIDER));
    assert_eq!(row.llm_model.as_deref(), Some("console-model"));

    // The answer names the environment's address, as the server's.
    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["url"],
        json!({"value": "https://env.example.com/v1", "source": "server"})
    );
    // The model is the console's own: the environment named none.
    assert_eq!(
        read["model"],
        json!({"value": "console-model", "source": "console"})
    );
}

/// The model half of the same rule (`ADM-9 review R4`).
#[tokio::test]
async fn the_environments_own_model_is_refused_and_nothing_is_written() {
    let mut env = env_with_internal_hosts();
    env.insert("LLM_MODEL".to_owned(), "env-model".to_owned());
    // The environment says nothing about the address, so that one is the
    // console's; only the model is the server's here.
    let TestApp { router, pool } = spin_up_without_env_llm_url(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    lakehouse_store::assistant_settings::upsert(
        &pool,
        &lakehouse_store::assistant_settings::AssistantSettings {
            llm_url: Some(CONSOLE_PROVIDER.to_owned()),
            llm_model: Some("console-model".to_owned()),
            llm_key: None,
        },
        &admin.to_string(),
    )
    .await
    .unwrap();

    // A save that sends the model is refused, and nothing is written.
    let (status, got) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"model":"another-console-model"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{got}");
    assert_eq!(got["error"].as_str(), Some(SET_BY_THE_SERVER), "{got}");

    let row = lakehouse_store::assistant_settings::get(&pool)
        .await
        .unwrap()
        .expect("the console's own row");
    assert_eq!(row.llm_url.as_deref(), Some(CONSOLE_PROVIDER));
    assert_eq!(row.llm_model.as_deref(), Some("console-model"));

    // The answer names the environment's model, as the server's, and the
    // console's address stays the console's.
    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["model"],
        json!({"value": "env-model", "source": "server"})
    );
    assert_eq!(
        read["url"],
        json!({"value": CONSOLE_PROVIDER, "source": "console"})
    );
}

/// With no key, no address and no model in the environment, the console owns
/// all three: its saved values are used, and it may change them.
#[tokio::test]
async fn the_console_owns_the_provider_when_the_environment_says_nothing() {
    let TestApp { router, pool } = spin_up_without_env_llm_url(&env_with_internal_hosts()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":"https://localhost:8443/v1","model":"console-model","key":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["url"],
        json!({"value": CONSOLE_PROVIDER, "source": "console"})
    );
    assert_eq!(
        read["model"],
        json!({"value": "console-model", "source": "console"})
    );
    assert_eq!(read["key"], json!({"set": true, "source": "console"}));
}

/// Plan D13: the address must be `https://`, and it must not point inside
/// the deployment's network.
#[tokio::test]
async fn an_address_that_is_not_https_or_is_internal_is_refused() {
    let TestApp { router, pool } = spin_up_without_env_llm_url(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    for (body, sentence) in [
        (
            r#"{"url":"http://api.example.com/v1"}"#,
            "url must be an https:// address of the provider, such as https://api.example.com/v1.",
        ),
        (
            r#"{"url":"https://localhost:8443/v1"}"#,
            "url must not point inside this deployment's network.",
        ),
    ] {
        let (status, got) = call(&router, "PUT", ASSISTANT, &cookie, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {got}");
        assert_eq!(got["error"].as_str(), Some(sentence), "{body}: {got}");
    }
    // Nothing was written by the refused saves.
    assert!(
        lakehouse_store::assistant_settings::get(&pool)
            .await
            .unwrap()
            .is_none()
    );
}

/// Plan D6: one audit row per save, naming the fields that changed and never
/// the key.
#[tokio::test]
async fn a_save_writes_one_audit_row_naming_the_fields_and_never_the_key() {
    let TestApp { router, pool } = spin_up_without_env_llm_url(&env_with_internal_hosts()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":"https://localhost:8443/v1","model":"console-model","key":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    let rows: Vec<(String, Value)> = sqlx::query_as(
        "SELECT action, args FROM audit_event WHERE action = 'settings.assistant.update'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1, "one save, one audit row");
    let (action, args) = &rows[0];
    assert_eq!(action, "settings.assistant.update");
    let fields: Vec<&str> = args["fields"]
        .as_array()
        .expect("the row names the fields")
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(fields.contains(&"url"), "{fields:?}");
    assert!(fields.contains(&"model"), "{fields:?}");
    // The key's NAME may be there; its value never is.
    assert!(fields.contains(&"key"), "{fields:?}");
    assert!(!args.to_string().contains("not-a-real-secret"), "{args}");
}

/// Plan D13: the test answers one of the fixed sentences, never the
/// provider's own words. With nothing reachable, that is the sentence for a
/// provider that could not be reached.
#[tokio::test]
async fn the_test_answers_one_of_the_fixed_sentences() {
    let TestApp { router, pool } = spin_up_with_env(&HashMap::new()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(&router, "POST", ASSISTANT_TEST, &cookie, "").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["answered"], json!(false), "{body}");
    assert_eq!(
        body["message"],
        json!("Could not reach the provider."),
        "{body}"
    );
}

/// A field that is not sent keeps what is stored, and an explicit `null`
/// clears it (plan D5). The address is one exception: it cannot be cleared
/// while a key is set, because the key would go to the default provider.
#[tokio::test]
async fn a_field_that_is_not_sent_keeps_its_value_and_null_clears_it() {
    let TestApp { router, pool } = spin_up_without_env_llm_url(&env_with_internal_hosts()).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;
    let (status, body) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":"https://localhost:8443/v1","model":"console-model","key":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    // A save that does not send the model keeps it.
    let (status, again) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"key":"not-a-real-secret"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["model"],
        json!({"value": "console-model", "source": "console"})
    );

    // Clearing the address while the key is set is refused: the key would
    // go to the deployment's default provider.
    let (status, refused) = call(&router, "PUT", ASSISTANT, &cookie, r#"{"url":null}"#).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{refused}");
    assert_eq!(
        refused["error"].as_str(),
        Some(
            "The provider address cannot be cleared while a key is set: the key would go to the \
             default provider."
        ),
        "{refused}"
    );

    // Clearing the key and the address together is fine: the assistant is
    // off.
    let (status, cleared) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":null,"key":null}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cleared}");
    assert_eq!(cleared["key"], json!({"set": false, "source": "default"}));
}

/// `ADM-9 review R6`: a deployment whose environment is built the way
/// `docker-compose.yml` builds it — `LLM_URL`, `LLM_MODEL`, `SMTP_PORT`
/// and `SMTP_FROM` all present and EMPTY — may set the console's own
/// address and port.
///
/// Before the fix an empty value still read as a value, so the environment
/// owned all four and both fields were read-only on every compose
/// installation: a save that sent them was refused with a 409 and nothing
/// was written. The four are one deployment's environment, so this one
/// case asks both routes — the assistant's address (`ADM-9`) and the
/// email's port (`ADM-7`).
#[tokio::test]
async fn a_deployment_built_the_way_compose_builds_it_may_set_the_address_and_the_port() {
    let mut env = env_with_internal_hosts();
    // Exactly what compose passes when the operator set none.
    env.insert("LLM_URL".to_owned(), String::new());
    env.insert("LLM_MODEL".to_owned(), String::new());
    env.insert("SMTP_PORT".to_owned(), String::new());
    env.insert("SMTP_FROM".to_owned(), String::new());
    let TestApp { router, pool } = spin_up_with_env(&env).await;
    let admin = create_principal_with_permissions(&pool, "settings:write").await;
    let cookie = session_cookie_for_user(&pool, admin).await;

    // Before anything is saved, the four are the code's own defaults and
    // the console's to change: the environment says nothing about them.
    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["url"],
        json!({"value": lakehouse_api::config::DEFAULT_LLM_URL, "source": "default"})
    );
    assert_eq!(
        read["model"],
        json!({"value": lakehouse_api::config::DEFAULT_LLM_MODEL, "source": "default"})
    );
    let (status, read) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["port"], json!({"value": 587, "source": "default"}));
    assert_eq!(
        read["from"],
        json!({"value": "rantai-lake@localhost", "source": "default"})
    );

    // The address is the console's to set.
    let (status, body) = call(
        &router,
        "PUT",
        ASSISTANT,
        &cookie,
        r#"{"url":"https://localhost:8443/v1"}"#,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["url"],
        json!({"value": CONSOLE_PROVIDER, "source": "console"})
    );

    // And so is the port.
    let (status, body) = call(&router, "PUT", EMAIL, &cookie, r#"{"port":2525}"#).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["port"], json!({"value": 2525, "source": "console"}));

    // The reads say the same.
    let (status, read) = call(&router, "GET", ASSISTANT, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["url"],
        json!({"value": CONSOLE_PROVIDER, "source": "console"})
    );
    let (status, read) = call(&router, "GET", EMAIL, &cookie, "").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["port"], json!({"value": 2525, "source": "console"}));
}
